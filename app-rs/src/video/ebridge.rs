//! One-shot handoff of a real MediaCodec YUV AHardwareBuffer to the guest.
//!
//! This is a measurement bridge, not the final per-frame presentation path. It
//! uses Android's public `AHardwareBuffer_sendHandleToUnixSocket` API so the
//! guest receives every descriptor in the native handle, including the
//! gralloc-extra descriptor and the real `/dmabuf:` descriptor. The guest must
//! discover the dma-buf by inspecting `/proc/self/fd`, never by assuming fd[0].

use super::*;
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

static EXPORTED: AtomicBool = AtomicBool::new(false);
const ACCEPT_TIMEOUT: Duration = Duration::from_secs(90);
const ACK_TIMEOUT: Duration = Duration::from_secs(30);

pub(super) fn enabled(files: &Path) -> bool {
    files.join("video-ahb").exists()
}

fn metadata(
    desc: &AhbDesc,
    planes: &AhbPlanes,
    timestamp_ns: i64,
    plane_hashes: &[u64; 4],
    plane_bytes: &[usize; 4],
) -> String {
    let count = (planes.plane_count as usize).min(planes.planes.len());
    let mut base = usize::MAX;
    for index in 0..count {
        let data = planes.planes[index].data as usize;
        if data != 0 {
            base = base.min(data);
        }
    }
    if base == usize::MAX {
        base = 0;
    }

    let mut text = format!(
        "HUSK-VIDEO-AHB-1\nwidth={}\nheight={}\nstride={}\nformat={}\nusage=0x{:x}\ntimestamp_ns={timestamp_ns}\nplane_count={count}\nluma_fnv={}\nluma_bytes={}\ncookies=0,0,0,0,0,0,0,0,0\n",
        desc.width,
        desc.height,
        desc.stride,
        desc.format,
        desc.usage,
        plane_hashes[0],
        plane_bytes[0],
    );
    for index in 0..count {
        let plane = planes.planes[index];
        let offset = (plane.data as usize).saturating_sub(base);
        text.push_str(&format!(
            "plane{index}_offset={offset}\nplane{index}_row_stride={}\nplane{index}_pixel_stride={}\nplane{index}_fnv={}\nplane{index}_bytes={}\n",
            plane.row_stride, plane.pixel_stride, plane_hashes[index], plane_bytes[index]
        ));
    }
    text
}

/// Send the first decoded frame to a waiting guest when `files/video-ahb` is
/// present. The marker keeps this diagnostic out of normal playback.
pub(super) unsafe fn export_if_enabled(
    files: &Path,
    api: &ImageApi,
    buffer: *mut Ahb,
    desc: &AhbDesc,
    planes: &AhbPlanes,
    lock_rc: c_int,
    timestamp_ns: i64,
    plane_hashes: &[u64; 4],
    plane_bytes: &[usize; 4],
) {
    if !enabled(files) || EXPORTED.swap(true, Ordering::SeqCst) {
        return;
    }
    crate::log("video|ahb-export|BEGIN");
    if lock_rc != 0 || planes.plane_count == 0 {
        crate::log(&format!(
            "video|ahb-export|FAIL|layout lock_rc={lock_rc} planes={}",
            planes.plane_count
        ));
        return;
    }

    let dir = files.join("rootfs-full/run/husk");
    if let Err(error) = std::fs::create_dir_all(&dir) {
        crate::log(&format!("video|ahb-export|FAIL|mkdir {error}"));
        return;
    }
    let path = dir.join("video-ahb.sock");
    let _ = std::fs::remove_file(&path);
    let listener = match UnixListener::bind(&path) {
        Ok(listener) => listener,
        Err(error) => {
            crate::log(&format!("video|ahb-export|FAIL|bind {error}"));
            return;
        }
    };
    if let Err(error) = listener.set_nonblocking(true) {
        crate::log(&format!("video|ahb-export|FAIL|nonblocking {error}"));
        let _ = std::fs::remove_file(&path);
        return;
    }
    crate::log(&format!("video|ahb-export|LISTEN|{}", path.display()));

    let deadline = Instant::now() + ACCEPT_TIMEOUT;
    let mut stream = loop {
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    crate::log("video|ahb-export|FAIL|accept timeout");
                    let _ = std::fs::remove_file(&path);
                    return;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(error) => {
                crate::log(&format!("video|ahb-export|FAIL|accept {error}"));
                let _ = std::fs::remove_file(&path);
                return;
            }
        }
    };
    let _ = stream.set_read_timeout(Some(ACK_TIMEOUT));
    let meta = metadata(desc, planes, timestamp_ns, plane_hashes, plane_bytes);
    let meta_sent = stream
        .write_all(&(meta.len() as u32).to_be_bytes())
        .and_then(|_| stream.write_all(meta.as_bytes()));
    crate::log(&format!(
        "video|ahb-export|META|rc={meta_sent:?}|{}x{}|format={}|planes={}|bytes={}",
        desc.width,
        desc.height,
        desc.format,
        planes.plane_count,
        meta.len()
    ));
    if meta_sent.is_err() {
        let _ = std::fs::remove_file(&path);
        return;
    }

    let send_rc = (api.ahb_send_to_socket)(buffer, stream.as_raw_fd());
    crate::log(&format!(
        "video|ahb-export|HANDLE|rc={send_rc}|socket={}",
        stream.as_raw_fd()
    ));
    if send_rc == 0 {
        let mut ack = [0u8; 1];
        match stream.read_exact(&mut ack) {
            Ok(()) => crate::log(&format!("video|ahb-export|ACK|byte=0x{:02x}", ack[0])),
            Err(error) => crate::log(&format!("video|ahb-export|ACK_FAIL|{error}")),
        }
    }
    let _ = std::fs::remove_file(&path);
    crate::log("video|ahb-export|DONE");
}
