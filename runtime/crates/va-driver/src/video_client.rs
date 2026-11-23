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
const KEY_FRAME: u32 = 1;

/// True when any Annex-B NAL unit in `data` is an IDR slice (type 5).
fn is_idr(data: &[u8]) -> bool {
    let mut index = 0;
    while index + 4 <= data.len() {
        if data[index] == 0 && data[index + 1] == 0 {
            let (header, skip) = if data[index + 2] == 1 {
                (index + 3, 3)
            } else if data[index + 2] == 0 && index + 4 <= data.len() && data[index + 3] == 1 {
                (index + 4, 4)
            } else {
                index += 1;
                continue;
            };
            if header < data.len() && data[header] & 0x1f == 5 {
                return true;
            }
            index = header + skip.min(1);
            continue;
        }
        index += 1;
    }
    false
}

/// One decoded picture, already repacked to tight NV12 by the app side.
pub struct Frame {
    pub width: u32,
    pub height: u32,
    /// Tight NV12: `width*height` luma followed by `width*height/2` interleaved UV.
    pub nv12: Vec<u8>,
}

pub struct VideoClient {
    stream: UnixStream,
    ring: MappedRing,
    next_seq: u64,
    width: u32,
    height: u32,
    /// Annex-B SPS/PPS, prepended to the first access unit of the session so a
    /// decoder that ignores the codec-config buffers still starts from a
    /// self-describing stream.
    csd: Vec<u8>,
    first: bool,
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

        // Test hook: an explicit Annex-B CSD overrides the synthesized one.
        // The synthesized SPS/PPS is a reconstruction from VA picture/slice
        // parameters; when a stream's real parameter sets differ, decoding can
        // silently produce nothing. This lets the transport be tested against
        // known-good parameter sets while the synthesis is being fixed.
        let (csd0, csd1) = override_csd(csd0, csd1);
        if std::env::var_os("HUSK_VA_TRACE").is_some() {
            eprintln!(
                "HUSK_VA_CSD sps={} pps={}",
                hex_encode(&csd0),
                hex_encode(&csd1)
            );
        }
        let csd0 = csd0.as_slice();
        let csd1 = csd1.as_slice();
        let mut csd = Vec::with_capacity(csd0.len() + csd1.len());
        csd.extend_from_slice(csd0);
        csd.extend_from_slice(csd1);

        Ok(Self {
            stream,
            ring,
            next_seq: 1,
            width,
            height,
            csd,
            first: true,
        })
    }

    pub fn matches(&self, width: u32, height: u32) -> bool {
        self.width == width && self.height == height
    }

    /// Submit one access unit. The reply is only an acknowledgement: decoding
    /// is asynchronous, so the pixels are claimed later with `fetch_frame`.
    pub fn submit(&mut self, access_unit: &[u8], pts_us: i64, flags: u32) -> std::io::Result<()> {
        let seq = self.next_seq;
        let mut payload = Vec::with_capacity(self.csd.len() + access_unit.len());
        if self.first {
            payload.extend_from_slice(&self.csd);
            self.first = false;
        }
        payload.extend_from_slice(access_unit);
        // Tell the decoder what the bitstream says instead of assuming: an IDR
        // sample without the key-frame flag is dropped by MediaCodec on this
        // MTK codec.
        let flags = if is_idr(access_unit) {
            flags | KEY_FRAME
        } else {
            flags
        };
        self.ring.write_sample(seq, pts_us, 0, flags, &payload)?;
        send_line(
            &mut self.stream,
            &format!(
                "SAMPLE seq={seq} len={} pts_us={pts_us} flags={flags}",
                payload.len()
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

    /// Claim the decoded picture that carries `pts_us`. The app keeps decoded
    /// frames until asked for them, because a codec may need several samples
    /// before it emits anything and output order is not input order.
    pub fn fetch_frame(&mut self, pts_us: i64) -> std::io::Result<Option<Frame>> {
        send_line(&mut self.stream, &format!("FRAME pts={pts_us}"))?;
        let reply = read_line(&mut self.stream)?;
        if reply.starts_with("FRAME_NONE") {
            return Ok(None);
        }
        if !reply.starts_with("FRAME_OK ") {
            return Err(std::io::Error::other(format!(
                "video bridge FRAME failed: {reply}"
            )));
        }
        let fields = parse_fields(&reply);
        let width = parse_field::<u32>(&fields, "w")?;
        let height = parse_field::<u32>(&fields, "h")?;
        let bytes = parse_field::<usize>(&fields, "bytes")?;
        let expected = width as usize * height as usize + (width as usize * height as usize).div_ceil(2);
        if bytes != expected {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("frame size mismatch: reply={bytes} expected={expected} for {width}x{height}"),
            ));
        }
        let mut nv12 = vec![0u8; bytes];
        self.stream.read_exact(&mut nv12)?;
        Ok(Some(Frame { width, height, nv12 }))
    }
}

fn u_padded(luma: &[u8], expected: usize) -> &[u8] {
    // The app already sends tight luma; the slice keeps the contract explicit
    // so a future padded format cannot silently shift the UV plane.
    debug_assert_eq!(luma.len(), expected);
    luma
}

fn parse_fields(line: &str) -> Vec<(&str, &str)> {
    line.split_whitespace()
        .filter_map(|field| field.split_once('='))
        .collect()
}

fn field<'a>(fields: &[(&'a str, &'a str)], key: &str) -> Option<&'a str> {
    fields.iter().find_map(|(k, v)| (*k == key).then_some(*v))
}

fn parse_field<T: std::str::FromStr>(fields: &[(&str, &str)], key: &str) -> std::io::Result<T> {
    let raw = field(fields, key).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("frame reply missing {key}"),
        )
    })?;
    raw.parse::<T>().map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("frame reply {key}={raw} is not a number"),
        )
    })
}

fn override_csd(sps: &[u8], pps: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let read = |key: &str, fallback: &[u8]| -> Vec<u8> {
        let Some(text) = std::env::var_os(key) else {
            return fallback.to_vec();
        };
        let text = text.to_string_lossy();
        match video_protocol::hex_decode(text.trim()) {
            Ok(bytes) if !bytes.is_empty() => {
                let mut out = Vec::with_capacity(bytes.len() + 4);
                if !bytes.starts_with(&[0, 0, 0, 1]) && !bytes.starts_with(&[0, 0, 1]) {
                    out.extend_from_slice(&[0, 0, 0, 1]);
                }
                out.extend_from_slice(&bytes);
                out
            }
            _ => fallback.to_vec(),
        }
    };
    (read("HUSK_VA_SPS", sps), read("HUSK_VA_PPS", pps))
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
