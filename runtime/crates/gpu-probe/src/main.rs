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
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::os::unix::io::AsRawFd;
use std::os::unix::net::UnixStream;
use std::path::Path;

extern "C" {
    fn __errno_location() -> *mut c_int;
    fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
    fn mmap(
        addr: *mut c_void,
        len: usize,
        prot: c_int,
        flags: c_int,
        fd: c_int,
        off: i64,
    ) -> *mut c_void;
    fn munmap(addr: *mut c_void, len: usize) -> c_int;
    fn poll(fds: *mut core::ffi::c_void, nfds: u64, timeout: c_int) -> c_int;
    fn read(fd: c_int, buf: *mut core::ffi::c_void, count: usize) -> isize;
    fn recvmsg(fd: c_int, msg: *mut MsgHdr, flags: c_int) -> isize;
    fn close(fd: c_int) -> c_int;
    fn dup(fd: c_int) -> c_int;
    fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlclose(handle: *mut c_void) -> c_int;
}

const SOL_SOCKET: c_int = 1;
const SCM_RIGHTS: c_int = 1;
const PROT_READ: c_int = 1;
const PROT_WRITE: c_int = 2;
const MAP_SHARED: c_int = 1;

#[repr(C)]
struct IoVec {
    base: *mut c_void,
    len: usize,
}

#[repr(C)]
struct MsgHdr {
    name: *mut c_void,
    name_len: u32,
    iov: *mut IoVec,
    iov_len: usize,
    control: *mut c_void,
    control_len: usize,
    flags: c_int,
}

#[repr(C)]
struct CmsgHdr {
    len: usize,
    level: c_int,
    ty: c_int,
}

#[repr(C)]
struct MemImport {
    flags: u64,
    phandle: u64,
    ty: u32,
    padding: u32,
}

#[repr(C)]
struct StickyResourceMap {
    count: u64,
    address: u64,
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
                        format!(
                            "{path} errno={rw} ro_errno={}",
                            e2.raw_os_error().unwrap_or(0)
                        ),
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

// ----------------------------------------------------------- peripherals mode

/// One row per peripheral family, node-first then sysfs marker.
///
/// Purpose: make "this machine has no sensors/GPS/USB/BT/NFC" a *measured*
/// statement instead of an assumption, and prove the guest sees a plain ENOENT
/// (never a fake device, never a crash). Exits 0 either way.
fn periph() {
    let nodes: [(&str, &str); 12] = [
        ("sensor", "/dev/iio:device0"),
        ("sensor", "/dev/sensorhub"),
        ("gps", "/dev/gps0"),
        ("usb", "/dev/bus/usb"),
        ("bluetooth", "/dev/rfkill"),
        ("bluetooth", "/dev/hci0"),
        ("nfc", "/dev/nfc0"),
        ("fingerprint", "/dev/fingerprint"),
        ("input", "/dev/input/event0"),
        ("hidraw", "/dev/hidraw0"),
        ("i2c", "/dev/i2c-0"),
        ("spi", "/dev/spidev0.0"),
    ];
    for (kind, path) in nodes {
        match OpenOptions::new().read(true).open(path) {
            Ok(f) => println!(
                "periph|{kind}|PASS|{path} r fd={}",
                std::os::fd::AsRawFd::as_raw_fd(&f)
            ),
            Err(e) => println!(
                "periph|{kind}|MISSING|{path} errno={}",
                e.raw_os_error().unwrap_or(0)
            ),
        }
    }
    for (kind, dir) in [
        ("sensor", "/sys/class/iio"),
        ("gps", "/sys/class/gps"),
        ("bluetooth", "/sys/class/bluetooth"),
        ("nfc", "/sys/class/nfc"),
        ("input", "/sys/class/input"),
        ("usb", "/sys/bus/usb"),
    ] {
        match std::fs::read_dir(dir) {
            Ok(rd) => {
                let n = rd.filter_map(|e| e.ok()).count();
                println!("periph|{kind}|SYSDIR|{dir} entries={n}");
            }
            Err(e) => println!(
                "periph|{kind}|SYSDIR-MISSING|{dir} errno={}",
                e.raw_os_error().unwrap_or(0)
            ),
        }
    }
    println!("periph|verdict|DONE|no device is faked; missing families are plain ENOENT");
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
        format!(
            "highest accepted {major}.{minor} ({} pairs)",
            accepted.len()
        ),
    );

    // The next thing a real client does is read/set the process flags. Getting
    // that far is the difference between "the ioctl path answers" and "we can
    // actually allocate and submit work".
    let mut flags: u64 = 0;
    let r = ioctl_errno(
        fd,
        KBASE_IOCTL_GET_FLAGS,
        &mut flags as *mut u64 as *mut core::ffi::c_void,
    );
    if r == 0 {
        line("kbase_get_flags", "PASS", format!("flags={flags:#x}"));
        let want = flags | 1; // bit 0: job-manager (in-flight) mode, what mesa asks for
        let r = ioctl_errno(
            fd,
            KBASE_IOCTL_SET_FLAGS,
            &want as *const u64 as *mut core::ffi::c_void,
        );
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
        line(
            "kbase_mmap",
            "PASS",
            format!("page0[0..64]={}", hex.join("")),
        );
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
            line(
                "kbase_open",
                "FAIL",
                format!("errno={}", e.raw_os_error().unwrap_or(0)),
            );
            return;
        }
    };
    let fd = f.as_raw_fd();
    // 11.13 is the pair a 2024-era kbase on a Valhall part answers; 99.99 is
    // never supported, so it separates "wrong command" from "wrong version".
    for (dname, dir) in [
        ("RW", IOC_READ | IOC_WRITE),
        ("W", IOC_WRITE),
        ("R", IOC_READ),
        ("N", 0u32),
    ] {
        for size in [0u32, 4, 8, 12, 16] {
            for (major, minor) in [(11u16, 13u16), (99u16, 99u16)] {
                let cmd = ioc(dir, KBASE_TYPE, 0, size);
                let args = KbaseVersionCheck { major, minor };
                let e = ioctl_errno(
                    fd,
                    cmd,
                    &args as *const KbaseVersionCheck as *mut core::ffi::c_void,
                );
                line(
                    "enc",
                    "INFO",
                    format!(
                        "cmd={cmd:#x} dir={dname} size={size} ask={major}.{minor} -> {}",
                        if e == 0 {
                            "OK".to_string()
                        } else {
                            format!("errno={e}")
                        }
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
            line(
                "kbase_open",
                "FAIL",
                format!("errno={}", e.raw_os_error().unwrap_or(0)),
            );
            return;
        }
    };
    match version_check(f.as_raw_fd(), major, minor) {
        Ok(()) => line("one", "PASS", format!("{major}.{minor}")),
        Err(e) => line("one", "FAIL", format!("{major}.{minor} errno={e}")),
    }
}

// --------------------------------------------------- vendor-documented path

/// The exact ioctl words the vendor's own userspace driver sends, read out of
/// `/vendor/lib64/egl/mt6893/libGLES_mali.so` (r32p1) instead of guessed from a
/// header. Addresses in that binary:
///   882614  VERSION_CHECK 0xc0048000  (struct {u16 major, u16 minor})
///   882670  SET_FLAGS     0x40048001  (u32 flags)
///   8826cc  JOB_SUBMIT    0x40108002  (struct {u64 addr, u32 nr_atoms, u32 stride})
///
/// Sending nr_atoms = 0 makes the driver reject the submission during validation
/// (EINVAL) before it can touch hardware, so this answers "is the command
/// reachable at all" without ever putting a job on the GPU. That distinction is
/// the whole point: ENOTTY means the command number is wrong, EINVAL means it is
/// right and the payload was refused.
#[repr(C)]
struct JobSubmit {
    addr: u64,
    nr_atoms: u32,
    stride: u32,
}

fn submit0() {
    let f = match OpenOptions::new().read(true).write(true).open("/dev/mali0") {
        Ok(f) => f,
        Err(e) => {
            line(
                "submit0_open",
                "FAIL",
                format!("errno={}", e.raw_os_error().unwrap_or(0)),
            );
            return;
        }
    };
    let fd = f.as_raw_fd();

    let vc = KbaseVersionCheck { major: 0, minor: 0 };
    let e = ioctl_errno(
        fd,
        KBASE_IOCTL_VERSION_CHECK,
        &vc as *const _ as *mut core::ffi::c_void,
    );
    line(
        "version_check(0,0) 0xc0048000",
        if e == 0 { "PASS" } else { "FAIL" },
        format!("errno={e} -> {}.{}", vc.major, vc.minor),
    );

    // Same word the vendor uses: _IOW(0x80, 1, 4).
    let mut flags: u32 = 0;
    let e = ioctl_errno(
        fd,
        0x4004_8001,
        &mut flags as *mut u32 as *mut core::ffi::c_void,
    );
    line(
        "set_flags 0x40048001",
        if e == 0 { "PASS" } else { "FAIL" },
        format!("errno={e}"),
    );

    let mut js = JobSubmit {
        addr: 0,
        nr_atoms: 0,
        stride: 0,
    };
    let e = ioctl_errno(
        fd,
        0x4010_8002,
        &mut js as *mut JobSubmit as *mut core::ffi::c_void,
    );
    let verdict = match e {
        0 => "PASS (accepted, unexpected)",
        25 => "ACCEPTED (ENOTTY=25 would mean wrong command)",
        _ => "ANSWERED",
    };
    line(
        "job_submit 0x40108002 nr_atoms=0",
        verdict,
        format!("errno={e} ({})", strerror_guess(e)),
    );
}

fn strerror_guess(e: i32) -> &'static str {
    match e {
        0 => "ok",
        1 => "EPERM",
        2 => "ENOENT",
        12 => "ENOMEM",
        13 => "EACCES",
        14 => "EFAULT",
        16 => "EBUSY",
        22 => "EINVAL",
        25 => "ENOTTY",
        38 => "ENOSYS",
        95 => "EOPNOTSUPP",
        _ => "?",
    }
}

// --------------------------------------------------------- GPU memory (C5)

#[repr(C)]
struct MemAlloc {
    va_pages: u64,
    commit_pages: u64,
    extension: u64,
    flags: u64,
}

#[repr(C)]
struct MemQuery {
    gpu_va: u64,
    va_pages: u16,
    commit_pages: u16,
    extension: u16,
    flags: u8,
    padding: u8,
}

/// Can we get GPU-visible memory that the CPU can write, and map it?
///
/// This is the floor everything else stands on: panfrost/panvk need BOs it can
/// fill from the CPU and hand to the GPU. All three commands are from evidence 35
/// (read out of the vendor driver), and none of them puts work on the GPU.
fn alloc1() {
    let f = match OpenOptions::new().read(true).write(true).open("/dev/mali0") {
        Ok(f) => f,
        Err(e) => {
            line(
                "alloc_open",
                "FAIL",
                format!("errno={}", e.raw_os_error().unwrap_or(0)),
            );
            return;
        }
    };
    let fd = f.as_raw_fd();
    let vc = KbaseVersionCheck { major: 0, minor: 0 };
    let e = ioctl_errno(
        fd,
        KBASE_IOCTL_VERSION_CHECK,
        &vc as *const _ as *mut core::ffi::c_void,
    );
    line(
        "version_check",
        if e == 0 { "PASS" } else { "FAIL" },
        format!("errno={e} {}.{}", vc.major, vc.minor),
    );
    let mut flags: u32 = 0;
    let e = ioctl_errno(
        fd,
        0x4004_8001,
        &mut flags as *mut u32 as *mut core::ffi::c_void,
    );
    line(
        "set_flags",
        if e == 0 { "PASS" } else { "FAIL" },
        format!("errno={e}"),
    );

    // BASE_MEM_PROT_CPU_RW|GPU_RD|GPU_WR = 1|4|8
    for (vap, cmp_, probe_flags) in [(1u64, 1u64, 0xDu64), (1, 1, 0xFu64), (1, 1, 0xBu64)] {
        let mut a = MemAlloc {
            va_pages: vap,
            commit_pages: cmp_,
            extension: 0,
            flags: probe_flags,
        };
        let cmd = 0xc020_8005u64;
        let r = unsafe { ioctl(fd, cmd, &mut a as *mut MemAlloc as *mut core::ffi::c_void) };
        let tail = format!(
            "in(va={vap},commit={cmp_},flags={probe_flags:#x}) ret={r} back=[{:#x},{:#x},{:#x},{:#x}]",
            a.va_pages, a.commit_pages, a.extension, a.flags
        );
        if r < 0 {
            let e = errno();
            line(
                "mem_alloc",
                "FAIL",
                format!("errno={e} ({}) {tail}", strerror_guess(e)),
            );
            continue;
        }
        // Measured writeback on this kernel (see the tail line): field0 becomes
        // the flags the driver actually used, field1 becomes the GPU VA, and the
        // ioctl return is 0. Two different allocations land on 0x41000 / 0x42000,
        // i.e. consecutive one-page VAs, which is what a real VA allocator does.
        let gpu_va = a.commit_pages;
        let out_flags = a.va_pages;
        line(
            "mem_alloc",
            "PASS",
            format!("{tail} => gpu_va={gpu_va:#x} flags_out={out_flags:#x}"),
        );

        let mut q = MemQuery {
            gpu_va,
            va_pages: 0,
            commit_pages: 0,
            extension: 0,
            flags: 0,
            padding: 0,
        };
        let e = ioctl_errno(
            fd,
            0xc010_8006,
            &mut q as *mut MemQuery as *mut core::ffi::c_void,
        );
        line(
            "mem_query",
            if e == 0 { "PASS" } else { "FAIL" },
            format!(
                "errno={e} va_pages={} commit_pages={} ext={} flags={:#x}",
                q.va_pages, q.commit_pages, q.extension, q.flags
            ),
        );

        // Vendor convention (disassembly 0x881a4c): prot comes straight from the
        // low two bits of the alloc flags, length is va_pages << 12, offset is the
        // VA the kernel wrote back. Try each prot the driver itself would use.
        let prot = (probe_flags & 3) as i32;
        let p = unsafe {
            mmap(
                core::ptr::null_mut(),
                4096,
                prot,
                1, /* MAP_SHARED */
                fd,
                gpu_va as i64,
            )
        };
        if p as isize == -1 {
            let e = errno();
            line(
                "mmap(gpu_va)",
                "FAIL",
                format!("prot={prot} errno={e} ({})", strerror_guess(e)),
            );
        } else {
            // Only touch memory we were granted write access to -- writing a
            // PROT_READ mapping is a SIGSEGV, not an errno (learned the hard way).
            unsafe {
                if prot & 2 != 0 {
                    let b = p as *mut u8;
                    for i in 0..16 {
                        *b.add(i) = 0xA5;
                    }
                    let ok = (0..16).all(|i| *b.add(i) == 0xA5);
                    line(
                        "mmap(gpu_va)",
                        if ok { "PASS" } else { "FAIL" },
                        format!("prot={prot} mapped at {:p}, cpu write+readback {}", p, ok),
                    );
                } else {
                    line(
                        "mmap(gpu_va)",
                        "PASS",
                        format!("prot={prot} mapped at {:p} (read-only, no write test)", p),
                    );
                }
                munmap(p, 4096);
            }
        }
    }
}

// ------------------------------------------------- ABIs from the kernel side

/// `union kbase_ioctl_mem_query`: in {u64 gpu_addr; u64 query}, out {u64 value}.
/// Query ids: 1 = commit size, 2 = VA size, 3 = flags.
#[repr(C)]
struct MemQueryU {
    a: u64,
    b: u64,
}

fn kbase_open() -> Option<std::fs::File> {
    let f = OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/mali0")
        .ok()?;
    let fd = f.as_raw_fd();
    let vc = KbaseVersionCheck { major: 0, minor: 0 };
    if ioctl_errno(
        fd,
        KBASE_IOCTL_VERSION_CHECK,
        &vc as *const _ as *mut core::ffi::c_void,
    ) != 0
    {
        return None;
    }
    let mut flags: u32 = 0;
    if ioctl_errno(
        fd,
        0x4004_8001,
        &mut flags as *mut u32 as *mut core::ffi::c_void,
    ) != 0
    {
        return None;
    }
    Some(f)
}

fn alloc_pages(fd: c_int, pages: u64, flags: u64) -> Option<u64> {
    let mut a = MemAlloc {
        va_pages: pages,
        commit_pages: pages,
        extension: 0,
        flags,
    };
    let r = unsafe {
        ioctl(
            fd,
            0xc020_8005u64,
            &mut a as *mut MemAlloc as *mut core::ffi::c_void,
        )
    };
    if r < 0 {
        None
    } else {
        Some(a.commit_pages)
    }
}

/// Verify the documented mem_query union against this driver: query the size and
/// flags of a region we just allocated. If the numbers come back consistent with
/// what we asked for, the ABI is confirmed (not guessed).
fn query1() {
    let Some(f) = kbase_open() else {
        line("open", "FAIL", "version_check/set_flags");
        return;
    };
    let fd = f.as_raw_fd();
    let Some(va) = alloc_pages(fd, 1, 0xF) else {
        line("mem_alloc", "FAIL", "flags=0xf");
        return;
    };
    line("mem_alloc", "PASS", format!("gpu_va={va:#x}"));

    for (q, name) in [(1u64, "COMMIT_SIZE"), (2, "VA_SIZE"), (3, "FLAGS")] {
        let mut mq = MemQueryU { a: va, b: q };
        let e = ioctl_errno(
            fd,
            0xc010_8006,
            &mut mq as *mut MemQueryU as *mut core::ffi::c_void,
        );
        line(
            "mem_query",
            if e == 0 { "PASS" } else { "FAIL" },
            format!("{name} errno={e} value={:#x}", mq.a),
        );
    }
}

/// One atom, all zero bytes, stride 72 (measured in the vendor driver).
///
/// The point is not to run anything: it separates "the kernel will look at my
/// atom array" from "the command word is wrong". A zeroed atom has jc = 0, so a
/// driver that reads it must refuse with something other than ENOTTY.
fn submit1() {
    let Some(f) = kbase_open() else {
        line("open", "FAIL", "version_check/set_flags");
        return;
    };
    let fd = f.as_raw_fd();
    let atom = [0u8; 72];
    for n in [1u32, 0] {
        let mut js = JobSubmit {
            addr: atom.as_ptr() as u64,
            nr_atoms: n,
            stride: 72,
        };
        let e = ioctl_errno(
            fd,
            0x4010_8002,
            &mut js as *mut JobSubmit as *mut core::ffi::c_void,
        );
        line(
            "job_submit",
            "ANSWERED",
            format!("nr_atoms={n} stride=72 errno={e} ({})", strerror_guess(e)),
        );
    }
}

/// Confirm the command-word mapping from evidence 38 with a round trip:
/// allocate, free, allocate again. A VA allocator that reuses the freed slot makes
/// `MEM_FREE = 0x40088007` (nr 7, one u64) impossible to confuse with anything else.
fn free1() {
    let Some(f) = kbase_open() else {
        line("open", "FAIL", "version_check/set_flags");
        return;
    };
    let fd = f.as_raw_fd();
    let Some(a) = alloc_pages(fd, 1, 0xF) else {
        line("alloc#1", "FAIL", "flags=0xf");
        return;
    };
    line("alloc#1", "PASS", format!("gpu_va={a:#x}"));

    let mut mf = a;
    let e = ioctl_errno(
        fd,
        0x4008_8007,
        &mut mf as *mut u64 as *mut core::ffi::c_void,
    );
    line(
        "mem_free 0x40088007",
        if e == 0 { "PASS" } else { "FAIL" },
        format!("errno={e} ({}) va={a:#x}", strerror_guess(e)),
    );

    match alloc_pages(fd, 1, 0xF) {
        Some(b) => line(
            "alloc#2",
            if b == a { "PASS" } else { "PASS" },
            format!("gpu_va={b:#x} reused={}", b == a),
        ),
        None => line("alloc#2", "FAIL", "after free"),
    }
}

/// Completion channel, step 1: does the driver hand us a timeline fd?
/// `STREAM_CREATE` takes a 32-byte NUL-padded name and returns an fd (or -errno).
fn stream1() {
    let Some(f) = kbase_open() else {
        line("open", "FAIL", "version_check/set_flags");
        return;
    };
    let fd = f.as_raw_fd();
    let mut name = [0u8; 32];
    for (i, b) in b"husk".iter().enumerate() {
        name[i] = *b;
    }
    let r = unsafe {
        ioctl(
            fd,
            0x4020_8018u64,
            name.as_mut_ptr() as *mut core::ffi::c_void,
        )
    };
    if r < 0 {
        let e = errno();
        line(
            "stream_create 0x40208018",
            "FAIL",
            format!("errno={e} ({})", strerror_guess(e)),
        );
        return;
    }
    line(
        "stream_create 0x40208018",
        "PASS",
        format!("timeline fd={r}"),
    );
    // A timeline fd is pollable. Anything other than POLLERR means the channel is real.
    let mut pfd = [0i32, 0i32];
    pfd[0] = r;
    let n = unsafe { poll(pfd.as_mut_ptr() as *mut core::ffi::c_void, 1, 0) };
    line(
        "poll(timeline)",
        if n >= 0 { "PASS" } else { "FAIL" },
        format!("revents={:#x} errno={}", pfd[1], errno()),
    );
    unsafe { close(r) };
}

// ------------------------------------------- completion channel (GOALS B2)

/// `struct base_fence { int fd; int stream_fd; }` — the object the vendor driver
/// parks in an atom's `jc` for a soft fence-trigger atom
/// (base_jd_fence_trigger_setup_v2, mali_base_kernel.h).
#[repr(C)]
#[derive(Clone, Copy)]
struct BaseFence {
    fd: i32,
    stream_fd: i32,
}

/// Atom stride, measured twice independently: the vendor driver passes
/// `mov w9, #0x48` into the submit struct (0x88120c) and indexes the atom array
/// with `madd x8, x9, #0x48, x8` (0x7c4cb0). Stock headers say 56; on this
/// kernel 56 is not what the ioctl wants.
const ATOM_STRIDE: usize = 72;
const ATOM_STRIDE_V2: usize = 64;

/// Historical probe offsets. They are kept so the old measurements remain
/// reproducible; they are not the r32p1 `base_jd_atom_v2` layout. `fence8` and
/// `fence11` use the real 64-byte layout below.
const ATOM_CORE_REQ_VENDOR: usize = 0x40;
const ATOM_CORE_REQ_STOCK_HDR: usize = 0x34;
/// r32p1 GPL layout: `core_req` is a u32 at 0x34. The historical vendor
/// probes below use 0x40 to preserve their original measurements.
const ATOM_CORE_REQ_R32: usize = 0x34;
const ATOM_JC: usize = 0x00;
/// Historical offset used by the early 72-byte probes. It is not the location
/// of `atom_number` in either r32p1 atom structure.
const ATOM_ATOM_NUMBER: usize = 0x2c;

// r32p1 `base_jd_atom_v2`: atom_number is u8 at 0x28, core_req is u32 at 0x2c.
const ATOM_V2_ATOM_NUMBER: usize = 0x28;
const ATOM_V2_CORE_REQ: usize = 0x2c;
const ATOM_V2_PRE_DEP: usize = 0x24;
const BASE_JD_DEP_TYPE_ORDER: u8 = 2;

const BASE_JD_REQ_SOFT_FENCE_TRIGGER: u32 = (1 << 9) | 0x2; // 0x202

#[repr(C)]
struct PollFd {
    fd: i32,
    events: i16,
    revents: i16,
}

fn hex(buf: &[u8]) -> String {
    buf.iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join("")
}

fn put_u16(buf: &mut [u8], off: usize, v: u16) {
    buf[off..off + 2].copy_from_slice(&v.to_le_bytes());
}

fn put_u32(buf: &mut [u8], off: usize, v: u32) {
    buf[off..off + 4].copy_from_slice(&v.to_le_bytes());
}

fn put_u64(buf: &mut [u8], off: usize, v: u64) {
    buf[off..off + 8].copy_from_slice(&v.to_le_bytes());
}

/// Submit exactly one atom and then ask the timeline whether it completed.
///
/// `jc_is_gpu_va = false` passes a user pointer (what the vendor driver does);
/// `true` allocates a BO, writes the fence into it and passes the GPU VA, which
/// is the other possible reading of `kbase_pointer`.
fn fence_probe(label: &str, core_req_off: usize, jc_is_gpu_va: bool) {
    let Some(f) = kbase_open() else {
        line("open", "FAIL", "version_check/set_flags");
        return;
    };
    let fd = f.as_raw_fd();

    // timeline fd: the raw ioctl return *is* the fd (measured in stream1)
    let mut name = [0u8; 32];
    name[..4].copy_from_slice(b"husk");
    let sfd = unsafe {
        ioctl(
            fd,
            0x4020_8018u64,
            name.as_mut_ptr() as *mut core::ffi::c_void,
        )
    };
    if sfd < 0 {
        let e = errno();
        line(
            &format!("{label}/stream_create"),
            "FAIL",
            format!("errno={e}"),
        );
        return;
    }
    line(
        &format!("{label}/stream_create"),
        "PASS",
        format!("timeline fd={sfd}"),
    );

    // The fence object: fd = -1 (INVALID_PLATFORM_FENCE), stream_fd = timeline.
    let fence = BaseFence {
        fd: -1,
        stream_fd: sfd,
    };
    // Keep the BO alive for the whole submission; the kernel reads the fence
    // during JOB_SUBMIT, and base_kernel.h only forbids freeing it before the
    // submit call returns.
    let mut _bo_guard: Option<*mut core::ffi::c_void> = None;
    let mut _bo_va: u64 = 0;
    let jc: u64 = if jc_is_gpu_va {
        match alloc_pages(fd, 1, 0xD) {
            Some(va) => {
                let p = unsafe { mmap(core::ptr::null_mut(), 4096, 3, 1, fd, va as i64) };
                if p as isize == -1 {
                    let e = errno();
                    line(&format!("{label}/alloc"), "FAIL", format!("mmap errno={e}"));
                    unsafe { close(sfd) };
                    return;
                }
                unsafe {
                    let b = p as *mut u8;
                    let src = &fence as *const BaseFence as *const u8;
                    for i in 0..8 {
                        *b.add(i) = *src.add(i);
                    }
                }
                _bo_guard = Some(p);
                _bo_va = va;
                va
            }
            None => {
                line(&format!("{label}/alloc"), "FAIL", "mem_alloc flags=0xd");
                unsafe { close(sfd) };
                return;
            }
        }
    } else {
        &fence as *const BaseFence as u64
    };

    let mut atom = [0u8; ATOM_STRIDE];
    put_u64(&mut atom, ATOM_JC, jc);
    put_u32(&mut atom, ATOM_ATOM_NUMBER as usize, 1);
    put_u32(&mut atom, core_req_off, BASE_JD_REQ_SOFT_FENCE_TRIGGER);

    let mut js = JobSubmit {
        addr: atom.as_ptr() as u64,
        nr_atoms: 1,
        stride: ATOM_STRIDE as u32,
    };
    let e = ioctl_errno(
        fd,
        0x4010_8002,
        &mut js as *mut JobSubmit as *mut core::ffi::c_void,
    );
    line(
        &format!("{label}/job_submit"),
        if e == 0 { "PASS" } else { "FAIL" },
        format!(
            "jc={jc:#x} ({} ) core_req@{core_req_off:#x}=0x202 errno={e} ({})",
            if jc_is_gpu_va { "gpu_va" } else { "user_ptr" },
            strerror_guess(e)
        ),
    );
    if e != 0 {
        unsafe { close(sfd) };
        return;
    }

    // Completion: the timeline fd becomes readable when the fence is signalled.
    let mut pfd = PollFd {
        fd: sfd,
        events: 1, /* POLLIN */
        revents: 0,
    };
    let t0 = std::time::Instant::now();
    let n = unsafe { poll(&mut pfd as *mut PollFd as *mut core::ffi::c_void, 1, 2000) };
    let ms = t0.elapsed().as_millis();
    let verdict = match (n, pfd.revents & 0xF) {
        (0, _) => "TIMEOUT",
        (_, 0) => "POLLERR",
        _ => "PASS",
    };
    line(
        &format!("{label}/poll(timeline)"),
        verdict,
        format!("poll={n} revents={:#x} after {ms}ms", pfd.revents),
    );

    unsafe { close(sfd) };
    if let Some(p) = _bo_guard {
        unsafe {
            munmap(p, 4096);
            if _bo_va != 0 {
                let mut v = _bo_va;
                let _ = ioctl_errno(
                    fd,
                    0x4008_8007,
                    &mut v as *mut u64 as *mut core::ffi::c_void,
                );
            }
        }
    }
}

/// `fence0`: control. Same atom, same timeline, but `core_req` left at 0.
/// If this also makes the timeline readable then "readable" says nothing about
/// the fence and fence1 proved nothing.
fn fence0() {
    fence_probe_core_req("fence0/control", ATOM_CORE_REQ_VENDOR, 0);
}

/// Same helper as `fence_probe` but with a caller-chosen `core_req` value, so the
/// only difference between a passing and a failing run is that one word.
fn fence_probe_core_req(label: &str, core_req_off: usize, core_req: u32) {
    let Some(f) = kbase_open() else {
        line("open", "FAIL", "version_check/set_flags");
        return;
    };
    let fd = f.as_raw_fd();
    let mut name = [0u8; 32];
    name[..4].copy_from_slice(b"husk");
    let sfd = unsafe {
        ioctl(
            fd,
            0x4020_8018u64,
            name.as_mut_ptr() as *mut core::ffi::c_void,
        )
    };
    if sfd < 0 {
        let e = errno();
        line(
            &format!("{label}/stream_create"),
            "FAIL",
            format!("errno={e}"),
        );
        return;
    }
    let fence = BaseFence {
        fd: -1,
        stream_fd: sfd,
    };
    let mut atom = [0u8; ATOM_STRIDE];
    put_u64(&mut atom, ATOM_JC, &fence as *const BaseFence as u64);
    put_u32(&mut atom, ATOM_ATOM_NUMBER as usize, 1);
    put_u32(&mut atom, core_req_off, core_req);
    let mut js = JobSubmit {
        addr: atom.as_ptr() as u64,
        nr_atoms: 1,
        stride: ATOM_STRIDE as u32,
    };
    let e = ioctl_errno(
        fd,
        0x4010_8002,
        &mut js as *mut JobSubmit as *mut core::ffi::c_void,
    );
    line(
        &format!("{label}/job_submit"),
        if e == 0 { "PASS" } else { "FAIL" },
        format!(
            "core_req@{core_req_off:#x}={core_req:#x} errno={e} ({})",
            strerror_guess(e)
        ),
    );
    let mut pfd = PollFd {
        fd: sfd,
        events: 1,
        revents: 0,
    };
    let t0 = std::time::Instant::now();
    let n = unsafe { poll(&mut pfd as *mut PollFd as *mut core::ffi::c_void, 1, 1500) };
    let ms = t0.elapsed().as_millis();
    line(
        &format!("{label}/poll(timeline)"),
        if n == 0 {
            "TIMEOUT(expected)"
        } else {
            "READABLE"
        },
        format!("poll={n} revents={:#x} after {ms}ms", pfd.revents),
    );
    unsafe { close(sfd) };
}

/// `fence1`: vendor atom layout (core_req @ 0x40), fence as a user pointer.
fn fence1() {
    fence_probe("fence1/vendor", ATOM_CORE_REQ_VENDOR, false);
}

/// `fence2`: same but core_req at the offset the 4.19 stock header implies.
fn fence2() {
    fence_probe("fence2/stock", ATOM_CORE_REQ_STOCK_HDR, false);
}

/// `fence3`: vendor layout, fence placed in GPU memory and passed as GPU VA.
fn fence3() {
    fence_probe("fence3/gpuva", ATOM_CORE_REQ_VENDOR, true);
}

/// `struct kbase_ioctl_soft_event_update { u64 event; u32 new_status; u32 flags; }`
/// (`0x4010801c`, public kbase UAPI, 16 bytes).
#[repr(C)]
struct SoftEventUpdate {
    event: u64,
    new_status: u32,
    flags: u32,
}

#[repr(C)]
#[derive(Default)]
struct SyncFileInfo {
    name: [u8; 32],
    status: i32,
    flags: u32,
    num_fences: u32,
    pad: u32,
    sync_fence_info: u64,
}

const SYNC_IOC_FILE_INFO: u64 = 0xc038_3e04;

fn sync_file_info(label: &str, fd: c_int) {
    let mut info = SyncFileInfo::default();
    let e = ioctl_errno(
        fd,
        SYNC_IOC_FILE_INFO,
        &mut info as *mut SyncFileInfo as *mut core::ffi::c_void,
    );
    let name_end = info
        .name
        .iter()
        .position(|b| *b == 0)
        .unwrap_or(info.name.len());
    let name = core::str::from_utf8(&info.name[..name_end]).unwrap_or("?");
    line(
        label,
        if e == 0 { "PASS" } else { "FAIL" },
        format!(
            "errno={e} name={name:?} status={} num_fences={} flags={:#x}",
            info.status, info.num_fences, info.flags
        ),
    );
}

const BASE_JD_REQ_SOFT_EVENT_WAIT: u32 = (1 << 9) | 0x5; // 0x205

/// A soft-event atom that can only complete when we say so.
///
/// `fence4` submits EVENT_WAIT on an event word that stays 0: the atom must stay
/// pending, so the timeline must stay NOT readable. That is the control the
/// previous experiments lacked -- `fence0` proved a *completed* no-op atom makes
/// the timeline readable, which says nothing about whether "readable" tracks
/// completion or just "some submit happened".
///
/// `fence5` is the same atom plus `SOFT_EVENT_UPDATE`: we flip the event and the
/// blocked atom must complete, so the timeline must turn readable only then.
fn fence_event(label: &str, set_after: bool) {
    let Some(f) = kbase_open() else {
        line("open", "FAIL", "version_check/set_flags");
        return;
    };
    let fd = f.as_raw_fd();

    let mut name = [0u8; 32];
    name[..4].copy_from_slice(b"husk");
    let sfd = unsafe {
        ioctl(
            fd,
            0x4020_8018u64,
            name.as_mut_ptr() as *mut core::ffi::c_void,
        )
    };
    if sfd < 0 {
        let e = errno();
        line(
            &format!("{label}/stream_create"),
            "FAIL",
            format!("errno={e}"),
        );
        return;
    }
    line(
        &format!("{label}/stream_create"),
        "PASS",
        format!("timeline fd={sfd}"),
    );

    // one page of GPU memory to hold the event word
    let Some(va) = alloc_pages(fd, 1, 0xF) else {
        line(&format!("{label}/alloc"), "FAIL", "mem_alloc flags=0xf");
        unsafe { close(sfd) };
        return;
    };
    let p = unsafe { mmap(core::ptr::null_mut(), 4096, 3, 1, fd, va as i64) };
    if p as isize == -1 {
        let e = errno();
        line(
            &format!("{label}/alloc"),
            "FAIL",
            format!("mmap errno={e} gpu_va={va:#x}"),
        );
        unsafe { close(sfd) };
        return;
    }
    unsafe { *(p as *mut u8) = 0 };
    line(
        &format!("{label}/event"),
        "INFO",
        format!("gpu_va={va:#x} value=0"),
    );

    let mut atom = [0u8; ATOM_STRIDE];
    put_u64(&mut atom, ATOM_JC, va);
    put_u32(&mut atom, ATOM_ATOM_NUMBER as usize, 1);
    put_u32(&mut atom, ATOM_CORE_REQ_VENDOR, BASE_JD_REQ_SOFT_EVENT_WAIT);
    let mut js = JobSubmit {
        addr: atom.as_ptr() as u64,
        nr_atoms: 1,
        stride: ATOM_STRIDE as u32,
    };
    let e = ioctl_errno(
        fd,
        0x4010_8002,
        &mut js as *mut JobSubmit as *mut core::ffi::c_void,
    );
    line(
        &format!("{label}/job_submit"),
        if e == 0 { "PASS" } else { "FAIL" },
        format!("EVENT_WAIT on {va:#x} errno={e} ({})", strerror_guess(e)),
    );
    if e != 0 {
        unsafe { close(sfd) };
        return;
    }

    let mut pfd = PollFd {
        fd: sfd,
        events: 1,
        revents: 0,
    };
    let t0 = std::time::Instant::now();
    let n = unsafe { poll(&mut pfd as *mut PollFd as *mut core::ffi::c_void, 1, 1500) };
    let ms = t0.elapsed().as_millis();
    line(
        &format!("{label}/poll(before)"),
        if n == 0 { "NOT-READABLE" } else { "READABLE" },
        format!("poll={n} revents={:#x} after {ms}ms", pfd.revents),
    );

    if set_after {
        let mut upd = SoftEventUpdate {
            event: va,
            new_status: 1,
            flags: 0,
        };
        let e = ioctl_errno(
            fd,
            0x4010_801c,
            &mut upd as *mut SoftEventUpdate as *mut core::ffi::c_void,
        );
        line(
            &format!("{label}/soft_event_update"),
            if e == 0 { "PASS" } else { "FAIL" },
            format!(
                "event={va:#x} new_status=1 errno={e} ({})",
                strerror_guess(e)
            ),
        );
        let mut pfd2 = PollFd {
            fd: sfd,
            events: 1,
            revents: 0,
        };
        let t1 = std::time::Instant::now();
        let n2 = unsafe { poll(&mut pfd2 as *mut PollFd as *mut core::ffi::c_void, 1, 2000) };
        let ms2 = t1.elapsed().as_millis();
        line(
            &format!("{label}/poll(after set)"),
            if n2 == 0 { "NOT-READABLE" } else { "READABLE" },
            format!("poll={n2} revents={:#x} after {ms2}ms", pfd2.revents),
        );
    }

    unsafe {
        munmap(p, 4096);
        close(sfd);
        let mut v = va;
        let _ = ioctl_errno(
            fd,
            0x4008_8007,
            &mut v as *mut u64 as *mut core::ffi::c_void,
        );
    }
}

/// `fence4`: blocked atom, expect the timeline to stay quiet.
fn fence4() {
    fence_event("fence4/wait", false);
}

/// `fence5`: blocked atom + SOFT_EVENT_UPDATE, expect the timeline to speak only
/// after the update.
fn fence5() {
    fence_event("fence5/wait+set", true);
}

/// Does the kernel write the fence's *sync_file* fd back into `base_fence`?
///
/// `base_jd_fence_trigger_setup_v2` asserts `basep.fd == INVALID_PLATFORM_FENCE`
/// before submit, and `base_jd_fence_wait_setup_v2` asserts `basep.fd >= 0`, so
/// the fd is an *output* of the trigger atom -- the kernel creates a platform
/// fence and hands it back. If that is the case here, the completion channel is
/// "poll the fd the kernel wrote", not "poll the stream".
fn fence6() {
    let Some(f) = kbase_open() else {
        line("open", "FAIL", "version_check/set_flags");
        return;
    };
    let fd = f.as_raw_fd();

    let mut name = [0u8; 32];
    name[..4].copy_from_slice(b"husk");
    let sfd = unsafe {
        ioctl(
            fd,
            0x4020_8018u64,
            name.as_mut_ptr() as *mut core::ffi::c_void,
        )
    };
    if sfd < 0 {
        let e = errno();
        line("fence6/stream_create", "FAIL", format!("errno={e}"));
        return;
    }

    let mut fence = BaseFence {
        fd: -1,
        stream_fd: sfd,
    };
    let mut atom = [0u8; ATOM_STRIDE];
    put_u64(&mut atom, ATOM_JC, &mut fence as *mut BaseFence as u64);
    put_u32(&mut atom, ATOM_ATOM_NUMBER as usize, 1);
    put_u32(
        &mut atom,
        ATOM_CORE_REQ_VENDOR,
        BASE_JD_REQ_SOFT_FENCE_TRIGGER,
    );
    let mut js = JobSubmit {
        addr: atom.as_ptr() as u64,
        nr_atoms: 1,
        stride: ATOM_STRIDE as u32,
    };
    let e = ioctl_errno(
        fd,
        0x4010_8002,
        &mut js as *mut JobSubmit as *mut core::ffi::c_void,
    );
    let raw: [i32; 2] = [fence.fd, fence.stream_fd];
    line(
        "fence6/job_submit",
        if e == 0 { "PASS" } else { "FAIL" },
        format!(
            "errno={e} fence after submit = {{fd: {}, stream_fd: {}}} (raw {:?})",
            raw[0], raw[1], raw
        ),
    );
    if e != 0 {
        unsafe { close(sfd) };
        return;
    }

    if fence.fd >= 0 {
        let mut pfd = PollFd {
            fd: fence.fd,
            events: 1,
            revents: 0,
        };
        let t0 = std::time::Instant::now();
        let n = unsafe { poll(&mut pfd as *mut PollFd as *mut core::ffi::c_void, 1, 1000) };
        let ms = t0.elapsed().as_millis();
        line(
            "fence6/poll(sync_file)",
            if n > 0 && pfd.revents & 1 != 0 {
                "READABLE"
            } else {
                "NOT-READABLE"
            },
            format!(
                "fd={} poll={n} revents={:#x} after {ms}ms",
                fence.fd, pfd.revents
            ),
        );
        unsafe { close(fence.fd) };
    } else {
        line(
            "fence6/sync_file",
            "INFO",
            "kernel did not write an fd back",
        );
    }

    let mut pfd2 = PollFd {
        fd: sfd,
        events: 1,
        revents: 0,
    };
    let n2 = unsafe { poll(&mut pfd2 as *mut PollFd as *mut core::ffi::c_void, 1, 0) };
    line(
        "fence6/poll(stream)",
        "INFO",
        format!("poll={n2} revents={:#x}", pfd2.revents),
    );
    unsafe { close(sfd) };
}

/// Correct-layout soft-fence trigger.
///
/// Historical wrong-layout control. It writes a 16-bit atom number at 0x2c and
/// the r32 `core_req` at 0x34 while also using stride 72. The kernel therefore
/// parses a different structure and returns `JOB_CONFIG_FAULT`. Keep this as a
/// negative control proving that a plausible-looking mixed layout is rejected.
fn fence7() {
    let Some(f) = kbase_open() else {
        line("open", "FAIL", "version_check/set_flags");
        return;
    };
    let fd = f.as_raw_fd();

    let mut name = [0u8; 32];
    name[..4].copy_from_slice(b"husk");
    let sfd = unsafe {
        ioctl(
            fd,
            0x4020_8018u64,
            name.as_mut_ptr() as *mut core::ffi::c_void,
        )
    };
    if sfd < 0 {
        let e = errno();
        line(
            "fence7/stream_create",
            "FAIL",
            format!("errno={e} ({})", strerror_guess(e)),
        );
        return;
    }
    line("fence7/stream_create", "PASS", format!("stream_fd={sfd}"));

    let mut fence = BaseFence {
        fd: -1,
        stream_fd: sfd,
    };
    let mut atom = [0u8; ATOM_STRIDE];
    put_u64(&mut atom, ATOM_JC, &mut fence as *mut BaseFence as u64);
    put_u16(&mut atom, ATOM_ATOM_NUMBER, 1);
    put_u32(&mut atom, ATOM_CORE_REQ_R32, BASE_JD_REQ_SOFT_FENCE_TRIGGER);
    line(
        "fence7/atom",
        "INFO",
        format!(
            "stride={ATOM_STRIDE} atom_number@{ATOM_ATOM_NUMBER:#x}=1 core_req@{ATOM_CORE_REQ_R32:#x}={BASE_JD_REQ_SOFT_FENCE_TRIGGER:#x} jc=user_ptr"
        ),
    );

    // Drain any context-creation event before submitting the atom, otherwise a
    // stale atom-0 event can be mistaken for this submission's result.
    for i in 0..8 {
        let mut pfd = PollFd {
            fd,
            events: 1,
            revents: 0,
        };
        let n = unsafe { poll(&mut pfd as *mut PollFd as *mut core::ffi::c_void, 1, 0) };
        if n <= 0 || pfd.revents & 1 == 0 {
            break;
        }
        let mut ev = [0u8; 24];
        let got = unsafe { read(fd, ev.as_mut_ptr() as *mut core::ffi::c_void, ev.len()) };
        if got != ev.len() as isize {
            break;
        }
        let code = u32::from_le_bytes([ev[0], ev[1], ev[2], ev[3]]);
        let atom_no = ev[4];
        line(
            "fence7/pre_event",
            "INFO",
            format!("#{i} event_code={code:#x} atom_number={atom_no}"),
        );
    }

    let mut js = JobSubmit {
        addr: atom.as_ptr() as u64,
        nr_atoms: 1,
        stride: ATOM_STRIDE as u32,
    };
    unsafe { alarm(5) };
    let e = ioctl_errno(
        fd,
        0x4010_8002,
        &mut js as *mut JobSubmit as *mut core::ffi::c_void,
    );
    unsafe { alarm(0) };
    line(
        "fence7/job_submit",
        if e == 0 { "ACCEPTED" } else { "REFUSED" },
        format!(
            "errno={e} ({}) fence_after_submit={{fd:{},stream_fd:{}}}",
            strerror_guess(e),
            fence.fd,
            fence.stream_fd
        ),
    );

    if e == 0 && fence.fd >= 0 {
        let mut pfd = PollFd {
            fd: fence.fd,
            events: 1,
            revents: 0,
        };
        let t0 = std::time::Instant::now();
        let n = unsafe { poll(&mut pfd as *mut PollFd as *mut core::ffi::c_void, 1, 1000) };
        let ms = t0.elapsed().as_millis();
        let verdict = if n > 0 && pfd.revents & 1 != 0 {
            "READABLE(no-dependency control)"
        } else if n == 0 {
            "TIMEOUT(still pending)"
        } else {
            "POLLERR"
        };
        line(
            "fence7/poll(sync_file)",
            verdict,
            format!(
                "fd={} poll={n} revents={:#x} after {ms}ms",
                fence.fd, pfd.revents
            ),
        );
        unsafe { close(fence.fd) };
    } else if e == 0 {
        line(
            "fence7/sync_file",
            "FAIL",
            "accepted trigger did not write an fd back",
        );
    }

    // Completion events are read from the kbase fd itself. Read the event that
    // this atom produced, so an accepted-but-invalid soft job cannot masquerade
    // as a fence-creation failure.
    for i in 0..8 {
        let mut epfd = PollFd {
            fd,
            events: 1,
            revents: 0,
        };
        let n = unsafe {
            poll(
                &mut epfd as *mut PollFd as *mut core::ffi::c_void,
                1,
                if i == 0 { 500 } else { 0 },
            )
        };
        if n <= 0 || epfd.revents & 1 == 0 {
            if i == 0 {
                line(
                    "fence7/event",
                    "INFO",
                    format!("poll={n} revents={:#x}", epfd.revents),
                );
            }
            break;
        }
        let mut ev = [0u8; 24];
        let got = unsafe { read(fd, ev.as_mut_ptr() as *mut core::ffi::c_void, ev.len()) };
        if got == ev.len() as isize {
            let code = u32::from_le_bytes([ev[0], ev[1], ev[2], ev[3]]);
            let verdict = match code {
                1 => "DONE",
                0x40 => "CONFIG_FAULT",
                0x2003 => "JOB_INVALID",
                _ => "FAIL",
            };
            line(
                "fence7/event",
                verdict,
                format!(
                    "#{i} event_code={code:#x} atom_number={} bytes={}",
                    ev[4],
                    hex(&ev)
                ),
            );
        } else {
            let e = errno();
            line("fence7/event", "FAIL", format!("#{i} read={got} errno={e}"));
            break;
        }
    }

    unsafe { close(sfd) };
}

/// Submit a soft-fence trigger using a caller-selected atom layout.
///
/// The driver accepts both the 64-byte `base_jd_atom_v2` and the 72-byte
/// `base_jd_atom` (the latter has an extra `seq_nr` at offset zero). This
/// helper keeps every field offset explicit so the two layouts cannot be mixed.
fn soft_fence_layout(
    label: &str,
    stride: usize,
    jc_off: usize,
    atom_number_off: usize,
    core_req_off: usize,
) {
    let Some(f) = kbase_open() else {
        line("open", "FAIL", "version_check/set_flags");
        return;
    };
    let fd = f.as_raw_fd();

    let mut name = [0u8; 32];
    name[..4].copy_from_slice(b"husk");
    let sfd = unsafe {
        ioctl(
            fd,
            0x4020_8018u64,
            name.as_mut_ptr() as *mut core::ffi::c_void,
        )
    };
    if sfd < 0 {
        let e = errno();
        line(
            &format!("{label}/stream_create"),
            "FAIL",
            format!("errno={e} ({})", strerror_guess(e)),
        );
        return;
    }

    let mut fence = BaseFence {
        fd: -1,
        stream_fd: sfd,
    };
    let mut atom = vec![0u8; stride];
    put_u64(&mut atom, jc_off, &mut fence as *mut BaseFence as u64);
    atom[atom_number_off] = 1;
    put_u32(&mut atom, core_req_off, BASE_JD_REQ_SOFT_FENCE_TRIGGER);
    line(
        &format!("{label}/atom"),
        "INFO",
        format!(
            "stride={stride} jc@{jc_off:#x}=user_ptr atom_number@{atom_number_off:#x}=1 core_req@{core_req_off:#x}={BASE_JD_REQ_SOFT_FENCE_TRIGGER:#x}"
        ),
    );

    // Do not confuse a context-creation event with our atom's event.
    for i in 0..8 {
        let mut pfd = PollFd {
            fd,
            events: 1,
            revents: 0,
        };
        let n = unsafe { poll(&mut pfd as *mut PollFd as *mut core::ffi::c_void, 1, 0) };
        if n <= 0 || pfd.revents & 1 == 0 {
            break;
        }
        let mut ev = [0u8; 24];
        let got = unsafe { read(fd, ev.as_mut_ptr() as *mut core::ffi::c_void, ev.len()) };
        if got != ev.len() as isize {
            break;
        }
        let code = u32::from_le_bytes([ev[0], ev[1], ev[2], ev[3]]);
        line(
            &format!("{label}/pre_event"),
            "INFO",
            format!("#{i} event_code={code:#x} atom_number={}", ev[4]),
        );
    }

    let mut js = JobSubmit {
        addr: atom.as_ptr() as u64,
        nr_atoms: 1,
        stride: stride as u32,
    };
    unsafe { alarm(5) };
    let e = ioctl_errno(
        fd,
        0x4010_8002,
        &mut js as *mut JobSubmit as *mut core::ffi::c_void,
    );
    unsafe { alarm(0) };
    line(
        &format!("{label}/job_submit"),
        if e == 0 { "ACCEPTED" } else { "REFUSED" },
        format!(
            "errno={e} ({}) fence_after_submit={{fd:{},stream_fd:{}}}",
            strerror_guess(e),
            fence.fd,
            fence.stream_fd
        ),
    );

    if e == 0 && fence.fd >= 0 {
        let mut pfd = PollFd {
            fd: fence.fd,
            events: 1,
            revents: 0,
        };
        let t0 = std::time::Instant::now();
        let n = unsafe { poll(&mut pfd as *mut PollFd as *mut core::ffi::c_void, 1, 1000) };
        let ms = t0.elapsed().as_millis();
        line(
            &format!("{label}/poll(sync_file)"),
            if n > 0 && pfd.revents & 1 != 0 {
                "READABLE(no-dependency control)"
            } else if n == 0 {
                "TIMEOUT(still pending)"
            } else {
                "POLLERR"
            },
            format!(
                "fd={} poll={n} revents={:#x} after {ms}ms",
                fence.fd, pfd.revents
            ),
        );
        unsafe { close(fence.fd) };
    } else if e == 0 {
        line(
            &format!("{label}/sync_file"),
            "FAIL",
            "accepted trigger did not write an fd back",
        );
    }

    for i in 0..8 {
        let mut pfd = PollFd {
            fd,
            events: 1,
            revents: 0,
        };
        let n = unsafe {
            poll(
                &mut pfd as *mut PollFd as *mut core::ffi::c_void,
                1,
                if i == 0 { 500 } else { 0 },
            )
        };
        if n <= 0 || pfd.revents & 1 == 0 {
            if i == 0 {
                line(
                    &format!("{label}/event"),
                    "INFO",
                    format!("poll={n} revents={:#x}", pfd.revents),
                );
            }
            break;
        }
        let mut ev = [0u8; 24];
        let got = unsafe { read(fd, ev.as_mut_ptr() as *mut core::ffi::c_void, ev.len()) };
        if got != ev.len() as isize {
            break;
        }
        let code = u32::from_le_bytes([ev[0], ev[1], ev[2], ev[3]]);
        let verdict = match code {
            1 => "DONE",
            0x40 => "CONFIG_FAULT",
            0x2003 => "JOB_INVALID",
            _ => "FAIL",
        };
        line(
            &format!("{label}/event"),
            verdict,
            format!(
                "#{i} event_code={code:#x} atom_number={} bytes={}",
                ev[4],
                hex(&ev)
            ),
        );
    }

    unsafe { close(sfd) };
}

/// 64-byte `base_jd_atom_v2` from the r32p1 UAPI header.
fn fence8() {
    soft_fence_layout("fence8/v2-64", 64, 0x00, 0x28, 0x2c);
}

/// 72-byte `base_jd_atom`: v2 plus a leading `seq_nr`.
fn fence9() {
    soft_fence_layout("fence9/seq-72", 72, 0x08, 0x30, 0x34);
}

/// Pending producer fence using a soft-event dependency.
///
/// Layout is the proven 64-byte `base_jd_atom_v2`. Atom 1 waits on a GPU-memory
/// event byte; atom 2 is a `SOFT_FENCE_TRIGGER` with an ORDER dependency on atom
/// 1. The returned sync_file must stay pending until `SOFT_EVENT_UPDATE` sets
/// the event and atom 1 completes.
fn fence10() {
    const SOFT_EVENT_WAIT: u32 = (1 << 9) | 0x5;
    #[repr(C)]
    struct SoftEventUpdate {
        event: u64,
        new_status: u32,
        flags: u32,
    }

    let Some(f) = kbase_open() else {
        line("open", "FAIL", "version_check/set_flags");
        return;
    };
    let fd = f.as_raw_fd();

    let Some(va) = alloc_pages(fd, 1, 0x200F) else {
        line("fence10/alloc", "FAIL", "flags=0x200f");
        return;
    };
    let p = unsafe { mmap(core::ptr::null_mut(), 4096, 3, 1, fd, va as i64) };
    if p as isize == -1 {
        let e = errno();
        line("fence10/mmap", "FAIL", format!("errno={e} gpu_va={va:#x}"));
        return;
    }
    unsafe { *(p as *mut u8) = 0 };
    line(
        "fence10/event_bo",
        "PASS",
        format!("gpu_va={va:#x} flags=0x200f value=0"),
    );

    let mut name = [0u8; 32];
    name[..4].copy_from_slice(b"husk");
    let sfd = unsafe {
        ioctl(
            fd,
            0x4020_8018u64,
            name.as_mut_ptr() as *mut core::ffi::c_void,
        )
    };
    if sfd < 0 {
        let e = errno();
        line(
            "fence10/stream_create",
            "FAIL",
            format!("errno={e} ({})", strerror_guess(e)),
        );
        unsafe { munmap(p, 4096) };
        return;
    }

    let mut fence = BaseFence {
        fd: -1,
        stream_fd: sfd,
    };
    let mut atoms = [0u8; 128];

    // atom 1: wait for the event byte to become 1
    put_u64(&mut atoms, 0x00, va);
    put_u32(&mut atoms, 0x28, 1);
    put_u32(&mut atoms, 0x2c, SOFT_EVENT_WAIT);

    // atom 2: create a sync_file and complete only after atom 1
    put_u64(&mut atoms, 64, &mut fence as *mut BaseFence as u64);
    atoms[64 + 0x24] = 1; // pre_dep[0].atom_id
    atoms[64 + 0x25] = 2; // BASE_JD_DEP_TYPE_ORDER
    put_u32(&mut atoms, 64 + 0x28, 2);
    put_u32(&mut atoms, 64 + 0x2c, BASE_JD_REQ_SOFT_FENCE_TRIGGER);

    let mut js = JobSubmit {
        addr: atoms.as_ptr() as u64,
        nr_atoms: 2,
        stride: 64,
    };
    unsafe { alarm(5) };
    let e = ioctl_errno(
        fd,
        0x4010_8002,
        &mut js as *mut JobSubmit as *mut core::ffi::c_void,
    );
    unsafe { alarm(0) };
    line(
        "fence10/job_submit",
        if e == 0 { "ACCEPTED" } else { "REFUSED" },
        format!(
            "errno={e} ({}) fence_after_submit={{fd:{},stream_fd:{}}}",
            strerror_guess(e),
            fence.fd,
            fence.stream_fd
        ),
    );
    if e != 0 || fence.fd < 0 {
        if e == 0 {
            line(
                "fence10/sync_file",
                "FAIL",
                "accepted trigger did not write an fd back",
            );
        }
        unsafe {
            close(sfd);
            munmap(p, 4096);
            let mut v = va;
            let _ = ioctl_errno(
                fd,
                0x4008_8007,
                &mut v as *mut u64 as *mut core::ffi::c_void,
            );
        }
        return;
    }

    let mut pfd = PollFd {
        fd: fence.fd,
        events: 1,
        revents: 0,
    };
    let t0 = std::time::Instant::now();
    let before_n = unsafe { poll(&mut pfd as *mut PollFd as *mut core::ffi::c_void, 1, 200) };
    let before_ms = t0.elapsed().as_millis();
    line(
        "fence10/poll(before set)",
        if before_n == 0 {
            "TIMEOUT(expected pending)"
        } else if before_n > 0 && pfd.revents & 1 != 0 {
            "READABLE(too early)"
        } else {
            "POLLERR"
        },
        format!(
            "fd={} poll={before_n} revents={:#x} after {before_ms}ms",
            fence.fd, pfd.revents
        ),
    );

    let mut update = SoftEventUpdate {
        event: va,
        new_status: 1,
        flags: 0,
    };
    let ue = ioctl_errno(
        fd,
        0x4010_801c,
        &mut update as *mut SoftEventUpdate as *mut core::ffi::c_void,
    );
    line(
        "fence10/soft_event_set",
        if ue == 0 { "PASS" } else { "FAIL" },
        format!("event={va:#x} errno={ue} ({})", strerror_guess(ue)),
    );

    let mut pfd2 = PollFd {
        fd: fence.fd,
        events: 1,
        revents: 0,
    };
    let t1 = std::time::Instant::now();
    let after_n = unsafe { poll(&mut pfd2 as *mut PollFd as *mut core::ffi::c_void, 1, 1000) };
    let after_ms = t1.elapsed().as_millis();
    line(
        "fence10/poll(after set)",
        if after_n > 0 && pfd2.revents & 1 != 0 {
            "READABLE"
        } else if after_n == 0 {
            "TIMEOUT"
        } else {
            "POLLERR"
        },
        format!(
            "fd={} poll={after_n} revents={:#x} after {after_ms}ms",
            fence.fd, pfd2.revents
        ),
    );

    for i in 0..8 {
        let mut epfd = PollFd {
            fd,
            events: 1,
            revents: 0,
        };
        let n = unsafe {
            poll(
                &mut epfd as *mut PollFd as *mut core::ffi::c_void,
                1,
                if i == 0 { 500 } else { 0 },
            )
        };
        if n <= 0 || epfd.revents & 1 == 0 {
            break;
        }
        let mut ev = [0u8; 24];
        let got = unsafe { read(fd, ev.as_mut_ptr() as *mut core::ffi::c_void, ev.len()) };
        if got != ev.len() as isize {
            break;
        }
        let code = u32::from_le_bytes([ev[0], ev[1], ev[2], ev[3]]);
        let verdict = match code {
            1 => "DONE",
            0x40 => "CONFIG_FAULT",
            0x2003 => "JOB_INVALID",
            _ => "FAIL",
        };
        line(
            "fence10/event",
            verdict,
            format!("#{i} event_code={code:#x} atom_number={}", ev[4]),
        );
    }

    unsafe {
        close(fence.fd);
        close(sfd);
        munmap(p, 4096);
        let mut v = va;
        let _ = ioctl_errno(
            fd,
            0x4008_8007,
            &mut v as *mut u64 as *mut core::ffi::c_void,
        );
    }
}

/// End-to-end pending producer fence on the r32p1 JM kbase.
///
/// The event BO is allocated with `SAME_VA`.  Before `mmap`, `mem_alloc`
/// returns a cookie (used as the mmap offset), not a GPU VA.  After mmap the
/// region is mapped at the returned CPU address and that address *is* the GPU
/// VA.  Passing the pre-mmap cookie to `SOFT_EVENT_WAIT` makes `kbase_vmap()`
/// fail and produces `BASE_JD_SW_EVENT_JOB_CANCELLED`.
///
/// Atom 1 waits on the event byte.  Atom 2 creates a sync_file and has an ORDER
/// dependency on atom 1.  The sync_file must stay pending until we set the event
/// through `SOFT_EVENT_UPDATE`, then become readable without another submit.
fn fence11() {
    let Some(f) = kbase_open() else {
        line("fence11/open", "FAIL", "version_check/set_flags");
        return;
    };
    let fd = f.as_raw_fd();

    let Some(cookie) = alloc_pages(fd, 1, 0x300F) else {
        line("fence11/alloc", "FAIL", "flags=0x300f");
        return;
    };
    let p = unsafe { mmap(core::ptr::null_mut(), 4096, 3, 1, fd, cookie as i64) };
    if p as isize == -1 {
        let e = errno();
        line(
            "fence11/mmap",
            "FAIL",
            format!("cookie={cookie:#x} errno={e} ({})", strerror_guess(e)),
        );
        return;
    }
    let event_va = p as u64;
    unsafe { *(p as *mut u8) = 0 };
    line(
        "fence11/event_bo",
        "PASS",
        format!("cookie={cookie:#x} mapped={p:p} gpu_va={event_va:#x} flags=0x300f value=0"),
    );

    let mut name = [0u8; 32];
    name[..4].copy_from_slice(b"husk");
    let sfd = unsafe {
        ioctl(
            fd,
            0x4020_8018u64,
            name.as_mut_ptr() as *mut core::ffi::c_void,
        )
    };
    if sfd < 0 {
        let e = errno();
        line(
            "fence11/stream_create",
            "FAIL",
            format!("errno={e} ({})", strerror_guess(e)),
        );
        unsafe { munmap(p, 4096) };
        return;
    }

    let mut fence = BaseFence {
        fd: -1,
        stream_fd: sfd,
    };
    let mut atoms = vec![0u8; ATOM_STRIDE_V2 * 2];

    // Atom 1: block on the event byte.
    put_u64(&mut atoms, ATOM_JC, event_va);
    atoms[ATOM_V2_ATOM_NUMBER] = 1;
    put_u32(&mut atoms, ATOM_V2_CORE_REQ, BASE_JD_REQ_SOFT_EVENT_WAIT);

    // Atom 2: create a sync_file only after atom 1 completes.
    let a2 = ATOM_STRIDE_V2;
    put_u64(
        &mut atoms,
        a2 + ATOM_JC,
        &mut fence as *mut BaseFence as u64,
    );
    atoms[a2 + ATOM_V2_PRE_DEP] = 1;
    atoms[a2 + ATOM_V2_PRE_DEP + 1] = BASE_JD_DEP_TYPE_ORDER;
    atoms[a2 + ATOM_V2_ATOM_NUMBER] = 2;
    put_u32(
        &mut atoms,
        a2 + ATOM_V2_CORE_REQ,
        BASE_JD_REQ_SOFT_FENCE_TRIGGER,
    );

    // Drain context-creation events so they cannot be mistaken for our atoms.
    for i in 0..8 {
        let mut epfd = PollFd {
            fd,
            events: 1,
            revents: 0,
        };
        let n = unsafe { poll(&mut epfd as *mut PollFd as *mut core::ffi::c_void, 1, 0) };
        if n <= 0 || epfd.revents & 1 == 0 {
            break;
        }
        let mut ev = [0u8; 24];
        let got = unsafe { read(fd, ev.as_mut_ptr() as *mut core::ffi::c_void, ev.len()) };
        if got != ev.len() as isize {
            break;
        }
        let code = u32::from_le_bytes([ev[0], ev[1], ev[2], ev[3]]);
        line(
            "fence11/pre_event",
            "INFO",
            format!("#{i} event_code={code:#x} atom_number={}", ev[4]),
        );
    }

    let mut js = JobSubmit {
        addr: atoms.as_ptr() as u64,
        nr_atoms: 2,
        stride: ATOM_STRIDE_V2 as u32,
    };
    unsafe { alarm(5) };
    let e = ioctl_errno(
        fd,
        0x4010_8002,
        &mut js as *mut JobSubmit as *mut core::ffi::c_void,
    );
    unsafe { alarm(0) };
    line(
        "fence11/job_submit",
        if e == 0 { "ACCEPTED" } else { "REFUSED" },
        format!(
            "errno={e} ({}) layout=v2-64 atoms=2 fence_after_submit={{fd:{},stream_fd:{}}}",
            strerror_guess(e),
            fence.fd,
            fence.stream_fd
        ),
    );
    if e != 0 || fence.fd < 0 {
        if e == 0 {
            line(
                "fence11/sync_file",
                "FAIL",
                "accepted trigger did not write an fd back",
            );
        }
        unsafe {
            close(sfd);
            munmap(p, 4096);
            let mut v = event_va;
            let _ = ioctl_errno(
                fd,
                0x4008_8007,
                &mut v as *mut u64 as *mut core::ffi::c_void,
            );
        }
        return;
    }
    sync_file_info("fence11/sync_file(pending)", fence.fd);

    let mut before = PollFd {
        fd: fence.fd,
        events: 1,
        revents: 0,
    };
    let t0 = std::time::Instant::now();
    let before_n = unsafe { poll(&mut before as *mut PollFd as *mut core::ffi::c_void, 1, 250) };
    let before_ms = t0.elapsed().as_millis();
    line(
        "fence11/poll(before set)",
        if before_n == 0 {
            "TIMEOUT(expected pending)"
        } else if before_n > 0 && before.revents & 1 != 0 {
            "READABLE(too early)"
        } else {
            "POLLERR"
        },
        format!(
            "fd={} poll={before_n} revents={:#x} after {before_ms}ms",
            fence.fd, before.revents
        ),
    );

    let mut update = SoftEventUpdate {
        event: event_va,
        new_status: 1,
        flags: 0,
    };
    let ue = ioctl_errno(
        fd,
        0x4010_801c,
        &mut update as *mut SoftEventUpdate as *mut core::ffi::c_void,
    );
    line(
        "fence11/soft_event_set",
        if ue == 0 { "PASS" } else { "FAIL" },
        format!("event={event_va:#x} errno={ue} ({})", strerror_guess(ue)),
    );
    let mut after = PollFd {
        fd: fence.fd,
        events: 1,
        revents: 0,
    };
    let t1 = std::time::Instant::now();
    let after_n = unsafe { poll(&mut after as *mut PollFd as *mut core::ffi::c_void, 1, 2000) };
    let after_ms = t1.elapsed().as_millis();
    line(
        "fence11/poll(after set)",
        if after_n > 0 && after.revents & 1 != 0 {
            "READABLE"
        } else if after_n == 0 {
            "TIMEOUT"
        } else {
            "POLLERR"
        },
        format!(
            "fd={} poll={after_n} revents={:#x} after {after_ms}ms",
            fence.fd, after.revents
        ),
    );
    sync_file_info("fence11/sync_file(signaled)", fence.fd);

    for i in 0..8 {
        let mut epfd = PollFd {
            fd,
            events: 1,
            revents: 0,
        };
        let n = unsafe {
            poll(
                &mut epfd as *mut PollFd as *mut core::ffi::c_void,
                1,
                if i == 0 { 500 } else { 0 },
            )
        };
        if n <= 0 || epfd.revents & 1 == 0 {
            break;
        }
        let mut ev = [0u8; 24];
        let got = unsafe { read(fd, ev.as_mut_ptr() as *mut core::ffi::c_void, ev.len()) };
        if got != ev.len() as isize {
            break;
        }
        let code = u32::from_le_bytes([ev[0], ev[1], ev[2], ev[3]]);
        let verdict = match code {
            1 => "DONE",
            0x40 => "CONFIG_FAULT",
            0x2003 => "JOB_INVALID",
            0x4002 => "JOB_CANCELLED",
            _ => "FAIL",
        };
        line(
            "fence11/event",
            verdict,
            format!("#{i} event_code={code:#x} atom_number={}", ev[4]),
        );
    }

    unsafe {
        close(fence.fd);
        close(sfd);
        munmap(p, 4096);
        let mut v = event_va;
        let _ = ioctl_errno(
            fd,
            0x4008_8007,
            &mut v as *mut u64 as *mut core::ffi::c_void,
        );
    }
}

/// Which offset does this kernel actually read `core_req` from?
///
/// A bogus `core_req` must be refused during validation (EINVAL) -- kbase knows
/// exactly which requirement bits exist. So write 0x202 and 0xffff at each
/// candidate offset and see which one the driver reacts to. This turns the
/// "the vendor's binary writes it at 0x40" observation into a measurement about
/// *this* kernel, which is what we actually have to satisfy.
fn crmap() {
    let Some(f) = kbase_open() else {
        line("open", "FAIL", "version_check/set_flags");
        return;
    };
    let fd = f.as_raw_fd();
    let fence = BaseFence {
        fd: -1,
        stream_fd: 5,
    };
    for off in [0x34usize, 0x40usize] {
        for val in [BASE_JD_REQ_SOFT_FENCE_TRIGGER, 0xffffu32] {
            let mut atom = [0u8; ATOM_STRIDE];
            put_u64(&mut atom, ATOM_JC, &fence as *const BaseFence as u64);
            put_u32(&mut atom, ATOM_ATOM_NUMBER as usize, 1);
            put_u32(&mut atom, off, val);
            let mut js = JobSubmit {
                addr: atom.as_ptr() as u64,
                nr_atoms: 1,
                stride: ATOM_STRIDE as u32,
            };
            let e = ioctl_errno(
                fd,
                0x4010_8002,
                &mut js as *mut JobSubmit as *mut core::ffi::c_void,
            );
            line(
                "crmap",
                if e == 0 { "ACCEPTED" } else { "REFUSED" },
                format!(
                    "core_req@{off:#x}={val:#x} errno={e} ({})",
                    strerror_guess(e)
                ),
            );
        }
    }
}

extern "C" {
    fn alarm(secs: u32) -> u32;
}

/// One atom, one candidate `core_req` offset/value, one process.
///
/// The previous version submitted four atoms in one process and hung inside the
/// second ioctl, which told us nothing except "some combination blocks". One
/// case per process plus `alarm(5)` means a blocking combination costs one
/// process and prints exactly where it stopped.
fn cr(off_hex: &str, val_hex: &str) {
    let off = usize::from_str_radix(off_hex.trim_start_matches("0x"), 16).unwrap_or(0x40);
    let val = u32::from_str_radix(val_hex.trim_start_matches("0x"), 16).unwrap_or(0);
    let Some(f) = kbase_open() else {
        line("open", "FAIL", "version_check/set_flags");
        return;
    };
    let fd = f.as_raw_fd();
    // No stream: this probe is only about whether validation reads these bytes.
    let fence = BaseFence {
        fd: -1,
        stream_fd: -1,
    };
    let mut atom = [0u8; ATOM_STRIDE];
    put_u64(&mut atom, ATOM_JC, &fence as *const BaseFence as u64);
    put_u32(&mut atom, ATOM_ATOM_NUMBER as usize, 1);
    put_u32(&mut atom, off, val);
    let mut js = JobSubmit {
        addr: atom.as_ptr() as u64,
        nr_atoms: 1,
        stride: ATOM_STRIDE as u32,
    };
    line(
        "cr/pre_submit",
        "INFO",
        format!("core_req@{off:#x}={val:#x} stride={ATOM_STRIDE}"),
    );
    unsafe { alarm(5) };
    let e = ioctl_errno(
        fd,
        0x4010_8002,
        &mut js as *mut JobSubmit as *mut core::ffi::c_void,
    );
    unsafe { alarm(0) };
    line(
        "cr/post_submit",
        if e == 0 { "ACCEPTED" } else { "REFUSED" },
        format!("errno={e} ({})", strerror_guess(e)),
    );
    line("cr/done", "INFO", "survived exit");
}

/// What comes out of the stream fd?
///
/// POLLIN fires after *any* accepted submit, including one whose `core_req` is a
/// bit pattern the driver cannot possibly accept (0xffff). So the stream is not
/// "fence signalled" -- it is more likely a queue the driver pushes something
/// into. Read it and print the bytes instead of guessing.
fn rd(off_hex: &str, val_hex: &str, submit: bool) {
    let off = usize::from_str_radix(off_hex.trim_start_matches("0x"), 16).unwrap_or(0x40);
    let val = u32::from_str_radix(val_hex.trim_start_matches("0x"), 16).unwrap_or(0);
    let Some(f) = kbase_open() else {
        line("open", "FAIL", "version_check/set_flags");
        return;
    };
    let fd = f.as_raw_fd();
    let mut name = [0u8; 32];
    name[..4].copy_from_slice(b"husk");
    let sfd = unsafe {
        ioctl(
            fd,
            0x4020_8018u64,
            name.as_mut_ptr() as *mut core::ffi::c_void,
        )
    };
    if sfd < 0 {
        let e = errno();
        line("rd/stream_create", "FAIL", format!("errno={e}"));
        return;
    }
    line("rd/stream_create", "PASS", format!("timeline fd={sfd}"));

    if submit {
        let fence = BaseFence {
            fd: -1,
            stream_fd: sfd,
        };
        let mut atom = [0u8; ATOM_STRIDE];
        put_u64(&mut atom, ATOM_JC, &fence as *const BaseFence as u64);
        put_u32(&mut atom, ATOM_ATOM_NUMBER as usize, 1);
        put_u32(&mut atom, off, val);
        let mut js = JobSubmit {
            addr: atom.as_ptr() as u64,
            nr_atoms: 1,
            stride: ATOM_STRIDE as u32,
        };
        let e = ioctl_errno(
            fd,
            0x4010_8002,
            &mut js as *mut JobSubmit as *mut core::ffi::c_void,
        );
        line(
            "rd/job_submit",
            if e == 0 { "ACCEPTED" } else { "REFUSED" },
            format!("core_req@{off:#x}={val:#x} errno={e}"),
        );
    }

    for _ in 0..2 {
        let mut buf = [0u8; 64];
        let n = unsafe { read(sfd, buf.as_mut_ptr() as *mut core::ffi::c_void, buf.len()) };
        if n < 0 {
            let e = errno();
            line(
                "rd/read",
                "FAIL",
                format!("errno={e} ({})", strerror_guess(e)),
            );
            break;
        }
        let hex: Vec<String> = buf[..n as usize]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        line(
            "rd/read",
            "INFO",
            format!("n={n} bytes=[{}]", hex.join(" ")),
        );
        if n == 0 {
            break;
        }
    }
    unsafe { close(sfd) };
}

/// Is JOB_SUBMIT actually reading our array at all?
///
/// A pointer that maps nothing must fault (EFAULT) if the driver copies the
/// atoms; a stride the driver does not agree with must be refused. Both checks
/// are free -- neither can put work on the GPU. If they all come back 0, then
/// `JOB_SUBMIT` here is accepting and doing nothing, and the whole kbase route
/// needs a different entry point.
fn badptr() {
    let Some(f) = kbase_open() else {
        line("open", "FAIL", "version_check/set_flags");
        return;
    };
    let fd = f.as_raw_fd();
    for addr in [0xdead_0000u64, 0x1u64, 0u64] {
        let mut js = JobSubmit {
            addr,
            nr_atoms: 1,
            stride: ATOM_STRIDE as u32,
        };
        unsafe { alarm(5) };
        let e = ioctl_errno(
            fd,
            0x4010_8002,
            &mut js as *mut JobSubmit as *mut core::ffi::c_void,
        );
        unsafe { alarm(0) };
        line(
            "badptr",
            if e == 0 {
                "ACCEPTED(suspicious)"
            } else {
                "REFUSED"
            },
            format!(
                "addr={addr:#x} nr_atoms=1 stride=72 errno={e} ({})",
                strerror_guess(e)
            ),
        );
    }
}

/// Same atoms pointer, different strides. A driver that cares about the atom
/// size refuses the wrong ones.
fn stride_probe() {
    let Some(f) = kbase_open() else {
        line("open", "FAIL", "version_check/set_flags");
        return;
    };
    let fd = f.as_raw_fd();
    let atom = [0u8; 72];
    for stride in [72u32, 56, 64, 0] {
        let mut js = JobSubmit {
            addr: atom.as_ptr() as u64,
            nr_atoms: 1,
            stride,
        };
        unsafe { alarm(5) };
        let e = ioctl_errno(
            fd,
            0x4010_8002,
            &mut js as *mut JobSubmit as *mut core::ffi::c_void,
        );
        unsafe { alarm(0) };
        line(
            "stride",
            if e == 0 { "ACCEPTED" } else { "REFUSED" },
            format!("stride={stride} errno={e} ({})", strerror_guess(e)),
        );
    }
}

/// `struct kbase_ioctl_mem_sync { u64 handle; u64 user_addr; u64 size; u8 type;
/// u8 padding[7]; }` (`0x4020800f`). `type` 0 = clean, 1 = invalidate.
///
/// Without this the BO read-back can lie: the GPU/kernel writes into physical
/// memory while our mapping stays in cache, so a byte that really changed reads
/// back as the old value (a false negative that looks exactly like "the atom did
/// not run").
#[repr(C)]
struct MemSync {
    handle: u64,
    user_addr: u64,
    size: u64,
    kind: u8,
    padding: [u8; 7],
}

fn mem_sync(fd: c_int, gpu_va: u64, user_ptr: u64, size: u64, kind: u8) -> i32 {
    let mut ms = MemSync {
        handle: gpu_va,
        user_addr: user_ptr,
        size,
        kind,
        padding: [0; 7],
    };
    ioctl_errno(
        fd,
        0x4020_800f,
        &mut ms as *mut MemSync as *mut core::ffi::c_void,
    )
}

const BASE_JD_REQ_SOFT_EVENT_SET: u32 = (1 << 9) | 0x6; // 0x206

/// Direct evidence that an atom *ran*, with no reliance on any completion API.
///
/// `BASE_JD_REQ_SOFT_EVENT_SET` makes the *kernel's* soft-job handler write 1 to
/// the event word at `jc` (a GPU address). So: zero a BO, submit, wait, read the
/// BO back through the CPU mapping. Value 1 means the atom was decoded
/// (right `core_req` offset), scheduled, and executed. Value 0 means the atom
/// was ignored -- which is what "job_submit returned 0" alone cannot tell us.
fn sevt(off_hex: &str) {
    let off = usize::from_str_radix(off_hex.trim_start_matches("0x"), 16).unwrap_or(0x40);
    let Some(f) = kbase_open() else {
        line("open", "FAIL", "version_check/set_flags");
        return;
    };
    let fd = f.as_raw_fd();
    let Some(va) = alloc_pages(fd, 1, 0xF) else {
        line("sevt/alloc", "FAIL", "flags=0xf");
        return;
    };
    let p = unsafe { mmap(core::ptr::null_mut(), 4096, 3, 1, fd, va as i64) };
    if p as isize == -1 {
        let e = errno();
        line("sevt/alloc", "FAIL", format!("mmap errno={e}"));
        return;
    }
    let b = p as *mut u8;
    unsafe {
        for i in 0..64 {
            *b.add(i) = 0
        }
    };
    line(
        "sevt/bo",
        "INFO",
        format!("gpu_va={va:#x} pre=[0x{:02x}]", unsafe { *b }),
    );

    let mut atom = [0u8; ATOM_STRIDE];
    put_u64(&mut atom, ATOM_JC, va);
    put_u32(&mut atom, ATOM_ATOM_NUMBER as usize, 1);
    put_u32(&mut atom, off, BASE_JD_REQ_SOFT_EVENT_SET);
    let mut js = JobSubmit {
        addr: atom.as_ptr() as u64,
        nr_atoms: 1,
        stride: ATOM_STRIDE as u32,
    };
    unsafe { alarm(8) };
    let e = ioctl_errno(
        fd,
        0x4010_8002,
        &mut js as *mut JobSubmit as *mut core::ffi::c_void,
    );
    unsafe { alarm(0) };
    line(
        &format!("sevt/submit@{off:#x}"),
        if e == 0 { "ACCEPTED" } else { "REFUSED" },
        format!("core_req=0x206 errno={e} ({})", strerror_guess(e)),
    );

    for ms in [10u64, 40, 150] {
        std::thread::sleep(std::time::Duration::from_millis(ms));
        // invalidate before every read: see mem_sync's comment
        let se = mem_sync(fd, va, p as u64, 64, 1);
        let v = unsafe { *b };
        line(
            &format!("sevt/mem_sync +{ms}ms"),
            if se == 0 { "PASS" } else { "FAIL" },
            format!("errno={se}"),
        );
        line(
            &format!("sevt/read +{ms}ms"),
            if v == 1 { "EXECUTED" } else { "UNCHANGED" },
            format!("event word = {v}"),
        );
    }
    unsafe {
        munmap(p, 4096);
        let mut v = va;
        let _ = ioctl_errno(
            fd,
            0x4008_8007,
            &mut v as *mut u64 as *mut core::ffi::c_void,
        );
    }
}

/// `KBASE_IOCTL_GET_DDK_VERSION` (`0x4010800d`, nr 13): the kernel tells us which
/// userspace DDK build it was written against. Everything we assume about atom
/// layout is an assumption about that build, so ask instead of guessing.
fn ddkver() {
    let Some(f) = kbase_open() else {
        line("open", "FAIL", "version_check/set_flags");
        return;
    };
    let fd = f.as_raw_fd();
    let mut buf = [0u8; 128];
    let mut args: [u64; 2] = [buf.as_mut_ptr() as u64, 128];
    let r = unsafe {
        ioctl(
            fd,
            0x4010_800du64,
            args.as_mut_ptr() as *mut core::ffi::c_void,
        )
    };
    if r < 0 {
        let e = errno();
        line(
            "ddk_version",
            "FAIL",
            format!("errno={e} ({})", strerror_guess(e)),
        );
        return;
    }
    let n = r as usize;
    let txt: String = buf[..n.min(buf.len())]
        .iter()
        .take_while(|b| **b != 0)
        .map(|b| *b as char)
        .collect();
    line(
        "ddk_version",
        "PASS",
        format!("returned {n} bytes: {txt:?}"),
    );
}

/// r32 keeps a legacy slot: `compat_core_req` (u16 @0x22) is the pre-10.3
/// `core_req` and the header says it "can be used in ioctl call for job
/// submission as long as UK 10.2 legacy is supported". If this kernel takes the
/// legacy path, the u32 at 0x40 is ignored and only this one counts.
fn sevt_compat(off_hex: &str) {
    let off = usize::from_str_radix(off_hex.trim_start_matches("0x"), 16).unwrap_or(0x40);
    let Some(f) = kbase_open() else {
        line("open", "FAIL", "version_check/set_flags");
        return;
    };
    let fd = f.as_raw_fd();
    let Some(va) = alloc_pages(fd, 1, 0xF) else {
        line("sevt2/alloc", "FAIL", "flags=0xf");
        return;
    };
    let p = unsafe { mmap(core::ptr::null_mut(), 4096, 3, 1, fd, va as i64) };
    if p as isize == -1 {
        let e = errno();
        line("sevt2/alloc", "FAIL", format!("mmap errno={e}"));
        return;
    }
    let b = p as *mut u8;
    unsafe {
        for i in 0..64 {
            *b.add(i) = 0
        }
    };

    let mut atom = [0u8; ATOM_STRIDE];
    put_u64(&mut atom, ATOM_JC, va);
    put_u32(&mut atom, ATOM_ATOM_NUMBER as usize, 1);
    // compat slot (u16) and the modern slot (u32) both say SOFT_EVENT_SET
    atom[0x22..0x24].copy_from_slice(&(BASE_JD_REQ_SOFT_EVENT_SET as u16).to_le_bytes());
    put_u32(&mut atom, off, BASE_JD_REQ_SOFT_EVENT_SET);

    let mut js = JobSubmit {
        addr: atom.as_ptr() as u64,
        nr_atoms: 1,
        stride: ATOM_STRIDE as u32,
    };
    unsafe { alarm(8) };
    let e = ioctl_errno(
        fd,
        0x4010_8002,
        &mut js as *mut JobSubmit as *mut core::ffi::c_void,
    );
    unsafe { alarm(0) };
    line(
        &format!("sevt2/submit@{off:#x}+compat"),
        if e == 0 { "ACCEPTED" } else { "REFUSED" },
        format!("errno={e} ({})", strerror_guess(e)),
    );
    for ms in [20u64, 200] {
        std::thread::sleep(std::time::Duration::from_millis(ms));
        let v = unsafe { *b };
        line(
            &format!("sevt2/read +{ms}ms"),
            if v == 1 { "EXECUTED" } else { "UNCHANGED" },
            format!("event word = {v}"),
        );
    }
    unsafe {
        munmap(p, 4096);
        let mut v = va;
        let _ = ioctl_errno(
            fd,
            0x4008_8007,
            &mut v as *mut u64 as *mut core::ffi::c_void,
        );
    }
}

/// `KBASE_IOCTL_MEM_JIT_INIT` (`0x4018800e`, nr 14, 24 bytes). r32 grew this
/// struct past the 16 bytes of older headers, so the fields past `va_pages` are a
/// guess -- which is why the only thing this mode claims is "does calling it
/// unlock atom execution", nothing about its contents being right.
#[repr(C)]
struct MemJitInit {
    va_pages: u64,
    phys_pages: u64,
    max_allocations: u8,
    trim_level: u8,
    group_id: u8,
    padding: [u8; 5],
}

fn jitinit_then_sevt(off_hex: &str) {
    let off = usize::from_str_radix(off_hex.trim_start_matches("0x"), 16).unwrap_or(0x40);
    let Some(f) = kbase_open() else {
        line("open", "FAIL", "version_check/set_flags");
        return;
    };
    let fd = f.as_raw_fd();
    for (vap, php) in [(1u64, 1u64), (1024, 1024)] {
        let mut ji = MemJitInit {
            va_pages: vap,
            phys_pages: php,
            max_allocations: 1,
            trim_level: 0,
            group_id: 0,
            padding: [0; 5],
        };
        let e = ioctl_errno(
            fd,
            0x4018_800e,
            &mut ji as *mut MemJitInit as *mut core::ffi::c_void,
        );
        line(
            "jitinit",
            if e == 0 { "PASS" } else { "FAIL" },
            format!(
                "va_pages={vap} phys_pages={php} errno={e} ({})",
                strerror_guess(e)
            ),
        );
        if e == 0 {
            break;
        }
    }
    // same atom-shape test as sevt, now after a JIT init
    let Some(va) = alloc_pages(fd, 1, 0xF) else {
        line("sevtjit/alloc", "FAIL", "flags=0xf");
        return;
    };
    let p = unsafe { mmap(core::ptr::null_mut(), 4096, 3, 1, fd, va as i64) };
    if p as isize == -1 {
        let e = errno();
        line("sevtjit/alloc", "FAIL", format!("mmap errno={e}"));
        return;
    }
    let b = p as *mut u8;
    unsafe {
        for i in 0..64 {
            *b.add(i) = 0
        }
    };
    let mut atom = [0u8; ATOM_STRIDE];
    put_u64(&mut atom, ATOM_JC, va);
    put_u32(&mut atom, ATOM_ATOM_NUMBER as usize, 1);
    put_u32(&mut atom, off, BASE_JD_REQ_SOFT_EVENT_SET);
    let mut js = JobSubmit {
        addr: atom.as_ptr() as u64,
        nr_atoms: 1,
        stride: ATOM_STRIDE as u32,
    };
    unsafe { alarm(8) };
    let e = ioctl_errno(
        fd,
        0x4010_8002,
        &mut js as *mut JobSubmit as *mut core::ffi::c_void,
    );
    unsafe { alarm(0) };
    line(
        &format!("sevtjit/submit@{off:#x}"),
        if e == 0 { "ACCEPTED" } else { "REFUSED" },
        format!("errno={e}"),
    );
    std::thread::sleep(std::time::Duration::from_millis(200));
    let v = unsafe { *b };
    line(
        "sevtjit/read +200ms",
        if v == 1 { "EXECUTED" } else { "UNCHANGED" },
        format!("event word = {v}"),
    );
    unsafe {
        munmap(p, 4096);
        let mut vv = va;
        let _ = ioctl_errno(
            fd,
            0x4008_8007,
            &mut vv as *mut u64 as *mut core::ffi::c_void,
        );
    }
}

/// Same side-effect test with a caller-chosen allocation flags word.
///
/// The kernel-side soft-job handler has to reach the event word through a kernel
/// (kcpu) mapping; that mapping may only exist for certain allocation flags. If
/// `BASE_MEM_CACHED_CPU` (0x1000) or `BASE_MEM_COHERENT_SYSTEM` (0x400) changes
/// the result, then "soft jobs are dead here" was really "our BO was not
/// reachable from the kernel".
fn sevtf(off_hex: &str, flags_hex: &str) {
    let off = usize::from_str_radix(off_hex.trim_start_matches("0x"), 16).unwrap_or(0x40);
    let flags = u64::from_str_radix(flags_hex.trim_start_matches("0x"), 16).unwrap_or(0xF);
    let Some(f) = kbase_open() else {
        line("open", "FAIL", "version_check/set_flags");
        return;
    };
    let fd = f.as_raw_fd();
    let Some(va) = alloc_pages(fd, 1, flags) else {
        line("sevtf/alloc", "FAIL", format!("flags={flags:#x}"));
        return;
    };
    let p = unsafe { mmap(core::ptr::null_mut(), 4096, 3, 1, fd, va as i64) };
    if p as isize == -1 {
        let e = errno();
        line(
            "sevtf/alloc",
            "FAIL",
            format!("mmap errno={e} flags={flags:#x}"),
        );
        return;
    }
    let b = p as *mut u8;
    unsafe {
        for i in 0..64 {
            *b.add(i) = 0
        }
    };
    let mut atom = [0u8; ATOM_STRIDE];
    put_u64(&mut atom, ATOM_JC, va);
    put_u32(&mut atom, ATOM_ATOM_NUMBER as usize, 1);
    put_u32(&mut atom, off, BASE_JD_REQ_SOFT_EVENT_SET);
    let mut js = JobSubmit {
        addr: atom.as_ptr() as u64,
        nr_atoms: 1,
        stride: ATOM_STRIDE as u32,
    };
    unsafe { alarm(8) };
    let e = ioctl_errno(
        fd,
        0x4010_8002,
        &mut js as *mut JobSubmit as *mut core::ffi::c_void,
    );
    unsafe { alarm(0) };
    line(
        &format!("sevtf/submit@{off:#x} flags={flags:#x}"),
        if e == 0 { "ACCEPTED" } else { "REFUSED" },
        format!("gpu_va={va:#x} errno={e}"),
    );
    std::thread::sleep(std::time::Duration::from_millis(200));
    let v = unsafe { *b };
    line(
        "sevtf/read +200ms",
        if v == 1 { "EXECUTED" } else { "UNCHANGED" },
        format!("event word = {v} (flags={flags:#x})"),
    );
    unsafe {
        munmap(p, 4096);
        let mut vv = va;
        let _ = ioctl_errno(
            fd,
            0x4008_8007,
            &mut vv as *mut u64 as *mut core::ffi::c_void,
        );
    }
}

const BASE_JD_REQ_SOFT_DUMP_CPU_GPU_TIME: u32 = (1 << 9) | 0x1; // 0x201

/// Does *any* atom with a kernel-side memory effect run?
///
/// `SOFT_EVENT_SET` was a bad observable: the UAPI says the event word's address
/// is only a *key*, and userspace is the one that writes it, so "the byte stayed
/// 0" does not prove the atom did not run.
///
/// `SOFT_DUMP_CPU_GPU_TIME` is different: the driver itself writes the current
/// CPU/GPU timestamps into the buffer at `jc`. If those bytes change, the atom was
/// decoded and executed -- and *which* offset made it happen answers the
/// `core_req` question at the same time.
fn sdump(off_hex: &str) {
    let off = usize::from_str_radix(off_hex.trim_start_matches("0x"), 16).unwrap_or(0x40);
    let Some(f) = kbase_open() else {
        line("open", "FAIL", "version_check/set_flags");
        return;
    };
    let fd = f.as_raw_fd();
    let Some(va) = alloc_pages(fd, 1, 0xF) else {
        line("sdump/alloc", "FAIL", "flags=0xf");
        return;
    };
    let p = unsafe { mmap(core::ptr::null_mut(), 4096, 3, 1, fd, va as i64) };
    if p as isize == -1 {
        let e = errno();
        line("sdump/alloc", "FAIL", format!("mmap errno={e}"));
        return;
    }
    let b = p as *mut u8;
    unsafe {
        for i in 0..256 {
            *b.add(i) = 0
        }
    };
    let before: Vec<String> = unsafe { (0..16).map(|i| format!("{:02x}", *b.add(i))).collect() };

    let mut atom = [0u8; ATOM_STRIDE];
    put_u64(&mut atom, ATOM_JC, va);
    put_u32(&mut atom, ATOM_ATOM_NUMBER as usize, 1);
    put_u32(&mut atom, off, BASE_JD_REQ_SOFT_DUMP_CPU_GPU_TIME);
    let mut js = JobSubmit {
        addr: atom.as_ptr() as u64,
        nr_atoms: 1,
        stride: ATOM_STRIDE as u32,
    };
    unsafe { alarm(8) };
    let e = ioctl_errno(
        fd,
        0x4010_8002,
        &mut js as *mut JobSubmit as *mut core::ffi::c_void,
    );
    unsafe { alarm(0) };
    line(
        &format!("sdump/submit@{off:#x}"),
        if e == 0 { "ACCEPTED" } else { "REFUSED" },
        format!(
            "core_req=0x201 errno={e} ({}) gpu_va={va:#x}",
            strerror_guess(e)
        ),
    );
    std::thread::sleep(std::time::Duration::from_millis(250));
    let after: Vec<String> = unsafe { (0..16).map(|i| format!("{:02x}", *b.add(i))).collect() };
    let changed = before != after;
    line(
        "sdump/read +250ms",
        if changed { "WROTE-MEMORY" } else { "UNCHANGED" },
        format!("before=[{}] after=[{}]", before.join(""), after.join("")),
    );
    unsafe {
        munmap(p, 4096);
        let mut v = va;
        let _ = ioctl_errno(
            fd,
            0x4008_8007,
            &mut v as *mut u64 as *mut core::ffi::c_void,
        );
    }
}

/// Field-map probe: write `val` (default 0x1) at one offset of an otherwise zero
/// atom whose `jc` is 0, submit, print the answer. One process per offset.
///
/// `jc = 0` is the safety property: if the driver does accept the atom and puts it
/// on the GPU, the job-chain fetch faults at address 0 immediately instead of
/// running whatever the garbage fields describe. The information we want is only
/// "did validation read this field", which the errno answers either way.
fn crz(off_hex: &str, val_hex: &str) {
    let off = usize::from_str_radix(off_hex.trim_start_matches("0x"), 16).unwrap_or(0);
    let val = u32::from_str_radix(val_hex.trim_start_matches("0x"), 16).unwrap_or(1);
    let Some(f) = kbase_open() else {
        line("open", "FAIL", "version_check/set_flags");
        return;
    };
    let fd = f.as_raw_fd();
    let mut atom = [0u8; ATOM_STRIDE];
    put_u32(&mut atom, off, val);
    let mut js = JobSubmit {
        addr: atom.as_ptr() as u64,
        nr_atoms: 1,
        stride: ATOM_STRIDE as u32,
    };
    unsafe { alarm(6) };
    let e = ioctl_errno(
        fd,
        0x4010_8002,
        &mut js as *mut JobSubmit as *mut core::ffi::c_void,
    );
    unsafe { alarm(0) };
    line(
        &format!("crz@{off:#x}={val:#x}"),
        if e == 0 { "ACCEPTED" } else { "REFUSED" },
        format!("errno={e} ({})", strerror_guess(e)),
    );
    let _ = f;
}

/// `KBASE_IOCTL_KINSTR_JM_FD` (`0xc0088033`, nr 51, `union kbase_kinstr_jm_fd`).
///
/// r32's UAPI has a *real* atom-state channel: the kernel pushes
/// QUEUE / START / STOP / COMPLETE records into a circular buffer that we read
/// through an fd. That is the first mechanism on this device that is documented
/// to report "atom finished" to userspace, so it replaces the timeline fd guess
/// from evidence 40.
///
/// in:  {u16 count; u8 padding[6]}   count must be a power of two
/// out: {u16 size; u8 version; u8 padding[5]}   size = record size
#[repr(C)]
struct KinstrFd {
    a: u16,
    b: u8,
    pad: [u8; 5],
}

fn kinstr(count: u16, submit_atom: bool) {
    let Some(f) = kbase_open() else {
        line("open", "FAIL", "version_check/set_flags");
        return;
    };
    let fd = f.as_raw_fd();
    let mut kfd = KinstrFd {
        a: count,
        b: 0,
        pad: [0; 5],
    };
    let r = unsafe {
        ioctl(
            fd,
            0xc008_8033u64,
            &mut kfd as *mut KinstrFd as *mut core::ffi::c_void,
        )
    };
    if r < 0 {
        let e = errno();
        line(
            "kinstr/ioctl",
            "FAIL",
            format!("errno={e} ({})", strerror_guess(e)),
        );
        return;
    }
    line(
        "kinstr/ioctl",
        "PASS",
        format!("fd={r} record_size={} version={}", kfd.a, kfd.b),
    );
    let rec_size = kfd.a as usize;
    let sfd = r;

    // one atom: a soft fence trigger on a stream, so the atom must reach COMPLETE
    let mut name = [0u8; 32];
    name[..4].copy_from_slice(b"husk");
    let timeline = unsafe {
        ioctl(
            fd,
            0x4020_8018u64,
            name.as_mut_ptr() as *mut core::ffi::c_void,
        )
    };
    if submit_atom && timeline >= 0 {
        let fence = BaseFence {
            fd: -1,
            stream_fd: timeline,
        };
        let mut atom = [0u8; ATOM_STRIDE];
        put_u64(&mut atom, ATOM_JC, &fence as *const BaseFence as u64);
        put_u32(&mut atom, ATOM_ATOM_NUMBER as usize, 1);
        put_u32(
            &mut atom,
            ATOM_CORE_REQ_STOCK_HDR,
            BASE_JD_REQ_SOFT_FENCE_TRIGGER,
        );
        let mut js = JobSubmit {
            addr: atom.as_ptr() as u64,
            nr_atoms: 1,
            stride: ATOM_STRIDE as u32,
        };
        let e = ioctl_errno(
            fd,
            0x4010_8002,
            &mut js as *mut JobSubmit as *mut core::ffi::c_void,
        );
        line(
            "kinstr/job_submit",
            if e == 0 { "ACCEPTED" } else { "REFUSED" },
            format!("core_req@0x34=0x202 atom_number=1 errno={e}"),
        );
    }

    // poll, then drain
    let mut pfd = PollFd {
        fd: sfd,
        events: 1,
        revents: 0,
    };
    let n = unsafe { poll(&mut pfd as *mut PollFd as *mut core::ffi::c_void, 1, 1500) };
    line(
        "kinstr/poll",
        if n > 0 { "READABLE" } else { "TIMEOUT" },
        format!("poll={n} revents={:#x}", pfd.revents),
    );

    let mut buf = [0u8; 512];
    for round in 0..4 {
        let got = unsafe { read(sfd, buf.as_mut_ptr() as *mut core::ffi::c_void, buf.len()) };
        if got < 0 {
            let e = errno();
            line(
                &format!("kinstr/read#{round}"),
                "FAIL",
                format!("errno={e} ({})", strerror_guess(e)),
            );
            break;
        }
        if got == 0 {
            line(&format!("kinstr/read#{round}"), "INFO", "0 bytes");
            break;
        }
        let bytes: Vec<String> = buf[..got as usize]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        line(
            &format!("kinstr/read#{round}"),
            "INFO",
            format!("{got} bytes: {}", bytes.join(" ")),
        );
        if rec_size > 0 {
            let mut off = 0usize;
            while off + rec_size <= got as usize && off < 256 {
                line(
                    &format!("kinstr/rec#{round}"),
                    "INFO",
                    decode_state(&buf[off..off + rec_size]),
                );
                off += rec_size;
            }
        }
    }
    unsafe {
        close(sfd);
        if timeline >= 0 {
            close(timeline);
        }
    }
}

/// Decoded `struct kbase_kinstr_jm_atom_state_change` (kinstr version 2, 16 bytes):
/// `{ u64 timestamp; s8 state; u8 atom; u8 flags; u8 reserved; u8 data[4] }`
/// where state is 0 QUEUE / 1 START / 2 STOP / 3 COMPLETE.
fn decode_state(b: &[u8]) -> String {
    let ts = u64::from_le_bytes(b[0..8].try_into().unwrap_or([0; 8]));
    let st = b[8] as i8;
    let name = match st {
        0 => "QUEUE",
        1 => "START",
        2 => "STOP",
        3 => "COMPLETE",
        _ => "?",
    };
    format!(
        "ts={ts} state={name}({st}) atom={} flags={:#x} data={:02x?}",
        b[9],
        b[10],
        &b[12..16]
    )
}

/// A **hardware** atom: `core_req = BASE_JD_REQ_FS` with `jc = 0` (a GPU address
/// that maps nothing). If the driver really dispatches hardware work, the kinstr
/// channel must show QUEUE -> START(slot) and then STOP instead of COMPLETE, and
/// the GPU will fault on the job-chain fetch at address 0 instead of running
/// whatever the zeroed chain would describe.
///
/// This is the question that decides the whole "guest drives Mali directly" route:
/// soft atoms only prove the JD queue works, hardware atoms prove the GPU submit
/// path does.
fn kinstr_hw(off_hex: &str, core_req_hex: &str, jc_hex: &str) {
    let cr_off = usize::from_str_radix(off_hex.trim_start_matches("0x"), 16).unwrap_or(0x34);
    let core_req = u32::from_str_radix(core_req_hex.trim_start_matches("0x"), 16).unwrap_or(1);
    let jc = u64::from_str_radix(jc_hex.trim_start_matches("0x"), 16).unwrap_or(0);
    let Some(f) = kbase_open() else {
        line("open", "FAIL", "version_check/set_flags");
        return;
    };
    let fd = f.as_raw_fd();
    let mut kfd = KinstrFd {
        a: 256,
        b: 0,
        pad: [0; 5],
    };
    let kfd_ret = unsafe {
        ioctl(
            fd,
            0xc008_8033u64,
            &mut kfd as *mut KinstrFd as *mut core::ffi::c_void,
        )
    };
    if kfd_ret < 0 {
        let e = errno();
        line("kinstr_hw/ioctl", "FAIL", format!("errno={e}"));
        return;
    }
    let rec_size = kfd.a as usize;
    line(
        "kinstr_hw/kfd",
        "PASS",
        format!("fd={kfd_ret} record_size={rec_size} version={}", kfd.b),
    );

    let mut atom = [0u8; ATOM_STRIDE];
    put_u64(&mut atom, ATOM_JC, jc);
    put_u32(&mut atom, ATOM_ATOM_NUMBER as usize, 1);
    put_u32(&mut atom, cr_off, core_req);
    let mut js = JobSubmit {
        addr: atom.as_ptr() as u64,
        nr_atoms: 1,
        stride: ATOM_STRIDE as u32,
    };
    line(
        "kinstr_hw/before_submit",
        "INFO",
        format!("core_req@{cr_off:#x}={core_req:#x} jc={jc:#x}"),
    );
    unsafe { alarm(6) };
    let e = ioctl_errno(
        fd,
        0x4010_8002,
        &mut js as *mut JobSubmit as *mut core::ffi::c_void,
    );
    unsafe { alarm(0) };
    line(
        "kinstr_hw/job_submit",
        if e == 0 { "ACCEPTED" } else { "REFUSED" },
        format!(
            "core_req={core_req:#x} jc={jc:#x} errno={e} ({})",
            strerror_guess(e)
        ),
    );
    if e != 0 {
        unsafe { close(kfd_ret) };
        return;
    }

    // give the scheduler a moment, then drain whatever the kernel recorded
    for round in 0..3 {
        let mut pfd = PollFd {
            fd: kfd_ret,
            events: 1,
            revents: 0,
        };
        let n = unsafe { poll(&mut pfd as *mut PollFd as *mut core::ffi::c_void, 1, 800) };
        let mut buf = [0u8; 256];
        let got = unsafe {
            read(
                kfd_ret,
                buf.as_mut_ptr() as *mut core::ffi::c_void,
                buf.len(),
            )
        };
        if got <= 0 {
            line(
                &format!("kinstr_hw/round{round}"),
                "INFO",
                format!(
                    "poll={n} revents={:#x} read={got} (no more events)",
                    pfd.revents
                ),
            );
            break;
        }
        let mut off = 0usize;
        while off + rec_size <= got as usize {
            line(
                &format!("kinstr_hw/rec{round}"),
                "INFO",
                decode_state(&buf[off..off + rec_size]),
            );
            off += rec_size;
        }
    }
    unsafe { close(kfd_ret) };
}

/// Hardware atom whose `jc` points at a real, mapped BO (zeroed) instead of 0.
///
/// `jc = 0` completed without a START, which says the driver treats a null job
/// chain as "nothing to do". A zeroed chain in real GPU memory is different: it
/// has to be fetched, so a START (and then a STOP if the descriptor is invalid)
/// is what a working submit path looks like.
fn kinstr_hwbo(core_req_hex: &str, fill: u32) {
    let core_req = u32::from_str_radix(core_req_hex.trim_start_matches("0x"), 16).unwrap_or(1);
    let Some(f) = kbase_open() else {
        line("open", "FAIL", "version_check/set_flags");
        return;
    };
    let fd = f.as_raw_fd();
    let mut kfd = KinstrFd {
        a: 256,
        b: 0,
        pad: [0; 5],
    };
    let kfd_ret = unsafe {
        ioctl(
            fd,
            0xc008_8033u64,
            &mut kfd as *mut KinstrFd as *mut core::ffi::c_void,
        )
    };
    if kfd_ret < 0 {
        let e = errno();
        line("hwbo/kinstr", "FAIL", format!("errno={e}"));
        return;
    }
    let rec_size = kfd.a as usize;
    let Some(va) = alloc_pages(fd, 1, 0xF) else {
        line("hwbo/alloc", "FAIL", "flags=0xf");
        return;
    };
    let p = unsafe { mmap(core::ptr::null_mut(), 4096, 3, 1, fd, va as i64) };
    if p as isize == -1 {
        let e = errno();
        line("hwbo/alloc", "FAIL", format!("mmap errno={e}"));
        return;
    }
    // fill = 0 means "a job descriptor that cannot be valid"; anything else is a
    // recognisable byte pattern so a fault address would be identifiable later.
    let b = p as *mut u32;
    unsafe {
        for i in 0..256 {
            *b.add(i) = fill
        }
    };
    line(
        "hwbo/chain",
        "INFO",
        format!("gpu_va={va:#x} first word={fill:#x} core_req={core_req:#x}"),
    );

    let mut atom = [0u8; ATOM_STRIDE];
    put_u64(&mut atom, ATOM_JC, va);
    put_u32(&mut atom, ATOM_ATOM_NUMBER as usize, 1);
    put_u32(&mut atom, ATOM_CORE_REQ_STOCK_HDR, core_req);
    let mut js = JobSubmit {
        addr: atom.as_ptr() as u64,
        nr_atoms: 1,
        stride: ATOM_STRIDE as u32,
    };
    unsafe { alarm(8) };
    let e = ioctl_errno(
        fd,
        0x4010_8002,
        &mut js as *mut JobSubmit as *mut core::ffi::c_void,
    );
    unsafe { alarm(0) };
    line(
        "hwbo/job_submit",
        if e == 0 { "ACCEPTED" } else { "REFUSED" },
        format!("errno={e} ({})", strerror_guess(e)),
    );

    for round in 0..4 {
        let mut pfd = PollFd {
            fd: kfd_ret,
            events: 1,
            revents: 0,
        };
        let n = unsafe { poll(&mut pfd as *mut PollFd as *mut core::ffi::c_void, 1, 1000) };
        let mut buf = [0u8; 256];
        let got = unsafe {
            read(
                kfd_ret,
                buf.as_mut_ptr() as *mut core::ffi::c_void,
                buf.len(),
            )
        };
        if got <= 0 {
            line(
                &format!("hwbo/round{round}"),
                "INFO",
                format!("poll={n} revents={:#x} read={got}", pfd.revents),
            );
            break;
        }
        let mut off = 0usize;
        while off + rec_size <= got as usize {
            line(
                &format!("hwbo/rec{round}"),
                "INFO",
                decode_state(&buf[off..off + rec_size]),
            );
            off += rec_size;
        }
    }
    unsafe {
        close(kfd_ret);
        munmap(p, 4096);
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
            line(
                "kbase_open",
                "FAIL",
                format!("errno={}", e.raw_os_error().unwrap_or(0)),
            );
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
                    let head: Vec<String> = buf[..8.min(buf.len())]
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect();
                    line(
                        "nrscan",
                        "INFO",
                        format!(
                            "nr={nr} size={size} cmd={cmd:#x} -> {} buf0={}",
                            if e == 0 {
                                "OK".to_string()
                            } else {
                                format!("errno={e}")
                            },
                            head.join("")
                        ),
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
            line(
                "kbase_open",
                "FAIL",
                format!("errno={}", e.raw_os_error().unwrap_or(0)),
            );
            return;
        }
    };
    let fd = f.as_raw_fd();
    for (major, minor) in [(11u16, 13u16), (11u16, 38u16), (0u16, 0u16)] {
        // a fresh pair only works once per fd, so each is its own open
        let f = OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/mali0")
            .unwrap();
        let fd = f.as_raw_fd();
        let mut args = KbaseVersionCheck { major, minor };
        let e = ioctl_errno(
            fd,
            KBASE_IOCTL_VERSION_CHECK,
            &mut args as *mut _ as *mut core::ffi::c_void,
        );
        line(
            "ver",
            if e == 0 { "PASS" } else { "INFO" },
            format!(
                "asked {major}.{minor} -> errno={e} out={}.{}",
                args.major, args.minor
            ),
        );
        let _ = fd;
    }
    let _ = fd;
}

/// CS_GET_GLB_IFACE (nr 51, size 24 → 0xc0188033). JM 的 KINSTR_JM_FD 也是 nr 51，
/// 但 size 8（0xc0088033）。两个都活了才说明这棵树同时编进了 JM 和 CSF。
/// 判据：返回 0 且 glb_version != 0 → CSF 真的活着（可以走 Mesa panthor）。
fn csf() {
    let Some(f) = kbase_open() else {
        line("open", "FAIL", "version_check/set_flags");
        return;
    };
    let fd = f.as_raw_fd();
    let mut buf = [0u64; 3]; // 24 bytes: in{2×u32} + 2 pointers, 输出按 u32 解读
    let r = unsafe {
        ioctl(
            fd,
            0xc018_8033u64,
            buf.as_mut_ptr() as *mut core::ffi::c_void,
        )
    };
    if r < 0 {
        let e = errno();
        line(
            "csf/glb_iface",
            "FAIL",
            format!("errno={e} ({})  → 0xc0188033 不被接受", strerror_guess(e)),
        );
    } else {
        let u = unsafe { core::slice::from_raw_parts(buf.as_ptr() as *const u32, 6) };
        line("csf/glb_iface", "PASS", format!(
            "ret={r} glb_version={}.{} features={:#x} group_num={} prfcnt_size={} total_stream_num={} instr_features={:#x}",
            u[0] >> 16, u[0] & 0xffff, u[1], u[2], u[3], u[4], u[5]));
        line(
            "csf/verdict",
            if u[0] != 0 { "CSF_ALIVE" } else { "CSF_DEAD" },
            "glb_version==0 视为 CSF 未启用",
        );
    }
    // 对照：JM 的 KINSTR_JM_FD 同一 nr、不同 size，确认两条 ioctl 共存
    let mut kfd = KinstrFd {
        a: 16,
        b: 0,
        pad: [0; 5],
    };
    let r2 = unsafe {
        ioctl(
            fd,
            0xc008_8033u64,
            &mut kfd as *mut KinstrFd as *mut core::ffi::c_void,
        )
    };
    if r2 < 0 {
        let e = errno();
        line(
            "csf/kinstr_jm",
            "FAIL",
            format!("errno={e} ({})  → JM 侧不可用", strerror_guess(e)),
        );
    } else {
        line(
            "csf/kinstr_jm",
            "PASS",
            format!("fd={r2} record_size={} version={}", kfd.a, kfd.b),
        );
        unsafe { close(r2) };
    }
}

/// MEM_IMPORT (nr 22, union 24B → 0xc0188016)。r32 这份 UAPI **没有** export，
/// 只有 import：所以零拷贝方向是「app 分配 dma-buf → guest 导入 → GPU 直接写」。
/// `phandle` 是指向用户态 `int fd` 的指针，不是 fd 数值本身。
fn imp(arg: &str) {
    let Some(f) = kbase_open() else {
        line("open", "FAIL", "version_check/set_flags");
        return;
    };
    let fd = f.as_raw_fd();
    let mut cases: Vec<(String, i64)> = vec![("fd=-1".into(), -1)];
    if let Ok(n) = std::fs::File::open("/dev/null") {
        use std::os::fd::AsRawFd;
        cases.push((
            format!("/dev/null fd={}", n.as_raw_fd()),
            n.as_raw_fd() as i64,
        ));
    }
    if arg != "" {
        if let Ok(v) = arg.parse::<i64>() {
            cases.push((format!("arg={v}"), v));
        }
    }
    for (label, ph) in cases {
        let mut import_fd = ph as c_int;
        // CPU_RD | CPU_WR | GPU_RD | GPU_WR | COHERENT_SYSTEM | IMPORT_SHARED.
        let mut req = MemImport {
            flags: 0xf | (1 << 10) | (1 << 18),
            phandle: &mut import_fd as *mut c_int as usize as u64,
            ty: 2,
            padding: 0,
        };
        let r = unsafe {
            ioctl(
                fd,
                0xc018_8016u64,
                &mut req as *mut MemImport as *mut core::ffi::c_void,
            )
        };
        if r < 0 {
            let e = errno();
            let verdict = if e == 25 {
                "ENOTTY(命令不存在)"
            } else {
                "REACHED"
            };
            line(
                &format!("imp/{label}"),
                verdict,
                format!("errno={e} ({})", strerror_guess(e)),
            );
        } else {
            let va_pages = req.ty as u64 | ((req.padding as u64) << 32);
            line(
                &format!("imp/{label}"),
                "PASS",
                format!(
                    "flags={:#x} gpu_va={:#x} va_pages={va_pages}",
                    req.flags, req.phandle
                ),
            );
        }
    }
}

fn cmsg_align(n: usize) -> usize {
    let a = core::mem::size_of::<usize>();
    (n + a - 1) & !(a - 1)
}

unsafe fn recv_handle_fds(stream: &UnixStream) -> Result<Vec<c_int>, String> {
    let mut data = [0u8; 512];
    let mut control = [0u8; 512];
    let mut iov = IoVec {
        base: data.as_mut_ptr() as *mut c_void,
        len: data.len(),
    };
    let mut msg = MsgHdr {
        name: core::ptr::null_mut(),
        name_len: 0,
        iov: &mut iov,
        iov_len: 1,
        control: control.as_mut_ptr() as *mut c_void,
        control_len: control.len(),
        flags: 0,
    };
    let n = recvmsg(stream.as_raw_fd(), &mut msg, 0);
    if n < 0 {
        return Err(format!("recvmsg: {}", std::io::Error::last_os_error()));
    }
    line(
        "imp/socket",
        "recv",
        format!("bytes={n} control_len={}", msg.control_len),
    );

    let mut fds = Vec::new();
    let header = core::mem::size_of::<CmsgHdr>();
    let mut off = 0usize;
    while off + header <= msg.control_len {
        let h = control.as_ptr().add(off) as *const CmsgHdr;
        let len = (*h).len;
        if len < header || off + len > msg.control_len {
            break;
        }
        if (*h).level == SOL_SOCKET && (*h).ty == SCM_RIGHTS {
            let data_off = off + cmsg_align(header);
            let data_len = len.saturating_sub(cmsg_align(header));
            for i in 0..data_len / core::mem::size_of::<c_int>() {
                let p = control
                    .as_ptr()
                    .add(data_off + i * core::mem::size_of::<c_int>())
                    as *const c_int;
                fds.push(p.read_unaligned());
            }
        }
        off += cmsg_align(len);
    }
    if fds.is_empty() {
        return Err("SCM_RIGHTS contained no fds".to_string());
    }
    Ok(fds)
}

unsafe fn find_dmabuf(fds: &[c_int]) -> Option<c_int> {
    for &fd in fds {
        let target = std::fs::read_link(format!("/proc/self/fd/{fd}"))
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|e| format!("<readlink: {e}>"));
        let is_dmabuf = target.contains("dmabuf") || target.contains("dma-buf");
        line(
            "imp/socket",
            if is_dmabuf { "dmabuf" } else { "fd" },
            format!("fd={fd} target={target}"),
        );
        if is_dmabuf {
            return Some(fd);
        }
    }
    None
}

unsafe fn imp_socket(path: &str) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(25);
    let mut stream = loop {
        match UnixStream::connect(path) {
            Ok(stream) => break stream,
            Err(error) if std::time::Instant::now() < deadline => {
                let _ = error;
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(error) => {
                line("imp/socket", "FAIL", format!("connect {path}: {error}"));
                return;
            }
        }
    };
    line("imp/socket", "PASS", path);
    let mut frame_len = [0u8; 4];
    if let Err(e) = stream.read_exact(&mut frame_len) {
        line("imp/socket", "FAIL", format!("meta length: {e}"));
        return;
    }
    let meta_len = u32::from_be_bytes(frame_len) as usize;
    if meta_len == 0 || meta_len > 4096 {
        line("imp/socket", "FAIL", format!("meta length={meta_len}"));
        return;
    }
    let mut meta = vec![0u8; meta_len];
    if let Err(e) = stream.read_exact(&mut meta) {
        line("imp/socket", "FAIL", format!("meta: {e}"));
        return;
    }
    let meta = String::from_utf8_lossy(&meta);
    let mut width = 0usize;
    let mut height = 0usize;
    let mut stride = 0usize;
    let mut format = 0u32;
    for l in meta.lines() {
        if let Some(v) = l.strip_prefix("width=") {
            width = v.parse().unwrap_or(0);
        }
        if let Some(v) = l.strip_prefix("height=") {
            height = v.parse().unwrap_or(0);
        }
        if let Some(v) = l.strip_prefix("stride=") {
            stride = v.parse().unwrap_or(0);
        }
        if let Some(v) = l.strip_prefix("format=") {
            format = v.parse().unwrap_or(0);
        }
    }
    line(
        "imp/socket",
        "meta",
        format!("{}x{} stride={stride} format={format}", width, height),
    );

    let fds = match recv_handle_fds(&stream) {
        Ok(v) => v,
        Err(e) => {
            line("imp/socket", "FAIL", e);
            return;
        }
    };
    let Some(dmabuf) = find_dmabuf(&fds) else {
        line("imp/socket", "FAIL", "no dma-buf fd");
        for &fd in &fds {
            close(fd);
        }
        return;
    };

    let Some(kbase) = kbase_open() else {
        line("imp/mem_import", "FAIL", "kbase_open");
        for &fd in &fds {
            close(fd);
        }
        return;
    };

    let mut import_fd = dmabuf;
    let mut req = MemImport {
        flags: 0xf | (1 << 10) | (1 << 18),
        phandle: &mut import_fd as *mut c_int as usize as u64,
        ty: 2,
        padding: 0,
    };
    let e = ioctl_errno(
        kbase.as_raw_fd(),
        0xc018_8016u64 as c_ulong,
        &mut req as *mut MemImport as *mut c_void,
    );
    if e != 0 {
        line(
            "imp/mem_import",
            "FAIL",
            format!("errno={e} ({})", strerror_guess(e)),
        );
        for &fd in &fds {
            close(fd);
        }
        return;
    }
    let va_pages = req.ty as u64 | ((req.padding as u64) << 32);
    line(
        "imp/mem_import",
        "PASS",
        format!(
            "flags={:#x} cookie={:#x} va_pages={va_pages}",
            req.flags, req.phandle
        ),
    );

    let size = (va_pages as usize).saturating_mul(4096);
    let mapping = mmap(
        core::ptr::null_mut(),
        size,
        PROT_READ | PROT_WRITE,
        MAP_SHARED,
        kbase.as_raw_fd(),
        req.phandle as i64,
    );
    if mapping as isize == -1 {
        let e = errno();
        line(
            "imp/mmap",
            "FAIL",
            format!("errno={e} ({})", strerror_guess(e)),
        );
        for &fd in &fds {
            close(fd);
        }
        return;
    }
    line("imp/mmap", "PASS", format!("size={size} ptr={mapping:p}"));

    // UMM imports must be permanently mapped before GPU translation. `address`
    // points to an array of GPU VAs; for SAME_VA imports the post-mmap CPU
    // address is the GPU VA, not the pre-mmap cookie returned by MEM_IMPORT.
    let mut gpu_vas = [mapping as u64];
    let mut sticky = StickyResourceMap {
        count: gpu_vas.len() as u64,
        address: gpu_vas.as_mut_ptr() as usize as u64,
    };
    let sticky_errno = ioctl_errno(
        kbase.as_raw_fd(),
        0x4010_801du64 as c_ulong,
        &mut sticky as *mut StickyResourceMap as *mut c_void,
    );
    line(
        "imp/sticky_map",
        if sticky_errno == 0 { "PASS" } else { "FAIL" },
        format!(
            "errno={sticky_errno} ({}) gpu_va={:#x}",
            strerror_guess(sticky_errno),
            gpu_vas[0]
        ),
    );

    // The kbase SAME_VA window is the GPU translation mapping. UMM imports are
    // not guaranteed CPU-coherent there; the authoritative CPU view is the
    // dma-buf mapping itself.
    let direct = mmap(
        core::ptr::null_mut(),
        size,
        PROT_READ | PROT_WRITE,
        MAP_SHARED,
        dmabuf,
        0,
    );
    let mut direct_ok = false;
    let mut direct_mapped = false;
    if direct as isize == -1 {
        let e = errno();
        line(
            "imp/dmabuf_mmap",
            "FAIL",
            format!("errno={e} ({})", strerror_guess(e)),
        );
    } else {
        direct_mapped = true;
        let before = (direct as *const u32).read_unaligned();
        let pitch = stride.saturating_mul(4);
        if format == 1 && width > 0 && height > 0 && stride >= width {
            for y in 0..height {
                for x in 0..width {
                    let p = direct.cast::<u8>().add(y * pitch + x * 4) as *mut u32;
                    p.write_unaligned(0xff00_00ff);
                }
            }
            let direct_first = (direct as *const u32).read_unaligned();
            let kbase_first = (mapping as *const u32).read_unaligned();
            direct_ok = direct_first == 0xff00_00ff;
            line(
                "imp/write",
                "PASS",
                format!("red={}x{} before={before:#x}", width, height),
            );
            line(
                "imp/dmabuf_read",
                if direct_ok { "PASS" } else { "FAIL" },
                format!("direct_first={direct_first:#x}"),
            );
            line(
                "imp/kbase_view",
                "INFO",
                format!("kbase_first={kbase_first:#x} (CPU coherency is not required here)"),
            );
        }
        if direct_ok || format != 1 {
            let first = (direct as *const u32).read_unaligned();
            line(
                "imp/dmabuf_probe",
                "INFO",
                format!("format={format} first={first:#x}"),
            );
        }
        munmap(direct, size);
    }
    munmap(mapping, size);

    let ok = sticky_errno == 0 && direct_mapped && (direct_ok || format != 1);
    use std::io::Write;
    let ack = if ok { b'D' } else { b'F' };
    let _ = stream.write_all(&[ack]);
    for &fd in &fds {
        close(fd);
    }
    line("imp/socket", "DONE", format!("ack={}", ack as char));
}

// ------------------------------------------------------------ Adreno / KGSL

const DRM_IOCTL_VERSION: c_ulong = 0xc040_6400;
#[repr(C)]
struct DrmVersion {
    major: c_int,
    minor: c_int,
    patch: c_int,
    name_len: usize,
    name: *mut c_char,
    date_len: usize,
    date: *mut c_char,
    desc_len: usize,
    desc: *mut c_char,
}

fn drm_version(fd: c_int, label: &str) {
    let mut name = [0 as c_char; 128];
    let mut date = [0 as c_char; 32];
    let mut desc = [0 as c_char; 256];
    let mut v = DrmVersion {
        major: 0,
        minor: 0,
        patch: 0,
        name_len: name.len(),
        name: name.as_mut_ptr(),
        date_len: date.len(),
        date: date.as_mut_ptr(),
        desc_len: desc.len(),
        desc: desc.as_mut_ptr(),
    };
    let e = ioctl_errno(
        fd,
        DRM_IOCTL_VERSION,
        &mut v as *mut DrmVersion as *mut core::ffi::c_void,
    );
    if e != 0 {
        line(label, "FAIL", format!("DRM_IOCTL_VERSION errno={e}"));
        return;
    }
    let n = name.iter().position(|c| *c == 0).unwrap_or(name.len());
    let got = String::from_utf8_lossy(unsafe {
        std::slice::from_raw_parts(name.as_ptr() as *const u8, n)
    });
    line(
        label,
        if got == "kgsl" { "PASS" } else { "FAIL" },
        format!("name={got} version={}.{}.{}", v.major, v.minor, v.patch),
    );
}

/// Exercise the exported `drmGetNodeTypeFromFd` interposer through a real
/// third-party caller. `libva-drm` calls that symbol through its PLT; if the
/// loader failed to patch libva-drm's GOT, the stock libdrm implementation sees
/// a memfd rather than a DRM node and returns NULL. On Mali this intentionally
/// uses Husk's fake render node, so the result directly tests the interpose path
/// without needing Adreno hardware.
fn va_node() {
    const RTLD_NOW: c_int = 2;
    let render = match OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/dri/renderD128")
    {
        Ok(f) => f,
        Err(e) => {
            line(
                "va_node_open",
                "FAIL",
                format!(
                    "/dev/dri/renderD128 errno={}",
                    e.raw_os_error().unwrap_or(0)
                ),
            );
            return;
        }
    };
    let lib = unsafe { dlopen(b"libva-drm.so.2\0".as_ptr() as *const c_char, RTLD_NOW) };
    if lib.is_null() {
        line("va_node_dlopen", "FAIL", "libva-drm.so.2 did not load");
        return;
    }
    let get_sym = unsafe { dlsym(lib, b"vaGetDisplayDRM\0".as_ptr() as *const c_char) };
    let term_sym = unsafe { dlsym(lib, b"vaTerminate\0".as_ptr() as *const c_char) };
    if get_sym.is_null() || term_sym.is_null() {
        line(
            "va_node_symbols",
            "FAIL",
            format!("vaGetDisplayDRM={get_sym:p} vaTerminate={term_sym:p}"),
        );
        unsafe {
            dlclose(lib);
        }
        return;
    }
    let va_get: unsafe extern "C" fn(c_int) -> *mut c_void =
        unsafe { core::mem::transmute(get_sym) };
    let va_term: unsafe extern "C" fn(*mut c_void) -> c_int =
        unsafe { core::mem::transmute(term_sym) };
    let display = unsafe { va_get(render.as_raw_fd()) };
    if display.is_null() {
        line(
            "va_node_type",
            "FAIL",
            "vaGetDisplayDRM=NULL; drmGetNodeTypeFromFd interpose did not reach libva-drm",
        );
    } else {
        line(
            "va_node_type",
            "PASS",
            "vaGetDisplayDRM returned a display for the guest render node",
        );
        unsafe {
            va_term(display);
        }
    }
    unsafe {
        dlclose(lib);
    }
}

/// Probe the Adreno adapter from *inside a Husk guest*.
///
/// This intentionally opens the guest-visible render node, not `/dev/kgsl-3d0`:
/// the point is to prove the shim's `/dev/dri/renderD128` -> real KGSL fd path,
/// then check that the descriptor is the same device and that libdrm-facing
/// behaviour (`DRM_IOCTL_VERSION`, node type, dup inheritance) is correct.
fn adreno() {
    if !Path::new("/dev/kgsl-3d0").exists() {
        line(
            "adreno_kgsl",
            "SKIP",
            "/dev/kgsl-3d0 missing (not an Adreno/KGSL device)",
        );
        return;
    }
    let kgsl = match OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/kgsl-3d0")
    {
        Ok(f) => f,
        Err(e) => {
            line(
                "adreno_kgsl_open",
                "FAIL",
                format!("errno={}", e.raw_os_error().unwrap_or(0)),
            );
            return;
        }
    };
    line(
        "adreno_kgsl_open",
        "PASS",
        format!("/dev/kgsl-3d0 fd={}", kgsl.as_raw_fd()),
    );
    let render = match OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/dri/renderD128")
    {
        Ok(f) => f,
        Err(e) => {
            line(
                "adreno_render_open",
                "FAIL",
                format!("errno={}", e.raw_os_error().unwrap_or(0)),
            );
            return;
        }
    };
    let fd = render.as_raw_fd();
    line(
        "adreno_render_open",
        "PASS",
        format!("/dev/dri/renderD128 fd={fd} (must be the KGSL fd)"),
    );

    match (kgsl.metadata(), render.metadata()) {
        (Ok(a), Ok(b)) => line(
            "adreno_same_rdev",
            if a.dev() == b.dev() && a.rdev() == b.rdev() {
                "PASS"
            } else {
                "FAIL"
            },
            format!(
                "kgsl={}:{} render={}:{}",
                a.dev(),
                a.rdev(),
                b.dev(),
                b.rdev()
            ),
        ),
        (a, b) => line(
            "adreno_same_rdev",
            "FAIL",
            format!("metadata kgsl={a:?} render={b:?}"),
        ),
    }

    drm_version(fd, "adreno_drm_version");
    unsafe {
        let dfd = dup(fd);
        if dfd < 0 {
            line("adreno_dup", "FAIL", format!("dup errno={}", errno()));
        } else {
            line("adreno_dup", "PASS", format!("fd={dfd}"));
            drm_version(dfd, "adreno_dup_version");
            close(dfd);
        }
    }

    match std::fs::read_link(format!("/proc/self/fd/{fd}")) {
        Ok(p) => line(
            "adreno_proc_fd",
            if p.to_string_lossy().contains("/dev/kgsl-3d0") {
                "PASS"
            } else {
                "FAIL"
            },
            p.display().to_string(),
        ),
        Err(e) => line(
            "adreno_proc_fd",
            "FAIL",
            format!("readlink errno={}", e.raw_os_error().unwrap_or(0)),
        ),
    }
    line(
        "adreno_verdict",
        "DONE",
        "guest render node must be a real KGSL fd; Turnip enumeration is a separate test",
    );
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(|s| s.as_str()).unwrap_or("") {
        "nodes" => nodes(),
        "periph" => periph(),
        "adreno" => adreno(),
        "va-node" => va_node(),
        "kbase" => kbase(),
        "enc" => enc(),
        "nrscan" => nrscan(),
        "ver" => ver(),
        "submit0" => submit0(),
        "alloc1" => alloc1(),
        "query1" => query1(),
        "submit1" => submit1(),
        "free1" => free1(),
        "stream1" => stream1(),
        "fence0" => fence0(),
        "fence1" => fence1(),
        "fence2" => fence2(),
        "fence3" => fence3(),
        "fence4" => fence4(),
        "fence5" => fence5(),
        "fence6" => fence6(),
        "fence7" => fence7(),
        "fence8" => fence8(),
        "fence9" => fence9(),
        "fence10" => fence10(),
        "fence11" => fence11(),
        "crmap" => crmap(),
        "badptr" => badptr(),
        "sevt" => sevt(args.get(2).map(|s| s.as_str()).unwrap_or("0x40")),
        "ddkver" => ddkver(),
        "sevt2" => sevt_compat(args.get(2).map(|s| s.as_str()).unwrap_or("0x40")),
        "sevtjit" => jitinit_then_sevt(args.get(2).map(|s| s.as_str()).unwrap_or("0x40")),
        "kinstr_hwbo" => kinstr_hwbo(
            args.get(2).map(|s| s.as_str()).unwrap_or("0x1"),
            args.get(3)
                .and_then(|s| u32::from_str_radix(s.trim_start_matches("0x"), 16).ok())
                .unwrap_or(0),
        ),
        "kinstr_hw" => kinstr_hw(
            args.get(2).map(|s| s.as_str()).unwrap_or("0x34"),
            args.get(3).map(|s| s.as_str()).unwrap_or("0x1"),
            args.get(4).map(|s| s.as_str()).unwrap_or("0x0"),
        ),
        "kinstr" => kinstr(
            args.get(2)
                .and_then(|s| s.parse::<u16>().ok())
                .unwrap_or(256),
            args.get(3).map(|s| s.as_str()) != Some("nosubmit"),
        ),
        "csf" => csf(),
        "imp" => imp(args.get(2).map(|s| s.as_str()).unwrap_or("")),
        "imp_socket" => unsafe { imp_socket(args.get(2).map(|s| s.as_str()).unwrap_or("")) },
        "crz" => crz(
            args.get(2).map(|s| s.as_str()).unwrap_or("0x0"),
            args.get(3).map(|s| s.as_str()).unwrap_or("0x1"),
        ),
        "sdump" => sdump(args.get(2).map(|s| s.as_str()).unwrap_or("0x40")),
        "sevtf" => sevtf(
            args.get(2).map(|s| s.as_str()).unwrap_or("0x40"),
            args.get(3).map(|s| s.as_str()).unwrap_or("0xf"),
        ),
        "stride_probe" => stride_probe(),
        "rd" => rd(
            args.get(2).map(|s| s.as_str()).unwrap_or("0x40"),
            args.get(3).map(|s| s.as_str()).unwrap_or("0x202"),
            args.get(4).map(|s| s.as_str()) != Some("nosubmit"),
        ),
        "cr" => cr(
            args.get(2).map(|s| s.as_str()).unwrap_or("0x40"),
            args.get(3).map(|s| s.as_str()).unwrap_or("0x202"),
        ),
        "one" => {
            let major: u16 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
            let minor: u16 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0);
            one(major, minor)
        }
        "" => {
            eprintln!("usage: gpu-probe nodes|kbase|adreno|va-node|enc|nrscan|ver|submit0|submit1|alloc1|query1|free1|stream1|one <major> <minor>");
            std::process::exit(2);
        }
        other => {
            eprintln!("unknown mode: {other}");
            std::process::exit(2);
        }
    }
    let _ = File::open("/dev/null");
}
