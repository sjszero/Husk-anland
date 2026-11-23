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
use std::collections::HashSet;
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
    buffer_key: u64,
    image: usize,
    slot: usize,
    release: Option<Sender<usize>>,

    buffer: usize,
    output_buffer: usize,
    output_key: u64,
    output_dma_key: u64,
    output_desc: AhbDesc,
    output_offset: u64,
    output_stride: u32,
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
    buffer_key: u64,
    dma_key: u64,
    desc: &AhbDesc,
    planes: &AhbPlanes,
    timestamp_ns: i64,
    plane_hashes: &[u64; 4],
    plane_bytes: &[usize; 4],
    output: Option<(u64, u64, &AhbDesc, u64, u32)>,
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

    let magic = if output.is_some() {
        "HUSK-VIDEO-AHB-3"
    } else {
        "HUSK-VIDEO-AHB-2"
    };
    let mut text = format!(
        "{magic}\nframe_index={index}\nbuffer_key={buffer_key:016x}\ndmabuf_key={dma_key:016x}\nwidth={}\nheight={}\nstride={}\nformat={}\nusage=0x{:x}\ntimestamp_ns={timestamp_ns}\nplane_count={count}\nluma_fnv={}\nluma_bytes={}\ncookies=0,0,0,0,0,0,0,0,0\n",
        desc.width,
        desc.height,
        desc.stride,
        desc.format,
        desc.usage,
        plane_hashes[0],
        plane_bytes[0],
    );
    if let Some((output_key, output_dma_key, output_desc, output_offset, output_stride)) = output {
        text.push_str(&format!(
            "handle_count=2\noutput_buffer_key={output_key:016x}\noutput_dmabuf_key={output_dma_key:016x}\noutput_width={}\noutput_height={}\noutput_stride={output_stride}\noutput_format={}\noutput_offset={output_offset}\noutput_usage=0x{:x}\n",
            output_desc.width, output_desc.height, output_desc.format, output_desc.usage
        ));
    }
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
    if job.image != 0 {
        unsafe {
            (api.image_delete)(job.image as *mut Image);
        }
    }
    if let Some(release) = &job.release {
        let _ = release.send(job.slot);
    }
    inflight.fetch_sub(1, Ordering::AcqRel);
    notify_done(done);
    if job.index <= 3 || job.index.is_multiple_of(30) {
        crate::log(&format!(
            "video|ahb-ring|RELEASE|index={}|slot={}",
            job.index, job.slot
        ));
    }
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
    let mut sent_keys = HashSet::new();
    let mut handle_count = 0u64;
    let mut reuse_count = 0u64;
    let mut presented_count = 0u64;
    let mut present_fail_count = 0u64;
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
            let new_key = sent_keys.insert(job.buffer_key);
            let handle_flag = if new_key { b'N' } else { b'R' };
            if let Err(error) = stream.write_all(&[handle_flag]) {
                release_job(&api, job, &inflight, &done);
                return Err(format!("send handle flag: {error}"));
            }
            let send_rc = if new_key {
                handle_count = handle_count.saturating_add(1);
                let input_rc = unsafe {
                    (api.ahb_send_to_socket)(job.buffer as *const Ahb, stream.as_raw_fd())
                };
                if input_rc != 0 {
                    input_rc
                } else if job.output_buffer != 0 {
                    unsafe {
                        (api.ahb_send_to_socket)(
                            job.output_buffer as *const Ahb,
                            stream.as_raw_fd(),
                        )
                    }
                } else {
                    -1
                }
            } else {
                reuse_count = reuse_count.saturating_add(1);
                0
            };
            if job.index <= 3 || job.index.is_multiple_of(30) || send_rc != 0 {
                crate::log(&format!(
                    "video|ahb-ring|FRAME|index={}|key={:016x}|output_key={:016x}|handle={}|send_rc={send_rc}|bytes={}|socket={}",
                    job.index,
                    job.buffer_key,
                    job.output_key,
                    if new_key { "NEW" } else { "REUSE" },
                    job.meta.len(),
                    stream.as_raw_fd()
                ));
            }
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
            if ok && job.output_buffer != 0 {
                let presented = unsafe {
                    super::present::present_if_enabled(
                        &files,
                        job.output_buffer as *mut Ahb,
                        job.output_desc.width,
                        job.output_desc.height,
                    )
                };
                if presented {
                    presented_count = presented_count.saturating_add(1);
                    if job.index <= 3 || job.index.is_multiple_of(30) {
                        crate::log(&format!(
                            "video|ahb-ring|PRESENT|index={}|output_key={:016x}|size={}x{}",
                            job.index,
                            job.output_key,
                            job.output_desc.width,
                            job.output_desc.height
                        ));
                    }
                } else {
                    present_fail_count = present_fail_count.saturating_add(1);
                }
            }
            if job.index <= 3 || job.index.is_multiple_of(30) || !ok {
                crate::log(&format!(
                    "video|ahb-ring|ACK|index={}|byte=0x{ack:02x}|ok={ok}|elapsed_ms={}",
                    job.index,
                    started.elapsed().as_millis()
                ));
            }
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
            "video|ahb-ring|SUMMARY|PASS|sent={send_count}|completed={completed}|presented={presented_count}|present_fail={present_fail_count}|handles={handle_count}|reuses={reuse_count}"
        )),
        Err(error) => crate::log(&format!(
            "video|ahb-ring|SUMMARY|FAIL|sent={send_count}|completed={completed}|presented={presented_count}|present_fail={present_fail_count}|handles={handle_count}|reuses={reuse_count}|{error}"
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
    buffer_key: u64,
    dma_key: u64,
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
    if buffer_key == 0 {
        crate::log(&format!(
            "video|ahb-ring|FAIL|frame={frame_index}|missing stable dma-buf key"
        ));
        return false;
    }
    if !ensure_session(files, api) {
        return false;
    }
    let meta = metadata(
        frame_index,
        buffer_key,
        dma_key,
        desc,
        planes,
        timestamp_ns,
        plane_hashes,
        plane_bytes,
        None,
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
        buffer_key,
        image: image as usize,
        slot: 0,
        release: None,
        buffer: buffer as usize,
        output_buffer: 0,
        output_key: 0,
        output_dma_key: 0,
        output_desc: AhbDesc::default(),
        output_offset: 0,
        output_stride: 0,
        meta,
    };
    if session.tx.send(job).is_err() {
        session.inflight.fetch_sub(1, Ordering::AcqRel);
        notify_done(&session.done);
        return false;
    }
    true
}

/// Queue one frame backed by an application-owned, fixed AHardwareBuffer.
/// Unlike `export_if_enabled`, there is no `AImage` to release: the caller
/// keeps the AHB alive and gets its slot back through `release` after the
/// guest acknowledges the frame.
#[allow(clippy::too_many_arguments)]
pub(super) unsafe fn export_fixed_if_enabled(
    files: &Path,
    api: &ImageApi,
    buffer: *mut Ahb,
    desc: &AhbDesc,
    planes: &AhbPlanes,
    buffer_key: u64,
    dma_key: u64,
    frame_index: u64,
    timestamp_ns: i64,
    plane_hashes: &[u64; 4],
    plane_bytes: &[usize; 4],
    output_buffer: *mut Ahb,
    output_key: u64,
    output_dma_key: u64,
    output_desc: &AhbDesc,
    output_offset: u64,
    output_stride: u32,
    slot: usize,
    release: Sender<usize>,
) -> bool {
    if !enabled(files) {
        return false;
    }
    if planes.plane_count == 0 {
        crate::log(&format!(
            "video|ahb-ring|FAIL|frame={frame_index}|fixed layout has no planes"
        ));
        return false;
    }
    if buffer_key == 0 {
        crate::log(&format!(
            "video|ahb-ring|FAIL|frame={frame_index}|missing stable AHB key"
        ));
        return false;
    }
    if output_buffer.is_null() || output_key == 0 || output_dma_key == 0 || output_stride == 0 {
        crate::log(&format!(
            "video|ahb-ring|FAIL|frame={frame_index}|missing output AHB metadata"
        ));
        return false;
    }
    if !ensure_session(files, api) {
        return false;
    }
    let meta = metadata(
        frame_index,
        buffer_key,
        dma_key,
        desc,
        planes,
        timestamp_ns,
        plane_hashes,
        plane_bytes,
        Some((
            output_key,
            output_dma_key,
            output_desc,
            output_offset,
            output_stride,
        )),
    );
    let mut active = session_slot().lock().expect("session mutex");
    let Some(session) = active.as_mut() else {
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
        buffer_key,
        image: 0,
        slot,
        release: Some(release),
        buffer: buffer as usize,
        output_buffer: output_buffer as usize,
        output_key,
        output_dma_key,
        output_desc: *output_desc,
        output_offset,
        output_stride,
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
