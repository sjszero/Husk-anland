//! Shared-memory ring and fd-passing protocol for the Husk video bridge.
//!
//! This crate is used by both the glibc guest and the bionic Android app. It has
//! no dependency on either libc personality beyond the Linux `libc` ABI, and it
//! keeps the ring layout in one place so producer and consumer cannot drift.

use std::ffi::CString;
use std::io;
use std::os::fd::{AsRawFd, RawFd};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

pub const RING_MAGIC: u32 = 0x4856_5231; // "HVR1"
pub const RING_VERSION: u32 = 1;
pub const DEFAULT_SLOTS: u32 = 4;
pub const DEFAULT_SLOT_SIZE: u32 = 2 * 1024 * 1024;
pub const RECORD_HEADER_SIZE: usize = 64;
pub const RING_HEADER_SIZE: usize = 4096;

#[repr(C)]
struct RingHeader {
    magic: AtomicU32,
    version: AtomicU32,
    slots: AtomicU32,
    slot_size: AtomicU32,
    stride: AtomicU32,
    reserved: AtomicU32,
    write_seq: AtomicU64,
    read_seq: AtomicU64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct SampleMeta {
    pub seq: u64,
    pub pts_us: i64,
    pub dts_us: i64,
    pub len: u32,
    pub flags: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct RingLayout {
    pub slots: u32,
    pub slot_size: u32,
    pub stride: u32,
    pub bytes: usize,
}

impl RingLayout {
    pub fn new(slots: u32, slot_size: u32) -> io::Result<Self> {
        if slots == 0 || slots > 64 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid slot count",
            ));
        }
        if slot_size < RECORD_HEADER_SIZE as u32 || slot_size > 64 * 1024 * 1024 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid slot size",
            ));
        }
        let stride = align_up(
            RECORD_HEADER_SIZE
                .checked_add(slot_size as usize)
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "ring too large"))?,
            64,
        )?;
        let bytes = RING_HEADER_SIZE
            .checked_add(
                stride
                    .checked_mul(slots as usize)
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "ring too large"))?,
            )
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "ring too large"))?;
        Ok(Self {
            slots,
            slot_size,
            stride: stride as u32,
            bytes,
        })
    }
}

fn align_up(value: usize, align: usize) -> io::Result<usize> {
    value
        .checked_add(align - 1)
        .map(|v| v & !(align - 1))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "alignment overflow"))
}

pub struct MappedRing {
    base: *mut u8,
    layout: RingLayout,
}

unsafe impl Send for MappedRing {}
unsafe impl Sync for MappedRing {}

impl MappedRing {
    /// Create a memfd-backed ring. The returned fd must be passed to the peer
    /// before closing it; the mapping itself remains valid after the fd closes.
    pub fn create(layout: RingLayout) -> io::Result<(Self, RawFd)> {
        let name = CString::new("husk-video-ring").expect("static name");
        let fd = unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        if unsafe { libc::ftruncate(fd, layout.bytes as libc::off_t) } != 0 {
            let e = io::Error::last_os_error();
            unsafe { libc::close(fd) };
            return Err(e);
        }
        let base = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                layout.bytes,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd,
                0,
            )
        };
        if base == libc::MAP_FAILED {
            let e = io::Error::last_os_error();
            unsafe { libc::close(fd) };
            return Err(e);
        }
        let ring = Self {
            base: base as *mut u8,
            layout,
        };
        ring.init_header();
        Ok((ring, fd))
    }

    /// Attach to a memfd received over SCM_RIGHTS. The fd remains owned by the
    /// caller; both this mapping and the caller can close it after validation.
    pub fn from_fd(fd: RawFd, layout: RingLayout) -> io::Result<Self> {
        let base = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                layout.bytes,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd,
                0,
            )
        };
        if base == libc::MAP_FAILED {
            return Err(io::Error::last_os_error());
        }
        let ring = Self {
            base: base as *mut u8,
            layout,
        };
        ring.validate_header()?;
        Ok(ring)
    }

    fn init_header(&self) {
        let h = self.header();
        h.magic.store(RING_MAGIC, Ordering::Relaxed);
        h.version.store(RING_VERSION, Ordering::Relaxed);
        h.slots.store(self.layout.slots, Ordering::Relaxed);
        h.slot_size.store(self.layout.slot_size, Ordering::Relaxed);
        h.stride.store(self.layout.stride, Ordering::Relaxed);
        h.reserved.store(0, Ordering::Relaxed);
        h.write_seq.store(0, Ordering::Release);
        h.read_seq.store(0, Ordering::Release);
    }

    fn validate_header(&self) -> io::Result<()> {
        let h = self.header();
        let magic = h.magic.load(Ordering::Acquire);
        let version = h.version.load(Ordering::Acquire);
        let slots = h.slots.load(Ordering::Acquire);
        let slot_size = h.slot_size.load(Ordering::Acquire);
        let stride = h.stride.load(Ordering::Acquire);
        if magic != RING_MAGIC || version != RING_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "bad ring magic/version",
            ));
        }
        if slots != self.layout.slots
            || slot_size != self.layout.slot_size
            || stride != self.layout.stride
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "ring layout mismatch: peer={slots}/{slot_size}/{stride} \
                     local={}/{}/{}",
                    self.layout.slots, self.layout.slot_size, self.layout.stride
                ),
            ));
        }
        Ok(())
    }

    fn header(&self) -> &RingHeader {
        unsafe { &*(self.base as *const RingHeader) }
    }

    fn slot(&self, index: u64) -> *mut u8 {
        let offset = RING_HEADER_SIZE
            + (index % self.layout.slots as u64) as usize * self.layout.stride as usize;
        unsafe { self.base.add(offset) }
    }

    /// Return the current producer/consumer sequence numbers.
    pub fn sequences(&self) -> (u64, u64) {
        let h = self.header();
        (
            h.read_seq.load(Ordering::Acquire),
            h.write_seq.load(Ordering::Acquire),
        )
    }

    pub fn slots(&self) -> u32 {
        self.layout.slots
    }

    pub fn slot_size(&self) -> u32 {
        self.layout.slot_size
    }

    /// Producer side. Returns `WouldBlock` when all slots are occupied.
    pub fn write_sample(
        &self,
        seq: u64,
        pts_us: i64,
        dts_us: i64,
        flags: u32,
        data: &[u8],
    ) -> io::Result<()> {
        if data.len() > self.layout.slot_size as usize {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "sample exceeds slot",
            ));
        }
        let h = self.header();
        let write = h.write_seq.load(Ordering::Acquire);
        let read = h.read_seq.load(Ordering::Acquire);
        if write.wrapping_sub(read) >= self.layout.slots as u64 {
            return Err(io::Error::new(io::ErrorKind::WouldBlock, "ring full"));
        }
        if seq != write + 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("producer sequence {seq} != expected {}", write + 1),
            ));
        }
        let slot = self.slot(write);
        unsafe {
            (slot as *mut u32).write_unaligned(data.len() as u32);
            (slot.add(4) as *mut u32).write_unaligned(flags);
            (slot.add(8) as *mut i64).write_unaligned(pts_us);
            (slot.add(16) as *mut i64).write_unaligned(dts_us);
            (slot.add(32) as *mut u64).write_unaligned(seq);
            std::ptr::copy_nonoverlapping(data.as_ptr(), slot.add(RECORD_HEADER_SIZE), data.len());
        }
        h.write_seq.store(write + 1, Ordering::Release);
        Ok(())
    }

    /// Consumer side. Copies one record into `out`, but does not release the
    /// slot until `commit(seq)` is called after MediaCodec has accepted it.
    pub fn read_into(&self, seq: u64, out: &mut Vec<u8>) -> io::Result<SampleMeta> {
        let h = self.header();
        let read = h.read_seq.load(Ordering::Acquire);
        let write = h.write_seq.load(Ordering::Acquire);
        if read >= write {
            return Err(io::Error::new(io::ErrorKind::WouldBlock, "ring empty"));
        }
        if seq != read + 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("consumer sequence {seq} != expected {}", read + 1),
            ));
        }
        let slot = self.slot(read);
        let (len, flags, pts_us, dts_us, record_seq) = unsafe {
            (
                (slot as *const u32).read_unaligned(),
                (slot.add(4) as *const u32).read_unaligned(),
                (slot.add(8) as *const i64).read_unaligned(),
                (slot.add(16) as *const i64).read_unaligned(),
                (slot.add(32) as *const u64).read_unaligned(),
            )
        };
        if record_seq != seq || len > self.layout.slot_size {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("bad record seq={record_seq} len={len} expected_seq={seq}"),
            ));
        }
        out.resize(len as usize, 0);
        unsafe {
            std::ptr::copy_nonoverlapping(
                slot.add(RECORD_HEADER_SIZE),
                out.as_mut_ptr(),
                len as usize,
            );
        }
        Ok(SampleMeta {
            seq,
            pts_us,
            dts_us,
            len,
            flags,
        })
    }

    pub fn commit(&self, seq: u64) -> io::Result<()> {
        let h = self.header();
        let read = h.read_seq.load(Ordering::Acquire);
        if seq != read + 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("commit {seq} != expected {}", read + 1),
            ));
        }
        h.read_seq.store(seq, Ordering::Release);
        Ok(())
    }
}

impl Drop for MappedRing {
    fn drop(&mut self) {
        unsafe {
            libc::munmap(self.base as *mut libc::c_void, self.layout.bytes);
        }
    }
}

pub fn send_fd(socket: RawFd, fd: RawFd) -> io::Result<()> {
    let mut byte = [0u8; 1];
    let mut iov = libc::iovec {
        iov_base: byte.as_mut_ptr() as *mut libc::c_void,
        iov_len: byte.len(),
    };
    let space = unsafe { libc::CMSG_SPACE(std::mem::size_of::<RawFd>() as u32) } as usize;
    let mut control = vec![0u8; space];
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = control.as_mut_ptr() as *mut libc::c_void;
    msg.msg_controllen = control.len() as _;
    unsafe {
        let cmsg = libc::CMSG_FIRSTHDR(&msg);
        if cmsg.is_null() {
            return Err(io::Error::other("CMSG_FIRSTHDR failed"));
        }
        (*cmsg).cmsg_level = libc::SOL_SOCKET;
        (*cmsg).cmsg_type = libc::SCM_RIGHTS;
        (*cmsg).cmsg_len = libc::CMSG_LEN(std::mem::size_of::<RawFd>() as u32) as _;
        std::ptr::write_unaligned(libc::CMSG_DATA(cmsg) as *mut RawFd, fd);
        let n = libc::sendmsg(socket, &msg, 0);
        if n < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

pub fn recv_fd(socket: RawFd) -> io::Result<RawFd> {
    let mut byte = [0u8; 1];
    let mut iov = libc::iovec {
        iov_base: byte.as_mut_ptr() as *mut libc::c_void,
        iov_len: byte.len(),
    };
    let space = unsafe { libc::CMSG_SPACE(std::mem::size_of::<RawFd>() as u32) } as usize;
    let mut control = vec![0u8; space];
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = control.as_mut_ptr() as *mut libc::c_void;
    msg.msg_controllen = control.len() as _;
    let n = unsafe { libc::recvmsg(socket, &mut msg, 0) };
    if n < 0 {
        return Err(io::Error::last_os_error());
    }
    if n == 0 {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "peer closed"));
    }
    unsafe {
        let cmsg = libc::CMSG_FIRSTHDR(&msg);
        if cmsg.is_null()
            || (*cmsg).cmsg_level != libc::SOL_SOCKET
            || (*cmsg).cmsg_type != libc::SCM_RIGHTS
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "missing SCM_RIGHTS",
            ));
        }
        Ok(std::ptr::read_unaligned(
            libc::CMSG_DATA(cmsg) as *const RawFd
        ))
    }
}

pub fn close_fd(fd: RawFd) {
    if fd >= 0 {
        unsafe { libc::close(fd) };
    }
}

/// The stream's raw fd is enough for fd passing; this helper keeps call sites
/// explicit about that contract.
pub fn stream_fd(stream: &std::os::unix::net::UnixStream) -> RawFd {
    stream.as_raw_fd()
}

pub fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0xf) as usize] as char);
    }
    out
}

pub fn hex_decode(text: &str) -> io::Result<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "odd hex length"));
    }
    let mut out = Vec::with_capacity(text.len() / 2);
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let hi = hex_nibble(bytes[i])?;
        let lo = hex_nibble(bytes[i + 1])?;
        out.push((hi << 4) | lo);
        i += 2;
    }
    Ok(out)
}

fn hex_nibble(b: u8) -> io::Result<u8> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        _ => Err(io::Error::new(io::ErrorKind::InvalidData, "non-hex digit")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_rejects_out_of_range_and_is_aligned() {
        assert!(RingLayout::new(0, 4096).is_err(), "0 slots must be refused");
        assert!(
            RingLayout::new(65, 4096).is_err(),
            ">64 slots must be refused"
        );
        assert!(
            RingLayout::new(4, RECORD_HEADER_SIZE as u32 - 1).is_err(),
            "slot smaller than the record header must be refused"
        );
        assert!(
            RingLayout::new(4, 64 * 1024 * 1024 + 1).is_err(),
            "slot above the 64 MiB ceiling must be refused"
        );
        let l = RingLayout::new(4, 2 * 1024 * 1024).unwrap();
        assert_eq!(l.slots, 4);
        assert_eq!(l.stride % 64, 0, "stride has to stay 64-byte aligned");
        assert_eq!(
            l.bytes,
            RING_HEADER_SIZE + l.stride as usize * l.slots as usize,
            "ring size is header + slots*stride, nothing else"
        );
    }

    #[test]
    fn hex_round_trips_and_refuses_junk() {
        let bytes: Vec<u8> = (0..=255u8).collect();
        assert_eq!(hex_decode(&hex_encode(&bytes)).unwrap(), bytes);
        assert_eq!(hex_encode(&[0x00, 0x0f, 0xff]), "000fff");
        assert!(hex_decode("abc").is_err(), "odd length must be refused");
        assert!(hex_decode("zz").is_err(), "non-hex digit must be refused");
    }

    #[test]
    fn ring_is_fifo_and_reports_full() {
        let (ring, fd) = MappedRing::create(RingLayout::new(2, 4096).unwrap()).unwrap();
        close_fd(fd);

        ring.write_sample(1, 1_000, 900, 1, b"first").unwrap();
        ring.write_sample(2, 2_000, 1_900, 0, b"second").unwrap();
        let err = ring.write_sample(3, 0, 0, 0, b"third").unwrap_err();
        assert_eq!(
            err.kind(),
            io::ErrorKind::WouldBlock,
            "ring full must block"
        );

        let mut out = Vec::new();
        let meta = ring.read_into(1, &mut out).unwrap();
        assert_eq!(meta.seq, 1);
        assert_eq!(meta.pts_us, 1_000);
        assert_eq!(meta.dts_us, 900);
        assert_eq!(meta.flags, 1);
        assert_eq!(meta.len, 5);
        assert_eq!(&out[..], b"first");
        // read_into 不推进 read_seq：没 commit 之前同一槽还能再读一次。
        assert!(ring.read_into(2, &mut out).is_ok());
        ring.commit(1).unwrap();

        let mut out = Vec::new();
        ring.read_into(2, &mut out).unwrap();
        assert_eq!(&out[..], b"second");
        ring.commit(2).unwrap();

        // 腾出槽位后必须还能写，且序号连续（这就是"卡死"类回归的探针）。
        ring.write_sample(3, 0, 0, 0, b"third").unwrap();
        let mut out = Vec::new();
        let meta = ring.read_into(3, &mut out).unwrap();
        assert_eq!(&out[..], b"third");
        assert_eq!(meta.seq, 3);
    }

    #[test]
    fn empty_ring_blocks_instead_of_returning_garbage() {
        let (ring, fd) = MappedRing::create(RingLayout::new(2, 4096).unwrap()).unwrap();
        close_fd(fd);
        let mut out = vec![0u8; 8];
        let err = ring.read_into(1, &mut out).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::WouldBlock);
    }

    #[test]
    fn oversized_and_out_of_order_writes_are_refused() {
        let (ring, fd) = MappedRing::create(RingLayout::new(2, 4096).unwrap()).unwrap();
        close_fd(fd);

        let too_big = vec![0u8; 4097];
        let err = ring.write_sample(1, 0, 0, 0, &too_big).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);

        // 序号必须从 1 开始连续，跳号会让消费者永远等不到第一条。
        let err = ring.write_sample(2, 0, 0, 0, b"skipped").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);

        // 空 ring 上 commit 一个不存在的序号也必须被拒。
        let err = ring.commit(1).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(ring.sequences(), (0, 0));
    }
}
