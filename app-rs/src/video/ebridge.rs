//! Persistent handoff of real MediaCodec YUV AHardwareBuffers to the guest.
//!
//! The guest receives full Android native handles through
//! `AHardwareBuffer_sendHandleToUnixSocket`. The dma-buf descriptor is found by
//! inspecting `/proc/self/fd`, never by assuming a descriptor index.
//!
//! This bridge keeps one Unix connection open for the whole playback session and
//! allows up to `MAX_INFLIGHT` decoded images to be owned by the sender at once.
//! A decoded `AImage` is released only after the guest acknowledges that the
//! frame has been imported and verified, which gives the decoder natural
//! backpressure and is the stepping stone to the persistent multi-slot ring.

use super::*;
use std::io::{Read, Write};
use std::net::Shutdown;
use std::os::fd::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const ACCEPT_TIMEOUT: Duration = Duration::from_secs(90);
const ACK_TIMEOUT: Duration = Duration::from_secs(30);
const WAIT_SLICE: Duration = Duration::from_millis(100);
const MAX_INFLIGHT: usize = 3;

static SESSION: OnceLock<Mutex<Option<Session>>> = OnceLock::new();

struct FrameJob {
    index: u64,
    image: usize,
    buffer: usize,
    meta: String,
}

#[derive(Clone, Copy)]
struct WorkerApi {
    image_delete: FnImageDelete,
    ahb_send_to_socket: FnAhbSendToSocket,
}

impl From<&ImageApi> for WorkerApi {
    fn from(api: &ImageApi) -> Self {
        Self {
            image_delete: api.image_delete,
            ahb_send_to_socket: api.ahb_send_to_socket,
        }
    }
}

struct Session {
    tx: Sender<FrameJob>,
    stop: Arc<AtomicBool>,
    failed: Arc<AtomicBool>,
    inflight: Arc<AtomicUsize>,
    done: Arc<(Mutex<usize>, Condvar)>,
    join: Option<JoinHandle<()>>,
}

pub(super) fn enabled(files: &Path) -> bool {
    files.join("video-ahb").exists()
}

fn metadata(
    index: u64,
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
        "HUSK-VIDEO-AHB-1\nframe_index={index}\nwidth={}\nheight={}\nstride={}\nformat={}\nusage=0x{:x}\ntimestamp_ns={timestamp_ns}\nplane_count={count}\nluma_fnv={}\nluma_bytes={}\ncookies=0,0,0,0,0,0,0,0,0\n",
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

fn session_slot() -> &'static Mutex<Option<Session>> {
    SESSION.get_or_init(|| Mutex::new(None))
}

fn notify_done(done: &(Mutex<usize>, Condvar)) {
    if let Ok(mut completed) = done.0.lock() {
        *completed = completed.saturating_add(1);
        done.1.notify_all();
    }
}

fn release_job(
    api: &WorkerApi,
    job: FrameJob,
    inflight: &AtomicUsize,
    done: &(Mutex<usize>, Condvar),
) {
    unsafe {
        (api.image_delete)(job.image as *mut Image);
    }
    inflight.fetch_sub(1, Ordering::AcqRel);
    notify_done(done);
    crate::log(&format!("video|ahb-ring|RELEASE|index={}", job.index));
}

fn marker_present(files: &Path) -> bool {
    enabled(files)
}

fn accept_client(files: &Path, stop: &AtomicBool) -> Result<(UnixListener, UnixStream), String> {
    let dir = files.join("rootfs-full/run/husk");
    std::fs::create_dir_all(&dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
    let path = dir.join("video-ahb.sock");
    let _ = std::fs::remove_file(&path);
    let listener =
        UnixListener::bind(&path).map_err(|e| format!("bind {}: {e}", path.display()))?;
    listener
        .set_nonblocking(true)
        .map_err(|e| format!("nonblocking listener: {e}"))?;
    crate::log(&format!("video|ahb-ring|LISTEN|{}", path.display()));

    let deadline = Instant::now() + ACCEPT_TIMEOUT;
    let mut retries = 0u32;
    loop {
        if stop.load(Ordering::Acquire) || !marker_present(files) {
            return Err("stopped before guest connected".into());
        }
        match listener.accept() {
            Ok((stream, _)) => {
                stream
                    .set_read_timeout(Some(WAIT_SLICE))
                    .map_err(|e| format!("read timeout: {e}"))?;
                stream
                    .set_write_timeout(Some(Duration::from_secs(5)))
                    .map_err(|e| format!("write timeout: {e}"))?;
                crate::log(&format!(
                    "video|ahb-ring|CONNECT|fd={}|path={}",
                    stream.as_raw_fd(),
                    path.display()
                ));
                return Ok((listener, stream));
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                retries = retries.saturating_add(1);
                if retries == 1 || retries % 100 == 0 {
                    crate::log(&format!("video|ahb-ring|WAIT|retry={retries}|{path:?}"));
                }
                if Instant::now() >= deadline {
                    return Err("accept timeout".into());
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(error) => return Err(format!("accept: {error}")),
        }
    }
}

fn wait_ack(stream: &mut UnixStream, stop: &AtomicBool, files: &Path) -> Option<u8> {
    let deadline = Instant::now() + ACK_TIMEOUT;
    let mut byte = [0u8; 1];
    loop {
        if stop.load(Ordering::Acquire) || !marker_present(files) {
            return None;
        }
        match stream.read(&mut byte) {
            Ok(0) => return None,
            Ok(_) => return Some(byte[0]),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                if Instant::now() >= deadline {
                    return None;
                }
            }
            Err(_) => return None,
        }
    }
}

fn worker(
    files: PathBuf,
    api: WorkerApi,
    rx: Receiver<FrameJob>,
    stop: Arc<AtomicBool>,
    failed: Arc<AtomicBool>,
    inflight: Arc<AtomicUsize>,
    done: Arc<(Mutex<usize>, Condvar)>,
) {
    let mut completed = 0u64;
    let mut send_count = 0u64;
    let outcome = (|| -> Result<(), String> {
        let (listener, mut stream) = accept_client(&files, &stop)?;
        let path = files.join("rootfs-full/run/husk/video-ahb.sock");
        loop {
            if stop.load(Ordering::Acquire) || !marker_present(&files) {
                break;
            }
            let job = match rx.recv_timeout(WAIT_SLICE) {
                Ok(job) => job,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            };
            if stop.load(Ordering::Acquire) || !marker_present(&files) {
                release_job(&api, job, &inflight, &done);
                break;
            }

            let started = Instant::now();
            let meta_result = stream
                .write_all(&(job.meta.len() as u32).to_be_bytes())
                .and_then(|_| stream.write_all(job.meta.as_bytes()));
            if let Err(error) = meta_result {
                crate::log(&format!(
                    "video|ahb-ring|FAIL|index={}|meta={error}",
                    job.index
                ));
                release_job(&api, job, &inflight, &done);
                return Err(format!("send metadata: {error}"));
            }
            let send_rc =
                unsafe { (api.ahb_send_to_socket)(job.buffer as *const Ahb, stream.as_raw_fd()) };
            crate::log(&format!(
                "video|ahb-ring|FRAME|index={}|bytes={}|send_rc={send_rc}|socket={}",
                job.index,
                job.meta.len(),
                stream.as_raw_fd()
            ));
            if send_rc != 0 {
                release_job(&api, job, &inflight, &done);
                return Err(format!("send native handle rc={send_rc}"));
            }
            send_count = send_count.saturating_add(1);

            let Some(ack) = wait_ack(&mut stream, &stop, &files) else {
                crate::log(&format!(
                    "video|ahb-ring|ACK_FAIL|index={}|timeout_or_closed",
                    job.index
                ));
                release_job(&api, job, &inflight, &done);
                return Err("guest acknowledgement timeout".into());
            };
            let ok = ack == b'D';
            crate::log(&format!(
                "video|ahb-ring|ACK|index={}|byte=0x{ack:02x}|ok={ok}|elapsed_ms={}",
                job.index,
                started.elapsed().as_millis()
            ));
            release_job(&api, job, &inflight, &done);
            if !ok {
                return Err(format!("guest rejected frame with 0x{ack:02x}"));
            }
            completed = completed.saturating_add(1);
        }
        let _ = stream.shutdown(Shutdown::Both);
        let _ = std::fs::remove_file(&path);
        drop(listener);
        Ok(())
    })();

    while let Ok(job) = rx.try_recv() {
        release_job(&api, job, &inflight, &done);
    }
    failed.store(true, Ordering::Release);
    notify_done(&done);
    match outcome {
        Ok(()) => crate::log(&format!(
            "video|ahb-ring|SUMMARY|PASS|sent={send_count}|completed={completed}"
        )),
        Err(error) => crate::log(&format!(
            "video|ahb-ring|SUMMARY|FAIL|sent={send_count}|completed={completed}|{error}"
        )),
    }
}

fn start_session(files: &Path, api: &ImageApi, slot: &mut Option<Session>) -> bool {
    let (tx, rx) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let failed = Arc::new(AtomicBool::new(false));
    let inflight = Arc::new(AtomicUsize::new(0));
    let done = Arc::new((Mutex::new(0usize), Condvar::new()));
    let worker_files = files.to_path_buf();
    let worker_api = WorkerApi::from(api);
    let worker_stop = stop.clone();
    let worker_failed = failed.clone();
    let worker_inflight = inflight.clone();
    let worker_done = done.clone();
    let join = std::thread::spawn(move || {
        worker(
            worker_files,
            worker_api,
            rx,
            worker_stop,
            worker_failed,
            worker_inflight,
            worker_done,
        )
    });
    *slot = Some(Session {
        tx,
        stop,
        failed,
        inflight,
        done,
        join: Some(join),
    });
    crate::log("video|ahb-ring|SESSION|START");
    true
}

fn stop_session(mut session: Session) {
    session.stop.store(true, Ordering::Release);
    session.failed.store(true, Ordering::Release);
    drop(session.tx);
    if let Some(join) = session.join.take() {
        let _ = join.join();
    }
    crate::log("video|ahb-ring|SESSION|STOP");
}

fn ensure_session(files: &Path, api: &ImageApi) -> bool {
    let stale = {
        let mut slot = session_slot().lock().expect("session mutex");
        match slot.as_ref() {
            Some(session) if !session.failed.load(Ordering::Acquire) => return true,
            _ => slot.take(),
        }
    };
    if let Some(session) = stale {
        stop_session(session);
    }
    let mut slot = session_slot().lock().expect("session mutex");
    if slot.is_some() {
        return true;
    }
    start_session(files, api, &mut slot)
}

fn wait_for_capacity(session: &Session, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    let mut completed = session.done.0.lock().expect("completion mutex");
    loop {
        if session.failed.load(Ordering::Acquire) {
            return false;
        }
        if session.inflight.load(Ordering::Acquire) < MAX_INFLIGHT {
            return true;
        }
        let now = Instant::now();
        if now >= deadline {
            return false;
        }
        let wait = (deadline - now).min(WAIT_SLICE);
        let (next, _) = session
            .done
            .1
            .wait_timeout(completed, wait)
            .expect("completion wait");
        completed = next;
    }
}

fn drain_before_stop(session: &Session, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    let mut completed = session.done.0.lock().expect("completion mutex");
    loop {
        if session.failed.load(Ordering::Acquire) {
            return false;
        }
        if session.inflight.load(Ordering::Acquire) == 0 {
            return true;
        }
        let now = Instant::now();
        if now >= deadline {
            return false;
        }
        let wait = (deadline - now).min(WAIT_SLICE);
        let (next, _) = session
            .done
            .1
            .wait_timeout(completed, wait)
            .expect("completion wait");
        completed = next;
    }
}

/// Queue one decoded frame. Returns true only when the worker owns and will
/// release the `AImage`; the caller must not delete that image on true.
pub(super) unsafe fn export_if_enabled(
    files: &Path,
    api: &ImageApi,
    image: *mut Image,
    buffer: *mut Ahb,
    desc: &AhbDesc,
    planes: &AhbPlanes,
    lock_rc: c_int,
    frame_index: u64,
    timestamp_ns: i64,
    plane_hashes: &[u64; 4],
    plane_bytes: &[usize; 4],
) -> bool {
    if !enabled(files) {
        return false;
    }
    if lock_rc != 0 || planes.plane_count == 0 {
        crate::log(&format!(
            "video|ahb-ring|FAIL|frame={frame_index}|layout lock_rc={lock_rc} planes={}",
            planes.plane_count
        ));
        return false;
    }
    if !ensure_session(files, api) {
        return false;
    }
    let meta = metadata(
        frame_index,
        desc,
        planes,
        timestamp_ns,
        plane_hashes,
        plane_bytes,
    );
    let mut slot = session_slot().lock().expect("session mutex");
    let Some(session) = slot.as_mut() else {
        return false;
    };
    if session.failed.load(Ordering::Acquire) {
        return false;
    }
    if !wait_for_capacity(session, Duration::from_secs(30)) {
        crate::log(&format!(
            "video|ahb-ring|FAIL|frame={frame_index}|capacity timeout"
        ));
        return false;
    }
    session.inflight.fetch_add(1, Ordering::AcqRel);
    let job = FrameJob {
        index: frame_index,
        image: image as usize,
        buffer: buffer as usize,
        meta,
    };
    if session.tx.send(job).is_err() {
        session.inflight.fetch_sub(1, Ordering::AcqRel);
        notify_done(&session.done);
        return false;
    }
    true
}

/// Stop the persistent sender. Any images still owned by the worker are deleted
/// before this returns, so callers may safely delete the `ImageReader` next.
pub(super) fn shutdown() {
    let Some(slot) = SESSION.get() else {
        return;
    };
    let session = slot.lock().expect("session mutex").take();
    if let Some(session) = session {
        let _ = drain_before_stop(&session, Duration::from_secs(30));
        stop_session(session);
    }
}
