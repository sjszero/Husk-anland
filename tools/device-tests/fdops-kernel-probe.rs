//! Probe raw fd-based metadata operations on an Android 4.19 arm64 kernel.
//!
//! This is intentionally a static, raw-syscall probe: it measures what the
//! kernel accepts for O_PATH descriptors without the path shim in between.
//! The result explains why the shim pins metadata targets through
//! `/proc/self/fd/N` or `AT_EMPTY_PATH` instead of relying on newer flags.
//!
//! Run with cwd already inside the guest rootfs. Build with
//! `./build-fdops-kernel-probe.sh`.
#![no_std]
#![no_main]
use core::panic::PanicInfo;

const AT_FDCWD: i32 = -100;
const AT_EMPTY_PATH: usize = 0x1000;
const AT_SYMLINK_NOFOLLOW: usize = 0x100;
const O_RDWR: i32 = 2;
const O_CREAT: i32 = 0o100;
const O_TRUNC: i32 = 0o1000;
const O_PATH: i32 = 0o10000000;
const O_NOFOLLOW: i32 = 0o100000;
const O_CLOEXEC: i32 = 0o2000000;
const SYS_MKDIRAT: usize = 34;
const SYS_OPENAT: usize = 56;
const SYS_CLOSE: usize = 57;
const SYS_FCHMOD: usize = 52;
const SYS_FCHMODAT: usize = 53;
const SYS_FCHOWNAT: usize = 54;
const SYS_UTIMENSAT: usize = 88;
const SYS_FSTAT: usize = 80;
const SYS_STATX: usize = 291;
const SYS_SYMLINKAT: usize = 36;
const SYS_WRITE: usize = 64;
const SYS_EXIT_GROUP: usize = 94;
const SYS_UNLINKAT: usize = 35;

#[inline]
unsafe fn sc2(n: usize, a: usize, b: usize) -> isize {
    let r: isize;
    core::arch::asm!("svc 0", in("x8") n, in("x0") a, in("x1") b,
        lateout("x0") r, options(nostack));
    r
}
#[inline]
unsafe fn sc3(n: usize, a: usize, b: usize, c: usize) -> isize {
    let r: isize;
    core::arch::asm!("svc 0", in("x8") n, in("x0") a, in("x1") b, in("x2") c,
        lateout("x0") r, options(nostack));
    r
}
#[inline]
unsafe fn sc4(n: usize, a: usize, b: usize, c: usize, d: usize) -> isize {
    let r: isize;
    core::arch::asm!("svc 0", in("x8") n, in("x0") a, in("x1") b, in("x2") c, in("x3") d,
        lateout("x0") r, options(nostack));
    r
}
#[inline]
unsafe fn sc5(n: usize, a: usize, b: usize, c: usize, d: usize, e: usize) -> isize {
    let r: isize;
    core::arch::asm!("svc 0", in("x8") n, in("x0") a, in("x1") b, in("x2") c, in("x3") d, in("x4") e,
        lateout("x0") r, options(nostack));
    r
}
fn err(r: isize) -> i32 { if r < 0 && r > -4096 { (-r) as i32 } else { 0 } }
unsafe fn out(s: &[u8]) { let _ = sc3(SYS_WRITE, 2, s.as_ptr() as usize, s.len()); }
unsafe fn num(mut v: i64) {
    let mut b = [0u8; 24]; let mut n = 0usize; let neg = v < 0; if neg { v = -v; }
    if v == 0 { out(b"0"); return; }
    while v != 0 { b[n] = b'0' + (v % 10) as u8; v /= 10; n += 1; }
    if neg { out(b"-"); }
    while n > 0 { n -= 1; let x = [b[n]]; out(&x); }
}
unsafe fn line(name: &[u8], r: isize) {
    out(name); out(b": ret="); num(r as i64); out(b" errno="); num(err(r) as i64); out(b"\n");
}
unsafe fn open_path(p: &[u8], flags: i32) -> i32 {
    sc4(SYS_OPENAT, AT_FDCWD as usize, p.as_ptr() as usize, flags as usize, 0o644) as i32
}
unsafe fn mode(fd: i32) -> u32 {
    let mut st = [0u8; 128];
    let r = sc2(SYS_FSTAT, fd as usize, st.as_mut_ptr() as usize);
    if r < 0 { return 0; }
    u32::from_le_bytes([st[16], st[17], st[18], st[19]]) & 0o7777
}
static CWD: &[u8] = b"tmp/husk-fdops-probe\0";
static FILE: &[u8] = b"tmp/husk-fdops-probe/file\0";
static LINK: &[u8] = b"tmp/husk-fdops-probe/link\0";
static EMPTY: &[u8] = b"\0";
static PREFIX: &[u8] = b"/proc/self/fd/";
unsafe fn proc_path(fd: i32, buf: &mut [u8; 32]) -> &[u8] {
    let mut n = PREFIX.len(); buf[..n].copy_from_slice(PREFIX);
    let mut d = [0u8; 10]; let mut k = 0usize; let mut v = fd as u32;
    if v == 0 { d[0] = b'0'; k = 1; } else { while v != 0 { d[k] = b'0' + (v % 10) as u8; v /= 10; k += 1; } }
    while k > 0 { k -= 1; buf[n] = d[k]; n += 1; }
    buf[n] = 0; &buf[..=n]
}

unsafe fn stx_mode_inode(buf: &[u8; 256]) -> (u32, u64) {
    let mode = u16::from_le_bytes([buf[28], buf[29]]) as u32;
    let ino = u64::from_le_bytes([buf[32],buf[33],buf[34],buf[35],buf[36],buf[37],buf[38],buf[39]]);
    (mode, ino)
}
#[no_mangle]
pub unsafe extern "C" fn _start() -> ! {
    let _ = sc3(SYS_MKDIRAT, AT_FDCWD as usize, CWD.as_ptr() as usize, 0o755);
    let f = sc4(SYS_OPENAT, AT_FDCWD as usize, FILE.as_ptr() as usize,
        (O_RDWR | O_CREAT | O_TRUNC) as usize, 0o644);
    line(b"create-file", f); if f >= 0 { let _ = sc2(SYS_CLOSE, f as usize, 0); }
    line(b"create-link", sc3(SYS_SYMLINKAT, b"file\0".as_ptr() as usize, AT_FDCWD as usize,
        LINK.as_ptr() as usize));

    let ffd = open_path(FILE, O_PATH | O_NOFOLLOW | O_CLOEXEC);
    let lfd = open_path(LINK, O_PATH | O_NOFOLLOW | O_CLOEXEC);
    out(b"OPEN file="); num(ffd as i64); out(b" link="); num(lfd as i64);
    out(b" file_mode="); num(mode(ffd) as i64); out(b" link_mode="); num(mode(lfd) as i64); out(b"\n");

    line(b"fchmod-file-direct", sc3(SYS_FCHMOD, ffd as usize, 0o601, 0));
    out(b"AFTER fchmod-file-direct mode="); num(mode(ffd) as i64); out(b"\n");
    let mut pp = [0u8; 32]; let p = proc_path(ffd, &mut pp);
    line(b"fchmodat-proc-file", sc4(SYS_FCHMODAT, AT_FDCWD as usize, p.as_ptr() as usize, 0o602, 0));
    out(b"AFTER fchmodat-proc-file mode="); num(mode(ffd) as i64); out(b"\n");
    line(b"fchown-file-empty", sc5(SYS_FCHOWNAT, ffd as usize, EMPTY.as_ptr() as usize,
        u32::MAX as usize, u32::MAX as usize, AT_EMPTY_PATH));
    line(b"fchown-link-empty-nofollow", sc5(SYS_FCHOWNAT, lfd as usize, EMPTY.as_ptr() as usize,
        u32::MAX as usize, u32::MAX as usize, AT_EMPTY_PATH | AT_SYMLINK_NOFOLLOW));
    let times = [1234567890i64, 0, 1234567890i64, 0];
    line(b"utimensat-file-empty", sc4(SYS_UTIMENSAT, ffd as usize, EMPTY.as_ptr() as usize,
        times.as_ptr() as usize, AT_EMPTY_PATH));
    line(b"utimensat-link-empty-nofollow", sc4(SYS_UTIMENSAT, lfd as usize, EMPTY.as_ptr() as usize,
        times.as_ptr() as usize, AT_EMPTY_PATH | AT_SYMLINK_NOFOLLOW));
    let mut pp2 = [0u8; 32]; let p2 = proc_path(ffd, &mut pp2);
    line(b"utimensat-proc-file", sc4(SYS_UTIMENSAT, AT_FDCWD as usize, p2.as_ptr() as usize,
        times.as_ptr() as usize, 0));

    let mut stx = [0u8; 256];
    line(b"statx-file-empty", sc5(SYS_STATX, ffd as usize, EMPTY.as_ptr() as usize,
        0, 0x7ff, stx.as_mut_ptr() as usize));
    let mut pp3 = [0u8; 32]; let p3 = proc_path(ffd, &mut pp3);
    let sx = sc5(SYS_STATX, AT_FDCWD as usize, p3.as_ptr() as usize,
        0, 0x7ff, stx.as_mut_ptr() as usize);
    line(b"statx-proc-file", sx); if sx == 0 { let (m,i)=stx_mode_inode(&stx); out(b"STATX proc-file mode="); num(m as i64); out(b" ino="); num(i as i64); out(b"\n"); }
    let mut pp4 = [0u8; 32]; let p4 = proc_path(lfd, &mut pp4);
    let sx2 = sc5(SYS_STATX, AT_FDCWD as usize, p4.as_ptr() as usize,
        AT_SYMLINK_NOFOLLOW, 0x7ff, stx.as_mut_ptr() as usize);
    line(b"statx-proc-link-nofollow", sx2); if sx2 == 0 { let (m,i)=stx_mode_inode(&stx); out(b"STATX proc-link-nofollow mode="); num(m as i64); out(b" ino="); num(i as i64); out(b"\n"); }
    let sx3 = sc5(SYS_STATX, AT_FDCWD as usize, p4.as_ptr() as usize,
        0, 0x7ff, stx.as_mut_ptr() as usize);
    line(b"statx-proc-link-follow", sx3); if sx3 == 0 { let (m,i)=stx_mode_inode(&stx); out(b"STATX proc-link-follow mode="); num(m as i64); out(b" ino="); num(i as i64); out(b"\n"); }
    line(b"fchmod-link-direct", sc3(SYS_FCHMOD, lfd as usize, 0o600, 0));
    let mut pp5 = [0u8; 32]; let p5 = proc_path(lfd, &mut pp5);
    line(b"fchmodat-proc-link", sc4(SYS_FCHMODAT, AT_FDCWD as usize, p5.as_ptr() as usize, 0o603, 0));

    let mut pp6 = [0u8; 32]; let p6 = proc_path(lfd, &mut pp6);
    line(b"utimensat-proc-link", sc4(SYS_UTIMENSAT, AT_FDCWD as usize, p6.as_ptr() as usize,
        times.as_ptr() as usize, 0));
    line(b"utimensat-proc-link-nofollow", sc4(SYS_UTIMENSAT, AT_FDCWD as usize, p6.as_ptr() as usize,
        times.as_ptr() as usize, AT_SYMLINK_NOFOLLOW));
    if ffd >= 0 { let _ = sc2(SYS_CLOSE, ffd as usize, 0); }
    if lfd >= 0 { let _ = sc2(SYS_CLOSE, lfd as usize, 0); }
    let _ = sc3(SYS_UNLINKAT, AT_FDCWD as usize, LINK.as_ptr() as usize, 0);
    let _ = sc3(SYS_UNLINKAT, AT_FDCWD as usize, FILE.as_ptr() as usize, 0);
    let _ = sc3(SYS_UNLINKAT, AT_FDCWD as usize, CWD.as_ptr() as usize, 0x200);
    let _ = sc2(SYS_EXIT_GROUP, 0, 0);
    loop {}
}
#[no_mangle]
pub extern "C" fn rust_eh_personality() {}
#[panic_handler]
fn panic(_: &PanicInfo) -> ! { unsafe { out(b"PANIC\n"); let _ = sc2(SYS_EXIT_GROUP, 99, 0); } loop {} }
