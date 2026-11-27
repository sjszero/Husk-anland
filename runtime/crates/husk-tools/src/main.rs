//! Host-side repository tooling. Everything here used to be a Python script
//! (`tools/ci/check-links.py`, `tools/bench/summarize.py`,
//! `tools/bench/compare.py`); the repository is Rust-only, so the tools are too.
//!
//!   husk-tools check-links                  fail on a broken relative link/anchor
//!   husk-tools bench-summarize <glob|log>.. per-file median / min / max table
//!   husk-tools bench-compare <h> <p> <d>    the eight-dimension comparison table
//!   husk-tools screenshot-audit <raw>       reject black/large-missing raw screenshots
//!   husk-tools strace-path-candidates <dir> <rootfs>
//!                                             extract unresolved absolute guest paths
//!   husk-tools now-ms                       Unix epoch milliseconds
//!
//! No external crates: these run on the host (macOS or Linux) with nothing but
//! `git` on PATH.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("check-links") => check_links::run(),
        Some("bench-summarize") => bench::summarize(&args[1..]),
        Some("bench-compare") => bench::compare(&args[1..]),
        Some("screenshot-audit") => screenshot_audit::run(&args[1..]),
        Some("strace-path-candidates") => strace_paths::run(&args[1..]),
        Some("now-ms") => now_ms(),
        _ => {
            eprintln!(
                "usage: husk-tools <check-links | bench-summarize <glob>.. | bench-compare <husk.log> <proot.log> <proot-distro.log> | screenshot-audit <raw> | strace-path-candidates <out-dir> <rootfs> | now-ms>"
            );
            2
        }
    };
    std::process::exit(code);
}

fn now_ms() -> i32 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(elapsed) => {
            println!("{}", elapsed.as_millis());
            0
        }
        Err(err) => {
            eprintln!("now-ms: system clock is before Unix epoch: {err}");
            1
        }
    }
}

// ---------------------------------------------------------------- link gate

mod check_links {
    use super::*;

    /// Fenced code is quoted output: a "link" inside it is not a link.
    fn strip_fences(body: &str) -> String {
        let mut out = String::with_capacity(body.len());
        let mut in_fence = false;
        for line in body.lines() {
            if line.starts_with("```") {
                in_fence = !in_fence;
                continue;
            }
            if !in_fence {
                out.push_str(line);
                out.push('\n');
            }
        }
        out
    }

    /// GitHub's heading anchor: lowercase, drop punctuation, spaces to dashes.
    fn slug(text: &str) -> String {
        let mut s = String::new();
        for ch in text.trim().to_lowercase().chars() {
            match ch {
                '`' | '*' | '_' | '[' | ']' | '(' | ')' | '.' | ',' | ':' | '"' | '\'' | '!'
                | '?' | '/' => {}
                c if c.is_whitespace() => s.push('-'),
                c => s.push(c),
            }
        }
        s
    }

    fn anchors(path: &Path) -> Vec<String> {
        let body = match fs::read_to_string(path) {
            Ok(b) => b,
            Err(_) => return Vec::new(),
        };
        body.lines()
            .filter_map(|line| {
                let hashes = line.chars().take_while(|c| *c == '#').count();
                if !(1..=6).contains(&hashes) {
                    return None;
                }
                let rest = &line[hashes..];
                if !rest.starts_with(' ') && !rest.starts_with('\t') {
                    return None;
                }
                Some(slug(rest))
            })
            .collect()
    }

    /// Every `[text](target)` / `![alt](target)` in the body, fenced code removed.
    fn targets(body: &str) -> Vec<String> {
        let bytes: Vec<char> = body.chars().collect();
        let mut found = Vec::new();
        let mut i = 0;
        while i + 1 < bytes.len() {
            if bytes[i] == ']' && bytes[i + 1] == '(' {
                // `\[[^\]]*\]\(` — the bracket pair may not contain a `]`.
                let mut open = None;
                let mut j = i as isize - 1;
                while j >= 0 {
                    let c = bytes[j as usize];
                    if c == '[' {
                        open = Some(j as usize);
                        break;
                    }
                    if c == ']' {
                        break;
                    }
                    j -= 1;
                }
                if open.is_some() {
                    let mut k = i + 2;
                    let start = k;
                    while k < bytes.len() && !bytes[k].is_whitespace() && bytes[k] != ')' {
                        k += 1;
                    }
                    if k > start {
                        let target: String = bytes[start..k].iter().collect();
                        // optional `"title"`, then the closing paren
                        let mut m = k;
                        while m < bytes.len() && bytes[m].is_whitespace() {
                            m += 1;
                        }
                        if m < bytes.len() && bytes[m] == '"' {
                            m += 1;
                            while m < bytes.len() && bytes[m] != '"' {
                                m += 1;
                            }
                            m += 1;
                            while m < bytes.len() && bytes[m].is_whitespace() {
                                m += 1;
                            }
                        }
                        if m < bytes.len() && bytes[m] == ')' {
                            found.push(target);
                            i = m;
                        }
                    }
                }
            }
            i += 1;
        }
        found
    }

    fn is_external(target: &str) -> bool {
        if let Some(colon) = target.find(':') {
            let scheme = &target[..colon];
            let mut chars = scheme.chars();
            if let Some(first) = chars.next() {
                if first.is_ascii_lowercase()
                    && chars
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "+.-".contains(c))
                {
                    return true;
                }
            }
        }
        target.starts_with('#')
    }

    /// Lexical `normpath`: resolve `.` / `..` without touching the filesystem.
    fn normalize(path: &str) -> String {
        let mut parts: Vec<&str> = Vec::new();
        for part in path.split('/') {
            match part {
                "" | "." => {}
                ".." => {
                    parts.pop();
                }
                p => parts.push(p),
            }
        }
        parts.join("/")
    }

    pub fn run() -> i32 {
        let listed = match Command::new("git").args(["ls-files", "*.md"]).output() {
            Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
            _ => {
                eprintln!("check-links: git ls-files failed (run from the repository root)");
                return 1;
            }
        };
        let files: Vec<&str> = listed.split_whitespace().collect();
        let mut bad = 0usize;
        for path in &files {
            let body = match fs::read_to_string(path) {
                Ok(b) => b,
                Err(_) => continue,
            };
            let body = strip_fences(&body);
            for target in targets(&body) {
                if is_external(&target) {
                    continue;
                }
                let (file_part, anchor) = match target.split_once('#') {
                    Some((f, a)) => (f, Some(a)),
                    None => (target.as_str(), None),
                };
                let resolved = if file_part.is_empty() {
                    (*path).to_string()
                } else {
                    let dir = Path::new(path).parent().unwrap_or(Path::new(""));
                    normalize(&format!("{}/{}", dir.to_string_lossy(), file_part))
                };
                let resolved_path = PathBuf::from(&resolved);
                if !resolved_path.exists() {
                    println!("{path}: missing target {target}");
                    bad += 1;
                    continue;
                }
                if let Some(anchor) = anchor {
                    if anchor.is_empty() {
                        continue;
                    }
                    if resolved.ends_with(".md") {
                        if !anchors(&resolved_path).contains(&anchor.to_lowercase()) {
                            println!("{path}: missing anchor {target}");
                            bad += 1;
                        }
                    }
                }
            }
        }
        println!(
            "checked {} markdown files, {} broken link(s)",
            files.len(),
            bad
        );
        if bad == 0 {
            0
        } else {
            1
        }
    }
}

// ------------------------------------------------------------------- benches

mod bench {
    use super::*;

    /// `name<TAB>value` lines, honouring the `#rep N` repetition markers that
    /// `guest/measure.sh` writes.
    fn read_blocks(path: &Path) -> Vec<Vec<(String, f64)>> {
        let body = match fs::read_to_string(path) {
            Ok(b) => b,
            Err(_) => return Vec::new(),
        };
        let mut blocks: Vec<Vec<(String, f64)>> = Vec::new();
        let mut section = "";
        for line in body.lines() {
            if line.starts_with('#') {
                if line.starts_with("## ") {
                    section = line[3..].split_whitespace().next().unwrap_or("");
                    blocks.push(Vec::new());
                } else if line.starts_with("#rep") {
                    blocks.push(Vec::new());
                }
                continue;
            }
            if section == "cold_start_ms" {
                if let Some(value) = line.split_whitespace().next() {
                    if let Ok(value) = value.parse::<f64>() {
                        if let Some(block) = blocks.last_mut() {
                            block.push(("cold_start_ms".to_string(), value));
                        }
                    }
                }
                continue;
            }
            let Some((name, value)) = line.split_once('\t') else {
                continue;
            };
            let Ok(v) = value.trim().parse::<f64>() else {
                continue;
            };
            match blocks.last_mut() {
                Some(block) => {
                    if let Some(slot) = block.iter_mut().find(|(n, _)| n == name) {
                        slot.1 = v;
                    } else {
                        block.push((name.to_string(), v));
                    }
                }
                None => continue,
            }
        }
        blocks
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn reads_plain_cold_start_and_tab_separated_samples() {
            let path = env::temp_dir().join(format!(
                "husk-bench-summary-{}-{}.log",
                std::process::id(),
                std::thread::current().name().unwrap_or("main")
            ));
            fs::write(
                &path,
                "## cold_start_ms\n100\n90\n## measure\n#rep 1\nstat\t12\n#rep 2\nstat\t10\n",
            )
            .unwrap();
            let blocks = read_blocks(&path);
            assert_eq!(
                blocks[0],
                vec![
                    ("cold_start_ms".to_string(), 100.0),
                    ("cold_start_ms".to_string(), 90.0)
                ]
            );
            assert_eq!(blocks[2], vec![("stat".to_string(), 12.0)]);
            assert_eq!(blocks[3], vec![("stat".to_string(), 10.0)]);
            fs::remove_file(path).unwrap();
        }
    }

    fn median(values: &[f64]) -> f64 {
        let mut v = values.to_vec();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let n = v.len();
        if n == 0 {
            return 0.0;
        }
        if n % 2 == 1 {
            v[n / 2]
        } else {
            (v[n / 2 - 1] + v[n / 2]) / 2.0
        }
    }

    fn expand(pattern: &str) -> Vec<String> {
        if !pattern.contains('*') && !pattern.contains('?') {
            return vec![pattern.to_string()];
        }
        let (dir, name) = match pattern.rfind('/') {
            Some(i) => (&pattern[..i], &pattern[i + 1..]),
            None => (".", pattern),
        };
        let mut out: Vec<String> = match fs::read_dir(dir) {
            Ok(entries) => entries
                .filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().to_string())
                .filter(|n| glob_match(name, n))
                .map(|n| format!("{dir}/{n}"))
                .collect(),
            Err(_) => Vec::new(),
        };
        out.sort();
        out
    }

    fn glob_match(pattern: &str, name: &str) -> bool {
        let p: Vec<char> = pattern.chars().collect();
        let n: Vec<char> = name.chars().collect();
        fn go(p: &[char], n: &[char]) -> bool {
            match p.first() {
                None => n.is_empty(),
                Some('*') => (0..=n.len()).any(|i| go(&p[1..], &n[i..])),
                Some('?') => !n.is_empty() && go(&p[1..], &n[1..]),
                Some(c) => !n.is_empty() && n[0] == *c && go(&p[1..], &n[1..]),
            }
        }
        go(&p, &n)
    }

    /// `{:,.0f}` — thousands separators, no decimals.
    fn thousands(value: f64) -> String {
        let raw = format!("{:.0}", value);
        let (sign, digits) = match raw.strip_prefix('-') {
            Some(d) => ("-", d.to_string()),
            None => ("", raw),
        };
        let mut out = String::new();
        for (i, c) in digits.chars().enumerate() {
            if i > 0 && (digits.len() - i) % 3 == 0 {
                out.push(',');
            }
            out.push(c);
        }
        format!("{sign}{out}")
    }

    pub fn summarize(args: &[String]) -> i32 {
        if args.is_empty() {
            eprintln!("usage: husk-tools bench-summarize <raw-bench-log|glob> ..");
            return 2;
        }
        let mut paths: Vec<String> = Vec::new();
        for a in args {
            paths.extend(expand(a));
        }
        for path in &paths {
            let blocks = read_blocks(Path::new(path));
            let mut names: Vec<String> = Vec::new();
            let mut per: std::collections::HashMap<String, Vec<f64>> =
                std::collections::HashMap::new();
            for block in &blocks {
                for (name, value) in block {
                    if !per.contains_key(name) {
                        names.push(name.clone());
                    }
                    per.entry(name.clone()).or_default().push(*value);
                }
            }
            names.sort();
            let label = path.rsplit('/').next().unwrap_or(path.as_str());
            println!("\n### {label}\n");
            println!(
                "{:24}{:>10}{:>10}{:>10}{:>8}",
                "item", "median", "min", "max", "n"
            );
            for name in names {
                let values = &per[&name];
                let ok: Vec<f64> = values.iter().copied().filter(|v| *v >= 0.0).collect();
                if ok.is_empty() {
                    println!(
                        "{:24}{:>10}{:>10}{:>10}{:>8}",
                        name,
                        "NA",
                        "",
                        "",
                        format!("0/{}", values.len())
                    );
                    continue;
                }
                let lo = ok.iter().cloned().fold(f64::INFINITY, f64::min);
                let hi = ok.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
                println!(
                    "{:24}{:>10}{:>10}{:>10}{:>8}",
                    name,
                    format!("{:.1}", median(&ok)),
                    format!("{:.0}", lo),
                    format!("{:.0}", hi),
                    format!("{}/{}", ok.len(), values.len())
                );
            }
        }
        0
    }

    const DIMENSIONS: &[(&str, &[&str])] = &[
        (
            "1 · 启动",
            &["cold_start_ms", "apt_update_cold", "apt_update_hot"],
        ),
        (
            "2 · 进程与 syscall 密度",
            &[
                "fork_exec_100",
                "fork_50_true",
                "sh_loop_1000",
                "getpid_20k",
                "stat_20k",
                "open_close_20k",
                "make_j4",
            ],
        ),
        (
            "3 · 文件 I/O",
            &[
                "find_usr",
                "tar_doc",
                "grep_include",
                "write_256m",
                "read_256m",
                "small_files",
                "git_clone",
            ],
        ),
        (
            "4 · 包管理与开发工作流",
            &[
                "dpkg_l",
                "apt_simulate",
                "gcc_hello",
                "python_json",
                "python_numpy",
            ],
        ),
    ];

    fn label(key: &str) -> String {
        match key {
            "cold_start_ms" => "冷启动到 `/bin/true`（含 adb）".into(),
            "apt_update_cold" => "`apt-get update` 冷（删索引）".into(),
            "apt_update_hot" => "`apt-get update` 热".into(),
            "fork_exec_100" => "fork+exec × 100".into(),
            "fork_50_true" => "`/bin/true` × 50".into(),
            "sh_loop_1000" => "shell 循环 1000 次".into(),
            "getpid_20k" => "`getpid` × 20 000".into(),
            "stat_20k" => "`stat` × 20 000".into(),
            "open_close_20k" => "`open`+`close` × 20 000".into(),
            "make_j4" => "`make -j4`（200 目标链接）".into(),
            "find_usr" => "`find /usr`".into(),
            "tar_doc" => "源码树 tar 解包".into(),
            "grep_include" => "`grep -r` 头文件树".into(),
            "write_256m" => "顺序写 256 MB".into(),
            "read_256m" => "顺序读 256 MB（页缓存）".into(),
            "small_files" => "1 万小文件建删".into(),
            "git_clone" => "本地 `git clone` + `status`".into(),
            "dpkg_l" => "`dpkg -l`".into(),
            "apt_simulate" => "`apt-get -s install` 解析".into(),
            "gcc_hello" => "`gcc` 编译 hello".into(),
            "python_json" => "`python3` import json".into(),
            "python_numpy" => "`python3` import numpy".into(),
            other => other.to_string(),
        }
    }

    fn unit(key: &str) -> &'static str {
        if key.starts_with("have_") {
            return "";
        }
        "ms"
    }

    /// Note: the reference implementation collected every sample in a file
    /// regardless of `#rep` boundaries; that behaviour is preserved here.
    fn medians(path: &str) -> std::collections::HashMap<String, (f64, f64, f64, usize)> {
        let body = match fs::read_to_string(path) {
            Ok(b) => b,
            Err(_) => return std::collections::HashMap::new(),
        };
        let mut per: std::collections::HashMap<String, Vec<f64>> = std::collections::HashMap::new();
        for line in body.lines() {
            if line.starts_with('#') || !line.contains('\t') {
                continue;
            }
            let (name, value) = line.split_once('\t').unwrap_or((line, ""));
            if let Ok(v) = value.trim().parse::<f64>() {
                per.entry(name.to_string()).or_default().push(v);
            }
        }
        per.into_iter()
            .filter_map(|(k, v)| {
                if v.is_empty() {
                    return None;
                }
                let med = median(&v);
                if med < 0.0 {
                    return None;
                }
                let lo = v.iter().cloned().fold(f64::INFINITY, f64::min);
                let hi = v.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
                Some((k, (med, lo, hi, v.len())))
            })
            .collect()
    }

    fn cell(
        m: &std::collections::HashMap<String, (f64, f64, f64, usize)>,
        key: &str,
    ) -> (String, Option<f64>) {
        match m.get(key) {
            None => ("n/a".to_string(), None),
            Some((med, _, _, _)) => {
                let unit = unit(key);
                let text = if unit.is_empty() {
                    thousands(*med)
                } else {
                    format!("{} {}", thousands(*med), unit)
                };
                (text, Some(*med))
            }
        }
    }

    pub fn compare(args: &[String]) -> i32 {
        if args.len() != 3 {
            eprintln!("usage: husk-tools bench-compare <bench-husk.log> <bench-proot.log> <bench-proot-distro.log>");
            return 2;
        }
        let husk = medians(&args[0]);
        let proot = medians(&args[1]);
        let distro = medians(&args[2]);
        println!("| 维度 / 项 | Husk | proot | proot-distro | Husk 相对 proot |");
        println!("|---|---:|---:|---:|---:|");
        for (dim, keys) in DIMENSIONS {
            println!("| **{dim}** | | | | |");
            for key in keys.iter() {
                let (husk_text, husk_value) = cell(&husk, key);
                let (proot_text, proot_value) = cell(&proot, key);
                let (distro_text, _) = cell(&distro, key);
                let ratio = match (husk_value, proot_value) {
                    (Some(h), Some(p)) if h != 0.0 && p != 0.0 => format!("{:.1}×", p / h),
                    _ => "—".to_string(),
                };
                println!(
                    "| {} | {husk_text} | {proot_text} | {distro_text} | {ratio} |",
                    label(key)
                );
            }
        }
        println!("| **8 · 兼容性** | | | | |");
        let mut keys: Vec<&String> = husk
            .keys()
            .chain(proot.keys())
            .chain(distro.keys())
            .filter(|k| k.starts_with("have_"))
            .collect();
        keys.sort();
        keys.dedup();
        for key in keys {
            let mark =
                |m: &std::collections::HashMap<String, (f64, f64, f64, usize)>| match m.get(key) {
                    Some((med, _, _, _)) if *med == 1.0 => "可用",
                    _ => "无",
                };
            println!(
                "| `{}` | {} | {} | {} | — |",
                &key[5..],
                mark(&husk),
                mark(&proot),
                mark(&distro)
            );
        }
        0
    }
}

// ------------------------------------------------------- coverage-sweep paths

mod strace_paths {
    use super::*;
    use std::collections::BTreeSet;

    fn paths_in_line(line: &str, rootfs: &str) -> Vec<String> {
        // `execve` prints the argv array after `[`. Arguments are not syscall
        // path operands and used to create false positives for interpreter paths.
        let syscall = line.split_once('[').map_or(line, |(head, _)| head);
        syscall
            .split('"')
            .enumerate()
            .filter_map(|(index, value)| {
                (index % 2 == 1 && value.starts_with('/') && !value.starts_with(rootfs))
                    .then(|| value.to_string())
            })
            .collect()
    }

    fn collect(out_dir: &Path, rootfs: &str) -> Result<Vec<String>, String> {
        let mut logs = fs::read_dir(out_dir)
            .map_err(|err| format!("read {}: {err}", out_dir.display()))?
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .file_type()
                    .map(|kind| kind.is_file())
                    .unwrap_or(false)
            })
            .map(|entry| entry.path())
            .filter(|path| {
                let name = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("");
                name.starts_with("cov-") && name.ends_with(".log")
            })
            .collect::<Vec<_>>();
        logs.sort();

        let mut seen = BTreeSet::new();
        for path in logs {
            let bytes = fs::read(&path).map_err(|err| format!("read {}: {err}", path.display()))?;
            let body = String::from_utf8_lossy(&bytes);
            for line in body.lines() {
                seen.extend(paths_in_line(line, rootfs));
            }
        }
        Ok(seen.into_iter().collect())
    }

    pub fn run(args: &[String]) -> i32 {
        if args.len() != 2 {
            eprintln!("usage: husk-tools strace-path-candidates <out-dir> <rootfs>");
            return 2;
        }
        match collect(Path::new(&args[0]), &args[1]) {
            Ok(paths) => {
                for path in paths {
                    println!("{path}");
                }
                0
            }
            Err(err) => {
                eprintln!("strace-path-candidates: {err}");
                1
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn extracts_quoted_paths_and_ignores_exec_argv() {
            let line = r#"openat(AT_FDCWD, "/etc/passwd", O_RDONLY) = 3"#;
            assert_eq!(paths_in_line(line, "/rootfs"), vec!["/etc/passwd"]);

            let line = r#"execve("/usr/bin/ls", ["/usr/bin/ls", "/etc/passwd"], 0x0) = 0"#;
            assert_eq!(paths_in_line(line, "/rootfs"), vec!["/usr/bin/ls"]);
        }

        #[test]
        fn filters_translated_paths_and_requires_absolute_operands() {
            let line = r#"openat(AT_FDCWD, "/rootfs/etc/passwd", O_RDONLY) = 3"#;
            assert!(paths_in_line(line, "/rootfs").is_empty());
            let line = r#"openat(AT_FDCWD, "relative/path", O_RDONLY) = 3"#;
            assert!(paths_in_line(line, "/rootfs").is_empty());
        }

        #[test]
        fn collect_sorts_and_deduplicates_logs() {
            let dir = env::temp_dir().join(format!("husk-strace-paths-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            fs::write(
                dir.join("cov-2.log"),
                "openat(AT_FDCWD, \"/etc/passwd\", O_RDONLY) = 3\n",
            )
            .unwrap();
            fs::write(
                dir.join("cov-1.log"),
                "stat(\"/etc/passwd\") = 0\nstat(\"/usr/lib\") = 0\n",
            )
            .unwrap();
            fs::write(dir.join("ignored.log"), "stat(\"/ignored\") = 0\n").unwrap();

            let paths = collect(&dir, "/rootfs").unwrap();
            assert_eq!(paths, vec!["/etc/passwd", "/usr/lib"]);
            fs::remove_dir_all(dir).unwrap();
        }
    }
}

mod screenshot_audit;
