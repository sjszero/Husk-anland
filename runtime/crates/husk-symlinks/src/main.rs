//! Rewrite absolute symlinks inside a guest rootfs into relative ones.
//!
//! Why: the guest loader is a *host* process. When glibc's `_dl_map_object`
//! builds `<library-path>/<soname>`, the kernel resolves a symlink target
//! against the *host* root, so `/etc/alternatives/libblas.so.3-aarch64-linux-gnu`
//! escapes the rootfs and the library "cannot be found" although it is there.
//! A relative target resolves the same way in both namespaces, so normalising
//! costs nothing and removes the whole failure class (evidence 155: 551 of 552
//! links, `numpy` / `libblas` / `liblapack` only load after this).
//!
//! `husk-symlinks --root <dir> --apply` rewrites; without `--apply` it only
//! reports. `--verify` exits non-zero when any absolute link is left, which is
//! what `install-husk.sh` runs before declaring an install good.
//!
//! Pseudo-filesystems (`proc`, `sys`, `dev`, `run`, `tmp`) are skipped: their
//! contents are created at session start by the runtime, not by the image.
//! A link whose target does not exist inside the root is left alone and
//! reported as dangling — rewriting it would only hide a broken image.

use std::env;
use std::fs;
use std::io;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

const SKIP_TOP: [&str; 5] = ["proc", "sys", "dev", "run", "tmp"];

#[derive(Default)]
struct Counts {
    links: u64,
    absolute: u64,
    rewritten: u64,
    dangling: u64,
    left: u64,
}

fn main() {
    let mut root = PathBuf::from("/");
    let mut apply = false;
    let mut verify = false;
    let mut quiet = false;
    let mut it = env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--root" => match it.next() {
                Some(p) => root = PathBuf::from(p),
                None => fail("--root needs a value"),
            },
            "--apply" => apply = true,
            "--verify" => verify = true,
            "--quiet" => quiet = true,
            "-h" | "--help" => {
                println!("usage: husk-symlinks [--root DIR] [--apply] [--verify] [--quiet]");
                return;
            }
            other => {
                eprintln!("husk-symlinks: unknown argument {other}");
                std::process::exit(64);
            }
        }
    }

    let root = match fs::canonicalize(&root) {
        Ok(p) => p,
        Err(e) => fail(&format!("cannot use root {}: {e}", root.display())),
    };

    let mut c = Counts::default();
    if let Err(e) = walk(&root, &root, apply, quiet, &mut c) {
        fail(&format!("walk failed: {e}"));
    }

    println!(
        "links={} absolute={} rewritten={} dangling={} left={}",
        c.links, c.absolute, c.rewritten, c.dangling, c.left
    );
    if verify && c.left > 0 {
        eprintln!(
            "husk-symlinks: {0} absolute link(s) still escape the rootfs",
            c.left
        );
        std::process::exit(1);
    }
}

fn fail(msg: &str) -> ! {
    eprintln!("husk-symlinks: {msg}");
    std::process::exit(65);
}

fn walk(root: &Path, dir: &Path, apply: bool, quiet: bool, c: &mut Counts) -> io::Result<()> {
    let mut entries: Vec<_> = fs::read_dir(dir)?.filter_map(Result::ok).collect();
    entries.sort_by_key(|e| e.file_name());
    for ent in entries {
        let path = ent.path();
        let ft = ent.file_type()?;
        if ft.is_symlink() {
            c.links += 1;
            let target = fs::read_link(&path)?;
            let Some(target) = target.to_str() else {
                continue;
            };
            if !target.starts_with('/') {
                continue;
            }
            c.absolute += 1;
            // Resolve inside the guest root, never against the host root.
            let inside = root.join(target.trim_start_matches('/'));
            if fs::symlink_metadata(&inside).is_err() {
                c.dangling += 1;
                if !quiet {
                    println!(
                        "skip (dangling): {} -> {target}",
                        rel(root, &path).display()
                    );
                }
                continue;
            }
            let link_dir = path.parent().unwrap_or(root);
            let new_target = relative_from(link_dir, &inside);
            if apply {
                fs::remove_file(&path)?;
                symlink(&new_target, &path)?;
                c.rewritten += 1;
                if !quiet {
                    println!(
                        "rewrote: {} : {target} -> {new_target}",
                        rel(root, &path).display()
                    );
                }
            } else {
                c.left += 1;
                if !quiet {
                    println!(
                        "would rewrite: {} : {target} -> {new_target}",
                        rel(root, &path).display()
                    );
                }
            }
        } else if ft.is_dir() {
            if dir == root && SKIP_TOP.contains(&ent.file_name().to_str().unwrap_or("")) {
                continue;
            }
            walk(root, &path, apply, quiet, c)?;
        }
    }
    Ok(())
}

fn rel(root: &Path, p: &Path) -> PathBuf {
    p.strip_prefix(root)
        .map(PathBuf::from)
        .unwrap_or_else(|_| p.to_path_buf())
}

/// Lexical relative path from directory `from` to path `to`, both absolute.
fn relative_from(from: &Path, to: &Path) -> String {
    let f: Vec<_> = from
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .collect();
    let t: Vec<_> = to
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .collect();
    let mut i = 0;
    while i < f.len() && i < t.len() && f[i] == t[i] {
        i += 1;
    }
    let mut out: Vec<&str> = Vec::new();
    for _ in i..f.len() {
        out.push("..");
    }
    for c in &t[i..] {
        out.push(c);
    }
    if out.is_empty() {
        ".".to_string()
    } else {
        out.join("/")
    }
}
