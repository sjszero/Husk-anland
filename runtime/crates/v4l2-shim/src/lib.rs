//! v4l2-shim — guest `/dev/videoN` over the app's `camera.sock`.
//!
//! **no_std** (it is linked into the loader-embedded path shim). Design and the
//! prior art it follows: `docs/design/v4l2-prior-art.md`.
//!
//! Deltas from DroidNative's `dn_v4l2` (ptrace, FIFO + fd-passed shm):
//! - the guest-facing fd remains a memfd for identity and `close/dup`; preload
//!   maps its epoll watch to a hidden eventfd so SPA gets real readiness;
//! - `DQBUF` is one synchronous `FRAME` transaction on `camera.sock`;
//! - NV12 is the wire format; `YUYV` is converted here.
//!
//! All struct offsets below were dumped from the guest's own
//! `/usr/include/linux/videodev2.h` on the RMX3357 (see the prior-art doc), not
//! written from memory.

#![no_std]

pub const MAX_DEVS: usize = 4;
pub const MAX_BUFFERS: usize = 8;
pub const MAX_SIZES: usize = 16;
pub const V4L2_MAJOR: u32 = 81;

const SYS_READ: usize = 63;
const SYS_WRITE: usize = 64;
const SYS_CLOSE: usize = 57;
const SYS_OPENAT: usize = 56;
const SYS_GETDENTS64: usize = 61;
const SYS_MMAP: usize = 222;
const SYS_SOCKET: usize = 198;
const SYS_CONNECT: usize = 203;
const SYS_SETSOCKOPT: usize = 54;
// AArch64 generically numbered ABI. Do not copy x86_64 eventfd2=290.
const SYS_EVENTFD2: usize = 19;
const SOL_SOCKET: usize = 1;
const SO_RCVTIMEO: usize = 20;
const SO_SNDTIMEO: usize = 21;
const SYS_MEMFD_CREATE: usize = 279;
const SYS_READLINKAT: usize = 78;
const SYS_CLOCK_GETTIME: usize = 113;
const AT_FDCWD: isize = -100;
const O_DIRECTORY: usize = 0o40000; // AArch64 UAPI, not asm-generic 0o200000
const O_CLOEXEC: usize = 0o2000000;
const O_NONBLOCK: usize = 0o4000;
const CLOCK_MONOTONIC: usize = 1;
const CLOCK_BOOTTIME: usize = 7;
const CLOCK_REALTIME: usize = 0;

// ---- ioctls (verified on the guest header, aarch64) ----
pub const QUERYCAP: u64 = 0x8068_5600;
pub const ENUM_FMT: u64 = 0xc040_5602;
pub const G_FMT: u64 = 0xc0d0_5604;
pub const S_FMT: u64 = 0xc0d0_5605;
pub const REQBUFS: u64 = 0xc014_5608;
pub const QUERYBUF: u64 = 0xc058_5609;
pub const QBUF: u64 = 0xc058_560f;
pub const DQBUF: u64 = 0xc058_5611;
pub const STREAMON: u64 = 0x4004_5612;
pub const STREAMOFF: u64 = 0x4004_5613;
pub const G_PARM: u64 = 0xc0cc_5615;
pub const S_PARM: u64 = 0xc0cc_5616;
pub const ENUMINPUT: u64 = 0xc050_561a;
pub const G_INPUT: u64 = 0x8004_5626;
pub const S_INPUT: u64 = 0xc004_5627;
pub const TRY_FMT: u64 = 0xc0d0_5640;
pub const G_PRIORITY: u64 = 0x8004_5643;
pub const S_PRIORITY: u64 = 0x4004_5644;
pub const ENUM_FRAMESIZES: u64 = 0xc02c_564a;
pub const ENUM_FRAMEINTERVALS: u64 = 0xc034_564b;
pub const CROPCAP: u64 = 0xc02c_563a;
pub const G_CROP: u64 = 0xc014_563b;
pub const S_CROP: u64 = 0x4014_563c;
pub const G_SELECTION: u64 = 0xc040_565e;
pub const S_SELECTION: u64 = 0xc040_565f;

// ---- constants ----
pub const BUF_TYPE_VIDEO_CAPTURE: u32 = 1;
pub const MEMORY_MMAP: u32 = 1;
pub const MEMORY_USERPTR: u32 = 2;
pub const FRMSIZE_DISCRETE: u32 = 1;
pub const FRMIVAL_DISCRETE: u32 = 1;
pub const FIELD_NONE: u32 = 1;
pub const COLORSPACE_JPEG: u32 = 7;
pub const CAP_VIDEO_CAPTURE: u32 = 0x1;
pub const CAP_READWRITE: u32 = 0x0100_0000;
pub const CAP_STREAMING: u32 = 0x0400_0000;
pub const CAP_DEVICE_CAPS: u32 = 0x8000_0000;
pub const CAP_TIMEPERFRAME: u32 = 0x1000;
pub const BUF_FLAG_MAPPED: u32 = 1;
pub const BUF_FLAG_QUEUED: u32 = 2;
pub const BUF_FLAG_TIMESTAMP_MONOTONIC: u32 = 8192;
pub const INPUT_TYPE_CAMERA: u32 = 2;
pub const FMT_NV12: u32 = 0x3231_564e;
pub const FMT_YUYV: u32 = 0x5659_5559;
pub const SEL_TGT_CROP: u32 = 0;
pub const SEL_TGT_CROP_DEFAULT: u32 = 1;
pub const SEL_TGT_CROP_BOUNDS: u32 = 2;
const PRIORITY_DEFAULT: u32 = 2;
const EINVAL: i64 = 22;
const EACCES: i64 = 13;
const EBADF: i64 = 9;
const ENODEV: i64 = 19;
const EAGAIN: i64 = 11;
const EBUSY: i64 = 16;
const ENOTTY: i64 = 25;
const EFAULT: i64 = 14;
const EIO: i64 = 5;
const BUFFER_OFFSET_SHIFT: u32 = 24;
const MAX_TIMESTAMP_AGE_NS: u64 = 10_000_000_000;
// Linux sockaddr_un.sun_path is 108 bytes including the terminating NUL.
// The host-rooted camera.sock path is longer than a guest path and must be
// rejected before it reaches copy_from_slice, never discovered via panic.
const UNIX_PATH_MAX: usize = 108;
const SCRATCH_BYTES: usize = 1920 * 1080 * 3 / 2 + 4096;
const READV_BYTES: usize = 1920 * 1080 * 2 + 4096;

// 每个 guest 侧 fd 都是一个命名带上设备下标的 memfd，所以 fd 的身份可以由内核
// 单独判出来 —— 谁打开的都不影响。规则本身在 husk-fd-identity 里（只写一遍）。

/// Failure tracing: the preload/shim run before any logging facility exists, so
/// failures go to fd 2 as fixed strings. Cheap and only on error paths.
unsafe fn note(msg: &[u8]) {
    syscall3(SYS_WRITE, 2, msg.as_ptr() as usize, msg.len());
}

#[inline]
unsafe fn syscall6(n: usize, a: usize, b: usize, c: usize, d: usize, e: usize, f: usize) -> isize {
    let ret: isize;
    core::arch::asm!("svc 0", in("x8") n, in("x0") a, in("x1") b, in("x2") c, in("x3") d,
        in("x4") e, in("x5") f, lateout("x0") ret, options(nostack));
    ret
}

#[inline]
unsafe fn syscall3(n: usize, a: usize, b: usize, c: usize) -> isize {
    syscall6(n, a, b, c, 0, 0, 0)
}

#[inline]
unsafe fn ld32(p: *const u8, off: usize) -> u32 {
    core::ptr::read_unaligned(p.add(off) as *const u32)
}
#[inline]
unsafe fn st32(p: *mut u8, off: usize, v: u32) {
    core::ptr::write_unaligned(p.add(off) as *mut u32, v)
}
#[inline]
unsafe fn ld64(p: *const u8, off: usize) -> u64 {
    core::ptr::read_unaligned(p.add(off) as *const u64)
}
#[inline]
unsafe fn st64(p: *mut u8, off: usize, v: u64) {
    core::ptr::write_unaligned(p.add(off) as *mut u64, v)
}
unsafe fn st_bytes(dst: *mut u8, off: usize, src: &[u8]) {
    core::ptr::copy_nonoverlapping(src.as_ptr(), dst.add(off), src.len());
}
unsafe fn zero(dst: *mut u8, n: usize) {
    core::ptr::write_bytes(dst, 0, n);
}

/// Which device does `fd` refer to? Asked of the kernel, never of a private
/// table: an fd reaches us by dup, by fd passing or through the other
/// interposition layer, and a table keyed by number cannot see any of those.
/// Prior art: DroidNative's `dn_v4l2.c:device_of_fd` does exactly this with
/// /proc/<pid>/fd/<n> + st_dev/st_ino on every ioctl.
unsafe fn fd_dev_of_kernel(fd: i32) -> Option<usize> {
    if fd < 0 {
        return None;
    }
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

    let mut buf = [0u8; 96];
    // aarch64 has no plain readlink(): 78 is readlinkat(dirfd, path, buf, len).
    let r = syscall6(
        SYS_READLINKAT,
        AT_FDCWD as usize,
        path.as_ptr() as usize,
        buf.as_mut_ptr() as usize,
        buf.len(),
        0,
        0,
    );
    if r <= 0 {
        return None;
    }
    // 名字规则只写一遍（runtime/crates/fd-identity）：必须是 `/memfd:husk-v4l2-<N>`
    // 完整匹配，N 在 0..MAX_DEVS。旧实现只吃前缀 + 数字前缀，于是
    // `/memfd:husk-v4l2-12abc` 会被当成 12 号（再被 MAX_DEVS 挡掉，纯属运气），
    // `/memfd:husk-drm` 也会被同一个前缀吞进来。
    match husk_fd_identity::classify(&buf[..r as usize]) {
        Some(husk_fd_identity::FakeFd::V4l2(index)) if (index as usize) < MAX_DEVS => {
            Some(index as usize)
        }
        _ => None,
    }
}

#[derive(Clone, Copy)]
struct Buf {
    addr: usize,
    len: usize,
    queued: bool,
}

impl Buf {
    const fn new() -> Self {
        Buf {
            addr: 0,
            len: 0,
            queued: false,
        }
    }
}

struct Dev {
    opened: bool,
    fd: i32, // guest-visible memfd
    /// Internal epoll notification for the memfd facade. Regular files cannot
    /// be registered with epoll, so SPA would otherwise fall back to an idle
    /// source and stop issuing DQBUF once the PipeWire graph becomes active.
    event_fd: i32,
    sock: i32,
    w: u32,
    h: u32,
    fourcc: u32,
    memory: u32,
    nbuffers: u32,
    bufs: [Buf; MAX_BUFFERS],
    streaming: bool,
    nonblock: bool,
    nsizes: u32,
    sizes: [(u32, u32); MAX_SIZES],
    cam_id: [u8; 8],
    cam_id_len: usize,
    name: [u8; 32],
    name_len: usize,
}

impl Dev {
    const fn new() -> Self {
        Dev {
            opened: false,
            fd: -1,
            event_fd: -1,
            sock: -1,
            w: 0,
            h: 0,
            fourcc: FMT_YUYV,
            memory: 0,
            nbuffers: 0,
            bufs: [Buf::new(); MAX_BUFFERS],
            streaming: false,
            nonblock: false,
            nsizes: 0,
            sizes: [(0, 0); MAX_SIZES],
            cam_id: [0; 8],
            cam_id_len: 0,
            name: [0; 32],
            name_len: 0,
        }
    }

    fn size_image(&self) -> u32 {
        if self.fourcc == FMT_YUYV {
            self.w * self.h * 2
        } else {
            self.w * self.h * 3 / 2
        }
    }

    fn bytes_per_line(&self) -> u32 {
        if self.fourcc == FMT_YUYV {
            self.w * 2
        } else {
            self.w
        }
    }
}

pub struct Ctx {
    path: [u8; 256],
    path_len: usize,
    ready: bool,
    /// Why the last open failed: 0 none, 1 permission denied, 2 other.
    last_err: u8,
    /// The errno of the last failed `open`, for layers that must forward it.
    last_errno: i32,
    devs: [Dev; MAX_DEVS],
    scratch: *mut u8,
    scratch_len: usize,
    readv_scratch: *mut u8,
    /// Prefix each camera-socket handshake with a short trace line. The
    /// preload enables this only when `NR_V4L2_AUDIT=1`; the production path
    /// stays silent.
    trace: bool,
    /// Queue/dequeue can run at frame rate. Trace only the first few calls and
    /// a periodic sample so an audit does not change the path it measures.
    audit_qbuf: u32,
    audit_dqbuf: u32,
}

unsafe impl Sync for Ctx {}

impl Ctx {
    pub const fn empty() -> Self {
        Ctx {
            path: [0; 256],
            path_len: 0,
            ready: false,
            last_err: 0,
            last_errno: 0,
            devs: [Dev::new(), Dev::new(), Dev::new(), Dev::new()],
            scratch: core::ptr::null_mut(),
            scratch_len: 0,
            readv_scratch: core::ptr::null_mut(),
            trace: false,
            audit_qbuf: 0,
            audit_dqbuf: 0,
        }
    }

    pub fn set_trace(&mut self, enabled: bool) {
        self.trace = enabled;
    }

    fn trace(&self, msg: &[u8]) {
        if self.trace {
            // SAFETY: trace is only used from the no_std preload/shim paths,
            // where fd 2 is the process stderr inherited from the caller.
            unsafe { note(msg) };
        }
    }

    fn trace_dev(&self, prefix: &[u8], i: usize) {
        if !self.trace {
            return;
        }
        let mut buf = [0u8; 64];
        let n = prefix.len().min(buf.len() - 2);
        buf[..n].copy_from_slice(&prefix[..n]);
        buf[n] = b'0' + i as u8;
        buf[n + 1] = b'\n';
        unsafe { note(&buf[..n + 2]) };
    }

    fn trace_hex(&self, prefix: &[u8], value: usize) {
        if !self.trace {
            return;
        }
        let mut buf = [0u8; 96];
        let n = prefix.len().min(buf.len() - 20);
        buf[..n].copy_from_slice(&prefix[..n]);
        let mut at = n;
        buf[at] = b'0';
        at += 1;
        buf[at] = b'x';
        at += 1;
        let mut shift = (core::mem::size_of::<usize>() * 8) - 4;
        loop {
            let nibble = ((value >> shift) & 0xf) as u8;
            buf[at] = if nibble < 10 {
                b'0' + nibble
            } else {
                b'a' + nibble - 10
            };
            at += 1;
            if shift == 0 {
                break;
            }
            shift -= 4;
        }
        buf[at] = b'\n';
        unsafe { note(&buf[..at + 1]) };
    }

    /// Emit one bounded diagnostic line for the DQBUF state machine. This is
    /// only called from the audit-sampled path, so the production hot path
    /// never formats or writes anything.
    fn trace_dqbuf_state(&self, i: usize, streaming: bool, nbuffers: u32, queued: u32) {
        if !self.trace {
            return;
        }
        let mut buf = [0u8; 96];
        let prefix = b"[v4l2] DQBUF enter dev=";
        buf[..prefix.len()].copy_from_slice(prefix);
        let mut n = prefix.len();
        buf[n] = b'0' + i as u8;
        n += 1;
        for (label, value) in [
            (b" streaming=".as_ref(), streaming as u32),
            (b" nbuffers=".as_ref(), nbuffers),
            (b" queued=".as_ref(), queued),
        ] {
            buf[n..n + label.len()].copy_from_slice(label);
            n += label.len();
            if value >= 10 {
                buf[n] = b'0' + ((value / 10) % 10) as u8;
                n += 1;
            }
            buf[n] = b'0' + (value % 10) as u8;
            n += 1;
        }
        buf[n] = b'\n';
        n += 1;
        unsafe { note(&buf[..n]) };
    }

    /// `path` is the host path of `run/husk/camera.sock` inside the rootfs.
    pub unsafe fn init(&mut self, path: &[u8]) -> bool {
        if path.len() == 0 || path.len() > UNIX_PATH_MAX || path.len() >= self.path.len() {
            return false;
        }
        self.path[..path.len()].copy_from_slice(path);
        self.path_len = path.len();
        self.ready = true;
        true
    }

    unsafe fn scratch(&mut self) -> Option<*mut u8> {
        if !self.scratch.is_null() {
            return Some(self.scratch);
        }
        let p = syscall6(SYS_MMAP, 0, SCRATCH_BYTES, 3, 0x22, usize::MAX, 0);
        if p < 0 {
            return None;
        }
        self.scratch = p as *mut u8;
        self.scratch_len = SCRATCH_BYTES;
        Some(self.scratch)
    }

    unsafe fn readv_scratch(&mut self) -> Option<*mut u8> {
        if self.readv_scratch.is_null() {
            let p = syscall6(SYS_MMAP, 0, READV_BYTES, 3, 0x22, usize::MAX, 0);
            if p < 0 {
                return None;
            }
            self.readv_scratch = p as *mut u8;
        }
        Some(self.readv_scratch)
    }

    fn dev_of_fd(&mut self, fd: i32) -> Option<usize> {
        if fd < 0 {
            return None;
        }
        let mut i = 0;
        let mut any = false;
        while i < MAX_DEVS {
            if self.devs[i].opened {
                any = true;
                break;
            }
            i += 1;
        }
        // Nothing open: no readlinkat at all for processes that never touch a
        // camera, so the hooks stay free.
        if !any {
            return None;
        }
        // The descriptor number is never the identity. dup2/dup3 can replace
        // the number with an ordinary file, and a stale number match must not
        // keep routing ioctls into the camera emulation. The memfd name is
        // checked through /proc/self/fd on every access instead.
        let idx = unsafe { fd_dev_of_kernel(fd) }?;
        if self.devs[idx].opened {
            Some(idx)
        } else {
            unsafe { note(b"[husk v4l2] fd belongs to the other interposition layer\n") };
            None
        }
    }

    fn dev_of_path(n: u32) -> Option<usize> {
        if (n as usize) < MAX_DEVS {
            Some(n as usize)
        } else {
            None
        }
    }

    pub fn is_video_fd(&mut self, fd: i32) -> bool {
        self.dev_of_fd(fd).is_some()
    }

    /// Return the pollable object SPA should watch instead of the regular-file
    /// memfd exposed to the guest. The eventfd is internal and never replaces
    /// the guest-visible descriptor identity.
    pub fn poll_fd_for(&mut self, fd: i32) -> Option<i32> {
        let i = self.dev_of_fd(fd)?;
        let event_fd = self.devs[i].event_fd;
        if self.trace {
            unsafe { note(b"[v4l2] epoll facade -> eventfd\n") };
        }
        (event_fd >= 0).then_some(event_fd)
    }

    unsafe fn consume_event(&mut self, i: usize) {
        let event_fd = self.devs[i].event_fd;
        if event_fd < 0 {
            return;
        }
        let mut value = 0u64;
        syscall3(
            SYS_READ,
            event_fd as usize,
            (&mut value as *mut u64) as usize,
            8,
        );
    }

    unsafe fn arm_event(&mut self, i: usize) {
        let event_fd = self.devs[i].event_fd;
        if event_fd < 0 {
            return;
        }
        let value = 1u64;
        syscall3(
            SYS_WRITE,
            event_fd as usize,
            (&value as *const u64) as usize,
            8,
        );
    }

    pub fn update_nonblock(&mut self, fd: i32, on: bool) {
        if let Some(i) = self.dev_of_fd(fd) {
            self.devs[i].nonblock = on;
        }
    }

    // ---- socket plumbing ----

    unsafe fn connect(&mut self, i: usize) -> bool {
        let d = &mut self.devs[i];
        if d.sock >= 0 {
            return true;
        }
        let s = syscall3(SYS_SOCKET, 1, 1, 0); // AF_UNIX, SOCK_STREAM
        if s < 0 {
            return false;
        }
        let mut addr = [0u8; 110];
        addr[0] = 1; // AF_UNIX
        addr[1] = 0;
        let n = self.path_len;
        addr[2..2 + n].copy_from_slice(&self.path[..n]);
        let rc = syscall3(SYS_CONNECT, s as usize, addr.as_ptr() as usize, 110);
        if rc < 0 {
            syscall3(SYS_CLOSE, s as usize, 0, 0);
            return false;
        }
        self.devs[i].sock = s as i32;
        // A protocol peer that accepts and then goes silent must not hang the
        // whole V4L2 consumer. This timeout covers INFO/OPEN/frame replies; the
        // App may keep an idle camera device open between commands, but it only
        // reads from us when it has sent a command and is waiting for a reply.
        let tv = [5_i64, 0_i64];
        for opt in [SO_RCVTIMEO, SO_SNDTIMEO] {
            syscall6(
                SYS_SETSOCKOPT,
                s as usize,
                SOL_SOCKET,
                opt,
                tv.as_ptr() as usize,
                core::mem::size_of_val(&tv),
                0,
            );
        }
        true
    }

    unsafe fn send(&mut self, i: usize, data: &[u8]) -> bool {
        let d = &mut self.devs[i];
        if d.sock < 0 {
            return false;
        }
        let mut off = 0;
        while off < data.len() {
            let n = syscall3(
                SYS_WRITE,
                d.sock as usize,
                data[off..].as_ptr() as usize,
                data.len() - off,
            );
            if n <= 0 {
                self.drop_sock(i);
                return false;
            }
            off += n as usize;
        }
        true
    }

    unsafe fn recv_byte(&mut self, i: usize, out: &mut u8) -> bool {
        let d = &mut self.devs[i];
        if d.sock < 0 {
            return false;
        }
        let n = syscall3(SYS_READ, d.sock as usize, out as *mut u8 as usize, 1);
        if n == 1 {
            true
        } else {
            self.drop_sock(i);
            false
        }
    }

    unsafe fn recv_line(&mut self, i: usize, out: &mut [u8]) -> Option<usize> {
        let mut n = 0;
        loop {
            let mut b = 0u8;
            if !self.recv_byte(i, &mut b) {
                return None;
            }
            if b == b'\n' {
                return Some(n);
            }
            if b == b'\r' {
                continue;
            }
            if n >= out.len() {
                return None;
            }
            out[n] = b;
            n += 1;
        }
    }

    unsafe fn recv_exact(&mut self, i: usize, out: *mut u8, len: usize) -> bool {
        let mut off = 0;
        while off < len {
            let n = syscall3(
                SYS_READ,
                self.devs[i].sock as usize,
                out.add(off) as usize,
                len - off,
            );
            if n <= 0 {
                self.drop_sock(i);
                return false;
            }
            off += n as usize;
        }
        true
    }

    unsafe fn drop_sock(&mut self, i: usize) {
        let d = &mut self.devs[i];
        if d.sock >= 0 {
            syscall3(SYS_CLOSE, d.sock as usize, 0, 0);
            d.sock = -1;
        }
        d.streaming = false;
        d.nbuffers = 0;
        for b in d.bufs.iter_mut() {
            *b = Buf::new();
        }
    }

    // ---- camera discovery (INFO) ----

    unsafe fn query_info(&mut self, i: usize) -> bool {
        if !self.connect(i) {
            note(b"[husk v4l2] camera.sock connect failed\n");
            return false;
        }
        self.trace(b"[husk v4l2] camera.sock connected; sending INFO\n");
        if !self.send(i, b"INFO\n") {
            note(b"[husk v4l2] camera.sock write failed\n");
            return false;
        }
        self.trace(b"[husk v4l2] INFO sent; waiting for reply\n");
        let mut line = [0u8; 512];
        loop {
            let n = match self.recv_line(i, &mut line) {
                Some(n) => n,
                None => return false,
            };
            let l = &line[..n];
            self.trace(b"[husk v4l2] INFO reply: ");
            self.trace(l);
            self.trace(b"\n");
            if starts(l, b"OK") {
                return true;
            }
            if starts(l, b"ERR ") {
                // The app refused to serve: permission denied has to reach the
                // guest as EACCES, not as a generic "no such device".
                self.last_err = if contains(l, b"no_permission") { 1 } else { 2 };
                return false;
            }
            if !starts(l, b"CAM ") {
                continue;
            }
            // CAM k id=0 facing=back name=BACK sizes=1280x720,1920x1080
            let k = field(l, b"k=").and_then(parse_u32).unwrap_or(u32::MAX);
            if k as usize != i {
                continue;
            }
            let d = &mut self.devs[i];
            if let Some(id) = field(l, b"id=") {
                let l2 = id.len().min(d.cam_id.len());
                d.cam_id[..l2].copy_from_slice(&id[..l2]);
                d.cam_id_len = l2;
            }
            if let Some(name) = field(l, b"name=") {
                let l2 = name.len().min(d.name.len() - 1);
                d.name[..l2].copy_from_slice(&name[..l2]);
                d.name_len = l2;
            }
            if let Some(sz) = field(l, b"sizes=") {
                d.nsizes = 0;
                let mut it = sz.split(|c| *c == b',');
                while let Some(s) = it.next() {
                    if d.nsizes as usize >= MAX_SIZES {
                        break;
                    }
                    if let Some((w, h)) = parse_wxh(s) {
                        d.sizes[d.nsizes as usize] = (w, h);
                        d.nsizes += 1;
                    }
                }
            }
        }
    }

    // ---- open ----

    /// errno of the last failed `open` (0 if none).
    pub fn last_errno(&self) -> i32 {
        self.last_errno
    }

    pub unsafe fn open(&mut self, n: u32) -> i32 {
        self.last_err = 0;
        self.last_errno = ENODEV as i32;
        if !self.ready {
            return err(ENODEV);
        }
        let i = match Self::dev_of_path(n) {
            Some(i) => i,
            None => return err(ENODEV),
        };
        if !self.query_info(i) {
            if self.last_err == 1 {
                note(b"[husk v4l2] open: camera permission denied (app side)\n");
                self.last_errno = EACCES as i32;
                return err(EACCES);
            }
            return err(ENODEV);
        }
        if self.devs[i].nsizes == 0 {
            self.drop_sock(i);
            return err(ENODEV);
        }
        // DroidNative default: 640x480 when offered, else the first size.
        let d = &mut self.devs[i];
        let mut w = d.sizes[0].0;
        let mut h = d.sizes[0].1;
        let mut k = 0;
        while k < d.nsizes as usize {
            if d.sizes[k] == (640, 480) {
                w = 640;
                h = 480;
                break;
            }
            k += 1;
        }
        d.w = w;
        d.h = h;
        d.fourcc = FMT_YUYV;
        d.memory = 0;
        d.nbuffers = 0;
        d.streaming = false;
        d.bufs = [Buf::new(); MAX_BUFFERS];
        // Name carries the index: `husk-v4l2-<n>`.
        let mut name = [0u8; 24];
        const NAME_PREFIX: &[u8] = b"husk-v4l2-";
        name[..NAME_PREFIX.len()].copy_from_slice(NAME_PREFIX);
        let mut w = NAME_PREFIX.len();
        if n >= 100 {
            name[w] = b'0' + (n / 100) as u8;
            w += 1;
        }
        if n >= 10 {
            name[w] = b'0' + ((n / 10) % 10) as u8;
            w += 1;
        }
        name[w] = b'0' + (n % 10) as u8;
        w += 1;
        let fd = syscall3(SYS_MEMFD_CREATE, name.as_ptr() as usize, 0, 0);
        if fd < 0 {
            self.drop_sock(i);
            return err(ENODEV);
        }
        let event_fd = syscall6(SYS_EVENTFD2, 0, O_CLOEXEC | O_NONBLOCK, 0, 0, 0, 0);
        if event_fd < 0 {
            syscall3(SYS_CLOSE, fd as usize, 0, 0);
            self.drop_sock(i);
            return err(ENODEV);
        }
        d.fd = fd as i32;
        d.event_fd = event_fd as i32;
        d.opened = true;
        fd as i32
    }

    /// Fds in this process that still refer to device `i`. A dup shares the
    /// file description, so the device lives until the last one is closed.
    ///
    /// Enumerate `/proc/self/fd`; never assume the descriptor fits below 1024.
    /// A process can legitimately `dup()` a camera fd above that bound, and the
    /// old fixed scan then destroyed the shared device state on the first close.
    /// `None` means the kernel directory could not be enumerated; callers must
    /// fail closed instead of guessing that the fd was the last reference.
    unsafe fn refs(&self, i: usize) -> Option<u32> {
        let dfd = syscall6(
            SYS_OPENAT,
            AT_FDCWD as usize,
            b"/proc/self/fd\0".as_ptr() as usize,
            O_DIRECTORY | O_CLOEXEC,
            0,
            0,
            0,
        );
        if dfd < 0 {
            return None;
        }

        let mut count = 0u32;
        let mut buf = [0u8; 1024];
        loop {
            let n = syscall6(
                SYS_GETDENTS64,
                dfd as usize,
                buf.as_mut_ptr() as usize,
                buf.len(),
                0,
                0,
                0,
            );
            if n < 0 {
                syscall3(SYS_CLOSE, dfd as usize, 0, 0);
                return None;
            }
            if n == 0 {
                break;
            }
            let n = n as usize;
            let mut off = 0usize;
            while off < n {
                if off + 20 > n {
                    syscall3(SYS_CLOSE, dfd as usize, 0, 0);
                    return None;
                }
                // Linux `struct linux_dirent64`: d_reclen at +16, d_name at +19.
                let reclen =
                    core::ptr::read_unaligned(buf.as_ptr().add(off + 16) as *const u16) as usize;
                if reclen < 20 || off + reclen > n {
                    syscall3(SYS_CLOSE, dfd as usize, 0, 0);
                    return None;
                }

                let mut fd: u32 = 0;
                let mut any = false;
                let mut valid = true;
                let mut k = off + 19;
                while k < off + reclen {
                    let c = *buf.as_ptr().add(k);
                    if c == 0 {
                        break;
                    }
                    if !c.is_ascii_digit() {
                        valid = false;
                        break;
                    }
                    let digit = (c - b'0') as u32;
                    match fd.checked_mul(10).and_then(|v| v.checked_add(digit)) {
                        Some(v) => fd = v,
                        None => {
                            valid = false;
                            break;
                        }
                    }
                    any = true;
                    k += 1;
                }
                if valid && any && fd_dev_of_kernel(fd as i32) == Some(i) {
                    count += 1;
                }
                off += reclen;
            }
        }
        syscall3(SYS_CLOSE, dfd as usize, 0, 0);
        Some(count)
    }

    unsafe fn destroy(&mut self, i: usize) {
        self.trace(b"[v4l2] destroy\n");
        if self.devs[i].streaming {
            self.send(i, b"STOP\n");
        }
        if self.devs[i].event_fd >= 0 {
            syscall3(SYS_CLOSE, self.devs[i].event_fd as usize, 0, 0);
            self.devs[i].event_fd = -1;
        }
        self.drop_sock(i);
        self.devs[i].opened = false;
        self.devs[i].fd = -1;
    }

    pub unsafe fn close(&mut self, fd: i32) {
        let i = match self.dev_of_fd(fd) {
            Some(i) => i,
            None => return,
        };
        // The fd being closed is still open here, so 1 means "last one".
        // Zero is possible when dup2/dup3 already replaced the original fake
        // descriptor: the device has no live memfd reference left and its
        // private camera socket must be closed, not leaked.
        // If `/proc/self/fd` cannot be read, keep the device state alive: a
        // leak is safer than tearing down a socket still used by a dup.
        match self.refs(i) {
            Some(0 | 1) => self.destroy(i),
            Some(_) => {
                if self.devs[i].fd == fd {
                    self.devs[i].fd = -1;
                }
            }
            None => {
                if self.devs[i].fd == fd {
                    self.devs[i].fd = -1;
                }
                note(b"[husk v4l2] close: cannot enumerate refs; preserving state\n");
            }
        }
    }

    /// Clean the device whose original returned descriptor number was replaced
    /// by dup2/dup3. This runs after the kernel installed the new source, so a
    /// zero reference count is the only proof that the old camera session has
    /// no live duplicate left.
    pub unsafe fn forget_target(&mut self, fd: i32) {
        if fd < 0 {
            return;
        }
        let mut hit = None;
        let mut i = 0;
        while i < MAX_DEVS {
            if self.devs[i].opened && self.devs[i].fd == fd {
                hit = Some(i);
                break;
            }
            i += 1;
        }
        let Some(i) = hit else {
            return;
        };
        match self.refs(i) {
            Some(0) => self.destroy(i),
            Some(_) | None => {
                // Another dup of the old device is still live, or the kernel
                // directory could not be read. Keep the session, but stop
                // treating the reused number as its original fd.
                self.devs[i].fd = -1;
            }
        }
    }

    // ---- ioctl ----

    /// V4L2 request numbers are 32-bit `_IOC` values. Some userspace variants
    /// pass the high bit set value through a signed variadic argument, so the
    /// AArch64 register arrives sign-extended as `0xffffffff80685600`. The
    /// kernel compares only the low 32 bits; doing the same here prevents a
    /// valid QUERYCAP from being misclassified as ENOTTY.
    fn normalize_ioctl(req: u64) -> u64 {
        req & u32::MAX as u64
    }

    pub unsafe fn ioctl(&mut self, fd: i32, req: u64, arg: *mut u8) -> i64 {
        let req = Self::normalize_ioctl(req);
        let i = match self.dev_of_fd(fd) {
            Some(i) => i,
            None => return -ENOTTY,
        };
        if arg.is_null() {
            return -EINVAL;
        }
        match req {
            QUERYCAP => self.querycap(i, arg),
            ENUM_FMT => self.enum_fmt(i, arg),
            ENUM_FRAMESIZES => self.enum_framesizes(i, arg),
            ENUM_FRAMEINTERVALS => self.enum_frameintervals(i, arg),
            G_FMT | S_FMT | TRY_FMT => self.fmt(i, req, arg),
            G_PARM | S_PARM => self.parm(i, arg),
            CROPCAP => self.cropcap(i, arg),
            G_CROP | S_CROP => self.crop(i, req, arg),
            G_SELECTION | S_SELECTION => self.selection(i, req, arg),
            ENUMINPUT => self.enum_input(i, arg),
            G_INPUT => {
                st32(arg, 0, 0);
                0
            }
            S_INPUT => {
                if ld32(arg, 0) == 0 {
                    0
                } else {
                    -EINVAL
                }
            }
            G_PRIORITY => {
                st32(arg, 0, PRIORITY_DEFAULT);
                0
            }
            S_PRIORITY => 0,
            REQBUFS => self.reqbufs(i, arg),
            QUERYBUF => self.querybuf(i, arg),
            QBUF => self.qbuf(i, arg),
            DQBUF => self.dqbuf(i, arg),
            STREAMON => self.streamon(i, arg),
            STREAMOFF => self.streamoff(i, arg),
            _ => -ENOTTY,
        }
    }

    unsafe fn querycap(&mut self, i: usize, arg: *mut u8) -> i64 {
        zero(arg, 104);
        st_bytes(arg, 0, b"husk\0");
        let d = &self.devs[i];
        st_bytes(arg, 16, &d.name[..d.name_len]);
        let mut bus = [0u8; 32];
        let b = b"platform:husk-";
        bus[..b.len()].copy_from_slice(b);
        let mut n = b.len();
        let mut x = i as u32;
        let mut digits = [0u8; 4];
        let mut k = 0;
        loop {
            digits[k] = b'0' + (x % 10) as u8;
            x /= 10;
            k += 1;
            if x == 0 {
                break;
            }
        }
        while k > 0 {
            k -= 1;
            bus[n] = digits[k];
            n += 1;
        }
        st_bytes(arg, 48, &bus[..n]);
        st32(arg, 80, (6 << 16) | (1 << 8));
        let caps = CAP_VIDEO_CAPTURE | CAP_READWRITE | CAP_STREAMING;
        st32(arg, 84, caps | CAP_DEVICE_CAPS);
        st32(arg, 88, caps);
        0
    }

    unsafe fn enum_fmt(&mut self, _i: usize, arg: *mut u8) -> i64 {
        let index = ld32(arg, 0);
        let type_ = ld32(arg, 4);
        if type_ != BUF_TYPE_VIDEO_CAPTURE || index >= 2 {
            return -EINVAL;
        }
        zero(arg, 64);
        st32(arg, 0, index);
        st32(arg, 4, BUF_TYPE_VIDEO_CAPTURE);
        if index == 0 {
            st_bytes(arg, 12, b"YUYV 4:2:2\0");
            st32(arg, 44, FMT_YUYV);
        } else {
            st_bytes(arg, 12, b"Y/UV 4:2:0\0");
            st32(arg, 44, FMT_NV12);
        }
        0
    }

    unsafe fn enum_framesizes(&mut self, i: usize, arg: *mut u8) -> i64 {
        let index = ld32(arg, 0);
        let pf = ld32(arg, 4);
        let d = &self.devs[i];
        if (pf != FMT_YUYV && pf != FMT_NV12) || index >= d.nsizes {
            return -EINVAL;
        }
        let (w, h) = d.sizes[index as usize];
        // keep pixel_format, rewrite the rest (layout verified: type@8, discrete@12)
        st32(arg, 8, FRMSIZE_DISCRETE);
        st32(arg, 12, w);
        st32(arg, 16, h);
        st32(arg, 36, 0);
        st32(arg, 40, 0);
        0
    }

    unsafe fn enum_frameintervals(&mut self, _i: usize, arg: *mut u8) -> i64 {
        let index = ld32(arg, 0);
        let pf = ld32(arg, 4);
        if (pf != FMT_YUYV && pf != FMT_NV12) || index != 0 {
            return -EINVAL;
        }
        st32(arg, 16, FRMIVAL_DISCRETE);
        st32(arg, 20, 1);
        st32(arg, 24, 30);
        st32(arg, 44, 0);
        st32(arg, 48, 0);
        0
    }

    /// Nearest offered size, like dn_v4l2's fit_format().
    unsafe fn fit(&self, i: usize, w_req: u32, h_req: u32) -> (u32, u32) {
        let d = &self.devs[i];
        if d.nsizes == 0 {
            return (w_req, h_req);
        }
        let mut best = u64::MAX;
        let (mut bw, mut bh) = d.sizes[0];
        let mut k = 0;
        while k < d.nsizes as usize {
            let (w, h) = d.sizes[k];
            let dw = w as i64 - w_req as i64;
            let dh = h as i64 - h_req as i64;
            let dist = (dw * dw + dh * dh) as u64;
            if dist < best {
                best = dist;
                bw = w;
                bh = h;
            }
            k += 1;
        }
        (bw, bh)
    }

    unsafe fn write_pix(&self, i: usize, base: *mut u8, off: usize, w: u32, h: u32, fourcc: u32) {
        let d = &self.devs[i];
        let bpl = if fourcc == FMT_YUYV { w * 2 } else { w };
        let size = if fourcc == FMT_YUYV {
            w * h * 2
        } else {
            w * h * 3 / 2
        };
        st32(base, off, w);
        st32(base, off + 4, h);
        st32(base, off + 8, fourcc);
        st32(base, off + 12, FIELD_NONE);
        st32(base, off + 16, bpl);
        st32(base, off + 20, size);
        st32(base, off + 24, COLORSPACE_JPEG);
        st32(base, off + 28, 0);
        st32(base, off + 32, 0);
        st32(base, off + 36, 0);
        st32(base, off + 40, 0);
        st32(base, off + 44, 0);
        let _ = d;
    }

    unsafe fn fmt(&mut self, i: usize, req: u64, arg: *mut u8) -> i64 {
        if ld32(arg, 0) != BUF_TYPE_VIDEO_CAPTURE {
            return -EINVAL;
        }
        // pix begins at offset 8 (verified)
        let mut w = ld32(arg, 8);
        let mut h = ld32(arg, 12);
        let mut pf = ld32(arg, 16);
        if req == G_FMT {
            let d = &self.devs[i];
            w = d.w;
            h = d.h;
            pf = d.fourcc;
        } else if pf != FMT_YUYV && pf != FMT_NV12 {
            pf = FMT_YUYV;
        }
        let (w, h) = self.fit(i, w, h);
        self.write_pix(i, arg, 8, w, h, pf);
        if req == S_FMT {
            // newest request wins (dn_v4l2 does the same: a dead client cannot be told
            // apart from a live one without ptrace).
            if self.devs[i].streaming {
                self.send(i, b"STOP\n");
                self.devs[i].streaming = false;
            }
            let d = &mut self.devs[i];
            d.w = w;
            d.h = h;
            d.fourcc = pf;
            for b in d.bufs.iter_mut() {
                b.queued = false;
            }
        }
        0
    }

    unsafe fn parm(&mut self, _i: usize, arg: *mut u8) -> i64 {
        if ld32(arg, 0) != BUF_TYPE_VIDEO_CAPTURE {
            return -EINVAL;
        }
        // capture: capability@4, capturemode@8, timeperframe{num,den}@12/16, readbuffers@24
        st32(arg, 4, CAP_TIMEPERFRAME);
        st32(arg, 8, 0);
        st32(arg, 12, 1);
        st32(arg, 16, 30);
        st32(arg, 24, 2);
        0
    }

    unsafe fn write_rect(&self, i: usize, base: *mut u8, off: usize) {
        let d = &self.devs[i];
        st32(base, off, 0);
        st32(base, off + 4, 0);
        st32(base, off + 8, d.w);
        st32(base, off + 12, d.h);
    }

    unsafe fn rect_is_full_frame(&self, i: usize, base: *const u8, off: usize) -> bool {
        let d = &self.devs[i];
        ld32(base, off) == 0
            && ld32(base, off + 4) == 0
            && ld32(base, off + 8) == d.w
            && ld32(base, off + 12) == d.h
    }

    /// The host bridge always delivers the complete negotiated frame. Expose that
    /// geometry through the legacy crop API and the modern selection API instead of
    /// returning ENOTTY: GStreamer probes both before streaming and warns twice when
    /// neither is present. A non-full crop would require the bridge to crop pixels,
    /// so it is rejected rather than silently ignored.
    unsafe fn cropcap(&self, i: usize, arg: *mut u8) -> i64 {
        if ld32(arg, 0) != BUF_TYPE_VIDEO_CAPTURE {
            return -EINVAL;
        }
        zero(arg, 44);
        st32(arg, 0, BUF_TYPE_VIDEO_CAPTURE);
        self.write_rect(i, arg, 4);
        self.write_rect(i, arg, 20);
        st32(arg, 36, 1);
        st32(arg, 40, 1);
        0
    }

    unsafe fn crop(&mut self, i: usize, req: u64, arg: *mut u8) -> i64 {
        if ld32(arg, 0) != BUF_TYPE_VIDEO_CAPTURE {
            return -EINVAL;
        }
        if req == S_CROP && !self.rect_is_full_frame(i, arg, 4) {
            return -EINVAL;
        }
        self.write_rect(i, arg, 4);
        0
    }

    unsafe fn selection(&mut self, i: usize, req: u64, arg: *mut u8) -> i64 {
        if ld32(arg, 0) != BUF_TYPE_VIDEO_CAPTURE {
            return -EINVAL;
        }
        let target = ld32(arg, 4);
        if target != SEL_TGT_CROP && target != SEL_TGT_CROP_DEFAULT && target != SEL_TGT_CROP_BOUNDS
        {
            // Compose targets are output-only. ENOTTY tells GStreamer this capture
            // facade does not implement them, which is the documented no-op path.
            return -ENOTTY;
        }
        if req == S_SELECTION
            && (target != SEL_TGT_CROP || ld32(arg, 8) != 0 || !self.rect_is_full_frame(i, arg, 12))
        {
            return -EINVAL;
        }
        zero(arg, 64);
        st32(arg, 0, BUF_TYPE_VIDEO_CAPTURE);
        st32(arg, 4, target);
        self.write_rect(i, arg, 12);
        0
    }

    unsafe fn enum_input(&mut self, _i: usize, arg: *mut u8) -> i64 {
        if ld32(arg, 0) != 0 {
            return -EINVAL;
        }
        zero(arg, 80);
        st_bytes(arg, 4, b"Camera\0");
        st32(arg, 36, INPUT_TYPE_CAMERA);
        0
    }

    unsafe fn reqbufs(&mut self, i: usize, arg: *mut u8) -> i64 {
        self.trace(b"[v4l2] REQBUFS\n");
        let mut count = ld32(arg, 0);
        let type_ = ld32(arg, 4);
        let memory = ld32(arg, 8);
        if type_ != BUF_TYPE_VIDEO_CAPTURE || (memory != MEMORY_MMAP && memory != MEMORY_USERPTR) {
            return -EINVAL;
        }
        if self.devs[i].streaming {
            self.send(i, b"STOP\n");
            self.devs[i].streaming = false;
        }
        if count > MAX_BUFFERS as u32 {
            count = MAX_BUFFERS as u32;
        }
        let d = &mut self.devs[i];
        d.bufs = [Buf::new(); MAX_BUFFERS];
        d.nbuffers = count;
        d.memory = memory;
        st32(arg, 0, count);
        st32(arg, 12, 0);
        0
    }

    unsafe fn fill_buf(&self, i: usize, arg: *mut u8, index: u32) -> bool {
        let d = &self.devs[i];
        if index >= d.nbuffers {
            return false;
        }
        let b = d.bufs[index as usize];
        zero(arg, 88);
        st32(arg, 0, index);
        st32(arg, 4, BUF_TYPE_VIDEO_CAPTURE);
        st32(arg, 16, FIELD_NONE);
        st32(arg, 60, d.memory);
        let mut flags = BUF_FLAG_TIMESTAMP_MONOTONIC;
        if b.queued {
            flags |= BUF_FLAG_QUEUED;
        }
        if d.memory == MEMORY_MMAP {
            if b.addr != 0 {
                flags |= BUF_FLAG_MAPPED;
            }
            st32(arg, 12, flags);
            st64(arg, 64, (index << BUFFER_OFFSET_SHIFT) as u64);
            st32(arg, 72, d.size_image());
        } else {
            st32(arg, 12, flags);
            st64(arg, 64, b.addr as u64);
            st32(
                arg,
                72,
                if b.addr != 0 {
                    b.len as u32
                } else {
                    d.size_image()
                },
            );
        }
        true
    }

    unsafe fn querybuf(&mut self, i: usize, arg: *mut u8) -> i64 {
        if ld32(arg, 4) != BUF_TYPE_VIDEO_CAPTURE {
            return -EINVAL;
        }
        let index = ld32(arg, 0);
        if index >= self.devs[i].nbuffers {
            return -EINVAL;
        }
        self.fill_buf(i, arg, index);
        0
    }

    unsafe fn qbuf(&mut self, i: usize, arg: *mut u8) -> i64 {
        let audit = self.audit_qbuf;
        self.audit_qbuf = self.audit_qbuf.wrapping_add(1);
        if audit < 8 || audit % 30 == 0 {
            self.trace(b"[v4l2] QBUF\n");
        }
        if ld32(arg, 4) != BUF_TYPE_VIDEO_CAPTURE {
            return -EINVAL;
        }
        let index = ld32(arg, 0);
        let memory = ld32(arg, 60);
        let d = &mut self.devs[i];
        if index >= d.nbuffers || memory != d.memory {
            return -EINVAL;
        }
        if d.memory == MEMORY_USERPTR {
            let ptr = ld64(arg, 64) as usize;
            let len = ld32(arg, 72) as usize;
            if ptr == 0 || len < d.size_image() as usize {
                return -EINVAL;
            }
            d.bufs[index as usize].addr = ptr;
            d.bufs[index as usize].len = len;
        }
        d.bufs[index as usize].queued = true;
        self.fill_buf(i, arg, index);
        0
    }

    unsafe fn streamon(&mut self, i: usize, arg: *mut u8) -> i64 {
        self.trace(b"[v4l2] STREAMON\n");
        if ld32(arg, 0) != BUF_TYPE_VIDEO_CAPTURE || self.devs[i].nbuffers == 0 {
            return -EINVAL;
        }
        let rc = self.open_session(i);
        if rc == 0 {
            self.arm_event(i);
            self.trace_dev(b"[v4l2] STREAMON ok dev=", i);
        } else {
            self.trace_dev(b"[v4l2] STREAMON fail dev=", i);
        }
        rc
    }

    /// Start the app-side capture session shared by STREAMON and read().
    unsafe fn open_session(&mut self, i: usize) -> i64 {
        if self.devs[i].streaming {
            return 0;
        }
        if !self.connect(i) {
            note(b"[v4l2] STREAMON: connect failed\n");
            return -ENODEV;
        }
        let d = &self.devs[i];
        let mut cmd = [0u8; 96];
        let mut n = 0;
        for b in b"OPEN w=" {
            cmd[n] = *b;
            n += 1;
        }
        n += write_u32(&mut cmd[n..], d.w);
        for b in b" h=" {
            cmd[n] = *b;
            n += 1;
        }
        n += write_u32(&mut cmd[n..], d.h);
        if d.cam_id_len > 0 {
            for b in b" id=" {
                cmd[n] = *b;
                n += 1;
            }
            cmd[n..n + d.cam_id_len].copy_from_slice(&d.cam_id[..d.cam_id_len]);
            n += d.cam_id_len;
        }
        cmd[n] = b'\n';
        n += 1;
        self.trace(b"[v4l2] OPEN ");
        self.trace(&cmd[..n]);
        if !self.send(i, &cmd[..n]) {
            note(b"[v4l2] STREAMON: send OPEN failed\n");
            return -ENODEV;
        }
        let mut line = [0u8; 256];
        let ln = match self.recv_line(i, &mut line) {
            Some(n) => n,
            None => {
                note(b"[v4l2] STREAMON: no reply to OPEN\n");
                return -ENODEV;
            }
        };
        let l = &line[..ln];
        if !starts(l, b"OK") {
            note(b"[v4l2] STREAMON: app said: ");
            note(l);
            note(b"\n");
            return -EBUSY;
        }
        if let (Some(w), Some(h)) = (
            field(l, b"w=").and_then(parse_u32),
            field(l, b"h=").and_then(parse_u32),
        ) {
            let d = &mut self.devs[i];
            d.w = w;
            d.h = h;
        }
        self.devs[i].streaming = true;
        0
    }

    /// V4L2 read() streaming: no REQBUFS/mmap, one frame per call. Mixing read
    /// with the streaming I/O methods is rejected, matching videodev2's rule.
    pub unsafe fn read_frame(&mut self, fd: i32, out: *mut u8, count: usize) -> isize {
        let i = match self.dev_of_fd(fd) {
            Some(i) => i,
            None => return -(EBADF as isize),
        };
        if out.is_null() {
            return -(EFAULT as isize);
        }
        if self.devs[i].nbuffers != 0 || self.devs[i].memory != 0 {
            return -(EINVAL as isize);
        }
        let need = self.devs[i].size_image() as usize;
        if count < need {
            return -(EINVAL as isize);
        }
        if !self.devs[i].streaming {
            let rc = self.open_session(i);
            if rc < 0 {
                return rc as isize;
            }
        }
        let nonblock = self.devs[i].nonblock;
        loop {
            if !self.send(i, b"FRAME\n") {
                return -(ENODEV as isize);
            }
            let mut line = [0u8; 256];
            let n = match self.recv_line(i, &mut line) {
                Some(n) => n,
                None => return -(EIO as isize),
            };
            let l = &line[..n];
            if starts(l, b"NOFRAME") {
                if nonblock {
                    return -(EAGAIN as isize);
                }
                continue;
            }
            if !starts(l, b"FRAME ") {
                return -(EIO as isize);
            }
            let bytes = field(l, b"bytes=").and_then(parse_u32).unwrap_or(0) as usize;
            let fw = field(l, b"w=")
                .and_then(parse_u32)
                .unwrap_or(self.devs[i].w);
            let fh = field(l, b"h=")
                .and_then(parse_u32)
                .unwrap_or(self.devs[i].h);
            let (fourcc, w, h) = {
                let d = &self.devs[i];
                (d.fourcc, fw.min(d.w), fh.min(d.h))
            };
            if bytes == 0 || w == 0 || h == 0 || bytes > count {
                return -(EIO as isize);
            }
            if fourcc == FMT_YUYV {
                let dst_len = w as usize * h as usize * 2;
                if dst_len > count {
                    return -(EINVAL as isize);
                }
                let src = match self.scratch() {
                    Some(p) if bytes <= self.scratch_len => p,
                    _ => return -(EIO as isize),
                };
                if !self.recv_exact(i, src, bytes) {
                    return -(EIO as isize);
                }
                if !nv12_to_yuyv(src, out, w as usize, h as usize, bytes, dst_len) {
                    return -(EIO as isize);
                }
                return dst_len as isize;
            }
            if !self.recv_exact(i, out, bytes) {
                return -(EIO as isize);
            }
            return bytes as isize;
        }
    }

    /// Scatter one read()-streaming frame across the caller's iovec. The fake
    /// camera fd is a memfd, so allowing glibc's readv() to reach it would
    /// silently return EOF instead of a frame.
    pub unsafe fn readv_frame(&mut self, fd: i32, iov: *mut u8, iovcnt: i32) -> isize {
        let i = match self.dev_of_fd(fd) {
            Some(i) => i,
            None => return -(EBADF as isize),
        };
        if iov.is_null() {
            return -(EFAULT as isize);
        }
        if iovcnt <= 0 || iovcnt > 1024 {
            return -(EINVAL as isize);
        }

        let mut total = 0usize;
        let mut k = 0usize;
        while k < iovcnt as usize {
            let entry = iov.add(k * 16);
            let base = ld64(entry, 0);
            let len = ld64(entry, 8);
            if len != 0 && base == 0 {
                return -(EFAULT as isize);
            }
            total = match total.checked_add(len as usize) {
                Some(v) => v,
                None => return -(EINVAL as isize),
            };
            k += 1;
        }

        let d = &self.devs[i];
        let need = if d.fourcc == FMT_YUYV {
            d.w as usize * d.h as usize * 2
        } else {
            d.size_image() as usize
        };
        if total < need {
            return -(EINVAL as isize);
        }
        let scratch = match self.readv_scratch() {
            Some(p) => p,
            None => return -(EIO as isize),
        };
        let n = self.read_frame(fd, scratch, READV_BYTES);
        if n < 0 {
            return n;
        }
        let n = n as usize;
        let mut copied = 0usize;
        k = 0;
        while k < iovcnt as usize && copied < n {
            let entry = iov.add(k * 16);
            let base = ld64(entry, 0) as *mut u8;
            let len = ld64(entry, 8) as usize;
            let take = len.min(n - copied);
            if take != 0 {
                core::ptr::copy_nonoverlapping(scratch.add(copied), base, take);
                copied += take;
            }
            k += 1;
        }
        n as isize
    }

    unsafe fn streamoff(&mut self, i: usize, arg: *mut u8) -> i64 {
        self.trace(b"[v4l2] STREAMOFF\n");
        if ld32(arg, 0) != BUF_TYPE_VIDEO_CAPTURE {
            return -EINVAL;
        }
        if self.devs[i].streaming {
            self.send(i, b"STOP\n");
            let mut line = [0u8; 64];
            let _ = self.recv_line(i, &mut line);
        }
        self.consume_event(i);
        let d = &mut self.devs[i];
        d.streaming = false;
        for b in d.bufs.iter_mut() {
            b.queued = false;
        }
        0
    }

    unsafe fn dqbuf(&mut self, i: usize, arg: *mut u8) -> i64 {
        let audit = self.audit_dqbuf;
        self.audit_dqbuf = self.audit_dqbuf.wrapping_add(1);
        let sampled = audit < 16 || audit % 30 == 0;
        self.consume_event(i);
        if sampled {
            self.trace(b"[v4l2] DQBUF\n");
            let mut queued = 0;
            let mut n = 0;
            while n < self.devs[i].nbuffers {
                if self.devs[i].bufs[n as usize].queued {
                    queued += 1;
                }
                n += 1;
            }
            self.trace_dqbuf_state(i, self.devs[i].streaming, self.devs[i].nbuffers, queued);
        }
        if ld32(arg, 4) != BUF_TYPE_VIDEO_CAPTURE {
            if sampled {
                self.trace(b"[v4l2] DQBUF EINVAL bad_type\n");
            }
            return -EINVAL;
        }
        if !self.devs[i].streaming {
            if sampled {
                self.trace(b"[v4l2] DQBUF EINVAL not_streaming\n");
            }
            return -EINVAL;
        }
        let nbuffers = self.devs[i].nbuffers;
        let mut idx = 0;
        while idx < nbuffers && !self.devs[i].bufs[idx as usize].queued {
            idx += 1;
        }
        if idx == nbuffers {
            if sampled {
                self.trace(b"[v4l2] DQBUF EINVAL no_queued\n");
            }
            return -EINVAL;
        }
        let nonblock = self.devs[i].nonblock;
        let mut attempts = 0;
        loop {
            if sampled && attempts == 0 {
                self.trace(b"[v4l2] DQBUF send FRAME\n");
            }
            if !self.send(i, b"FRAME\n") {
                if sampled {
                    self.trace(b"[v4l2] DQBUF ENODEV send_frame\n");
                }
                return -ENODEV;
            }
            let mut line = [0u8; 256];
            let n = match self.recv_line(i, &mut line) {
                Some(n) => n,
                None => {
                    if sampled {
                        self.trace(b"[v4l2] DQBUF EIO reply\n");
                    }
                    return -EIO;
                }
            };
            let l = &line[..n];
            if starts(l, b"NOFRAME") {
                if sampled && attempts == 0 {
                    self.trace(b"[v4l2] DQBUF NOFRAME\n");
                }
                attempts += 1;
                if nonblock || attempts >= 5 {
                    self.arm_event(i);
                    return -EAGAIN;
                }
                continue;
            }
            if !starts(l, b"FRAME ") {
                if sampled {
                    self.trace(b"[v4l2] DQBUF EIO unexpected_reply\n");
                }
                return -EIO;
            }
            let bytes = field(l, b"bytes=").and_then(parse_u32).unwrap_or(0) as usize;
            let seq = field(l, b"seq=").and_then(parse_u32).unwrap_or(0);
            let ts = field(l, b"ts=").and_then(parse_u64).unwrap_or(0);
            let fw = field(l, b"w=")
                .and_then(parse_u32)
                .unwrap_or(self.devs[i].w);
            let fh = field(l, b"h=")
                .and_then(parse_u32)
                .unwrap_or(self.devs[i].h);
            if bytes == 0 {
                if sampled {
                    self.trace(b"[v4l2] DQBUF EIO zero_bytes\n");
                }
                return -EIO;
            }
            // Reserve the destination before reading the payload. NV12 needs no
            // conversion, so the socket bytes can go straight into the guest's
            // mmap address. The old path always read into scratch and copied a
            // second time, which is pure overhead at 1080p.
            let (fourcc, w, h, dst, cap) = {
                let d = &self.devs[i];
                let b = d.bufs[idx as usize];
                if b.addr == 0 {
                    return -EFAULT;
                }
                let cap = if d.memory == MEMORY_MMAP {
                    d.size_image() as usize
                } else {
                    b.len
                };
                (d.fourcc, fw.min(d.w), fh.min(d.h), b.addr as *mut u8, cap)
            };
            if sampled {
                self.trace_hex(b"[v4l2] DQBUF dst=", dst as usize);
                self.trace_hex(b"[v4l2] DQBUF cap=", cap);
                self.trace_hex(b"[v4l2] DQBUF bytes=", bytes);
            }
            if cap == 0 {
                return -EFAULT;
            }
            let direct_nv12 = fourcc != FMT_YUYV && bytes <= cap;
            let audit_mmap = direct_nv12 && self.trace;
            let read_dst = if direct_nv12 && !audit_mmap {
                dst
            } else {
                match self.scratch() {
                    Some(p) if bytes <= self.scratch_len => p,
                    _ => return -EIO,
                }
            };
            if !self.recv_exact(i, read_dst, bytes) {
                return -EIO;
            }
            if sampled {
                let mut min = u8::MAX;
                let mut max = 0u8;
                let mut sum = 0u64;
                for k in 0..bytes.min(4096) {
                    let v = core::ptr::read(read_dst.add(k));
                    min = min.min(v);
                    max = max.max(v);
                    sum += v as u64;
                }
                self.trace_hex(b"[v4l2] DQBUF payload min=", min as usize);
                self.trace_hex(b"[v4l2] DQBUF payload max=", max as usize);
                self.trace_hex(b"[v4l2] DQBUF payload sum=", sum as usize);
            }
            if audit_mmap {
                core::ptr::copy_nonoverlapping(read_dst, dst, bytes);
            }
            let used = if fourcc == FMT_YUYV {
                let need = (w * h * 2) as usize;
                let n = need.min(cap);
                if !nv12_to_yuyv(read_dst, dst, w as usize, h as usize, bytes, n) {
                    return -EIO;
                }
                n
            } else if direct_nv12 {
                bytes
            } else {
                let n = bytes.min(cap);
                core::ptr::copy_nonoverlapping(read_dst, dst, n);
                n
            };
            let d = &mut self.devs[i];
            d.bufs[idx as usize].queued = false;
            self.fill_buf(i, arg, idx);
            st32(arg, 8, used as u32);
            st32(arg, 56, seq);
            let ts = normalize_timestamp(ts);
            st64(arg, 24, ts / 1_000_000_000);
            st64(arg, 32, (ts % 1_000_000_000) / 1_000);
            self.arm_event(i);
            if sampled {
                self.trace(b"[v4l2] DQBUF ok\n");
            }
            return 0;
        }
    }

    // ---- mmap ----

    /// `off >> 24` is the buffer index (dn_v4l2's BUFFER_OFFSET_SHIFT).
    pub unsafe fn mmap(&mut self, fd: i32, len: usize, off: i64) -> Option<*mut u8> {
        let i = self.dev_of_fd(fd)?;
        self.trace(b"[v4l2] MMAP\n");
        let index = ((off as u64) >> BUFFER_OFFSET_SHIFT) as u32;
        if self.devs[i].memory != MEMORY_MMAP || index >= self.devs[i].nbuffers {
            return None;
        }
        let p = syscall6(SYS_MMAP, 0, len, 3, 0x22, usize::MAX, 0);
        if p < 0 {
            return None;
        }
        self.trace_hex(b"[v4l2] MMAP ptr=", p as usize);
        self.trace_hex(b"[v4l2] MMAP len=", len);
        let d = &mut self.devs[i];
        d.bufs[index as usize].addr = p as usize;
        d.bufs[index as usize].len = len;
        Some(p as *mut u8)
    }

    /// `fstat`/`statx` of an opened device must look like char 81:N.
    pub fn char_device_rdev(&mut self, fd: i32) -> Option<u32> {
        let i = self.dev_of_fd(fd)?;
        Some((V4L2_MAJOR << 8) | (i as u32))
    }
}

#[cfg(test)]
mod tests {
    use super::{nv12_frame_size, nv12_to_yuyv, Ctx, UNIX_PATH_MAX};

    #[test]
    fn signed_variadic_ioctl_request_keeps_low_32_bits() {
        use super::QUERYCAP;
        assert_eq!(Ctx::normalize_ioctl(0xffff_ffff_8068_5600), QUERYCAP);
        assert_eq!(Ctx::normalize_ioctl(QUERYCAP), QUERYCAP);
    }

    #[test]
    fn nv12_frame_size_accepts_even_camera_dimensions() {
        assert_eq!(nv12_frame_size(2, 2), Some(6));
        assert_eq!(nv12_frame_size(1920, 1080), Some(3_110_400));
    }

    #[test]
    fn nv12_frame_size_rejects_short_or_odd_geometry() {
        assert_eq!(nv12_frame_size(0, 2), None);
        assert_eq!(nv12_frame_size(2, 0), None);
        assert_eq!(nv12_frame_size(3, 2), None);
        assert_eq!(nv12_frame_size(2, 3), None);
    }

    #[test]
    fn unix_socket_path_limit_matches_sockaddr_un() {
        assert_eq!(UNIX_PATH_MAX, 108);
    }

    #[test]
    fn init_rejects_path_that_cannot_fit_sockaddr_un() {
        let mut ctx = Ctx::empty();
        assert!(!unsafe { ctx.init(&[b'x'; UNIX_PATH_MAX + 1]) });
        assert!(unsafe { ctx.init(&[b'x'; UNIX_PATH_MAX]) });
    }

    #[test]
    fn nv12_conversion_rejects_short_source_or_destination() {
        let src = [0u8; 6];
        let mut dst = [0u8; 8];
        assert!(!unsafe { nv12_to_yuyv(src.as_ptr(), dst.as_mut_ptr(), 2, 2, 5, 8) });
        assert!(!unsafe { nv12_to_yuyv(src.as_ptr(), dst.as_mut_ptr(), 2, 2, 6, 7) });
        assert!(unsafe { nv12_to_yuyv(src.as_ptr(), dst.as_mut_ptr(), 2, 2, 6, 8) });
    }
}

// ---- small helpers ----

/// Failure value for `open`: negative, like the ioctl paths return. It used to
/// be positive, so every caller's `fd < 0` check passed and the guest received
/// the errno *number* as a file descriptor (measured: /dev/video0 -> fd 13,
/// /dev/video9 -> fd 19, both then failing every ioctl with EBADF).
fn err(e: i64) -> i32 {
    -(e as i32)
}

fn contains(h: &[u8], p: &[u8]) -> bool {
    if p.is_empty() || h.len() < p.len() {
        return false;
    }
    let mut i = 0;
    while i + p.len() <= h.len() {
        if &h[i..i + p.len()] == p {
            return true;
        }
        i += 1;
    }
    false
}

fn starts(h: &[u8], p: &[u8]) -> bool {
    h.len() >= p.len() && &h[..p.len()] == p
}

fn field<'a>(line: &'a [u8], key: &[u8]) -> Option<&'a [u8]> {
    let mut i = 0;
    while i + key.len() <= line.len() {
        if &line[i..i + key.len()] == key {
            let mut j = i + key.len();
            while j < line.len() && line[j] != b' ' {
                j += 1;
            }
            return Some(&line[i + key.len()..j]);
        }
        i += 1;
    }
    None
}

fn parse_u32(s: &[u8]) -> Option<u32> {
    let mut v: u32 = 0;
    let mut any = false;
    for c in s {
        if *c < b'0' || *c > b'9' {
            break;
        }
        v = v.checked_mul(10)?.checked_add((*c - b'0') as u32)?;
        any = true;
    }
    if any {
        Some(v)
    } else {
        None
    }
}

fn parse_u64(s: &[u8]) -> Option<u64> {
    let mut v: u64 = 0;
    let mut any = false;
    for c in s {
        if *c < b'0' || *c > b'9' {
            break;
        }
        v = v.checked_mul(10)?.checked_add((*c - b'0') as u64)?;
        any = true;
    }
    if any {
        Some(v)
    } else {
        None
    }
}

unsafe fn clock_ns(clock: usize) -> Option<u64> {
    let mut ts = [0u64; 2];
    if syscall3(SYS_CLOCK_GETTIME, clock, ts.as_mut_ptr() as usize, 0) != 0 {
        return None;
    }
    ts[0]
        .checked_mul(1_000_000_000)
        .and_then(|v| v.checked_add(ts[1]))
}

fn timestamp_age(now: u64, stamp: u64) -> Option<u64> {
    if stamp == 0 || stamp > now {
        return None;
    }
    let age = now - stamp;
    (age <= MAX_TIMESTAMP_AGE_NS).then_some(age)
}

/// Camera2 timestamps can use the monotonic, boot-time (Android's
/// `elapsedRealtimeNanos`) or realtime clock. V4L2 only has one monotonic flag, so
/// convert a source timestamp that correlates with one of those clocks into the
/// monotonic base GStreamer checks. If none correlate, use frame-arrival time rather
/// than exposing a bogus timestamp such as zero.
unsafe fn normalize_timestamp(stamp: u64) -> u64 {
    let now = match clock_ns(CLOCK_MONOTONIC) {
        Some(v) => v,
        None => return stamp,
    };
    let sources = [
        (now, 0u8),
        (clock_ns(CLOCK_BOOTTIME).unwrap_or(0), 1u8),
        (clock_ns(CLOCK_REALTIME).unwrap_or(0), 2u8),
    ];
    let mut best_age = u64::MAX;
    let mut best_source = 3u8;
    let mut k = 0;
    while k < sources.len() {
        let (source_now, source) = sources[k];
        if source_now != 0 {
            if let Some(age) = timestamp_age(source_now, stamp) {
                if age < best_age {
                    best_age = age;
                    best_source = source;
                }
            }
        }
        k += 1;
    }
    match best_source {
        0 => stamp,
        1 | 2 => now.saturating_sub(best_age),
        _ => now,
    }
}

fn parse_wxh(s: &[u8]) -> Option<(u32, u32)> {
    let mut i = 0;
    while i < s.len() && s[i] != b'x' {
        i += 1;
    }
    if i == 0 || i >= s.len() {
        return None;
    }
    Some((parse_u32(&s[..i])?, parse_u32(&s[i + 1..])?))
}

fn write_u32(dst: &mut [u8], v: u32) -> usize {
    let mut tmp = [0u8; 10];
    let mut n = 0;
    let mut x = v;
    if x == 0 {
        tmp[0] = b'0';
        n = 1;
    } else {
        while x > 0 {
            tmp[n] = b'0' + (x % 10) as u8;
            x /= 10;
            n += 1;
        }
    }
    let mut k = 0;
    while k < n {
        dst[k] = tmp[n - 1 - k];
        k += 1;
    }
    n
}

/// NV12 (Y plane + interleaved UV) → YUYV (Y0 U Y1 V).
///
/// Returns false when the dimensions are not even, the source payload is
/// shorter than the advertised NV12 frame, or the destination cannot hold the
/// full YUYV result. The camera socket is an external trust boundary; malformed
/// lengths must become EIO, not a Rust bounds panic or an out-of-bounds read.
unsafe fn nv12_to_yuyv(
    src: *const u8,
    dst: *mut u8,
    w: usize,
    h: usize,
    src_len: usize,
    cap: usize,
) -> bool {
    let Some(nv12_len) = nv12_frame_size(w, h) else {
        return false;
    };
    let Some(yuyv_len) = w.checked_mul(h).and_then(|v| v.checked_mul(2)) else {
        return false;
    };
    if src_len < nv12_len || cap < yuyv_len {
        return false;
    }
    let y = src;
    let uv = src.add(w * h);
    let mut o = 0;
    let mut row = 0;
    while row < h {
        let yrow = y.add(row * w);
        let uvrow = uv.add((row / 2) * w);
        let mut x = 0;
        while x + 1 < w {
            // UV is interleaved: U at even byte offsets, V at odd ones.
            core::ptr::write_unaligned(dst.add(o), *yrow.add(x));
            core::ptr::write_unaligned(dst.add(o + 1), *uvrow.add(x));
            core::ptr::write_unaligned(dst.add(o + 2), *yrow.add(x + 1));
            core::ptr::write_unaligned(dst.add(o + 3), *uvrow.add(x + 1));
            o += 4;
            x += 2;
        }
        row += 1;
    }
    true
}

fn nv12_frame_size(w: usize, h: usize) -> Option<usize> {
    if w == 0 || h == 0 || w % 2 != 0 || h % 2 != 0 {
        return None;
    }
    let y = w.checked_mul(h)?;
    y.checked_add(y / 2)
}
