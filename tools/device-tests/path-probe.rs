//! Guest-side semantics probe for the loader-embedded path shim.
//!
//! These calls intentionally go through the binary's PLT/GOT: the loader
//! rewrites those slots to the shim before constructors run. A direct syscall
//! here would bypass the code under test and prove nothing.
use std::ffi::CString;
use std::os::raw::{c_char, c_int};
use std::os::unix::fs::symlink;

extern "C" {
    fn faccessat(dirfd: c_int, pathname: *const c_char, mode: c_int, flags: c_int) -> c_int;
    fn open(pathname: *const c_char, flags: c_int, ...) -> c_int;
    fn close(fd: c_int) -> c_int;
}

const AT_FDCWD: c_int = -100;
const AT_SYMLINK_NOFOLLOW: c_int = 0x100;
const O_RDONLY: c_int = 0;
const O_DIRECTORY: c_int = 0o40000; // AArch64 UAPI
const R_OK: c_int = 4;

fn call(dirfd: c_int, path: &str, mode: c_int, flags: c_int) -> bool {
    let path = CString::new(path).unwrap();
    unsafe { faccessat(dirfd, path.as_ptr(), mode, flags) == 0 }
}

fn check(dirfd: c_int, path: &str, mode: c_int, flags: c_int, want: bool, name: &str) -> bool {
    let got = call(dirfd, path, mode, flags);
    println!(
        "{} {} got={} want={}",
        if got == want { "PASS" } else { "FAIL" },
        name,
        got,
        want
    );
    got == want
}

/// Build a chain of `n` symlinks under `base`: `<tag>1 -> <tag>2 -> ... -> <terminal>`.
/// Returns the path of the first link. Existing links are replaced so one probe
/// binary can place 16, 17, 40 and 41 links in the same directory.
fn build_chain(base: &str, tag: char, n: usize, terminal: &str) -> String {
    for i in 1..=n {
        let link = format!("{base}/{tag}{i}");
        let _ = std::fs::remove_file(&link);
        let target = if i == n {
            terminal.to_string()
        } else {
            format!("{tag}{}", i + 1)
        };
        symlink(&target, &link).unwrap();
    }
    format!("{base}/{tag}1")
}

fn main() {
    let base = "/tmp/husk-access-probe";
    let _ = std::fs::remove_dir_all(base);
    std::fs::create_dir_all(base).unwrap();
    let dangling = format!("{base}/dangling");
    let escape = format!("{base}/escape");
    symlink("/definitely/missing/target", &dangling).unwrap();
    symlink("/system/build.prop", &escape).unwrap();

    let mut ok = true;
    ok &= check(AT_FDCWD, &dangling, 0, 0, false, "dangling-follow");
    ok &= check(
        AT_FDCWD,
        &dangling,
        0,
        AT_SYMLINK_NOFOLLOW,
        true,
        "dangling-nofollow",
    );
    ok &= check(
        AT_FDCWD,
        &dangling,
        R_OK,
        AT_SYMLINK_NOFOLLOW,
        true,
        "dangling-nofollow-r",
    );
    ok &= check(
        AT_FDCWD,
        "/etc/os-release",
        R_OK,
        AT_SYMLINK_NOFOLLOW,
        true,
        "regular-nofollow-r",
    );
    ok &= check(AT_FDCWD, &escape, 0, 0, false, "escape-follow");
    ok &= check(
        AT_FDCWD,
        &escape,
        0,
        AT_SYMLINK_NOFOLLOW,
        true,
        "escape-nofollow",
    );

    let base_c = CString::new(base).unwrap();
    let dfd = unsafe { open(base_c.as_ptr(), O_RDONLY | O_DIRECTORY) };
    if dfd < 0 {
        println!("FAIL relative-dirfd-open got=false");
        ok = false;
    } else {
        ok &= check(dfd, "dangling", 0, 0, false, "rel-follow");
        ok &= check(
            dfd,
            "dangling",
            0,
            AT_SYMLINK_NOFOLLOW,
            true,
            "rel-nofollow",
        );
        unsafe { close(dfd) };
    }

    // --- symlink depth: Linux follows 40 links, the 41st is ELOOP -----------
    // Two shapes, because they reach different resolver code:
    //   * final-component chains (f1 -> f2 -> ... -> target) are walked by the
    //     loop in resolve_access/resolve_open/resolve_final;
    //   * middle-component chains (m1 -> m2 -> ... -> midend) are walked by
    //     resolve_at, which used to recurse once per link and carried 16 KiB of
    //     stack per frame.
    let dbase = "/tmp/husk-depth-probe";
    let _ = std::fs::remove_dir_all(dbase);
    std::fs::create_dir_all(dbase).unwrap();
    std::fs::write(format!("{dbase}/target"), b"x").unwrap();
    std::fs::create_dir_all(format!("{dbase}/midend")).unwrap();
    std::fs::write(format!("{dbase}/midend/file"), b"x").unwrap();
    let mut checks = 8usize;
    for n in [16usize, 17, 40, 41] {
        let want = n <= 40;
        let f = build_chain(dbase, 'f', n, "target");
        ok &= check(AT_FDCWD, &f, 0, 0, want, &format!("depth-final-{n}"));
        checks += 1;
        let m = build_chain(dbase, 'm', n, "midend");
        let mp = format!("{m}/file");
        ok &= check(AT_FDCWD, &mp, 0, 0, want, &format!("depth-mid-{n}"));
        checks += 1;
    }
    // A 40-link chain has to resolve on a small guest thread stack. The old
    // resolver needed ~640 KiB of stack to walk that far, so the same call
    // turned a path-length answer into a SIGSEGV. The threshold is measured,
    // not assumed: the probe prints which stack size breaks.
    // The 16/17/40/41 loop above leaves m1 pointing at a 41-link chain, so the
    // stack tests rebuild the exact 40-link case they are named after.
    let path40 = build_chain(dbase, 'm', 40, "midend") + "/file";
    for kib in [64usize, 128, 256, 1024] {
        let p = path40.clone();
        let res = std::thread::Builder::new()
            .stack_size(kib * 1024)
            .spawn(move || {
                let ok = call(AT_FDCWD, &p, 0, 0);
                (ok, std::io::Error::last_os_error().raw_os_error())
            })
            .unwrap()
            .join();
        let (got, err) = match res {
            Ok(v) => v,
            Err(_) => (false, None),
        };
        println!(
            "{} depth-40-on-{kib}k-stack got={} errno={:?}",
            if got { "PASS" } else { "FAIL" },
            got,
            err
        );
        ok &= got;
        checks += 1;
    }
    let _ = std::fs::remove_dir_all(dbase);

    let _ = std::fs::remove_dir_all(base);
    println!("#checks={checks} fail={}", if ok { 0 } else { 1 });
    if !ok {
        std::process::exit(1);
    }
}
