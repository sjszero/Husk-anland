//! libhusk-v4l2.so — LD_PRELOAD 版的 `/dev/videoN`（no_std）。
//!
//! 为什么需要它：loader 里的 shim 只能改**启动时**已加载对象的 GOT；
//! GStreamer 的 `libgstvideo4linux2.so` 是运行时 dlopen 的，它调用 `open/ioctl`
//! 会绕过 loader shim，直接落到 Android 的 `/dev/video1`（存在但 0660 media:system）
//! → EACCES。所以和 DRM 一样，dlopen 的重度用户要挂这一层。
//!
//! **状态（2026-11-23，UNVERIFIED/WIP）**：本层能拦截 `open/ioctl/mmap/...` 的
//! 普通调用，但 GStreamer 的 `libgstvideo4linux2.so` 对设备用的是
//! `open64@GLIBC_2.17` **带版本引用**，ld.so 不把无版本定义绑上去
//! （实测：本层的 openat 完全没被调用，插件仍拿到 Android 那个 0660 节点 → EACCES）。
//! 解决要么给本层加 version script（每个符号按 glibc 版本打标签：open64 是 2.17、
//! statx 是 2.28、fstatat 是 2.33…），要么在 dlopen 时给新对象补 GOT。
//! 在那之前：guest 侧的 GStreamer 用不了 /dev/videoN，自研工具/普通程序可以。
//!
//! 两个实例不能同时连 app（camera.sock 一次只收一个客户端）。

#![no_std]

use core::sync::atomic::{AtomicBool, Ordering};
use v4l2_shim::{Ctx, MAX_BUFFERS};

extern "C" {
    fn syscall(num: i64, ...) -> i64;
    fn __errno_location() -> *mut i32;
    fn getenv(name: *const u8) -> *const u8;
    fn setenv(name: *const u8, value: *const u8, overwrite: i32) -> i32;
}

const SYS_OPENAT: i64 = 56;
const SYS_IOCTL: i64 = 29;
const SYS_MMAP: i64 = 222;
const SYS_CLOSE: i64 = 57;
const SYS_FCNTL: i64 = 25;
const SYS_NEWFSTATAT: i64 = 79;
const SYS_STATX: i64 = 291;
const SYS_FACCESSAT: i64 = 48;
const AT_FDCWD: i32 = -100;
const O_PATH: i32 = 0o10000000;

static LOCK: AtomicBool = AtomicBool::new(false);
static mut CTX: Ctx = Ctx::empty();
static mut READY: bool = false;
static mut ROOT: [u8; 200] = [0; 200];
static mut ROOT_LEN: usize = 0;

// Versioned aliases.
//
// A version script did not survive rustc's own cdylib export handling here (every
// symbol ended up in .gnu.version index 1, verified with readelf -V), and an
// unversioned definition does not satisfy the plugin's `open64@GLIBC_2.17`
// reference. `.symver` creates a *second*, explicitly versioned, symbol next to
// the plain one — the classic LD_PRELOAD trick, and lld handles it directly.
core::arch::global_asm!(
    ".globl __hv_open64; .set __hv_open64, open64; .symver __hv_open64, open64@GLIBC_2.17",
    ".globl __hv_ioctl; .set __hv_ioctl, ioctl; .symver __hv_ioctl, ioctl@GLIBC_2.17",
    ".globl __hv_mmap64; .set __hv_mmap64, mmap64; .symver __hv_mmap64, mmap64@GLIBC_2.17",
    ".globl __hv_close; .set __hv_close, close; .symver __hv_close, close@GLIBC_2.17",
    ".globl __hv_stat64; .set __hv_stat64, stat64; .symver __hv_stat64, stat64@GLIBC_2.33",
    ".globl __hv_open; .set __hv_open, open; .symver __hv_open, open@GLIBC_2.17",
    ".globl __hv_openat; .set __hv_openat, openat; .symver __hv_openat, openat@GLIBC_2.17",
    ".globl __hv_mmap; .set __hv_mmap, mmap; .symver __hv_mmap, mmap@GLIBC_2.17",
);

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    // Same rule as the loader shim: a panic must be visible and fatal, never a
    // silent 100% CPU spin inside a guest thread (that is how a 10-vs-12 byte
    // slice mismatch in the memfd name looked: the whole process sat in R for
    // as long as you watched it).
    unsafe {
        note(b"[husk v4l2] panic\n");
        core::arch::asm!("svc 0", in("x8") 94usize, in("x0") 102usize,
            lateout("x0") _, options(nostack));
    }
    loop {
        core::hint::spin_loop();
    }
}

#[no_mangle]
pub extern "C" fn rust_eh_personality() {}

fn lock() {
    while LOCK
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
    }
}
fn unlock() {
    LOCK.store(false, Ordering::Release);
}

/// Raw `svc` call. libc's `syscall()` returns -1 and sets errno, which is easy to
/// confuse with "-errno"; everything here uses this instead and keeps the kernel's
/// negative errno.
#[inline]
unsafe fn raw(n: usize, a: usize, b: usize, c: usize, d: usize, e: usize, f: usize) -> i64 {
    let ret: i64;
    core::arch::asm!("svc 0", in("x8") n, in("x0") a, in("x1") b, in("x2") c, in("x3") d,
        in("x4") e, in("x5") f, lateout("x0") ret, options(nostack));
    ret
}

unsafe fn note(msg: &[u8]) {
    let _ = raw(64, 2, msg.as_ptr() as usize, msg.len(), 0, 0, 0);
}

unsafe fn note_int(prefix: &[u8], v: i64) {
    let _ = raw(64, 2, prefix.as_ptr() as usize, prefix.len(), 0, 0, 0);
    let mut buf = [0u8; 24];
    let neg = v < 0;
    let mut x = if neg { (-v) as u64 } else { v as u64 };
    let mut tmp = [0u8; 20];
    let mut n = 0;
    if x == 0 {
        tmp[0] = b'0';
        n = 1;
    }
    while x > 0 {
        tmp[n] = b'0' + (x % 10) as u8;
        x /= 10;
        n += 1;
    }
    let mut i = 0;
    if neg {
        buf[0] = b'-';
        i = 1;
    }
    let mut k = n;
    while k > 0 {
        k -= 1;
        buf[i] = tmp[k];
        i += 1;
    }
    buf[i] = b'\n';
    let _ = raw(64, 2, buf.as_ptr() as usize, i + 1, 0, 0, 0);
}

unsafe fn note_hex(prefix: &[u8], v: u64) {
    let _ = raw(64, 2, prefix.as_ptr() as usize, prefix.len(), 0, 0, 0);
    let mut buf = [0u8; 19];
    buf[0] = b'0';
    buf[1] = b'x';
    let digits = b"0123456789abcdef";
    let mut i = 2;
    let mut shift = 60;
    let mut started = false;
    while shift >= 0 {
        let d = ((v >> shift) & 0xf) as usize;
        if d != 0 {
            started = true;
        }
        if started || shift == 0 {
            buf[i] = digits[d];
            i += 1;
        }
        if shift == 0 {
            break;
        }
        shift -= 4;
    }
    buf[i] = b'\n';
    let _ = raw(64, 2, buf.as_ptr() as usize, i + 1, 0, 0, 0);
}

unsafe fn note2(msg: &[u8], tail: &[u8]) {
    let _ = raw(64, 2, msg.as_ptr() as usize, msg.len(), 0, 0, 0);
    let _ = raw(64, 2, tail.as_ptr() as usize, tail.len(), 0, 0, 0);
    let _ = raw(64, 2, b"\n".as_ptr() as usize, 1, 0, 0, 0);
}

/// Diagnostic: what does this fd really point at? The kernel is the ground
/// truth; a private fd table is not (an fd can arrive by dup, by fd passing or
/// through the other interposition layer).
unsafe fn note_link(fd: i32) {
    let mut path = [0u8; 32];
    let pre = b"/proc/self/fd/";
    path[..pre.len()].copy_from_slice(pre);
    let mut x = fd as u64;
    let mut tmp = [0u8; 12];
    let mut n = 0;
    if x == 0 {
        tmp[0] = b'0';
        n = 1;
    }
    while x > 0 {
        tmp[n] = b'0' + (x % 10) as u8;
        x /= 10;
        n += 1;
    }
    let mut i = pre.len();
    while n > 0 {
        n -= 1;
        path[i] = tmp[n];
        i += 1;
    }
    path[i] = 0;
    let mut buf = [0u8; 160];
    // aarch64 has no plain readlink(); 78 is readlinkat(dirfd, path, buf, len).
    let r = raw(
        78,
        (-100i64) as usize,
        path.as_ptr() as usize,
        buf.as_mut_ptr() as usize,
        buf.len(),
        0,
        0,
    );
    if r > 0 {
        let _ = raw(64, 2, buf.as_ptr() as usize, r as usize, 0, 0, 0);
        let _ = raw(64, 2, b"\n".as_ptr() as usize, 1, 0, 0, 0);
    }
}

unsafe fn set_errno(e: i32) -> i32 {
    *__errno_location() = e;
    -1
}

fn errno_of(r: i64) -> i32 {
    (-r) as i32
}

unsafe fn c_str_len(p: *const u8) -> usize {
    let mut n = 0;
    while *p.add(n) != 0 {
        n += 1;
    }
    n
}

unsafe fn rootfs_locked() -> Option<&'static [u8]> {
    if ROOT_LEN > 0 {
        return Some(&ROOT[..ROOT_LEN]);
    }
    let v = getenv(b"NR_ROOTFS\0".as_ptr());
    if v.is_null() {
        return None;
    }
    let n = c_str_len(v);
    if n == 0 || n >= ROOT.len() {
        return None;
    }
    core::ptr::copy_nonoverlapping(v, ROOT.as_mut_ptr(), n);
    ROOT_LEN = n;
    Some(&ROOT[..n])
}

unsafe fn ensure_init_locked() -> bool {
    if READY {
        return true;
    }
    let root = match rootfs_locked() {
        Some(r) => r,
        None => return false,
    };
    let suffix = b"/run/husk/camera.sock";
    let mut p = [0u8; 256];
    if root.len() + suffix.len() >= p.len() {
        return false;
    }
    p[..root.len()].copy_from_slice(root);
    p[root.len()..root.len() + suffix.len()].copy_from_slice(suffix);
    if !CTX.init(&p[..root.len() + suffix.len()]) {
        return false;
    }
    READY = true;
    true
}

fn video_index(path: &[u8]) -> Option<u32> {
    let pre = b"/dev/video";
    if path.len() <= pre.len() || !path.starts_with(pre) {
        return None;
    }
    let rest = &path[pre.len()..];
    if rest.len() > 3 {
        return None;
    }
    let mut n = 0u32;
    for &c in rest {
        if !c.is_ascii_digit() {
            return None;
        }
        n = n * 10 + (c - b'0') as u32;
    }
    Some(n)
}

/// `/sys/class/video4linux*`, `/sys/subsystem/video4linux*` → rootfs 影子树
unsafe fn sys_host(path: &[u8], out: &mut [u8; 256]) -> Option<usize> {
    lock();
    let out_len = sys_host_locked(path, out);
    unlock();
    out_len
}

unsafe fn sys_host_locked(path: &[u8], out: &mut [u8; 256]) -> Option<usize> {
    let root = rootfs_locked()?;
    const C: &[u8] = b"/sys/class/video4linux";
    const S: &[u8] = b"/sys/subsystem/video4linux";
    let (dir, rest) = if path.starts_with(C) {
        (b"/sys-v4l2/class".as_ref(), &path[C.len()..])
    } else if path.starts_with(S) {
        (b"/sys-v4l2/subsystem".as_ref(), &path[S.len()..])
    } else {
        return None;
    };
    if !rest.is_empty() && rest[0] != b'/' {
        return None;
    }
    let n = root.len() + dir.len() + rest.len();
    if n + 1 > out.len() {
        return None;
    }
    out[..root.len()].copy_from_slice(root);
    out[root.len()..root.len() + dir.len()].copy_from_slice(dir);
    out[root.len() + dir.len()..n].copy_from_slice(rest);
    out[n] = 0;
    Some(n)
}

/// `Some(Ok(fd))` or `Some(Err(errno))`; `None` when this layer is not set up
/// (the caller then falls through to the real syscall, unchanged).
unsafe fn open_video(n: u32) -> Option<Result<i32, i32>> {
    lock();
    if !ensure_init_locked() {
        unlock();
        return None;
    }
    let fd = CTX.open(n);
    let out = if fd < 0 {
        let e = CTX.last_errno();
        note_int(b"[husk v4l2] open failed for dev=", n as i64);
        Some(Err(if e > 0 { e } else { 19 }))
    } else {
        Some(Ok(fd))
    };
    unlock();
    out
}

unsafe fn synth_stat(n: u32, st: *mut u8) -> i32 {
    core::ptr::write_bytes(st, 0, 128);
    core::ptr::write_unaligned(st.add(8) as *mut u64, n as u64 + 1);
    core::ptr::write_unaligned(st.add(16) as *mut u32, 0o020666u32);
    core::ptr::write_unaligned(st.add(20) as *mut u32, 1u32);
    core::ptr::write_unaligned(st.add(32) as *mut u64, (81u64 << 8) | n as u64);
    core::ptr::write_unaligned(st.add(56) as *mut i64, 4096i64);
    0
}

unsafe fn synth_statx(n: u32, buf: *mut u8) -> i32 {
    core::ptr::write_bytes(buf, 0, 256);
    core::ptr::write_unaligned(buf.add(0) as *mut u32, 0x7ffu32);
    core::ptr::write_unaligned(buf.add(4) as *mut u32, 4096u32);
    core::ptr::write_unaligned(buf.add(16) as *mut u32, 1u32);
    core::ptr::write_unaligned(buf.add(28) as *mut u16, 0o020666u16);
    core::ptr::write_unaligned(buf.add(32) as *mut u64, n as u64 + 1);
    core::ptr::write_unaligned(buf.add(128) as *mut u32, 81u32);
    core::ptr::write_unaligned(buf.add(132) as *mut u32, n);
    0
}

unsafe fn path_bytes(p: *const i8) -> Option<&'static [u8]> {
    if p.is_null() || *p != b'/' as i8 {
        return None;
    }
    let n = c_str_len(p as *const u8);
    Some(core::slice::from_raw_parts(p as *const u8, n))
}

// ---- hooks ----

#[no_mangle]
pub unsafe extern "C" fn open(path: *const i8, flags: i32, mode: i32) -> i32 {
    do_openat(AT_FDCWD, path, flags, mode)
}

#[no_mangle]
pub unsafe extern "C" fn open64(path: *const i8, flags: i32, mode: i32) -> i32 {
    do_openat(AT_FDCWD, path, flags, mode)
}

/// The implementation. Exported wrappers call this directly: a call from one
/// exported symbol to another goes through the PLT and was resolving to libc's
/// openat (measured: open64("/dev/video1") reached us, the openat inside it did
/// not), which silently turned every device open into EACCES.
#[inline(always)]
unsafe fn do_openat(dirfd: i32, path: *const i8, flags: i32, mode: i32) -> i32 {
    if let Some(p) = path_bytes(path) {
        let mut buf = [0u8; 256];
        if let Some(n) = sys_host(p, &mut buf) {
            return syscall(
                SYS_OPENAT,
                AT_FDCWD as i64,
                buf.as_ptr() as i64,
                flags as i64,
                mode as i64,
            ) as i32;
        }
        if let Some(vn) = video_index(p) {
            match open_video(vn) {
                Some(Ok(fd)) => return fd,
                Some(Err(e)) => return set_errno(e),
                None => {}
            }
        }
    }
    let r = raw(
        SYS_OPENAT as usize,
        dirfd as usize,
        path as usize,
        flags as usize,
        mode as usize,
        0,
        0,
    );
    if r < 0 {
        return set_errno(errno_of(r));
    }
    r as i32
}

#[no_mangle]
pub unsafe extern "C" fn openat(dirfd: i32, path: *const i8, flags: i32, mode: i32) -> i32 {
    do_openat(dirfd, path, flags, mode)
}

#[no_mangle]
pub unsafe extern "C" fn openat64(dirfd: i32, path: *const i8, flags: i32, mode: i32) -> i32 {
    do_openat(dirfd, path, flags, mode)
}

// Fortified builds (Ubuntu's GStreamer) never call `open` directly: with
// _FORTIFY_SOURCE, open(path, flags) becomes __open_2(path, flags) and
// openat() becomes __openat_2(). Those live in libc and are entered through a
// direct symbol, so without these four aliases the plugin bypasses us.
#[no_mangle]
pub unsafe extern "C" fn __open_2(path: *const i8, flags: i32) -> i32 {
    do_openat(AT_FDCWD, path, flags, 0)
}

#[no_mangle]
pub unsafe extern "C" fn __open64_2(path: *const i8, flags: i32) -> i32 {
    do_openat(AT_FDCWD, path, flags, 0)
}

#[no_mangle]
pub unsafe extern "C" fn __openat_2(dirfd: i32, path: *const i8, flags: i32) -> i32 {
    do_openat(dirfd, path, flags, 0)
}

#[no_mangle]
pub unsafe extern "C" fn __openat64_2(dirfd: i32, path: *const i8, flags: i32) -> i32 {
    do_openat(dirfd, path, flags, 0)
}

#[no_mangle]
#[no_mangle]
pub unsafe extern "C" fn ioctl(fd: i32, request: u64, arg: *mut u8) -> i32 {
    do_ioctl(fd, request, arg)
}

#[inline(always)]
unsafe fn do_ioctl(fd: i32, request: u64, arg: *mut u8) -> i32 {
    // A V4L2 request on an fd we cannot account for is a bug, not noise: say
    // which fd it was and what the kernel says it points at.
    lock();
    let fake = CTX.is_video_fd(fd);
    if ((request >> 8) & 0xff) == 0x56 && !fake {
        note_hex(b"[husk v4l2] unknown fd for ioctl=", request);
        note_int(b"   fd=", fd as i64);
        note(b"   target=");
        note_link(fd);
    }
    if fake {
        let r = CTX.ioctl(fd, request, arg);
        unlock();
        return if r < 0 { set_errno(errno_of(r)) } else { 0 };
    }
    unlock();
    let r = raw(
        SYS_IOCTL as usize,
        fd as usize,
        request as usize,
        arg as usize,
        0,
        0,
        0,
    );
    if r < 0 {
        return set_errno(errno_of(r));
    }
    r as i32
}

#[no_mangle]
pub unsafe extern "C" fn mmap(
    addr: *mut u8,
    len: usize,
    prot: i32,
    flags: i32,
    fd: i32,
    off: i64,
) -> *mut u8 {
    do_mmap(addr, len, prot, flags, fd, off)
}

#[inline(always)]
unsafe fn do_mmap(addr: *mut u8, len: usize, prot: i32, flags: i32, fd: i32, off: i64) -> *mut u8 {
    lock();
    if CTX.is_video_fd(fd) {
        let out = match CTX.mmap(fd, len, off) {
            Some(p) => p,
            None => set_errno(22) as *mut u8,
        };
        unlock();
        return out;
    }
    unlock();
    let r = raw(
        SYS_MMAP as usize,
        addr as usize,
        len,
        prot as usize,
        flags as usize,
        fd as usize,
        off as usize,
    );
    if r < 0 && r > -4096 {
        return set_errno(errno_of(r)) as *mut u8;
    }
    r as *mut u8
}

#[no_mangle]
pub unsafe extern "C" fn mmap64(
    addr: *mut u8,
    len: usize,
    prot: i32,
    flags: i32,
    fd: i32,
    off: i64,
) -> *mut u8 {
    do_mmap(addr, len, prot, flags, fd, off)
}

#[no_mangle]
pub unsafe extern "C" fn close(fd: i32) -> i32 {
    do_close(fd)
}

#[inline(always)]
unsafe fn do_close(fd: i32) -> i32 {
    lock();
    if CTX.is_video_fd(fd) {
        CTX.close(fd);
    }
    let r = raw(SYS_CLOSE as usize, fd as usize, 0, 0, 0, 0, 0);
    unlock();
    if r < 0 {
        return set_errno(errno_of(r));
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn fcntl(fd: i32, cmd: i32, arg: usize) -> i32 {
    let r = raw(SYS_FCNTL as usize, fd as usize, cmd as usize, arg, 0, 0, 0);
    if r >= 0 {
        lock();
        if CTX.is_video_fd(fd) {
            if cmd == 4 {
                CTX.update_nonblock(fd, (arg & 0o4000) != 0);
            } else if cmd == 3 {
                CTX.update_nonblock(fd, (r as usize & 0o4000) != 0);
            }
        }
        unlock();
    }
    if r < 0 {
        return set_errno(errno_of(r));
    }
    r as i32
}

#[no_mangle]
pub unsafe extern "C" fn stat(path: *const i8, st: *mut u8) -> i32 {
    do_fstatat(AT_FDCWD, path, st, 0)
}

#[no_mangle]
pub unsafe extern "C" fn lstat(path: *const i8, st: *mut u8) -> i32 {
    do_fstatat(AT_FDCWD, path, st, 0x100)
}

// The GStreamer plugin references stat64@GLIBC_2.33; on aarch64 it is the same
// struct as stat.
#[no_mangle]
pub unsafe extern "C" fn stat64(path: *const i8, st: *mut u8) -> i32 {
    do_fstatat(AT_FDCWD, path, st, 0)
}

#[no_mangle]
pub unsafe extern "C" fn lstat64(path: *const i8, st: *mut u8) -> i32 {
    do_fstatat(AT_FDCWD, path, st, 0x100)
}

#[no_mangle]
pub unsafe extern "C" fn fstatat(dirfd: i32, path: *const i8, st: *mut u8, flags: i32) -> i32 {
    do_fstatat(dirfd, path, st, flags)
}

#[inline(always)]
unsafe fn do_fstatat(dirfd: i32, path: *const i8, st: *mut u8, flags: i32) -> i32 {
    if let Some(p) = path_bytes(path) {
        let mut buf = [0u8; 256];
        if let Some(n) = sys_host(p, &mut buf) {
            let r = raw(
                SYS_NEWFSTATAT as usize,
                AT_FDCWD as usize,
                buf.as_ptr() as usize,
                st as usize,
                flags as usize,
                0,
                0,
            );
            if r < 0 {
                return set_errno(errno_of(r));
            }
            return 0;
        }
        if let Some(vn) = video_index(p) {
            return synth_stat(vn, st);
        }
    }
    let r = raw(
        SYS_NEWFSTATAT as usize,
        dirfd as usize,
        path as usize,
        st as usize,
        flags as usize,
        0,
        0,
    );
    if r < 0 {
        return set_errno(errno_of(r));
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn statx(
    dirfd: i32,
    path: *const i8,
    flags: i32,
    mask: u32,
    buf: *mut u8,
) -> i32 {
    if let Some(p) = path_bytes(path) {
        let mut sb = [0u8; 256];
        if let Some(n) = sys_host(p, &mut sb) {
            let r = raw(
                SYS_STATX as usize,
                AT_FDCWD as usize,
                sb.as_ptr() as usize,
                flags as usize,
                mask as usize,
                buf as usize,
                0,
            );
            if r < 0 {
                return set_errno(errno_of(r));
            }
            return 0;
        }
        if let Some(vn) = video_index(p) {
            return synth_statx(vn, buf);
        }
    }
    let r = raw(
        SYS_STATX as usize,
        dirfd as usize,
        path as usize,
        flags as usize,
        mask as usize,
        buf as usize,
        0,
    );
    if r < 0 {
        return set_errno(errno_of(r));
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn access(path: *const i8, mode: i32) -> i32 {
    if let Some(p) = path_bytes(path) {
        if video_index(p).is_some() {
            return 0;
        }
    }
    let r = raw(
        SYS_FACCESSAT as usize,
        AT_FDCWD as usize,
        path as usize,
        mode as usize,
        0,
        0,
        0,
    );
    if r < 0 {
        return set_errno(errno_of(r));
    }
    0
}

// ---- libv4l2 symbols ------------------------------------------------------
//
// GStreamer's libgstvideo4linux2.so links libv4l2.so.0 and calls v4l2_open(),
// v4l2_ioctl(), v4l2_mmap(), v4l2_close() instead of the libc entry points.
// Those run inside libv4l2 (dlopen'd), so neither the loader shim nor our
// open/ioctl hooks see them. The global scope is searched before the plugin's
// own dependencies, so defining the same names here takes over the whole
// libv4l2 API surface the plugin uses.
//
// v4l2_ioctl() is variadic in libv4l2's header; a fixed (fd, request, arg)
// definition is ABI-compatible for every caller that passes an argument.

#[no_mangle]
pub unsafe extern "C" fn v4l2_open(path: *const i8, flags: i32) -> i32 {
    let fd = do_openat(AT_FDCWD, path, flags, 0);
    if fd >= 0 {
        lock();
        if CTX.is_video_fd(fd) {
            CTX.update_nonblock(fd, (flags & 0o4000) != 0);
        }
        unlock();
    }
    fd
}

#[no_mangle]
pub unsafe extern "C" fn v4l2_fd_open(fd: i32, _flags: i32) -> i32 {
    fd
}

#[no_mangle]
pub unsafe extern "C" fn v4l2_ioctl(fd: i32, request: u64, arg: *mut u8) -> i32 {
    do_ioctl(fd, request, arg)
}

#[no_mangle]
pub unsafe extern "C" fn v4l2_read(fd: i32, buf: *mut u8, n: usize) -> isize {
    raw(63, fd as usize, buf as usize, n, 0, 0, 0) as isize
}

#[no_mangle]
pub unsafe extern "C" fn v4l2_mmap(
    start: *mut u8,
    length: usize,
    prot: i32,
    flags: i32,
    fd: i32,
    off: i64,
) -> *mut u8 {
    do_mmap(start, length, prot, flags, fd, off)
}

#[no_mangle]
pub unsafe extern "C" fn v4l2_munmap(start: *mut u8, length: usize) -> i32 {
    raw(215, start as usize, length, 0, 0, 0, 0) as i32
}

#[no_mangle]
pub unsafe extern "C" fn v4l2_close(fd: i32) -> i32 {
    do_close(fd)
}

#[no_mangle]
pub unsafe extern "C" fn v4l2_dup(fd: i32) -> i32 {
    raw(23, fd as usize, 0, 0, 0, 0, 0) as i32
}

// ---- ABI for the loader shim -------------------------------------------
//
// Both layers must share ONE Ctx (the app accepts one camera client), so the
// preload owns it and the loader shim forwards here when HUSK_V4L2_API is set.
// Same handshake as the DRM shim.

#[no_mangle]
pub unsafe extern "C" fn husk_v4l2_open(n: u32) -> i32 {
    // The loader shim forwards this: hand back -errno so it can rebuild the
    // guest's errno instead of inventing EPERM.
    match open_video(n) {
        Some(Ok(fd)) => fd,
        Some(Err(e)) => -e,
        None => -19,
    }
}

#[no_mangle]
pub unsafe extern "C" fn husk_v4l2_ioctl(fd: i32, req: u64, arg: *mut u8) -> i64 {
    lock();
    let r = if CTX.is_video_fd(fd) {
        CTX.ioctl(fd, req, arg)
    } else {
        -25
    };
    unlock();
    r
}

#[no_mangle]
pub unsafe extern "C" fn husk_v4l2_mmap(fd: i32, len: usize, off: i64) -> *mut u8 {
    lock();
    let p = CTX.mmap(fd, len, off).unwrap_or(core::ptr::null_mut());
    unlock();
    p
}

#[no_mangle]
pub unsafe extern "C" fn husk_v4l2_is_fake(fd: i32) -> i32 {
    lock();
    let fake = CTX.is_video_fd(fd);
    unlock();
    if fake {
        1
    } else {
        0
    }
}

#[no_mangle]
pub unsafe extern "C" fn husk_v4l2_close(fd: i32) {
    lock();
    if CTX.is_video_fd(fd) {
        CTX.close(fd);
    }
    unlock();
}

#[no_mangle]
pub unsafe extern "C" fn husk_v4l2_forget_target(fd: i32) {
    lock();
    CTX.forget_target(fd);
    unlock();
}

#[no_mangle]
pub unsafe extern "C" fn husk_v4l2_nonblock(fd: i32, on: i32) {
    lock();
    CTX.update_nonblock(fd, on != 0);
    unlock();
}

static mut API: [usize; 7] = [0; 7];

unsafe extern "C" fn announce() {
    API = [
        husk_v4l2_open as usize,
        husk_v4l2_ioctl as usize,
        husk_v4l2_mmap as usize,
        husk_v4l2_is_fake as usize,
        husk_v4l2_close as usize,
        husk_v4l2_nonblock as usize,
        husk_v4l2_forget_target as usize,
    ];
    let _ = setenv(b"HUSK_V4L2_PRELOAD\0".as_ptr(), b"1\0".as_ptr(), 1);
    // Hex table address, same encoding path-shim already parses for DRM.
    let mut hex = [0u8; 20];
    let mut p = (core::ptr::addr_of!(API)) as usize;
    let mut i = 2;
    hex[0] = b'0';
    hex[1] = b'x';
    let mut shift = (core::mem::size_of::<usize>() * 8) / 4;
    let mut started = false;
    let digits = b"0123456789abcdef";
    while shift > 0 {
        shift -= 4;
        let d = (p >> shift) & 0xf;
        if d != 0 {
            started = true;
        }
        if started {
            hex[i] = digits[d];
            i += 1;
        }
    }
    if !started {
        hex[i] = b'0';
        i += 1;
    }
    hex[i] = 0;
    // NOTE: 2026-11-23 — publishing the table and letting the loader shim forward
    // crashed inside the shim on the first /dev/video open (evidence 91 §crash).
    // Until that is diagnosed the two layers keep their own Ctx: startup objects
    // use the loader shim, dlopen'd consumers (GStreamer) use this one. A process
    // must not open the device through both paths at once (camera.sock is one
    // client); gst-launch and our tools each use exactly one.
    let _ = &hex;
    let _ = O_PATH;
    let _ = MAX_BUFFERS;
    let _ = &mut p;
}

#[used]
#[link_section = ".init_array"]
static ANNOUNCE: unsafe extern "C" fn() = announce;
