//! Guest-side client for the Husk system video bridge.
//!
//! The VA driver is the system-layer adapter: callers continue to use
//! standard VA-API, while this module forwards compressed access units to the
//! app-side MediaCodec bridge over an AF_UNIX socket and a memfd ring.

use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;
use video_protocol::{
    close_fd, hex_encode, recv_fd, MappedRing, RingLayout, DEFAULT_SLOTS, DEFAULT_SLOT_SIZE,
};

const CONTROL_TIMEOUT: Duration = Duration::from_secs(10);

pub struct VideoClient {
    stream: UnixStream,
    ring: MappedRing,
    next_seq: u64,
    width: u32,
    height: u32,
}

impl VideoClient {
    pub fn connect(width: u32, height: u32, csd0: &[u8], csd1: &[u8]) -> std::io::Result<Self> {
        let paths = candidate_paths();
        let mut last_error = None;
        let mut stream = None;
        for path in &paths {
            match UnixStream::connect(path) {
                Ok(connected) => {
                    stream = Some(connected);
                    break;
                }
                Err(error) => last_error = Some(error),
            }
        }
        let mut stream = stream.ok_or_else(|| {
            last_error.unwrap_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("no video bridge socket among {paths:?}"),
                )
            })
        })?;
        stream.set_read_timeout(Some(CONTROL_TIMEOUT))?;
        stream.set_write_timeout(Some(CONTROL_TIMEOUT))?;

        send_line(
            &mut stream,
            &format!(
                "OPEN mime=video/avc width={width} height={height} codec=c2.mtk.avc.decoder csd0={} csd1={}",
                hex_encode(csd0),
                hex_encode(csd1),
            ),
        )?;
        let reply = read_line(&mut stream)?;
        if !reply.starts_with("OPEN_READY ") {
            return Err(std::io::Error::other(format!(
                "video bridge OPEN failed: {reply}"
            )));
        }

        let layout = RingLayout::new(DEFAULT_SLOTS, DEFAULT_SLOT_SIZE)?;
        let fd = recv_fd(stream.as_raw_fd())?;
        let ring = match MappedRing::from_fd(fd, layout) {
            Ok(ring) => ring,
            Err(error) => {
                close_fd(fd);
                return Err(error);
            }
        };
        close_fd(fd);

        Ok(Self {
            stream,
            ring,
            next_seq: 1,
            width,
            height,
        })
    }

    pub fn matches(&self, width: u32, height: u32) -> bool {
        self.width == width && self.height == height
    }

    pub fn submit(&mut self, access_unit: &[u8], pts_us: i64, flags: u32) -> std::io::Result<()> {
        let seq = self.next_seq;
        self.ring.write_sample(seq, pts_us, 0, flags, access_unit)?;
        send_line(
            &mut self.stream,
            &format!(
                "SAMPLE seq={seq} len={} pts_us={pts_us} flags={flags}",
                access_unit.len()
            ),
        )?;
        let reply = read_line(&mut self.stream)?;
        if !reply.starts_with("SAMPLE_OK ") {
            return Err(std::io::Error::other(format!(
                "video bridge SAMPLE failed: {reply}"
            )));
        }
        self.next_seq += 1;
        Ok(())
    }
}

fn candidate_paths() -> Vec<PathBuf> {
    if let Some(path) = std::env::var_os("HUSK_VA_SOCKET") {
        return vec![PathBuf::from(path)];
    }
    let mut paths = vec![PathBuf::from("/run/husk/video.sock")];
    if let Some(rootfs) = std::env::var_os("NR_ROOTFS") {
        let physical = PathBuf::from(rootfs).join("run/husk/video.sock");
        if physical != paths[0] {
            paths.push(physical);
        }
    }
    paths
}

fn send_line(stream: &mut UnixStream, line: &str) -> std::io::Result<()> {
    stream.write_all(line.as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()
}

fn read_line(stream: &mut UnixStream) -> std::io::Result<String> {
    let mut bytes = Vec::with_capacity(128);
    let mut one = [0u8; 1];
    loop {
        match stream.read(&mut one)? {
            0 => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "video bridge closed connection",
                ))
            }
            _ if one[0] == b'\n' => break,
            _ if one[0] != b'\r' => bytes.push(one[0]),
            _ => {}
        }
        if bytes.len() > 64 * 1024 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "video bridge control line too long",
            ));
        }
    }
    String::from_utf8(bytes)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "non-utf8 bridge reply"))
}
