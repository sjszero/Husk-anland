//! `nr-tools rootfs-status` - write a dpkg status database that apt believes.
//!
//! A rootfs assembled from tarballs (rather than by dpkg itself) has no status
//! database, so apt thinks nothing is installed: it will happily "install"
//! libc6 over the runtime's own patched libc, and every `dpkg -l` is empty.
//! Staged images therefore need a status file, and it has to be *coherent*:
//! no duplicate stanzas (dpkg refuses: "multiple non-coinstallable package
//! instances present") and no stanza missing `Maintainer` (a warning per line,
//! thousands of them).
//!
//! Three sources, in decreasing authority:
//!   1. the base rootfs tarball's own `var/lib/dpkg/status`   - real metadata
//!   2. the status already in the tree                        - maybe real
//!   3. `<name>_<version>_<arch>.deb` manifests               - name/version only
//!
//! Fields the first two carry win outright; a manifest only ever fills a gap.
//! When two sources carry the same package the one with real metadata wins,
//! and among equals the higher version wins.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

fn die(msg: impl AsRef<str>) -> ! {
    eprintln!("{}", msg.as_ref());
    std::process::exit(1)
}

pub(crate) type Stanza = Vec<(String, String)>;

fn get<'a>(s: &'a Stanza, key: &str) -> Option<&'a str> {
    s.iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| v.as_str())
}

pub(crate) fn parse(text: &str) -> Vec<Stanza> {
    let mut out: Vec<Stanza> = Vec::new();
    let mut cur: Stanza = Vec::new();
    let mut last: Option<String> = None;
    for line in text.split('\n') {
        if line.trim().is_empty() {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            last = None;
            continue;
        }
        if line.starts_with(' ') || line.starts_with('\t') {
            // Continuation of the previous field (Description bodies).
            if let Some(k) = &last {
                if let Some(e) = cur.iter_mut().find(|(kk, _)| kk == k) {
                    e.1.push('\n');
                    e.1.push_str(line);
                }
            }
            continue;
        }
        if let Some(c) = line.find(':') {
            let k = line[..c].trim().to_string();
            let v = line[c + 1..].trim_start().to_string();
            last = Some(k.clone());
            cur.push((k, v));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn render(s: &Stanza) -> String {
    let mut out = String::new();
    for (k, v) in s {
        out.push_str(k);
        out.push_str(": ");
        out.push_str(v);
        out.push('\n');
    }
    out.push('\n');
    out
}

// ------------------------------------------------------------ dpkg versions
//
// Only used to break ties between two sources that both carry a package. It is
// the documented dpkg algorithm: epoch:upstream-revision, numeric runs compared
// as numbers, and '~' sorting before everything, including the empty string.

fn order(c: u8) -> i32 {
    if c.is_ascii_digit() {
        0
    } else if c.is_ascii_alphabetic() {
        c as i32
    } else if c == b'~' {
        -1
    } else if c == 0 {
        0
    } else {
        c as i32 + 256
    }
}

fn verrevcmp(a: &str, b: &str) -> i32 {
    let a = a.as_bytes();
    let b = b.as_bytes();
    let (mut i, mut j) = (0usize, 0usize);
    while i < a.len() || j < b.len() {
        let mut first_diff: i32 = 0;
        while (i < a.len() && !a[i].is_ascii_digit()) || (j < b.len() && !b[j].is_ascii_digit()) {
            let ac = order(if i < a.len() { a[i] } else { 0 });
            let bc = order(if j < b.len() { b[j] } else { 0 });
            if ac != bc {
                return ac - bc;
            }
            i += 1;
            j += 1;
        }
        while i < a.len() && a[i] == b'0' {
            i += 1;
        }
        while j < b.len() && b[j] == b'0' {
            j += 1;
        }
        while i < a.len() && a[i].is_ascii_digit() && j < b.len() && b[j].is_ascii_digit() {
            if first_diff == 0 {
                first_diff = a[i] as i32 - b[j] as i32;
            }
            i += 1;
            j += 1;
        }
        if i < a.len() && a[i].is_ascii_digit() {
            return 1;
        }
        if j < b.len() && b[j].is_ascii_digit() {
            return -1;
        }
        if first_diff != 0 {
            return first_diff;
        }
    }
    0
}

pub(crate) fn version_cmp(a: &str, b: &str) -> i32 {
    let split = |v: &str| -> (u64, String, String) {
        let (epoch, rest) = match v.find(':') {
            Some(i) => (v[..i].parse().unwrap_or(0), &v[i + 1..]),
            None => (0, v),
        };
        match rest.rfind('-') {
            Some(i) => (epoch, rest[..i].to_string(), rest[i + 1..].to_string()),
            None => (epoch, rest.to_string(), String::new()),
        }
    };
    let (ea, ua, ra) = split(a);
    let (eb, ub, rb) = split(b);
    if ea != eb {
        return if ea < eb { -1 } else { 1 };
    }
    let c = verrevcmp(&ua, &ub);
    if c != 0 {
        return c;
    }
    verrevcmp(&ra, &rb)
}

fn strip_epoch(v: &str) -> &str {
    match v.find(':') {
        Some(i) => &v[i + 1..],
        None => v,
    }
}

// ----------------------------------------------------------------- the merge

/// How much a stanza can be trusted. Real dpkg metadata carries Maintainer;
/// manifests cannot make one up.
fn score(s: &Stanza) -> u8 {
    if get(s, "Maintainer").is_some() {
        2
    } else if get(s, "Depends").is_some() || get(s, "Description").is_some() {
        1
    } else {
        0
    }
}

fn synth(name: &str, version: &str, arch: &str) -> Stanza {
    vec![
        ("Package".into(), name.into()),
        ("Status".into(), "install ok installed".into()),
        ("Priority".into(), "optional".into()),
        ("Section".into(), "misc".into()),
        ("Architecture".into(), arch.into()),
        ("Version".into(), version.into()),
        (
            "Maintainer".into(),
            "Ubuntu Developers <ubuntu-devel-discuss@lists.ubuntu.com>".into(),
        ),
        ("Description".into(), format!("{name} (staged into a husk rootfs)")),
    ]
}

/// `bash_5.2.21-2ubuntu4_arm64.deb` -> ("bash", "5.2.21-2ubuntu4", "arm64")
fn from_manifest_name(stem: &str) -> Option<(String, String, String)> {
    let stem = stem.trim().trim_end_matches(".deb");
    let mut it = stem.rsplitn(3, '_');
    let arch = it.next()?.to_string();
    let version = it.next()?.to_string();
    let name = it.next()?.to_string();
    if name.is_empty() || version.is_empty() || arch.is_empty() {
        return None;
    }
    Some((name, version, arch))
}

/// Read `var/lib/dpkg/status` out of a rootfs tarball without unpacking it.
fn status_from_tar(path: &Path) -> Option<String> {
    let gz = fs::read(path).ok()?;
    if !gz.starts_with(&[0x1f, 0x8b]) {
        return None;
    }
    let tar = crate::provision::gunzip(&gz);
    let mut p = 0usize;
    while p + 512 <= tar.len() {
        let h = &tar[p..p + 512];
        if h.iter().all(|&c| c == 0) {
            break;
        }
        let size = crate::provision::tar_octal(&h[124..136]) as usize;
        let kind = h[156];
        let mut name = String::from_utf8_lossy(&h[0..100]).trim_end_matches('\0').to_string();
        if let Some(i) = name.find('\0') {
            name.truncate(i);
        }
        let body = &tar[p + 512..(p + 512 + size).min(tar.len())];
        let name = name.trim_start_matches("./").to_string();
        if name == "var/lib/dpkg/status" && (kind == b'0' || kind == 0) {
            return Some(String::from_utf8_lossy(body).to_string());
        }
        p += 512 + size + ((512 - size % 512) % 512);
    }
    None
}

pub(crate) fn main(argv: &[String]) {
    let mut root = PathBuf::from("/tmp/husk-rootfs");
    let mut base: Option<PathBuf> = None;
    let mut lists: Vec<PathBuf> = Vec::new();
    let mut holds: Vec<String> = Vec::new();
    let mut suites: Vec<String> = Vec::new();
    let mut out: Option<PathBuf> = None;
    let mut i = 0;
    while i < argv.len() {
        let k = argv[i].as_str();
        let mut next = |i: &mut usize| -> String {
            *i += 1;
            argv.get(*i)
                .cloned()
                .unwrap_or_else(|| die(format!("{k} needs a value")))
        };
        match k {
            "--root" => root = PathBuf::from(next(&mut i)),
            "--base" => base = Some(PathBuf::from(next(&mut i))),
            "--list" => lists.push(PathBuf::from(next(&mut i))),
            "--hold" => holds.push(next(&mut i)),
            "--index-suite" => suites.push(next(&mut i)),
            "--out" => out = Some(PathBuf::from(next(&mut i))),
            other => die(format!(
                "unknown argument {other}\nusage: nr-tools rootfs-status [--root DIR] \
[--base TAR|STATUSFILE] [--list MANIFEST ...] [--hold PKG ...] \
[--index-suite S ...] [--out FILE]"
            )),
        }
        i += 1;
    }

    let mut map: BTreeMap<String, Stanza> = BTreeMap::new();
    let mut from: [usize; 3] = [0; 3];
    let mut collapsed = 0usize;

    let mut merge = |st: Stanza, rank: usize, map: &mut BTreeMap<String, Stanza>, from: &mut [usize; 3], collapsed: &mut usize| {
        let Some(name) = get(&st, "Package").map(|s| s.to_string()) else {
            return;
        };
        match map.get(&name) {
            None => {
                from[rank] += 1;
                map.insert(name, st);
            }
            Some(old) => {
                *collapsed += 1;
                let take = match (score(&st), score(old)) {
                    (a, b) if a != b => a > b,
                    _ => {
                        match (get(&st, "Version"), get(old, "Version")) {
                            (Some(a), Some(b)) => version_cmp(a, b) > 0,
                            _ => false,
                        }
                    }
                };
                if take {
                    from[rank] += 1;
                    map.insert(name, st);
                }
            }
        }
    };

    // 1. the tree as it stands
    let existing = root.join("var/lib/dpkg/status");
    if let Ok(text) = fs::read_to_string(&existing) {
        for st in parse(&text) {
            merge(st, 1, &mut map, &mut from, &mut collapsed);
        }
    }
    // 2. the base image's real database (imported *before* the tree wins ties,
    //    because it alone carries Depends/Maintainer for the base system)
    if let Some(b) = &base {
        let text = if b.extension().map(|e| e == "tar").unwrap_or(false) {
            status_from_tar(b)
        } else {
            fs::read_to_string(b).ok()
        };
        match text {
            Some(t) => {
                for st in parse(&t) {
                    merge(st, 0, &mut map, &mut from, &mut collapsed);
                }
            }
            None => die(format!("no dpkg status in {}", b.display())),
        }
    }
    // 3. manifests: names only, gaps only
    for l in &lists {
        let text = fs::read_to_string(l).unwrap_or_else(|e| die(format!("{}: {e}", l.display())));
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((name, version, arch)) = from_manifest_name(line) {
                if !map.contains_key(&name) {
                    from[2] += 1;
                    map.insert(name.clone(), synth(&name, &version, &arch));
                }
            }
        }
    }

    // ----------------------------------------------------------------- index ---
    //
    // The tarball manifests lose the epoch: `x11-common_1%3a7.7+23ubuntu3` is
    // recorded as `7.7+23ubuntu3`, which sorts *below* `1:7.7+23~` and makes apt
    // see a Breaks violation that does not exist ("debianutils : Breaks:
    // x11-common (< 1:7.7+23~) but 7.7+23ubuntu3 is to be installed"). The fix is
    // to take the version - and the architecture - from the archive index, but
    // only where the index version is the same release as what is on disk.
    if !suites.is_empty() {
        let mut idx: BTreeMap<String, (String, String)> = BTreeMap::new();
        for suite in &suites {
            for comp in ["main", "universe"] {
                let path = format!("/ubuntu-ports/dists/{suite}/{comp}/binary-arm64/Packages.gz");
                let Some(gz) = crate::provision::http_get(&path) else {
                    die(format!("index fetch failed: {path}"));
                };
                let text = String::from_utf8_lossy(&crate::provision::gunzip(&gz)).to_string();
                let mut name = String::new();
                let mut ver = String::new();
                let mut arch = String::new();
                for line in text.lines() {
                    if line.is_empty() {
                        if !name.is_empty() && !ver.is_empty() {
                            let e = idx.entry(name.clone()).or_insert((String::new(), String::new()));
                            // later suites win: noble-updates carries the newer build
                            e.0 = ver.clone();
                            e.1 = arch.clone();
                        }
                        name.clear();
                        ver.clear();
                        arch.clear();
                        continue;
                    }
                    if let Some(v) = line.strip_prefix("Package: ") {
                        name = v.trim().to_string();
                    } else if let Some(v) = line.strip_prefix("Version: ") {
                        ver = v.trim().to_string();
                    } else if let Some(v) = line.strip_prefix("Architecture: ") {
                        arch = v.trim().to_string();
                    }
                }
                if !name.is_empty() && !ver.is_empty() {
                    let e = idx.entry(name.clone()).or_insert((String::new(), String::new()));
                    e.0 = ver;
                    e.1 = arch;
                }
            }
        }
        let mut epochs = 0usize;
        let mut archs = 0usize;
        for (name, st) in map.iter_mut() {
            let Some((iver, iarch)) = idx.get(name) else { continue };
            let synthetic = get(st, "Description")
                .map(|d| d.contains("(staged "))
                .unwrap_or(false);
            if let Some(cur) = get(st, "Version").map(|s| s.to_string()) {
                if strip_epoch(iver) == strip_epoch(&cur) && iver != &cur {
                    if let Some(e) = st.iter_mut().find(|(k, _)| k == "Version") {
                        e.1 = iver.clone();
                        epochs += 1;
                    }
                }
            }
            if synthetic && !iarch.is_empty() {
                if let Some(e) = st.iter_mut().find(|(k, _)| k == "Architecture") {
                    if &e.1 != iarch {
                        e.1 = iarch.clone();
                        archs += 1;
                    }
                }
            }
        }
        println!("index: reconciled {epochs} version(s) (epoch) and {archs} architecture(s)");
    }

    // Fields every stanza dpkg will read must have.
    let mut repaired = 0usize;
    for st in map.values_mut() {
        let mut fixed = false;
        if get(st, "Status").is_none() {
            st.insert(0, ("Status".into(), "install ok installed".into()));
            fixed = true;
        }
        if get(st, "Maintainer").is_none() {
            st.push((
                "Maintainer".into(),
                "Ubuntu Developers <ubuntu-devel-discuss@lists.ubuntu.com>".into(),
            ));
            fixed = true;
        }
        if holds.iter().any(|h| get(st, "Package") == Some(h.as_str())) {
            // dpkg keeps the selection in the first word of Status:, and that is
            // what apt reads for `apt-mark hold`. Holding the packages the
            // runtime patched is what keeps an apt upgrade from overwriting them.
            if let Some(e) = st.iter_mut().find(|(k, _)| k == "Status") {
                let tail: String = e
                    .1
                    .splitn(3, ' ')
                    .skip(1)
                    .collect::<Vec<_>>()
                    .join(" ");
                e.1 = format!("hold {tail}");
                fixed = true;
            }
        }
        if fixed {
            repaired += 1;
        }
    }

    let mut text = String::new();
    for st in map.values() {
        text.push_str(&render(st));
    }
    let dst = out.unwrap_or_else(|| root.join("var/lib/dpkg/status"));
    if let Some(d) = dst.parent() {
        let _ = fs::create_dir_all(d);
    }
    fs::write(&dst, text.as_bytes()).unwrap_or_else(|e| die(format!("{}: {e}", dst.display())));

    let no_maint = map.values().filter(|s| get(s, "Maintainer").is_none()).count();
    println!(
        "dpkg status: {} packages -> {}\n  from base database: {}, from tree: {}, from manifests: {}\n  duplicates collapsed: {}, fields repaired: {}, missing Maintainer: {}",
        map.len(),
        dst.display(),
        from[0],
        from[1],
        from[2],
        collapsed,
        repaired,
        no_maint
    );
    if holds.is_empty() {
        println!("  holds: none");
    } else {
        println!("  holds: {}", holds.join(" "));
    }
}
