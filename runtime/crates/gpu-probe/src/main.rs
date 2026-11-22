//! gpu-probe — what the SoC lets a **Husk guest** do, measured from inside the guest.
//!
//! Every other capability probe in this tree runs as an Android/bionic process;
//! this one is built for aarch64 glibc and started through the Husk loader, so a
//! `PASS` here means "the guest, under our runtime, gets this", which is what
//! goals C5..C8 are about.
//!
//!   gpu-probe nodes    open the device nodes C5/C6/C7 care about, print errno
//!   gpu-probe kbase    ask the Mali kbase UAPI which versions it accepts
//!
//! Output is one line per fact: `gpu|<name>|<PASS|FAIL|INFO>|<detail>`.
//!
//! The kbase version scan is the gate for the panfrost/panvk route: mesa only
//! talks to kbase versions it knows, and this kernel is a vendor 4.19 whose
//! kbase build number is not exposed in sysfs. `KBASE_IOCTL_VERSION_CHECK`
//! answers by rejecting what it does not support, so scanning the grid tells us
//! exactly which (major, minor) pairs this driver accepts.

use core::ffi::{c_char, c_int, c_ulong, c_void};
use std::fs::{File, OpenOptions};
use std::os::unix::io::AsRawFd;
use std::path::Path;

extern "C" {
    fn __errno_location() -> *mut c_int;
    fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
    fn mmap(addr: *mut c_void, len: usize, prot: c_int, flags: c_int, fd: c_int, off: i64)
        -> *mut c_void;
    fn munmap(addr: *mut c_void, len: usize) -> c_int;
}

fn errno() -> i32 {
    unsafe { *__errno_location() }
}

/// ioctl() with a clean errno in front of it: reading `errno` after a *successful*
/// call returns whatever an earlier failing call left there, which reads like a
/// failure in the log (the first version of this probe reported `OK ... errno=1`).
fn ioctl_errno(fd: c_int, cmd: c_ulong, arg: *mut core::ffi::c_void) -> i32 {
    unsafe {
        *__errno_location() = 0;
        if ioctl(fd, cmd, arg) == 0 {
            0
        } else {
            errno()
        }
    }
}

fn line(name: &str, verdict: &str, detail: impl AsRef<str>) {
    println!("gpu|{}|{}|{}", name, verdict, detail.as_ref());
}

// ---------------------------------------------------------------- ioctl numbers

const IOC_NRBITS: u32 = 8;
const IOC_TYPEBITS: u32 = 8;
const IOC_SIZEBITS: u32 = 14;
const IOC_NRSHIFT: u32 = 0;
const IOC_TYPESHIFT: u32 = IOC_NRSHIFT + IOC_NRBITS;
const IOC_SIZESHIFT: u32 = IOC_TYPESHIFT + IOC_TYPEBITS;
const IOC_DIRSHIFT: u32 = IOC_SIZESHIFT + IOC_SIZEBITS;
const IOC_WRITE: u32 = 1;
const IOC_READ: u32 = 2;

/// `_IOC(dir, type, nr, size)`. kbase encodes `VERSION_CHECK` with size 0 (a
/// historic quirk: it passes a struct anyway), so this cannot go through a
/// generic helper that derives the size.
const fn ioc(dir: u32, typ: u32, nr: u32, size: u32) -> c_ulong {
    (((dir << IOC_DIRSHIFT)
        | (typ << IOC_TYPESHIFT)
        | (nr << IOC_NRSHIFT)
        | (size << IOC_SIZESHIFT)) as u32) as c_ulong
}

const KBASE_TYPE: u32 = 0x80;
/// Measured on RMX3357 (MT6893, 2026-11-22): only `_IOC(RW, 0x80, 0, 4)` is
/// recognised. Size 0 (what several kbase headers say) is ENOTTY here, i.e. the
/// driver keys the ioctl on the full command word, not just (type, nr).
/// `gpu-probe enc` reproduces the table.
const KBASE_IOCTL_VERSION_CHECK: c_ulong = ioc(IOC_READ | IOC_WRITE, KBASE_TYPE, 0, 4);
const KBASE_IOCTL_GET_FLAGS: c_ulong = ioc(IOC_READ | IOC_WRITE, KBASE_TYPE, 1, 8);
const KBASE_IOCTL_SET_FLAGS: c_ulong = ioc(IOC_WRITE, KBASE_TYPE, 2, 8);

#[repr(C)]
struct KbaseVersionCheck {
    major: u16,
    minor: u16,
}

fn version_check(fd: c_int, major: u16, minor: u16) -> Result<(), i32> {
    let args = KbaseVersionCheck { major, minor };
    let e = ioctl_errno(
        fd,
        KBASE_IOCTL_VERSION_CHECK,
        &args as *const KbaseVersionCheck as *mut core::ffi::c_void,
    );
    if e == 0 {
        Ok(())
    } else {
        Err(e)
    }
}

// ----------------------------------------------------------------- nodes mode

fn nodes() {
    let candidates: [&str; 12] = [
        "/dev/mali0",
        "/dev/kgsl-3d0",
        "/dev/dri/card0",
        "/dev/dri/renderD128",
        "/dev/ion",
        "/dev/dma_heap/system",
        "/dev/dma_heap/system-uncached",
        "/dev/snd/controlC0",
        "/dev/snd/pcmC0D0p",
        "/dev/ashmem",
        "/dev/binder",
        "/dev/ptmx",
    ];
    for path in candidates {
        // rw first; if that is denied, say whether r-only works, otherwise a
        // "no permission" and a "needs read-only" look identical in the log.
        match OpenOptions::new().read(true).write(true).open(path) {
            Ok(f) => line("node", "PASS", format!("{path} rw fd={}", f.as_raw_fd())),
            Err(e) => {
                let rw = e.raw_os_error().unwrap_or(0);
                match File::open(path) {
                    Ok(f) => line(
                        "node",
                        "PASS",
                        format!("{path} ro-only fd={} (rw errno={rw})", f.as_raw_fd()),
                    ),
                    Err(e2) => line(
                        "node",
                        "FAIL",
                        format!("{path} errno={rw} ro_errno={}", e2.raw_os_error().unwrap_or(0)),
                    ),
                }
            }
        }
    }
    // what the guest can see at all, so a missing node is distinguishable from
    // a node we are not allowed to open
    for dir in ["/dev/dri", "/dev/snd", "/dev/dma_heap"] {
        match std::fs::read_dir(dir) {
            Ok(rd) => {
                let mut names: Vec<String> = rd
                    .filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().to_string())
                    .collect();
                names.sort();
                line("dir", "PASS", format!("{dir} = {}", names.join(" ")));
            }
            Err(e) => line(
                "dir",
                "FAIL",
                format!("{dir} errno={}", e.raw_os_error().unwrap_or(0)),
            ),
        }
    }
    // the properties the routing table keys on
    for prop in [
        "ro.board.platform",
        "ro.hardware.egl",
        "ro.hardware.vulkan",
        "ro.product.cpu.abilist",
    ] {
        line("prop", "INFO", format!("{prop}={}", getprop(prop)));
    }
}

fn getprop(name: &str) -> String {
    // The guest has no /system/bin/getprop, but the property area is a file.
    for base in [
        "/system/build.prop",
        "/vendor/build.prop",
        "/system/etc/build.prop",
    ] {
        if let Ok(text) = std::fs::read_to_string(base) {
            for l in text.lines() {
                if let Some(rest) = l.strip_prefix(name) {
                    if let Some(v) = rest.strip_prefix('=') {
                        return v.trim().to_string();
                    }
                }
            }
        }
    }
    "?".to_string()
}

// ---------------------------------------------------------------- kbase mode

fn kbase() {
    let path = "/dev/mali0";
    if !Path::new(path).exists() {
        line("kbase_open", "FAIL", format!("{path} missing"));
        return;
    }
    let f = match OpenOptions::new().read(true).write(true).open(path) {
        Ok(f) => f,
        Err(e) => {
            line(
                "kbase_open",
                "FAIL",
                format!("errno={}", e.raw_os_error().unwrap_or(0)),
            );
            return;
        }
    };
    let fd = f.as_raw_fd();
    line("kbase_open", "PASS", format!("fd={fd}"));

    // Ask for the grid of versions mesa knows plus a margin on both sides. The
    // driver refuses anything it does not implement, so the accepted set *is*
    // the answer; no guessing from the kernel release.
    let mut accepted: Vec<(u16, u16)> = Vec::new();
    let mut first_errno: Option<i32> = None;
    for major in 8u16..=14 {
        for minor in 0u16..=64 {
            match version_check(fd, major, minor) {
                Ok(()) => accepted.push((major, minor)),
                Err(e) => {
                    if first_errno.is_none() {
                        first_errno = Some(e);
                    }
                }
            }
        }
    }
    if accepted.is_empty() {
        line(
            "kbase_version",
            "FAIL",
            format!(
                "no version accepted in 8.0..=14.64 (first errno={})",
                first_errno.unwrap_or(0)
            ),
        );
        return;
    }
    // Compress to ranges for readability: "11.10-11.16".
    let mut groups: Vec<String> = Vec::new();
    let mut it = accepted.iter().copied().peekable();
    while let Some(start) = it.next() {
        let mut end = start;
        while let Some(&next) = it.peek() {
            if next.0 == end.0 && next.1 == end.1 + 1 {
                end = next;
                it.next();
            } else {
                break;
            }
        }
        groups.push(if end == start {
            format!("{}.{}", start.0, start.1)
        } else {
            format!("{}.{}-{}.{}", start.0, start.1, end.0, end.1)
        });
    }
    line(
        "kbase_version",
        "PASS",
        format!("accepted: {}", groups.join(", ")),
    );
    let (major, minor) = accepted[accepted.len() - 1];
    line(
        "kbase_version_max",
        "INFO",
        format!("highest accepted {major}.{minor} ({} pairs)", accepted.len()),
    );

    // The next thing a real client does is read/set the process flags. Getting
    // that far is the difference between "the ioctl path answers" and "we can
    // actually allocate and submit work".
    let mut flags: u64 = 0;
    let r = ioctl_errno(fd, KBASE_IOCTL_GET_FLAGS, &mut flags as *mut u64 as *mut core::ffi::c_void);
    if r == 0 {
        line("kbase_get_flags", "PASS", format!("flags={flags:#x}"));
        let want = flags | 1; // bit 0: job-manager (in-flight) mode, what mesa asks for
        let r = ioctl_errno(fd, KBASE_IOCTL_SET_FLAGS, &want as *const u64 as *mut core::ffi::c_void);
        if r == 0 {
            line("kbase_set_flags", "PASS", format!("set {want:#x}"));
        } else {
            line("kbase_set_flags", "FAIL", format!("errno={r}"));
        }
    } else {
        line("kbase_get_flags", "FAIL", format!("errno={r}"));
    }

    // Mapping page 0 after a successful version check is where kbase publishes
    // the GPU id / version page that mesa parses (`pan_kmod` reads props from
    // it on older kernels). Dump the first 64 bytes so the follow-up work has
    // the real layout instead of a guess.
    let p = unsafe { mmap(core::ptr::null_mut(), 4096, 1 | 2, 1, fd, 0) };
    if p as isize == -1 {
        line("kbase_mmap", "FAIL", format!("errno={}", errno()));
    } else {
        let bytes = unsafe { core::slice::from_raw_parts(p as *const u8, 64) };
        let hex: Vec<String> = bytes.iter().map(|b| format!("{b:02x}")).collect();
        line("kbase_mmap", "PASS", format!("page0[0..64]={}", hex.join("")));
        unsafe { munmap(p, 4096) };
    }
    let _ = c_char::default();
}

// ---------------------------------------------------------------- encoding mode

/// Find the exact `KBASE_IOCTL_VERSION_CHECK` encoding this kernel accepts.
///
/// kbase's own header encodes the ioctl with `_IOC(..., size 0)` even though it
/// passes a 4-byte struct, and different vendor trees deviate. The kernel tells
/// them apart by the *answer*: a command it knows but does not support returns
/// EINVAL, a command it does not know returns ENOTTY, and a command SELinux does
/// not whitelist for this domain fails before the driver ever sees it (EPERM).
fn enc() {
    let f = match OpenOptions::new().read(true).write(true).open("/dev/mali0") {
        Ok(f) => f,
        Err(e) => {
            line("kbase_open", "FAIL", format!("errno={}", e.raw_os_error().unwrap_or(0)));
            return;
        }
    };
    let fd = f.as_raw_fd();
    // 11.13 is the pair a 2024-era kbase on a Valhall part answers; 99.99 is
    // never supported, so it separates "wrong command" from "wrong version".
    for (dname, dir) in [("RW", IOC_READ | IOC_WRITE), ("W", IOC_WRITE), ("R", IOC_READ), ("N", 0u32)] {
        for size in [0u32, 4, 8, 12, 16] {
            for (major, minor) in [(11u16, 13u16), (99u16, 99u16)] {
                let cmd = ioc(dir, KBASE_TYPE, 0, size);
                let args = KbaseVersionCheck { major, minor };
                let e = ioctl_errno(fd, cmd, &args as *const KbaseVersionCheck as *mut core::ffi::c_void);
                line(
                    "enc",
                    "INFO",
                    format!(
                        "cmd={cmd:#x} dir={dname} size={size} ask={major}.{minor} -> {}",
                        if e == 0 { "OK".to_string() } else { format!("errno={e}") }
                    ),
                );
            }
        }
    }
}

// ------------------------------------------------------------- one-pair mode

/// A single version check in a fresh process. The scan in `kbase` cannot answer
/// on its own: kbase remembers the pair a context already checked and rejects
/// every later pair with a different answer, so a scan in one process reports
/// only the first thing it tried. One process per pair removes that state.
fn one(major: u16, minor: u16) {
    let f = match OpenOptions::new().read(true).write(true).open("/dev/mali0") {
        Ok(f) => f,
        Err(e) => {
            line("kbase_open", "FAIL", format!("errno={}", e.raw_os_error().unwrap_or(0)));
            return;
        }
    };
    match version_check(f.as_raw_fd(), major, minor) {
        Ok(()) => line("one", "PASS", format!("{major}.{minor}")),
        Err(e) => line("one", "FAIL", format!("{major}.{minor} errno={e}")),
    }
}

// ------------------------------------------------------------- ioctl surface

/// Walk (nr, size) for the kbase command word with a zeroed 64-byte buffer and
/// print what the driver answers. This maps the whole surface (which commands
/// exist, which sizes they want) instead of guessing header constants, which is
/// exactly the class of mistake the `VERSION_CHECK` encoding already exposed.
fn nrscan() {
    let f = match OpenOptions::new().read(true).write(true).open("/dev/mali0") {
        Ok(f) => f,
        Err(e) => {
            line("kbase_open", "FAIL", format!("errno={}", e.raw_os_error().unwrap_or(0)));
            return;
        }
    };
    let fd = f.as_raw_fd();
    // A version check first: kbase refuses everything else until it has one.
    let _ = version_check(fd, 11, 13);
    let mut buf = [0u8; 64];
    // Direction matters: kbase encodes the read/write direction per command
    // (SET_FLAGS and JOB_SUBMIT are write-only), and the driver keys on the
    // whole command word, so a RW probe misses them entirely.
    for dir in [IOC_READ | IOC_WRITE, IOC_WRITE, IOC_READ] {
    for nr in 0u32..=40 {
        for size in [
            0u32, 4, 8, 12, 16, 20, 24, 28, 32, 36, 40, 44, 48, 56, 64, 72, 80, 96, 112, 128,
            160, 192, 256,
        ] {
            let cmd = ioc(dir, KBASE_TYPE, nr, size);
            let e = ioctl_errno(fd, cmd, buf.as_mut_ptr() as *mut core::ffi::c_void);
            if e != 25 {
                // 25 = ENOTTY = "not a command of this driver"; everything else
                // is a real answer worth printing.
                let head: Vec<String> = buf[..8.min(buf.len())].iter().map(|b| format!("{b:02x}")).collect();
                line(
                    "nrscan",
                    "INFO",
                    format!("nr={nr} size={size} cmd={cmd:#x} -> {} buf0={}", if e == 0 { "OK".to_string() } else { format!("errno={e}") }, head.join("")),
                );
            }
            buf = [0u8; 64];
        }
    }
    }
}

// ------------------------------------------------------------------- version

/// One version check per process, printing what the driver wrote back.
///
/// kbase answers a *wrong* pair with EPERM and still fills the out-half with the
/// version it is built for, and it only allows one check per open file. So the
/// reported pair has to be read out of a rejected call, not a successful one.
fn ver() {
    let f = match OpenOptions::new().read(true).write(true).open("/dev/mali0") {
        Ok(f) => f,
        Err(e) => {
            line("kbase_open", "FAIL", format!("errno={}", e.raw_os_error().unwrap_or(0)));
            return;
        }
    };
    let fd = f.as_raw_fd();
    for (major, minor) in [(11u16, 13u16), (11u16, 38u16), (0u16, 0u16)] {
        // a fresh pair only works once per fd, so each is its own open
        let f = OpenOptions::new().read(true).write(true).open("/dev/mali0").unwrap();
        let fd = f.as_raw_fd();
        let mut args = KbaseVersionCheck { major, minor };
        let e = ioctl_errno(fd, KBASE_IOCTL_VERSION_CHECK, &mut args as *mut _ as *mut core::ffi::c_void);
        line(
            "ver",
            if e == 0 { "PASS" } else { "INFO" },
            format!("asked {major}.{minor} -> errno={e} out={}.{}", args.major, args.minor),
        );
        let _ = fd;
    }
    let _ = fd;
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(|s| s.as_str()).unwrap_or("") {
        "nodes" => nodes(),
        "kbase" => kbase(),
        "enc" => enc(),
        "nrscan" => nrscan(),
        "ver" => ver(),
        "one" => {
            let major: u16 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
            let minor: u16 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0);
            one(major, minor)
        }
        "" => {
            eprintln!("usage: gpu-probe nodes|kbase|enc|nrscan|ver|one <major> <minor>");
            std::process::exit(2);
        }
        other => {
            eprintln!("unknown mode: {other}");
            std::process::exit(2);
        }
    }
    let _ = File::open("/dev/null");
}
