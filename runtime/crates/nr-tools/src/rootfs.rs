//! `nr-tools rootfs` - build a bootable guest rootfs on the host, from Ubuntu
//! packages, with the runtime's own libc patch baked in.
//!
//! This is the missing half of provisioning: until now a husk needed a rootfs
//! tarball from somewhere else. Now the same resolver that stages gcc can stage a
//! base system, fix the usr-merge symlinks and the account files, write a dpkg
//! status database so dpkg/apt believe the packages are installed, apply the two
//! `svc` -> `nop` edits to the guest libc, and hand back a tarball.
//!
//! No root, no chroot, no debootstrap: unpacking .debs into a directory is all it
//! takes when the kernel executes guest binaries directly.

use crate::provision::{
    control_stanza, extract_tar, gunzip, http_get, parse_index, resolve, unpack_deb,
    write_dpkg_info, Pkg,
};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const COMPS: [&str; 2] = ["main", "universe"];

/// What a husk needs to be usable: a shell, coreutils, dpkg/apt, and the few
/// libraries Ubuntu splits out of them.
const BASE: &str = "base-files base-passwd libc6 libgcc-s1 libcrypt1 libtinfo6 ncurses-base \
ncurses-bin bash coreutils dash dpkg diffutils findutils grep sed tar gzip xz-utils zlib1g \
libselinux1 libpcre2-8-0 libattr1 libacl1 libmd0 libbz2-1.0 liblzma5 libseccomp2 libcap2 \
util-linux procps less apt libgnutls30t64 libgcrypt20 libgpg-error0 libidn2-0 libunistring5 \
libp11-kit0 libffi8 libtasn1-6 libnettle8t64 libhogweed6t64 libgmp10 liblz4-1 libzstd1 \
libsystemd0 libudev1 libxxhash0 libapt-pkg6.0t64 ubuntu-keyring";

fn die(msg: impl AsRef<str>) -> ! {
    eprintln!("{}", msg.as_ref());
    std::process::exit(1)
}

/// Does this path exist, *including* a dangling symlink?
///
/// usr-merge links are allowed to point at directories the package set does not
/// provide (`sbin -> usr/sbin` in a tree with no /usr/sbin), and `Path::exists()`
/// follows the link, so it answers false for a link that is perfectly correct.
pub(crate) fn lexists(p: &Path) -> bool {
    fs::symlink_metadata(p).is_ok()
}

fn write(path: &Path, what: &str, bytes: &[u8]) {
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    fs::write(path, bytes).unwrap_or_else(|e| die(format!("{what} {}: {e}", path.display())));
}

pub fn main(argv: &[String]) {
    let mut out = PathBuf::from("/tmp/husk-rootfs");
    let mut suite = "noble".to_string();
    let mut arch = "arm64".to_string();
    let mut packages = BASE.to_string();
    let mut cache = PathBuf::from("/tmp/nr-rootfs/deb");
    let mut tar: Option<PathBuf> = None;
    let mut patch_libc = true;
    let mut fresh = false;
    let mut loader_out: Option<PathBuf> = None;
    let mut shim: Option<PathBuf> = None;
    let mut hook: Option<PathBuf> = None;
    // 2.0's asset tree is a base rootfs tarball plus a directory of .debs.
    // Pointing the builder at those two makes a husk from the same bits the
    // product ships, without the network and without dpkg.
    let mut base: Option<PathBuf> = None;
    let mut deb_dir: Option<PathBuf> = None;

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
            "--out" => out = PathBuf::from(next(&mut i)),
            "--suite" => suite = next(&mut i),
            "--arch" => arch = next(&mut i),
            "--packages" => packages = next(&mut i),
            "--cache" => cache = PathBuf::from(next(&mut i)),
            "--tar" => tar = Some(PathBuf::from(next(&mut i))),
            "--no-patch-libc" => patch_libc = false,
            "--loader-out" => loader_out = Some(PathBuf::from(next(&mut i))),
            "--shim" => shim = Some(PathBuf::from(next(&mut i))),
            "--hook" => hook = Some(PathBuf::from(next(&mut i))),
            "--fresh" => fresh = true,
            "--base" => base = Some(PathBuf::from(next(&mut i))),
            "--debs" => deb_dir = Some(PathBuf::from(next(&mut i))),
            other => die(format!("unknown argument {other}\nusage: nr-tools rootfs [--out DIR] [--suite noble] \
[--arch arm64] [--packages \"...\"] [--base TAR] [--debs DIR] [--cache DIR] [--tar FILE] [--no-patch-libc] [--fresh]")),
        }
        i += 1;
    }

    if fresh {
        let _ = fs::remove_dir_all(&out);
    }
    fs::create_dir_all(&out).unwrap_or_else(|e| die(format!("out: {e}")));
    fs::create_dir_all(&cache).unwrap_or_else(|e| die(format!("cache: {e}")));

    // ---------------------------------------------------------- base rootfs ---
    if let Some(t) = &base {
        let raw = fs::read(t).unwrap_or_else(|e| die(format!("{}: {e}", t.display())));
        // Ubuntu base tarballs are gzip; the .deb bodies are handled by unpack_deb.
        let tar = if raw.starts_with(&[0x1f, 0x8b]) {
            gunzip(&raw)
        } else {
            raw
        };
        extract_tar(&tar, &out);
        println!("base: {} extracted into {}", t.display(), out.display());
    }

    // ------------------------------------------------------------- local debs ---
    // When a deb directory is given, the index and the network are not used at
    // all: the versions that go into the image are exactly the ones on disk.
    if let Some(dir) = &deb_dir {
        let mut files: Vec<PathBuf> = fs::read_dir(dir)
            .unwrap_or_else(|e| die(format!("{}: {e}", dir.display())))
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().map(|x| x == "deb").unwrap_or(false))
            .collect();
        files.sort();
        if files.is_empty() {
            die(format!("no .deb files in {}", dir.display()));
        }
        println!("local debs: {} file(s) in {}", files.len(), dir.display());
        let mut installed: Vec<(String, String, Option<String>)> = Vec::new();
        for f in &files {
            let data = fs::read(f).unwrap_or_else(|e| die(format!("{}: {e}", f.display())));
            let stem = f
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let stem = stem.trim_end_matches(".deb");
            // pkg_version_arch; Debian package names cannot contain '_'
            let mut it = stem.splitn(3, '_');
            let name = it.next().unwrap_or("").to_string();
            let version = it.next().unwrap_or("0").to_string();
            let paths = unpack_deb(&data, &out);
            write_dpkg_info(&out, &name, &paths, &data);
            installed.push((name, version, control_stanza(&data)));
        }
        finish(
            out, tar, loader_out, shim, hook, patch_libc, &installed, &arch, &suite,
        );
        return;
    }

    // ------------------------------------------------------------- index ---
    let mut index: BTreeMap<String, Pkg> = BTreeMap::new();
    for s in [suite.as_str(), &format!("{suite}-updates")] {
        for c in COMPS {
            let path = format!("/ubuntu-ports/dists/{s}/{c}/binary-{arch}/Packages.gz");
            if let Some(raw) = http_get(&path) {
                parse_index(&String::from_utf8_lossy(&gunzip(&raw)), &mut index);
            }
        }
    }
    if index.is_empty() {
        die("no package index: check network access to ports.ubuntu.com");
    }

    // `libc6` is unpacked like the rest, then patched: the guest loader and the
    // guest libc must agree on exactly the build this image carries.
    let skip: HashSet<String> = HashSet::new();
    let wanted: Vec<String> = packages.split_whitespace().map(String::from).collect();
    let want = resolve(&index, &wanted, &skip);
    let total: u64 = want.iter().map(|p| p.size).sum();
    println!("{} packages, {:.1} MB", want.len(), total as f64 / 1e6);
    for name in &wanted {
        if !want.iter().any(|p| &p.name == name) {
            println!("  MISSING {name}");
        }
    }

    // ---------------------------------------------------------- download ---
    let mut installed: Vec<(String, String, Option<String>)> = Vec::new();
    for p in &want {
        let deb = cache.join(p.filename.rsplit('/').next().unwrap_or("x.deb"));
        if !fs::metadata(&deb)
            .map(|m| m.len() == p.size)
            .unwrap_or(false)
        {
            let raw = http_get(&format!("/ubuntu-ports/{}", p.filename))
                .unwrap_or_else(|| die(format!("download failed: {}", p.filename)));
            write(&deb, "cache", &raw);
        }
        let data = fs::read(&deb).unwrap_or_else(|e| die(format!("{}: {e}", deb.display())));
        let paths = unpack_deb(&data, &out);
        write_dpkg_info(&out, &p.name, &paths, &data);
        installed.push((p.name.clone(), p.version.clone(), control_stanza(&data)));
    }
    println!(
        "unpacked {} packages into {}",
        installed.len(),
        out.display()
    );

    // The index path used to stop here: packages on disk, no `var/lib/dpkg/status`,
    // no libc patch, no loader, no tarball. A tree built that way has every file
    // and no installed package as far as apt is concerned, and the run still
    // exited 0. Only `--base/--debs` reached `finish()`, which is why the
    // shipping trees were assembled through the local-deb path.
    finish(
        out, tar, loader_out, shim, hook, patch_libc, &installed, &arch, &suite,
    );

    /// Everything after the packages are on disk: usr-merge links, account files,
    /// the dpkg status database, the libc patch and the loader built from *this*
    /// image's own ld.so, then the tarball.
    #[allow(clippy::too_many_arguments)]
    fn finish(
        out: PathBuf,
        tar: Option<PathBuf>,
        loader_out: Option<PathBuf>,
        shim: Option<PathBuf>,
        hook: Option<PathBuf>,
        patch_libc: bool,
        installed: &[(String, String, Option<String>)],
        arch: &str,
        suite: &str,
    ) {
        // ------------------------------------------------------------- fixups ---
        // Ubuntu 24.04 is usr-merged: the packages ship /usr/*, the world asks for
        // /bin, /sbin, /lib.
        for (link, target) in [("bin", "usr/bin"), ("sbin", "usr/sbin"), ("lib", "usr/lib")] {
            let p = out.join(link);
            if !lexists(&p) {
                let _ = std::os::unix::fs::symlink(target, &p);
            }
        }
        for d in [
            "proc",
            "sys",
            "dev",
            "tmp",
            "run",
            "var/log",
            "var/lib/dpkg/info",
            "var/lib/dpkg/updates",
            "var/lib/dpkg/parts",
            "root",
            "home",
        ] {
            let _ = fs::create_dir_all(out.join(d));
        }
        let _ = fs::set_permissions(
            out.join("tmp"),
            std::os::unix::fs::PermissionsExt::from_mode(0o1777),
        );

        for (src, dst) in [
            ("usr/share/base-passwd/passwd.master", "etc/passwd"),
            ("usr/share/base-passwd/group.master", "etc/group"),
        ] {
            let p = out.join(src);
            if p.exists() {
                if let Ok(b) = fs::read(&p) {
                    write(&out.join(dst), dst, &b);
                }
            }
        }
        if !out.join("etc/passwd").exists() {
            write(
                &out.join("etc/passwd"),
                "etc/passwd",
                b"root:x:0:0:root:/root:/bin/bash\n",
            );
            write(&out.join("etc/group"), "etc/group", b"root:x:0:\n");
        }
        write(&out.join("etc/hostname"), "etc/hostname", b"husk\n");
        write(
            &out.join("etc/hosts"),
            "etc/hosts",
            b"127.0.0.1 localhost husk\n::1 localhost\n",
        );
        if !out.join("etc/resolv.conf").exists() {
            write(
                &out.join("etc/resolv.conf"),
                "etc/resolv.conf",
                b"nameserver 1.1.1.1\nnameserver 8.8.8.8\n",
            );
        }
        // apt's own sandbox would try to become _apt before downloading; in a
        // single-uid runtime that call cannot succeed and the method dies with
        // "Could not switch group". See tools/guest-config/apt-husk.conf.
        write(
            &out.join("etc/apt/apt.conf.d/99husk"),
            "apt sandbox config",
            b"APT::Sandbox::User \"root\";\n",
        );
        write(
            &out.join("etc/nsswitch.conf"),
            "etc/nsswitch.conf",
            b"passwd: files\ngroup: files\nshadow: files\nhosts: files dns\n",
        );
        write(
            &out.join("etc/apt/sources.list"),
            "etc/apt/sources.list",
            format!(
                "deb http://ports.ubuntu.com/ubuntu-ports {suite} main universe\n\
deb http://ports.ubuntu.com/ubuntu-ports {suite}-updates main universe\n"
            )
            .as_bytes(),
        );

        // dpkg and apt only believe a package is installed if the status database
        // says so; we did not run maintainer scripts, so we write the stanza for
        // them. Take the package's own control fields when the .deb is at hand -
        // `Depends` and friends are what apt resolves against installed packages,
        // and a name+version stub makes dpkg warn about the missing Maintainer
        // field on every single invocation.
        let mut status = String::new();
        for (name, version, ctl) in installed {
            match ctl {
                Some(ctl) => {
                    for line in ctl.lines() {
                        if line.starts_with("Status:") {
                            continue;
                        }
                        status.push_str(line);
                        status.push('\n');
                        if line.starts_with("Package:") {
                            status.push_str("Status: install ok installed\n");
                        }
                    }
                    if !status.ends_with("\n\n") {
                        status.push('\n');
                    }
                }
                None => status.push_str(&format!(
                    "Package: {name}\nStatus: install ok installed\nPriority: optional\n\
Section: misc\nArchitecture: {arch}\nVersion: {version}\n\
Description: {name} (staged by nr-tools rootfs)\n\n"
                )),
            }
        }
        write(
            &out.join("var/lib/dpkg/status"),
            "dpkg status",
            status.as_bytes(),
        );

        // ------------------------------------------------------------ libc ---
        let libc = out.join("usr/lib/aarch64-linux-gnu/libc.so.6");
        if patch_libc && libc.exists() {
            let patched = out.join("usr/lib/aarch64-linux-gnu/libc.so.6.husk");
            let rc = crate::scan_main(&[
                "patch".to_string(),
                libc.display().to_string(),
                patched.display().to_string(),
            ]);
            if rc == std::process::ExitCode::SUCCESS {
                let _ = fs::rename(&patched, &libc);
                println!("libc: optional syscalls patched in {}", libc.display());
            } else {
                die("libc patch refused: the image is not the glibc this runtime expects");
            }
        } else if patch_libc {
            die("libc6 missing from the staged tree");
        }

        // ------------------------------------------------------- loader for it ---
        // The loader must come from the same glibc build as the libc it runs against,
        // so the image and the loader it ships are produced together.
        if let Some(lo) = loader_out {
            let (Some(shim), Some(hook)) = (shim, hook) else {
                die("--loader-out needs --shim and --hook");
            };
            let orig = out.join("usr/lib/aarch64-linux-gnu/ld-linux-aarch64.so.1");
            if !orig.exists() {
                die("no ld-linux-aarch64.so.1 in the staged tree");
            }
            let stage1 = out.join("usr/lib/aarch64-linux-gnu/ld-linux-aarch64.so.1.nop");
            let rc = crate::scan_main(&[
                "patch".to_string(),
                orig.display().to_string(),
                stage1.display().to_string(),
            ]);
            if rc != std::process::ExitCode::SUCCESS {
                die("loader patch refused");
            }
            crate::embed(
                shim.to_str().unwrap_or(""),
                stage1.to_str().unwrap_or(""),
                hook.to_str().unwrap_or(""),
                lo.to_str().unwrap_or(""),
            );
            let _ = fs::remove_file(&stage1);
            println!(
                "loader: {} (built from this image's own ld.so)",
                lo.display()
            );
        }

        // ------------------------------------------------------------- report ---
        let mut bins = 0;
        if let Ok(rd) = fs::read_dir(out.join("usr/bin")) {
            bins = rd.count();
        }
        // `exists()` follows the link: a usr-merge link whose target is not part
        // of this package set is *correct* and still reports absent, which made
        // this line print MISSING on every minimal tree while the links were
        // right there. Ask about the link itself.
        let links_ok = ["bin", "sbin", "lib"].iter().all(|l| lexists(&out.join(l)));
        println!(
            "staged: {} entries in usr/bin, usr-merge links {}",
            bins,
            if links_ok { "ok" } else { "MISSING" }
        );

        if let Some(t) = tar {
            // App-private storage refuses hard links; every multi-link file becomes a
            // private copy so the device-side extractor never has to create one.
            crate::provision::break_hardlinks(&out);
            let mut child = Command::new("tar")
                .arg("czf")
                .arg(&t)
                .arg("-C")
                .arg(&out)
                .arg(".")
                .stdout(Stdio::null())
                .spawn()
                .unwrap_or_else(|e| die(format!("spawn tar: {e}")));
            let st = child.wait().unwrap_or_else(|e| die(format!("tar: {e}")));
            if !st.success() {
                die("tar failed");
            }
            let sz = fs::metadata(&t).map(|m| m.len()).unwrap_or(0);
            println!("tarball: {} ({:.0} MB)", t.display(), sz as f64 / 1e6);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A usr-merge link is allowed to dangle: `sbin -> usr/sbin` in a package set
    /// without /usr/sbin is correct, and `Path::exists()` follows the link and
    /// answers false - which made the staging report print "usr-merge links
    /// MISSING" on every minimal tree while the links were in place.
    #[test]
    fn lexists_counts_a_dangling_symlink() {
        let d = std::env::temp_dir().join(format!("nr-lexists-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        let link = d.join("sbin");
        std::os::unix::fs::symlink("usr/sbin", &link).unwrap();
        assert!(!link.exists(), "a dangling link must not report exists()");
        assert!(lexists(&link), "lexists must see the link itself");
        let _ = fs::remove_dir_all(&d);
    }
}
