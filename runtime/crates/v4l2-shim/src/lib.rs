//! v4l2-shim — guest `/dev/videoN` over the app's `camera.sock`.
//!
//! **no_std** (it is linked into the loader-embedded path shim). Design and the
//! prior art it follows: `docs/design/v4l2-prior-art.md`.
//!
//! Deltas from DroidNative's `dn_v4l2` (ptrace, FIFO + fd-passed shm):
//! - the guest-facing fd is a memfd, so `poll/select/epoll` report readable and
//!   `close/dup` are real; there is no FIFO and no fd passing;
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
const SYS_MEMFD_CREATE: usize = 279;
const SYS_READLINKAT: usize = 78;
const AT_FDCWD: isize = -100;
const O_DIRECTORY: usize = 0o40000; // AArch64 UAPI, not asm-generic 0o200000
const O_CLOEXEC: usize = 0o2000000;

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

// ---- constants ----
pub const BUF_TYPE_VIDEO_CAPTURE: u32 = 1;
pub const MEMORY_MMAP: u32 = 1;
pub const MEMORY_USERPTR: u32 = 2;
pub const FRMSIZE_DISCRETE: u32 = 1;
pub const FRMIVAL_DISCRETE: u32 = 1;
pub const FIELD_NONE: u32 = 1;
pub const COLORSPACE_JPEG: u32 = 7;
pub const CAP_VIDEO_CAPTURE: u32 = 0x1;
pub const CAP_STREAMING: u32 = 0x0400_0000;
pub const CAP_DEVICE_CAPS: u32 = 0x8000_0000;
pub const CAP_TIMEPERFRAME: u32 = 0x1000;
pub const BUF_FLAG_MAPPED: u32 = 1;
pub const BUF_FLAG_QUEUED: u32 = 2;
pub const BUF_FLAG_TIMESTAMP_MONOTONIC: u32 = 8192;
pub const INPUT_TYPE_CAMERA: u32 = 2;
pub const FMT_NV12: u32 = 0x3231_564e;
pub const FMT_YUYV: u32 = 0x5659_5559;
const PRIORITY_DEFAULT: u32 = 2;
const EINVAL: i64 = 22;
const EACCES: i64 = 13;
const ENODEV: i64 = 19;
const EAGAIN: i64 = 11;
const EBUSY: i64 = 16;
const ENOTTY: i64 = 25;
const EFAULT: i64 = 14;
const EIO: i64 = 5;
const BUFFER_OFFSET_SHIFT: u32 = 24;
const SCRATCH_BYTES: usize = 1920 * 1080 * 3 / 2 + 4096;

/// Every guest-facing fd is a memfd whose name carries the device index, so an
/// fd can be identified from the kernel alone, whoever opened it.
const MEMFD_PREFIX: &[u8] = b"/memfd:husk-v4l2-";

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
    let t = &buf[..r as usize];
    if !t.starts_with(MEMFD_PREFIX) {
        return None;
    }
    let mut k = MEMFD_PREFIX.len();
    let mut idx = 0usize;
    let start = k;
    while k < t.len() && t[k].is_ascii_digit() {
        idx = idx * 10 + (t[k] - b'0') as usize;
        k += 1;
    }
    if k == start || idx >= MAX_DEVS {
        return None;
    }
    Some(idx)
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
        }
    }

    /// `path` is the host path of `run/husk/camera.sock` inside the rootfs.
    pub unsafe fn init(&mut self, path: &[u8]) -> bool {
        if path.len() == 0 || path.len() >= self.path.len() {
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
        if !self.send(i, b"INFO\n") {
            note(b"[husk v4l2] camera.sock write failed\n");
            return false;
        }
        let mut line = [0u8; 512];
        loop {
            let n = match self.recv_line(i, &mut line) {
                Some(n) => n,
                None => return false,
            };
            let l = &line[..n];
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
        d.fd = fd as i32;
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
        if self.devs[i].streaming {
            self.send(i, b"STOP\n");
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

    pub unsafe fn ioctl(&mut self, fd: i32, req: u64, arg: *mut u8) -> i64 {
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
        let caps = CAP_VIDEO_CAPTURE | CAP_STREAMING;
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
        if ld32(arg, 0) != BUF_TYPE_VIDEO_CAPTURE || self.devs[i].nbuffers == 0 {
            return -EINVAL;
        }
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

    unsafe fn streamoff(&mut self, i: usize, arg: *mut u8) -> i64 {
        if ld32(arg, 0) != BUF_TYPE_VIDEO_CAPTURE {
            return -EINVAL;
        }
        if self.devs[i].streaming {
            self.send(i, b"STOP\n");
            let mut line = [0u8; 64];
            let _ = self.recv_line(i, &mut line);
        }
        let d = &mut self.devs[i];
        d.streaming = false;
        for b in d.bufs.iter_mut() {
            b.queued = false;
        }
        0
    }

    unsafe fn dqbuf(&mut self, i: usize, arg: *mut u8) -> i64 {
        if ld32(arg, 4) != BUF_TYPE_VIDEO_CAPTURE || !self.devs[i].streaming {
            return -EINVAL;
        }
        let nbuffers = self.devs[i].nbuffers;
        let mut idx = 0;
        while idx < nbuffers && !self.devs[i].bufs[idx as usize].queued {
            idx += 1;
        }
        if idx == nbuffers {
            return -EINVAL;
        }
        let nonblock = self.devs[i].nonblock;
        let mut attempts = 0;
        loop {
            if !self.send(i, b"FRAME\n") {
                return -ENODEV;
            }
            let mut line = [0u8; 256];
            let n = match self.recv_line(i, &mut line) {
                Some(n) => n,
                None => return -EIO,
            };
            let l = &line[..n];
            if starts(l, b"NOFRAME") {
                attempts += 1;
                if nonblock || attempts >= 5 {
                    return -EAGAIN;
                }
                continue;
            }
            if !starts(l, b"FRAME ") {
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
            let scratch = match self.scratch() {
                Some(p) => p,
                None => return -EIO,
            };
            if bytes == 0 || bytes > self.scratch_len {
                return -EIO;
            }
            if !self.recv_exact(i, scratch, bytes) {
                return -EIO;
            }
            // stash what we need before borrowing mutably again
            let d = &mut self.devs[i];
            let fourcc = d.fourcc;
            let (w, h) = (fw.min(d.w), fh.min(d.h));
            let b = d.bufs[idx as usize];
            if b.addr == 0 {
                return -EFAULT;
            }
            let dst = b.addr as *mut u8;
            let (used, capacity) = if fourcc == FMT_YUYV {
                let need = (w * h * 2) as usize;
                let cap = if d.memory == MEMORY_MMAP {
                    d.size_image() as usize
                } else {
                    b.len
                };
                let n = need.min(cap);
                nv12_to_yuyv(scratch, dst, w as usize, h as usize, n);
                (n, cap)
            } else {
                let cap = if d.memory == MEMORY_MMAP {
                    d.size_image() as usize
                } else {
                    b.len
                };
                let n = bytes.min(cap);
                core::ptr::copy_nonoverlapping(scratch, dst, n);
                (n, cap)
            };
            let _ = capacity;
            d.bufs[idx as usize].queued = false;
            self.fill_buf(i, arg, idx);
            st32(arg, 8, used as u32);
            st32(arg, 56, seq);
            st64(arg, 24, ts / 1_000_000);
            st64(arg, 32, ts % 1_000_000);
            return 0;
        }
    }

    // ---- mmap ----

    /// `off >> 24` is the buffer index (dn_v4l2's BUFFER_OFFSET_SHIFT).
    pub unsafe fn mmap(&mut self, fd: i32, len: usize, off: i64) -> Option<*mut u8> {
        let i = self.dev_of_fd(fd)?;
        let index = ((off as u64) >> BUFFER_OFFSET_SHIFT) as u32;
        let d = &mut self.devs[i];
        if d.memory != MEMORY_MMAP || index >= d.nbuffers {
            return None;
        }
        let p = syscall6(SYS_MMAP, 0, len, 3, 0x22, usize::MAX, 0);
        if p < 0 {
            return None;
        }
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
unsafe fn nv12_to_yuyv(src: *const u8, dst: *mut u8, w: usize, h: usize, cap: usize) {
    let y = src;
    let uv = src.add(w * h);
    let mut o = 0;
    let mut row = 0;
    while row < h {
        let yrow = y.add(row * w);
        let uvrow = uv.add((row / 2) * w);
        let mut x = 0;
        while x + 1 < w {
            if o + 4 > cap {
                return;
            }
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
}
