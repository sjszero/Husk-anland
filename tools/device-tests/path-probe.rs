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

    let _ = std::fs::remove_dir_all(base);
    println!("#checks=8 fail={}", if ok { 0 } else { 1 });
    if !ok {
        std::process::exit(1);
    }
}
