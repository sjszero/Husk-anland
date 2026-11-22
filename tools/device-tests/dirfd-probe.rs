//! Guest-side dirfd containment probe for the loader-embedded path shim.
//!
//! A real directory descriptor pins a starting inode, not a namespace boundary:
//! `openat(dfd, "../../../host/path")` can still climb out of the guest root.
//! This probe exercises the relative-path ABI through libc's PLT so the shim's
//! GOT interpose, not a raw syscall, is what is being measured.
//!
//! The caller must also check that `/data/local/tmp/husk-dirfd-escape` does not
//! exist on the host after the probe.
#![no_std]
#![no_main]

use core::panic::PanicInfo;

extern "C" {
    fn open(path: *const u8, flags: i32, ...) -> i32;
    fn openat(dirfd: i32, path: *const u8, flags: i32, ...) -> i32;
    fn close(fd: i32) -> i32;
    fn write(fd: i32, buf: *const u8, n: usize) -> isize;
    fn mkdir(path: *const u8, mode: u32) -> i32;
    fn mkdirat(dirfd: i32, path: *const u8, mode: u32) -> i32;
    fn rmdir(path: *const u8) -> i32;
    fn unlinkat(dirfd: i32, path: *const u8, flags: i32) -> i32;
    fn fstatat(dirfd: i32, path: *const u8, st: *mut u8, flags: i32) -> i32;
    fn faccessat(dirfd: i32, path: *const u8, mode: i32, flags: i32) -> i32;
    fn symlinkat(target: *const u8, dirfd: i32, linkpath: *const u8) -> i32;
    fn readlinkat(dirfd: i32, path: *const u8, out: *mut u8, n: usize) -> isize;
    fn fchmodat(dirfd: i32, path: *const u8, mode: u32, flags: i32) -> i32;
    fn utimensat(dirfd: i32, path: *const u8, times: *const i64, flags: i32) -> i32;
}

const O_RDONLY: i32 = 0;
const O_RDWR: i32 = 2;
const O_CREAT: i32 = 0o100;
const O_TRUNC: i32 = 0o1000;
const O_DIRECTORY: i32 = 0o40000;
const R_OK: i32 = 4;

static BASE: &[u8] = b"/tmp/husk-dirfd-probe\0";
static FILE: &[u8] = b"file\0";
static LINK: &[u8] = b"link\0";
static ABS_LINK: &[u8] = b"/etc/os-release\0";
static ESCAPE: &[u8] = b"../../../../../../../system/build.prop\0";
static HOST_ESCAPE_DIR: &[u8] = b"../../../../../../../data/local/tmp/husk-dirfd-escape\0";

unsafe fn out(s: &[u8]) {
    let _ = write(2, s.as_ptr(), s.len());
}

unsafe fn num(mut v: i64) {
    let mut b = [0u8; 24];
    let mut n = 0usize;
    if v == 0 {
        out(b"0");
        return;
    }
    let neg = v < 0;
    if neg { v = -v; }
    while v > 0 {
        b[n] = b'0' + (v % 10) as u8;
        v /= 10;
        n += 1;
    }
    if neg { out(b"-"); }
    while n > 0 {
        n -= 1;
        let x = [b[n]];
        out(&x);
    }
}

unsafe fn check(name: &[u8], got: i64, want: i64, failures: &mut usize) {
    if got == want {
        out(b"PASS ");
    } else {
        out(b"FAIL ");
        *failures += 1;
    }
    out(name);
    out(b" got=");
    num(got);
    out(b" want=");
    num(want);
    out(b"\n");
}

unsafe fn mode(st: &[u8; 256]) -> u32 {
    u32::from_le_bytes([st[16], st[17], st[18], st[19]]) & 0o7777
}

#[no_mangle]
pub unsafe extern "C" fn _start() -> ! {
    let mut failures = 0usize;
    let _ = rmdir(BASE.as_ptr());
    check(b"mkdir-base", mkdir(BASE.as_ptr(), 0o755) as i64, 0, &mut failures);
    let dfd = open(BASE.as_ptr(), O_RDONLY | O_DIRECTORY, 0);
    check(b"open-dirfd", (dfd >= 0) as i64, 1, &mut failures);

    let file = openat(dfd, FILE.as_ptr(), O_RDWR | O_CREAT | O_TRUNC, 0o644);
    check(b"openat-relative-file", (file >= 0) as i64, 1, &mut failures);
    if file >= 0 { let _ = close(file); }
    let mut st = [0u8; 256];
    check(b"fstatat-relative-file", fstatat(dfd, FILE.as_ptr(), st.as_mut_ptr(), 0) as i64, 0, &mut failures);

    check(b"openat-relative-escape-fails", openat(dfd, ESCAPE.as_ptr(), O_RDONLY, 0) as i64, -1, &mut failures);
    check(b"fstatat-relative-escape-fails", fstatat(dfd, ESCAPE.as_ptr(), st.as_mut_ptr(), 0) as i64, -1, &mut failures);
    check(b"faccessat-relative-escape-fails", faccessat(dfd, ESCAPE.as_ptr(), R_OK, 0) as i64, -1, &mut failures);
    check(b"mkdirat-relative-escape-fails", mkdirat(dfd, HOST_ESCAPE_DIR.as_ptr(), 0o755) as i64, -1, &mut failures);

    check(b"create-relative-absolute-symlink", symlinkat(ABS_LINK.as_ptr(), dfd, LINK.as_ptr()) as i64, 0, &mut failures);
    let mut link_buf = [0u8; 64];
    let link_len = readlinkat(dfd, LINK.as_ptr(), link_buf.as_mut_ptr(), 63);
    check(b"readlinkat-relative-link", link_len as i64, 15, &mut failures);
    check(b"absolute-symlink-value",
        (link_len == 15 && &link_buf[..15] == b"/etc/os-release") as i64, 1, &mut failures);
    let linked = openat(dfd, LINK.as_ptr(), O_RDONLY, 0);
    check(b"openat-relative-absolute-symlink-contained", (linked >= 0) as i64, 1, &mut failures);
    if linked >= 0 { let _ = close(linked); }

    check(b"fchmodat-relative-file", fchmodat(dfd, FILE.as_ptr(), 0o601, 0) as i64, 0, &mut failures);
    check(b"fstatat-relative-file-after-chmod", fstatat(dfd, FILE.as_ptr(), st.as_mut_ptr(), 0) as i64, 0, &mut failures);
    check(b"relative-file-mode", mode(&st) as i64, 0o601, &mut failures);
    let times = [1_600_000_000i64, 0, 1_600_000_000i64, 0];
    check(b"utimensat-relative-file", utimensat(dfd, FILE.as_ptr(), times.as_ptr(), 0) as i64, 0, &mut failures);

    let _ = unlinkat(dfd, LINK.as_ptr(), 0);
    let _ = unlinkat(dfd, FILE.as_ptr(), 0);
    let _ = close(dfd);
    let _ = rmdir(BASE.as_ptr());
    out(b"#checks=18 fail=");
    num(failures as i64);
    out(b"\n");
    core::arch::asm!("mov x8, #94", "svc #0", in("x0") failures as usize, options(noreturn));
}

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    unsafe {
        out(b"PANIC\n");
        core::arch::asm!("mov x8, #94", "svc #0", in("x0") 99usize, options(noreturn));
    }
}

#[no_mangle]
pub extern "C" fn rust_eh_personality() {}
