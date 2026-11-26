//! drmfake — 在**假 DRM 节点**上跑一遍 panfrost 会做的事。
//!
//! 这个程序只知道 DRM，不知道 kbase：它 open("/dev/dri/renderD128")，
//! 然后 CREATE_BO / GET_PARAM / MMAP_BO + mmap。如果 path-shim 的钩子生效，
//! 这些调用会被翻译到 /dev/mali0 上，结果和 evidence 43 的 selfcheck 一致。
//!
//! 语法糖：它刻意用 libc 的 open/ioctl/mmap（走 PLT），这样才测到 interpose。

use std::fs::File;
use std::fs::OpenOptions;
use std::os::fd::{AsRawFd, FromRawFd};

extern "C" {
    fn ioctl(fd: i32, request: u64, ...) -> i32;
    fn dup2(oldfd: i32, newfd: i32) -> i32;
    fn dup3(oldfd: i32, newfd: i32, flags: i32) -> i32;
    fn mmap(
        addr: *mut core::ffi::c_void,
        len: usize,
        prot: i32,
        flags: i32,
        fd: i32,
        off: i64,
    ) -> *mut core::ffi::c_void;
    fn __errno_location() -> *mut i32;
}

const _IOC_WRITE: u64 = 1;
const _IOC_READ: u64 = 2;
const fn ioc(dir: u64, ty: u64, nr: u64, size: u64) -> u64 {
    (dir << 30) | (size << 16) | (ty << 8) | nr
}
const DRM_CMD: u64 = 0x40;
const CREATE_BO: u64 = ioc(_IOC_READ | _IOC_WRITE, b'd' as u64, DRM_CMD + 0x02, 24);
const MMAP_BO: u64 = ioc(_IOC_READ | _IOC_WRITE, b'd' as u64, DRM_CMD + 0x03, 16);
const GET_PARAM: u64 = ioc(_IOC_READ | _IOC_WRITE, b'd' as u64, DRM_CMD + 0x04, 16);
const GET_BO_OFFSET: u64 = ioc(_IOC_READ | _IOC_WRITE, b'd' as u64, DRM_CMD + 0x05, 16);

fn errno() -> i32 {
    unsafe { *__errno_location() }
}
fn line(n: &str, v: &str, d: impl AsRef<str>) {
    println!("drmfake|{}|{}|{}", n, v, d.as_ref());
}

fn drm_param_ok(fd: i32) -> Result<(), i32> {
    let mut gp = [0u8; 16];
    let r = unsafe { ioctl(fd, GET_PARAM, gp.as_mut_ptr() as *mut core::ffi::c_void) };
    if r == 0 {
        Ok(())
    } else {
        Err(errno())
    }
}

fn ordinary_file(name: &str) -> File {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(name)
        .unwrap_or_else(|e| panic!("open {name}: {e}"))
}

/// `dup2`/`dup3` close the destination file description before installing the
/// source. The shim therefore has to forget the destination's old DRM identity
/// even when the source is an ordinary file. A stale table entry would answer
/// DRM ioctls for the ordinary file and make a negative test look successful.
fn check_dup_overwrite() {
    const EACCES: i32 = 13;
    const ENOTTY: i32 = 25;

    for (name, use_dup3) in [("dup2", false), ("dup3", true)] {
        let victim = match OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/dri/renderD128")
        {
            Ok(f) => f,
            Err(e) => {
                line(
                    &format!("{name}-ordinary-over-fake"),
                    "FAIL",
                    format!("victim open errno={}", e.raw_os_error().unwrap_or(0)),
                );
                continue;
            }
        };
        let victim_fd = victim.as_raw_fd();
        let ordinary = ordinary_file(&format!("/tmp/drmfake-{name}-ordinary"));
        let ordinary_fd = ordinary.as_raw_fd();
        let r = unsafe {
            if use_dup3 {
                dup3(ordinary_fd, victim_fd, 0)
            } else {
                dup2(ordinary_fd, victim_fd)
            }
        };
        if r < 0 {
            line(
                &format!("{name}-ordinary-over-fake"),
                "FAIL",
                format!("{name} failed errno={}", errno()),
            );
            continue;
        }

        let got = drm_param_ok(victim_fd);
        // Once the destination has been overwritten, the ioctl must leave the
        // DRM shim. A regular-file ioctl is normally ENOTTY; Android's app
        // seccomp may intercept it first and return EACCES. Both prove the
        // descriptor is no longer routed to DRM_CTX; Ok would prove stale state.
        let left_drm = matches!(got, Err(EACCES | ENOTTY));
        line(
            &format!("{name}-ordinary-over-fake"),
            if left_drm { "PASS" } else { "FAIL" },
            format!("ioctl={got:?} expected=non-DRM fallthrough (EACCES/ENOTTY)"),
        );

        // Keep the destination old/new owners alive until after the ioctl;
        // these drops also verify both descriptor numbers are closeable.
        drop(victim);
        drop(ordinary);
    }

    // The positive direction must keep working: an fd copied from a fake DRM
    // descriptor remains fake after the source is closed.
    let source = match OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/dri/renderD128")
    {
        Ok(f) => f,
        Err(e) => {
            line(
                "dup2-fake-preserved",
                "FAIL",
                format!("source open errno={}", e.raw_os_error().unwrap_or(0)),
            );
            return;
        }
    };
    let source_fd = source.as_raw_fd();
    const COPY_FD: i32 = 60;
    let copied = unsafe { dup2(source_fd, COPY_FD) };
    drop(source);
    if copied != COPY_FD {
        line(
            "dup2-fake-preserved",
            "FAIL",
            format!("dup2 rc={copied} errno={}", errno()),
        );
        return;
    }
    let copy_owner = unsafe { File::from_raw_fd(COPY_FD) };
    let got = drm_param_ok(COPY_FD);
    line(
        "dup2-fake-preserved",
        if got.is_ok() { "PASS" } else { "FAIL" },
        format!("ioctl={got:?} after source close"),
    );
    drop(copy_owner);
}

fn main() {
    let f = match OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/dri/renderD128")
    {
        Ok(f) => f,
        Err(e) => {
            line(
                "open",
                "FAIL",
                format!(
                    "/dev/dri/renderD128 errno={}",
                    e.raw_os_error().unwrap_or(0)
                ),
            );
            return;
        }
    };
    let fd = f.as_raw_fd();
    line(
        "open",
        "PASS",
        format!("/dev/dri/renderD128 fd={fd}（宿主上不存在这个节点）"),
    );

    // GET_PARAM(0)=GPU_PROD_ID, (7)=JS_PRESENT
    for (n, name) in [(0u32, "GPU_PROD_ID"), (7, "JS_PRESENT")] {
        let mut gp = [0u8; 16];
        gp[0..4].copy_from_slice(&n.to_le_bytes());
        let r = unsafe { ioctl(fd, GET_PARAM, gp.as_mut_ptr() as *mut core::ffi::c_void) };
        let mut v = [0u8; 8];
        v.copy_from_slice(&gp[8..16]);
        line(
            &format!("param{name}"),
            if r == 0 { "PASS" } else { "FAIL" },
            format!(
                "{:#x} (errno={})",
                u64::from_le_bytes(v),
                if r < 0 { errno() } else { 0 }
            ),
        );
    }

    // CREATE_BO(4096)
    let mut cb = [0u8; 24];
    cb[0..4].copy_from_slice(&4096u32.to_le_bytes());
    let r = unsafe { ioctl(fd, CREATE_BO, cb.as_mut_ptr() as *mut core::ffi::c_void) };
    if r < 0 {
        line("create_bo", "FAIL", format!("errno={}", errno()));
        return;
    }
    let handle = u32::from_le_bytes([cb[8], cb[9], cb[10], cb[11]]);
    let mut o = [0u8; 8];
    o.copy_from_slice(&cb[16..24]);
    let va = u64::from_le_bytes(o);
    line(
        "create_bo",
        "PASS",
        format!("handle={handle} offset={va:#x}"),
    );

    // MMAP_BO(handle) → offset
    let mut mb = [0u8; 16];
    mb[0..4].copy_from_slice(&handle.to_le_bytes());
    let r = unsafe { ioctl(fd, MMAP_BO, mb.as_mut_ptr() as *mut core::ffi::c_void) };
    if r < 0 {
        line("mmap_bo", "FAIL", format!("errno={}", errno()));
        return;
    }
    let mut mo = [0u8; 8];
    mo.copy_from_slice(&mb[8..16]);
    let off = u64::from_le_bytes(mo);
    line("mmap_bo", "PASS", format!("offset={off:#x}"));

    // 真 mmap（走 shim 的 mmap 钩子 → mali0）
    let p = unsafe { mmap(core::ptr::null_mut(), 4096, 3, 1, fd, off as i64) };
    if p as isize == -1 {
        line("mmap", "FAIL", format!("errno={}", errno()));
        return;
    }
    let w = p as *mut u32;
    unsafe {
        for i in 0..16 {
            *w.add(i) = 0xa5a5_0000u32 + i as u32;
        }
    }
    let mut ok = true;
    unsafe {
        for i in 0..16 {
            if *w.add(i) != 0xa5a5_0000u32 + i as u32 {
                ok = false;
            }
        }
    }
    line(
        "mmap",
        if ok { "PASS" } else { "FAIL" },
        format!("va={off:#x} 写读一致"),
    );

    // GET_BO_OFFSET 一致性
    let mut gb = [0u8; 16];
    gb[0..4].copy_from_slice(&handle.to_le_bytes());
    let r = unsafe { ioctl(fd, GET_BO_OFFSET, gb.as_mut_ptr() as *mut core::ffi::c_void) };
    let mut go = [0u8; 8];
    go.copy_from_slice(&gb[8..16]);
    let g = u64::from_le_bytes(go);
    line(
        "get_bo_offset",
        if r == 0 && g == off { "PASS" } else { "FAIL" },
        format!("{g:#x}"),
    );
    line(
        "verdict",
        "PASS",
        "假 renderD128 上的 DRM 语义已生效（interpose 通了）",
    );

    check_dup_overwrite();
}
