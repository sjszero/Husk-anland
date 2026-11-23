//! `nr-tools provision` - the Rust port of tools/device-tests/provision-toolchain.sh.
//!
//! Resolves a package set from ports.ubuntu.com for the suite the rootfs came from,
//! downloads and unpacks the .debs on the host, fixes up the generic program names
//! Ubuntu splits into aarch64-linux-gnu-* binaries, and pushes the result into the
//! app's private rootfs as a tar stream through `run-as`.
//!
//! No root, no chroot, no namespaces - same constraint as everything else here.
//!
//! Transport is a plain HTTP/1.0 GET (ports.ubuntu.com answers on port 80 with no
//! redirect, measured). Decompression shells out to gzip/xz/zstd and `tar`, which
//! every macOS and Linux install has; that keeps this crate dependency-free.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const HOST: &str = "ports.ubuntu.com";
const POOL: &str = "/ubuntu-ports/";
const COMPS: [&str; 2] = ["main", "universe"];

#[derive(Clone, Debug)]
pub(crate) struct Pkg {
    pub(crate) name: String,
    pub(crate) version: String,
    pub(crate) filename: String,
    pub(crate) size: u64,
    pub(crate) depends: String,
}

fn die(msg: impl AsRef<str>) -> ! {
    eprintln!("{}", msg.as_ref());
    std::process::exit(1)
}

// ------------------------------------------------------------------ transport

pub(crate) fn http_get(path: &str) -> Option<Vec<u8>> {
    let mut stream = TcpStream::connect((HOST, 80)).ok()?;
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(180)))
        .ok()?;
    let req = format!(
        "GET {path} HTTP/1.0\r\nHost: {HOST}\r\nUser-Agent: nr-tools\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(req.as_bytes()).ok()?;
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).ok()?;
    let hdr_end = raw.windows(4).position(|w| w == b"\r\n\r\n")? + 4;
    let head = String::from_utf8_lossy(&raw[..hdr_end]).to_string();
    let status: u16 = head.split_whitespace().nth(1)?.parse().ok()?;
    if status == 301 || status == 302 || status == 307 || status == 308 {
        let loc = head
            .lines()
            .find_map(|l| {
                l.strip_prefix("Location: ")
                    .or_else(|| l.strip_prefix("location: "))
            })
            .map(|s| s.trim().to_string())?;
        if let Some(rest) = loc.strip_prefix("http://ports.ubuntu.com") {
            return http_get(rest);
        }
        eprintln!("cannot follow {loc} (TLS is not supported by this tool)");
        return None;
    }
    if status != 200 {
        return None;
    }
    Some(raw[hdr_end..].to_vec())
}

/// Run a decompressor: stdin = input, stdout = output.
fn pipe(cmd: &mut Command, input: &[u8]) -> Vec<u8> {
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| die(format!("spawn {cmd:?}: {e}")));
    let mut stdin = child.stdin.take().expect("stdin");
    let inp = input.to_vec();
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(&inp);
    });
    let mut out = Vec::new();
    let mut stdout = child.stdout.take().expect("stdout");
    stdout
        .read_to_end(&mut out)
        .unwrap_or_else(|e| die(format!("read: {e}")));
    writer.join().ok();
    let status = child.wait().unwrap_or_else(|e| die(format!("wait: {e}")));
    if !status.success() {
        let mut err = String::new();
        if let Some(mut e) = child.stderr.take() {
            let _ = e.read_to_string(&mut err);
        }
        die(format!("decompressor failed: {err}"));
    }
    out
}

pub(crate) fn gunzip(d: &[u8]) -> Vec<u8> {
    pipe(Command::new("gzip").arg("-dc"), d)
}

// -------------------------------------------------------------- index + deps

pub(crate) fn parse_index(text: &str, merge: &mut BTreeMap<String, Pkg>) {
    for block in text.split("\n\n") {
        let mut name = None;
        let mut version = String::new();
        let mut filename = String::new();
        let mut size = 0u64;
        let mut depends = String::new();
        for line in block.lines() {
            if line.starts_with(' ') || line.starts_with('\t') {
                continue;
            }
            let Some((k, v)) = line.split_once(':') else {
                continue;
            };
            match k.trim() {
                "Package" => name = Some(v.trim().to_string()),
                "Version" => version = v.trim().to_string(),
                "Filename" => filename = v.trim().to_string(),
                "Size" => size = v.trim().parse().unwrap_or(0),
                "Depends" => depends = v.trim().to_string(),
                _ => {}
            }
        }
        let Some(name) = name else { continue };
        // a later suite wins when its version string is higher (same rule as the shell)
        let replace = match merge.get(&name) {
            Some(old) => version > old.version,
            None => true,
        };
        if replace {
            merge.insert(
                name.clone(),
                Pkg {
                    name,
                    version,
                    filename,
                    size,
                    depends,
                },
            );
        }
    }
}

fn dep_names(depends: &str) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    for part in depends.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let alts: Vec<String> = part
            .split('|')
            .map(|a| {
                let a = a.trim();
                let base = a.split('(').next().unwrap_or(a).trim();
                base.split(':').next().unwrap_or(base).trim().to_string()
            })
            .collect();
        out.push(alts);
    }
    out
}

pub(crate) fn resolve(
    index: &BTreeMap<String, Pkg>,
    wanted: &[String],
    skip: &HashSet<String>,
) -> Vec<Pkg> {
    let mut queue: Vec<String> = wanted.to_vec();
    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<Pkg> = Vec::new();
    while let Some(n) = queue.pop() {
        if seen.contains(&n) || skip.contains(&n) {
            continue;
        }
        let Some(pkg) = index.get(&n) else {
            println!("MISSING {n}");
            continue;
        };
        seen.insert(n.clone());
        out.push(pkg.clone());
        for alts in dep_names(&pkg.depends) {
            if alts.iter().any(|a| seen.contains(a) || skip.contains(a)) {
                continue;
            }
            if let Some(first) = alts.first() {
                queue.push(first.clone());
            }
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

// --------------------------------------------------------------- .deb unpack

fn ar_members(d: &[u8]) -> HashMap<String, Vec<u8>> {
    if !d.starts_with(b"!<arch>\n") {
        die("not an ar archive");
    }
    let mut p = 8;
    let mut out = HashMap::new();
    while p + 60 <= d.len() {
        let hdr = &d[p..p + 60];
        let name = String::from_utf8_lossy(&hdr[..16])
            .trim()
            .trim_end_matches('/')
            .to_string();
        let size: usize = String::from_utf8_lossy(&hdr[48..58])
            .trim()
            .parse()
            .unwrap_or(0);
        out.insert(name, d[p + 60..(p + 60 + size).min(d.len())].to_vec());
        p += 60 + size + (size & 1);
    }
    out
}

// ---------------------------------------------------------------- tar reader
//
// Extracting into the guest root has to follow directory symlinks, because
// Ubuntu 24.04 is usr-merged: `/bin` is a symlink to `usr/bin`, and a package
// that ships `bin/fusermount` must land in `usr/bin/fusermount`. dpkg does this;
// libarchive/bsdtar and toybox tar do not - they *replace* the symlink with a
// real directory, which silently produces a rootfs with no /bin/sh that nothing
// notices until a maintainer script runs. Measured on this project: a rootfs
// extracted by bsdtar could install nothing, because apt runs its hooks through
// /bin/sh.
//
// This is therefore a small ustar/GNU/pax reader that resolves members through
// whatever the filesystem says, exactly like dpkg. It is also the only part of
// provisioning that no longer depends on a host `tar`.

pub(crate) fn tar_octal(field: &[u8]) -> u64 {
    let end = field.iter().position(|&c| c == 0 || c == b' ').unwrap_or(field.len());
    let mut v: u64 = 0;
    for &c in &field[..end] {
        if !(b'0'..=b'7').contains(&c) {
            continue;
        }
        v = (v << 3) | (c - b'0') as u64;
    }
    v
}

fn field_str(field: &[u8]) -> String {
    let end = field.iter().position(|&c| c == 0).unwrap_or(field.len());
    String::from_utf8_lossy(&field[..end]).to_string()
}

/// pax extended header: "%d key=value\n" records.
fn pax_records(data: &[u8]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut p = 0;
    while p < data.len() {
        let sp = match data[p..].iter().position(|&c| c == b' ') {
            Some(i) => p + i,
            None => break,
        };
        let len: usize = String::from_utf8_lossy(&data[p..sp]).trim().parse().unwrap_or(0);
        if len == 0 || p + len > data.len() {
            break;
        }
        let rec = &data[sp + 1..p + len];
        let rec = if rec.last() == Some(&b'\n') { &rec[..rec.len() - 1] } else { rec };
        if let Some(eq) = rec.iter().position(|&c| c == b'=') {
            out.push((
                String::from_utf8_lossy(&rec[..eq]).to_string(),
                String::from_utf8_lossy(&rec[eq + 1..]).to_string(),
            ));
        }
        p += len;
    }
    out
}

fn safe_rel(name: &str) -> Option<&str> {
    let n = name.trim_start_matches("./").trim_end_matches('/');
    if n.is_empty() || n == "." {
        return Some(""); // "./" - the archive root itself, nothing to create
    }
    if n.starts_with('/') || n.split('/').any(|c| c == "..") {
        return None;
    }
    Some(n)
}

/// Remove whatever is at `p` so a new object can take its place. A symlink is
/// unlinked, never followed - that is the whole point of the fix.
fn clear_path(p: &Path) {
    match fs::symlink_metadata(p) {
        Ok(m) if m.is_dir() => {
            let _ = fs::remove_dir_all(p);
        }
        Ok(_) => {
            let _ = fs::remove_file(p);
        }
        Err(_) => {}
    }
}

pub(crate) fn extract_tar(tar: &[u8], stage: &Path) {
    let mut p = 0usize;
    let mut long_name: Option<String> = None;
    let mut long_link: Option<String> = None;
    let mut pax_path: Option<String> = None;
    let mut pax_link: Option<String> = None;
    while p + 512 <= tar.len() {
        let h = &tar[p..p + 512];
        if h.iter().all(|&c| c == 0) {
            break;
        }
        let size = tar_octal(&h[124..136]) as usize;
        let kind = h[156];
        let body_end = p + 512 + size;
        if body_end > tar.len() {
            die("truncated tar");
        }
        let body = &tar[p + 512..body_end];
        p = body_end + ((512 - size % 512) % 512);

        match kind {
            b'L' => {
                long_name = Some(field_str(body));
                continue;
            }
            b'K' => {
                long_link = Some(field_str(body));
                continue;
            }
            b'x' | b'X' => {
                for (k, v) in pax_records(body) {
                    match k.as_str() {
                        "path" => pax_path = Some(v),
                        "linkpath" => pax_link = Some(v),
                        _ => {}
                    }
                }
                continue;
            }
            b'g' => continue,
            _ => {}
        }

        let name = pax_path
            .take()
            .or_else(|| long_name.take())
            .unwrap_or_else(|| {
                let mut n = field_str(&h[0..100]);
                let prefix = field_str(&h[345..500]);
                if !prefix.is_empty() {
                    n = format!("{prefix}/{n}");
                }
                n
            });
        let link = pax_link
            .take()
            .or_else(|| long_link.take())
            .unwrap_or_else(|| field_str(&h[157..257]));
        let mode = tar_octal(&h[100..108]) as u32;
        let rel = match safe_rel(&name) {
            Some(r) => r.to_string(),
            None => die(format!("tar member escapes the root: {name}")),
        };
        if rel.is_empty() {
            continue;
        }
        let dest = stage.join(&rel);

        if kind == b'5' {
            // Directory. create_dir_all() *follows* an existing symlink, so a
            // "bin/" entry does not clobber the usr-merge link.
            fs::create_dir_all(&dest)
                .unwrap_or_else(|e| die(format!("mkdir {}: {e}", dest.display())));
            let _ = fs::set_permissions(&dest, std::os::unix::fs::PermissionsExt::from_mode(mode));
            continue;
        }

        if let Some(dir) = dest.parent() {
            fs::create_dir_all(dir).unwrap_or_else(|e| die(format!("mkdir {}: {e}", dir.display())));
        }

        match kind {
            b'2' => {
                clear_path(&dest);
                std::os::unix::fs::symlink(&link, &dest)
                    .unwrap_or_else(|e| die(format!("symlink {} -> {link}: {e}", dest.display())));
            }
            b'1' => {
                clear_path(&dest);
                let src = stage.join(link.trim_start_matches('/'));
                // Hard links exist in the archive but are not worth a failure:
                // Android's SELinux refuses link() inside the app home, and a
                // copy is what the runtime does for link(2) as well.
                if fs::hard_link(&src, &dest).is_err() {
                    match fs::read(&src) {
                        Ok(bytes) => {
                            let _ = fs::write(&dest, bytes);
                        }
                        Err(e) => die(format!("hardlink {}: {e}", src.display())),
                    }
                }
            }
            b'3' | b'4' => {
                // Device nodes: no CAP_MKNOD, so nothing to create. Record it
                // instead of pretending.
                println!("  note: skipping device node {rel}");
            }
            b'6' => {
                clear_path(&dest);
                std::os::unix::fs::symlink("", &dest).ok();
                let _ = fs::remove_file(&dest);
                println!("  note: skipping fifo {rel}");
            }
            _ => {
                clear_path(&dest);
                fs::write(&dest, body)
                    .unwrap_or_else(|e| die(format!("write {}: {e}", dest.display())));
                let _ = fs::set_permissions(&dest, std::os::unix::fs::PermissionsExt::from_mode(mode));
            }
        }
    }
}

fn extract_tar_bytes(tar: &[u8], stage: &Path) {
    extract_tar(tar, stage)
}

pub(crate) fn unpack_deb(deb: &[u8], stage: &Path) {
    let m = ar_members(deb);
    if let Some(xz) = m.get("data.tar.xz") {
        let tar = pipe(Command::new("xz").arg("-dc"), xz);
        extract_tar_bytes(&tar, stage);
    } else if let Some(zst) = m.get("data.tar.zst") {
        let tar = pipe(Command::new("zstd").arg("-dc"), zst);
        extract_tar_bytes(&tar, stage);
    } else if let Some(gz) = m.get("data.tar.gz") {
        let tar = gunzip(gz);
        extract_tar_bytes(&tar, stage);
    } else {
        die("no data.tar in the .deb");
    }
}

// ------------------------------------------------------------------ staging

fn generic_names(stage: &Path) {
    let bin = stage.join("usr/bin");
    if !bin.is_dir() {
        die(format!("no usr/bin in {}", stage.display()));
    }
    let ln = |target: &str, link: &str| {
        let p = bin.join(link);
        if !p.exists() {
            let _ = std::os::unix::fs::symlink(target, &p);
        }
    };
    for t in [
        "as",
        "ld",
        "ar",
        "ranlib",
        "nm",
        "objcopy",
        "objdump",
        "strip",
        "readelf",
        "addr2line",
        "size",
    ] {
        ln(&format!("aarch64-linux-gnu-{t}"), t);
    }
    ln("gcc-13", "gcc");
    ln("gcc-13", "cc");
    ln("cpp-13", "cpp");
    ln("aarch64-linux-gnu-gcc-13", "gcc-13");
    ln("aarch64-linux-gnu-cpp-13", "cpp-13");
    // dash ships /usr/bin/sh -> dash; that is what make(1) runs recipes with.
    if bin.join("dash").exists() {
        ln("dash", "sh");
    }
}

pub(crate) fn break_hardlinks(stage: &Path) {
    // App-private storage refuses hard links (packages ship gunzip -> uncompress),
    // so every multi-link file becomes a private copy first.
    fn walk(dir: &Path) {
        use std::os::unix::fs::MetadataExt;
        let Ok(rd) = fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            let Ok(md) = e.metadata() else { continue };
            if md.is_dir() {
                walk(&p);
            } else if md.is_file() && md.nlink() > 1 {
                let tmp = p.with_extension("nrtmp");
                let _ = fs::copy(&p, &tmp);
                let _ = fs::set_permissions(&tmp, md.permissions());
                let _ = fs::rename(&tmp, &p);
            }
        }
    }
    walk(stage);
}

pub fn main(argv: &[String]) {
    let mut suite = "noble".to_string();
    let mut suite_upd = "noble-updates".to_string();
    let mut arch = "arm64".to_string();
    let mut packages =
        "gcc make binutils libc6-dev dash sed grep tar gzip findutils diffutils".to_string();
    let mut stage = PathBuf::from("/tmp/nr-toolchain/root");
    let mut cache = PathBuf::from("/tmp/nr-toolchain/deb");
    let mut skip: HashSet<String> = ["libc6".to_string()].into_iter().collect();
    let mut no_push = false;
    let mut app = "com.linuxhub.probe".to_string();
    let mut guest_root = "files/nrfull/root".to_string();
    let mut adb = "adb".to_string();
    let mut serial: Option<String> = None;

    let mut i = 0;
    while i < argv.len() {
        let k = argv[i].as_str();
        let next = |i: &mut usize| -> String {
            *i += 1;
            argv.get(*i)
                .cloned()
                .unwrap_or_else(|| die(format!("{k} needs a value")))
        };
        match k {
            "--suite" => suite = next(&mut i),
            "--suite-updates" => suite_upd = next(&mut i),
            "--arch" => arch = next(&mut i),
            "--packages" => packages = next(&mut i),
            "--stage" => stage = PathBuf::from(next(&mut i)),
            "--cache" => cache = PathBuf::from(next(&mut i)),
            "--skip" => {
                skip = next(&mut i)
                    .split(',')
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .collect()
            }
            "--app" => app = next(&mut i),
            "--guest-root" => guest_root = next(&mut i),
            "--adb" => adb = next(&mut i),
            "--serial" => serial = Some(next(&mut i)),
            "--no-push" => no_push = true,
            other => die(format!("unknown argument {other}")),
        }
        i += 1;
    }

    fs::create_dir_all(&stage).unwrap_or_else(|e| die(format!("stage: {e}")));
    fs::create_dir_all(&cache).unwrap_or_else(|e| die(format!("cache: {e}")));

    // ------------------------------------------------------------- index ---
    let mut index: BTreeMap<String, Pkg> = BTreeMap::new();
    for s in [&suite, &suite_upd] {
        for c in COMPS {
            let path = format!("/ubuntu-ports/dists/{s}/{c}/binary-{arch}/Packages.gz");
            match http_get(&path) {
                Some(raw) => {
                    let text = String::from_utf8_lossy(&gunzip(&raw)).to_string();
                    parse_index(&text, &mut index);
                }
                None => println!("skip index {path}"),
            }
        }
    }

    let wanted: Vec<String> = packages.split_whitespace().map(String::from).collect();
    let want = resolve(&index, &wanted, &skip);
    let total: u64 = want.iter().map(|p| p.size).sum();
    println!("{} packages, {:.1} MB", want.len(), total as f64 / 1e6);

    // ----------------------------------------------------------- download ---
    for p in &want {
        let deb = cache.join(p.filename.rsplit('/').next().unwrap_or("x.deb"));
        let ok = fs::metadata(&deb)
            .map(|m| m.len() == p.size)
            .unwrap_or(false);
        if !ok {
            println!("fetch {} {}", p.name, p.version);
            let raw = http_get(&format!("{POOL}{}", p.filename))
                .unwrap_or_else(|| die(format!("download failed: {}", p.filename)));
            fs::write(&deb, &raw).unwrap_or_else(|e| die(format!("{}: {e}", deb.display())));
        }
        let data = fs::read(&deb).unwrap_or_else(|e| die(format!("{}: {e}", deb.display())));
        unpack_deb(&data, &stage);
    }
    println!("unpacked into {}", stage.display());

    generic_names(&stage);
    let entries = fs::read_dir(stage.join("usr/bin"))
        .map(|d| d.count())
        .unwrap_or(0);
    println!("staged: {} entries in usr/bin", entries);

    if no_push {
        return;
    }

    // --------------------------------------------------------------- push ---
    // tar(1) ships /etc/rmt, a symlink to an absolute path outside the destination;
    // the device-side extractor refuses those, so drop it.
    let _ = fs::remove_file(stage.join("etc/rmt"));
    break_hardlinks(&stage);

    let mut tar = Command::new("tar")
        .arg("czf")
        .arg("-")
        .arg("-C")
        .arg(&stage)
        .arg(".")
        .stdout(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| die(format!("spawn tar: {e}")));
    let mut tarball = Vec::new();
    tar.stdout
        .take()
        .unwrap()
        .read_to_end(&mut tarball)
        .unwrap_or_else(|e| die(format!("{e}")));
    let _ = tar.wait();

    println!("pushing to {app}:{guest_root}");
    let remote = format!("run-as {app} sh -c 'cd {guest_root} && tar xzf - && echo PUSHED'");
    let mut cmd = Command::new(&adb);
    if let Some(s) = &serial {
        cmd.arg("-s").arg(s);
    }
    let mut child = cmd
        .arg("shell")
        .arg(&remote)
        .stdin(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| die(format!("spawn adb: {e}")));
    let mut stdin = child.stdin.take().expect("stdin");
    let w = std::thread::spawn(move || {
        let _ = stdin.write_all(&tarball);
    });
    w.join().ok();
    let st = child.wait().unwrap_or_else(|e| die(format!("adb: {e}")));
    if !st.success() {
        die("push failed");
    }
}
