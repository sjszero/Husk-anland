//! AAudio output bridge, implemented with the public NDK C ABI.
//!
//! The guest cannot open `/dev/snd` under `untrusted_app`. This module runs in
//! the Android app process, where AAudio can reach AudioFlinger and the vendor
//! audio HAL. A same-UID glibc guest sends raw PCM over a dedicated Unix socket;
//! no input, clipboard, or video channel is reused.
//!
//! The bridge is deliberately gated by `files/audio-bridge` or the guest-side
//! `rootfs-full/tmp/husk-pulse/enabled` marker: a normal app launch never
//! opens an audio stream. Guests opt in through Rust `audio-setup --enable` and
//! connect to `files/audio-bridge.sock`.

use core::ffi::{c_char, c_int, c_void};
use std::ffi::CStr;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const RTLD_NOW: c_int = 2;
const AAUDIO_DIRECTION_OUTPUT: c_int = 0;
const AAUDIO_DIRECTION_INPUT: c_int = 1;
const AAUDIO_INPUT_PRESET_GENERIC: c_int = 1;
const AAUDIO_INPUT_PRESET_UNPROCESSED: c_int = 9;
const AAUDIO_FORMAT_PCM_I16: c_int = 1;
const AAUDIO_SHARING_MODE_SHARED: c_int = 1;
const AAUDIO_PERFORMANCE_MODE_LOW_LATENCY: c_int = 12;
const AAUDIO_USAGE_MEDIA: c_int = 1;
const AAUDIO_STREAM_STATE_STARTED: c_int = 4;
const AAUDIO_STREAM_STATE_DISCONNECTED: c_int = 13;
const AAUDIO_OK: c_int = 0;
const CLOCK_MONOTONIC: c_int = 1;
const WRITE_TIMEOUT_NS: i64 = 1_000_000_000;
const MAX_FRAMES_PER_STREAM: usize = 48_000 * 30;
const MAX_DATA_FRAMES: usize = 1_048_576;

static STARTED: AtomicBool = AtomicBool::new(false);

unsafe extern "C" {
    fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

type Builder = c_void;
type Stream = c_void;
type FnCreateBuilder = unsafe extern "C" fn(*mut *mut Builder) -> c_int;
type FnBuilderInt = unsafe extern "C" fn(*mut Builder, c_int);
type FnDeleteBuilder = unsafe extern "C" fn(*mut Builder);
type FnOpen = unsafe extern "C" fn(*mut Builder, *mut *mut Stream) -> c_int;
type FnStreamNoArg = unsafe extern "C" fn(*mut Stream) -> c_int;
type FnWrite = unsafe extern "C" fn(*mut Stream, *const c_void, i32, i64) -> i32;
type FnRead = unsafe extern "C" fn(*mut Stream, *mut c_void, i32, i64) -> i32;
type FnGetI32 = unsafe extern "C" fn(*mut Stream) -> i32;
type FnGetI64 = unsafe extern "C" fn(*mut Stream) -> i64;
type FnGetTimestamp = unsafe extern "C" fn(*mut Stream, c_int, *mut i64, *mut i64) -> c_int;
type FnWaitState = unsafe extern "C" fn(*mut Stream, c_int, *mut c_int, i64) -> c_int;
type FnSetBufferSize = unsafe extern "C" fn(*mut Stream, c_int) -> c_int;

struct Api {
    create_builder: FnCreateBuilder,
    delete_builder: FnDeleteBuilder,
    set_direction: FnBuilderInt,
    set_format: FnBuilderInt,
    set_sample_rate: FnBuilderInt,
    set_channel_count: FnBuilderInt,
    set_sharing_mode: FnBuilderInt,
    set_performance_mode: FnBuilderInt,
    set_usage: FnBuilderInt,
    set_input_preset: FnBuilderInt,
    open: FnOpen,
    request_start: FnStreamNoArg,
    request_stop: FnStreamNoArg,
    close: FnStreamNoArg,
    write: FnWrite,
    read: FnRead,
    get_state: FnGetI32,
    get_sample_rate: FnGetI32,
    get_channel_count: FnGetI32,
    get_format: FnGetI32,
    get_device_id: FnGetI32,
    get_frames_per_burst: FnGetI32,
    get_buffer_size: FnGetI32,
    get_buffer_capacity: FnGetI32,
    set_buffer_size: FnSetBufferSize,
    get_performance_mode: FnGetI32,
    get_sharing_mode: FnGetI32,
    get_xrun_count: FnGetI32,
    get_frames_written: FnGetI64,
    get_timestamp: FnGetTimestamp,
    wait_for_state_change: FnWaitState,
}

// `Api` 只是一组 AAudio 的函数指针（外加一个 dlopen 句柄，句柄是进程级的）。
// AAudio 的 builder/stream API 本身是线程安全的，所以把它共享给多个客户端线程
// 不需要再包一层锁——但 Rust 看不到这一点，裸指针默认既不 Send 也不 Sync。
unsafe impl Send for Api {}
unsafe impl Sync for Api {}

/// 同时服务的客户端上限。每个客户端一个线程，超出的直接关掉（客户端看到 EOF），
/// 免得一个坏掉的调用方把线程数拉爆。
const MAX_AUDIO_CLIENTS: usize = 12;
static AUDIO_CLIENTS: AtomicUsize = AtomicUsize::new(0);

fn client_slot() -> bool {
    let n = AUDIO_CLIENTS.fetch_add(1, Ordering::AcqRel);
    if n >= MAX_AUDIO_CLIENTS {
        AUDIO_CLIENTS.fetch_sub(1, Ordering::AcqRel);
        return false;
    }
    true
}

/// 每个客户端一个线程：`handle_client` 是一条长连接的会话（DATA frames=N 会一直
/// 等下一块 PCM），以前它是**在 accept 循环里**同步跑的。于是一个连上来就不说话的
/// 客户端会把整条监听堵住 30 秒（读超时），而两个监听口共用同一个循环——第二个
/// 客户端（另一条 socket、或者 PulseAudio 自己重连的那一个）只能干等。相机那侧
/// (`handle_capture`) 更是没有读超时，一个 idle 客户端能把它永久堵死。
fn spawn_client<F>(api: Arc<Api>, stream: UnixStream, f: F)
where
    F: FnOnce(&Api, UnixStream) + Send + 'static,
{
    if !client_slot() {
        crate::log("audio|client|BUSY|refused");
        drop(stream);
        return;
    }
    std::thread::spawn(move || {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(&api, stream)));
        AUDIO_CLIENTS.fetch_sub(1, Ordering::AcqRel);
    });
}

fn symbol<T: Copy>(handle: *mut c_void, name: &CStr) -> Result<T, String> {
    let ptr = unsafe { dlsym(handle, name.as_ptr()) };
    if ptr.is_null() {
        return Err(format!("missing {}", name.to_string_lossy()));
    }
    if core::mem::size_of::<T>() != core::mem::size_of::<*mut c_void>() {
        return Err(format!("bad symbol size for {}", name.to_string_lossy()));
    }
    Ok(unsafe { core::mem::transmute_copy::<*mut c_void, T>(&ptr) })
}

impl Api {
    unsafe fn load() -> Result<(*mut c_void, Api), String> {
        let handle = dlopen(c"libaaudio.so".as_ptr(), RTLD_NOW);
        if handle.is_null() {
            return Err("dlopen(libaaudio.so) failed".into());
        }
        let api = Api {
            create_builder: symbol(handle, c"AAudio_createStreamBuilder")?,
            delete_builder: symbol(handle, c"AAudioStreamBuilder_delete")?,
            set_direction: symbol(handle, c"AAudioStreamBuilder_setDirection")?,
            set_format: symbol(handle, c"AAudioStreamBuilder_setFormat")?,
            set_sample_rate: symbol(handle, c"AAudioStreamBuilder_setSampleRate")?,
            set_channel_count: symbol(handle, c"AAudioStreamBuilder_setChannelCount")?,
            set_sharing_mode: symbol(handle, c"AAudioStreamBuilder_setSharingMode")?,
            set_performance_mode: symbol(handle, c"AAudioStreamBuilder_setPerformanceMode")?,
            set_usage: symbol(handle, c"AAudioStreamBuilder_setUsage")?,
            set_input_preset: symbol(handle, c"AAudioStreamBuilder_setInputPreset")?,
            open: symbol(handle, c"AAudioStreamBuilder_openStream")?,
            request_start: symbol(handle, c"AAudioStream_requestStart")?,
            request_stop: symbol(handle, c"AAudioStream_requestStop")?,
            close: symbol(handle, c"AAudioStream_close")?,
            write: symbol(handle, c"AAudioStream_write")?,
            read: symbol(handle, c"AAudioStream_read")?,
            get_state: symbol(handle, c"AAudioStream_getState")?,
            get_sample_rate: symbol(handle, c"AAudioStream_getSampleRate")?,
            get_channel_count: symbol(handle, c"AAudioStream_getChannelCount")?,
            get_format: symbol(handle, c"AAudioStream_getFormat")?,
            get_device_id: symbol(handle, c"AAudioStream_getDeviceId")?,
            get_frames_per_burst: symbol(handle, c"AAudioStream_getFramesPerBurst")?,
            get_buffer_size: symbol(handle, c"AAudioStream_getBufferSizeInFrames")?,
            get_buffer_capacity: symbol(handle, c"AAudioStream_getBufferCapacityInFrames")?,
            set_buffer_size: symbol(handle, c"AAudioStream_setBufferSizeInFrames")?,
            get_performance_mode: symbol(handle, c"AAudioStream_getPerformanceMode")?,
            get_sharing_mode: symbol(handle, c"AAudioStream_getSharingMode")?,
            get_xrun_count: symbol(handle, c"AAudioStream_getXRunCount")?,
            get_frames_written: symbol(handle, c"AAudioStream_getFramesWritten")?,
            get_timestamp: symbol(handle, c"AAudioStream_getTimestamp")?,
            wait_for_state_change: symbol(handle, c"AAudioStream_waitForStateChange")?,
        };
        Ok((handle, api))
    }

    unsafe fn wait_started(&self, stream: *mut Stream) -> bool {
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut state = (self.get_state)(stream);
        while Instant::now() < deadline {
            if state == AAUDIO_STREAM_STATE_STARTED {
                return true;
            }
            if state == AAUDIO_STREAM_STATE_DISCONNECTED {
                return false;
            }
            let mut next = state;
            let _ = (self.wait_for_state_change)(stream, state, &mut next, 100_000_000);
            state = (self.get_state)(stream);
        }
        state == AAUDIO_STREAM_STATE_STARTED
    }
}

fn files_dir() -> Option<std::path::PathBuf> {
    let act = crate::ACTIVITY.load(Ordering::SeqCst);
    if act.is_null() {
        return None;
    }
    let path = unsafe { (*act).internalDataPath };
    if path.is_null() {
        return None;
    }
    Some(std::path::PathBuf::from(
        unsafe { CStr::from_ptr(path) }
            .to_string_lossy()
            .into_owned(),
    ))
}

fn read_line(stream: &mut UnixStream) -> std::io::Result<String> {
    let mut bytes = Vec::with_capacity(128);
    let mut one = [0u8; 1];
    loop {
        match stream.read(&mut one)? {
            0 => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "peer closed",
                ))
            }
            _ if one[0] == b'\n' => break,
            _ if one[0] != b'\r' => {
                if bytes.len() >= 1024 {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "line too long",
                    ));
                }
                bytes.push(one[0]);
            }
            _ => {}
        }
    }
    String::from_utf8(bytes)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "non-utf8 request"))
}

fn send_line(stream: &mut UnixStream, line: &str) -> std::io::Result<()> {
    stream.write_all(line.as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()
}

fn parse_kv(line: &str) -> Vec<(&str, &str)> {
    line.split_whitespace()
        .filter_map(|field| field.split_once('='))
        .collect()
}

fn kv<'a>(fields: &'a [(&str, &str)], key: &str) -> Option<&'a str> {
    fields.iter().find_map(|(k, v)| (*k == key).then_some(*v))
}

fn parse_usize(fields: &[(&str, &str)], key: &str, default: usize) -> Result<usize, String> {
    match kv(fields, key) {
        Some(value) => value
            .parse::<usize>()
            .map_err(|_| format!("bad {key}={value}")),
        None => Ok(default),
    }
}

fn parse_i32(fields: &[(&str, &str)], key: &str, default: i32) -> Result<i32, String> {
    match kv(fields, key) {
        Some(value) => value
            .parse::<i32>()
            .map_err(|_| format!("bad {key}={value}")),
        None => Ok(default),
    }
}

struct OpenedStream {
    native: *mut Stream,
    rate: i32,
    channels: i32,
    format: i32,
    device: i32,
    burst: i32,
    capacity: i32,
    buffer: i32,
    perf: i32,
    sharing: i32,
}

impl OpenedStream {
    unsafe fn open(api: &Api, rate: i32, channels: i32) -> Result<Self, String> {
        if rate <= 0 || rate > 384_000 || channels <= 0 || channels > 8 {
            return Err(format!(
                "bad stream format: rate={rate} channels={channels}"
            ));
        }

        let mut builder: *mut Builder = core::ptr::null_mut();
        let rc = (api.create_builder)(&mut builder);
        if rc != AAUDIO_OK || builder.is_null() {
            return Err(format!("AAudio_createStreamBuilder rc={rc}"));
        }
        (api.set_direction)(builder, AAUDIO_DIRECTION_OUTPUT);
        (api.set_format)(builder, AAUDIO_FORMAT_PCM_I16);
        (api.set_sample_rate)(builder, rate);
        (api.set_channel_count)(builder, channels);
        (api.set_sharing_mode)(builder, AAUDIO_SHARING_MODE_SHARED);
        (api.set_performance_mode)(builder, AAUDIO_PERFORMANCE_MODE_LOW_LATENCY);
        (api.set_usage)(builder, AAUDIO_USAGE_MEDIA);

        let mut native: *mut Stream = core::ptr::null_mut();
        let open_rc = (api.open)(builder, &mut native);
        (api.delete_builder)(builder);
        if open_rc != AAUDIO_OK || native.is_null() {
            return Err(format!("AAudioStreamBuilder_openStream rc={open_rc}"));
        }

        let burst = (api.get_frames_per_burst)(native);
        let requested_buffer = (api.set_buffer_size)(native, burst);
        let actual_rate = (api.get_sample_rate)(native);
        let actual_channels = (api.get_channel_count)(native);
        let actual_format = (api.get_format)(native);
        let device_id = (api.get_device_id)(native);
        let capacity = (api.get_buffer_capacity)(native);
        let buffer_size = (api.get_buffer_size)(native);
        let perf = (api.get_performance_mode)(native);
        let sharing = (api.get_sharing_mode)(native);
        let xruns_at_open = (api.get_xrun_count)(native);
        crate::log(&format!(
            "audio|stream|open|rc={open_rc}|rate={actual_rate}|channels={actual_channels}|format={actual_format}|device={device_id}|burst={burst}|capacity={capacity}|buffer={buffer_size}|perf={perf}|sharing={sharing}|xruns={xruns_at_open}|set_buffer={requested_buffer}"
        ));

        let start_rc = (api.request_start)(native);
        if start_rc != AAUDIO_OK || !api.wait_started(native) {
            let state = (api.get_state)(native);
            let _ = (api.request_stop)(native);
            let _ = (api.close)(native);
            return Err(format!("AAudio start failed: rc={start_rc} state={state}"));
        }

        Ok(Self {
            native,
            rate: actual_rate,
            channels: actual_channels,
            format: actual_format,
            device: device_id,
            burst,
            capacity,
            buffer: buffer_size,
            perf,
            sharing,
        })
    }

    fn ready_line(&self) -> String {
        format!(
            "OPEN_READY rate={} channels={} format={} device={} burst={} capacity={} buffer={} perf={} sharing={}",
            self.rate, self.channels, self.format, self.device, self.burst,
            self.capacity, self.buffer, self.perf, self.sharing
        )
    }

    unsafe fn close(&self, api: &Api) {
        let _ = (api.request_stop)(self.native);
        let _ = (api.close)(self.native);
    }
}

struct ContinuousStream {
    info: OpenedStream,
    input_bytes: u64,
    written_frames: i64,
    latency_sum: f64,
    latency_count: u64,
    latency_max: f64,
    started: Instant,
}

impl ContinuousStream {
    fn new(info: OpenedStream) -> Self {
        Self {
            info,
            input_bytes: 0,
            written_frames: 0,
            latency_sum: 0.0,
            latency_count: 0,
            latency_max: 0.0,
            started: Instant::now(),
        }
    }

    unsafe fn write_pcm(&mut self, api: &Api, pcm: &[u8]) -> Result<(), String> {
        let frame_bytes = (self.info.channels as usize).saturating_mul(2);
        if frame_bytes == 0 || pcm.len() % frame_bytes != 0 {
            return Err(format!("PCM length {} is not frame-aligned", pcm.len()));
        }
        let mut frames_left = pcm.len() / frame_bytes;
        let mut offset = 0usize;
        while frames_left > 0 {
            let wrote = (api.write)(
                self.info.native,
                pcm[offset..].as_ptr() as *const c_void,
                frames_left as i32,
                WRITE_TIMEOUT_NS,
            );
            if wrote < 0 {
                return Err(format!(
                    "AAudioStream_write rc={wrote} state={}",
                    (api.get_state)(self.info.native)
                ));
            }
            if wrote == 0 {
                return Err(format!(
                    "AAudioStream_write returned 0 at frame {}",
                    self.written_frames
                ));
            }
            frames_left -= wrote as usize;
            offset += wrote as usize * frame_bytes;
            self.written_frames += wrote as i64;
            self.input_bytes += (wrote as usize * frame_bytes) as u64;

            let mut frame_pos = 0i64;
            let mut time_ns = 0i64;
            let ts_rc = (api.get_timestamp)(
                self.info.native,
                CLOCK_MONOTONIC,
                &mut frame_pos,
                &mut time_ns,
            );
            if ts_rc == AAUDIO_OK {
                let frames_written = (api.get_frames_written)(self.info.native);
                let queued = (frames_written - frame_pos).max(0) as f64;
                let latency_ms = queued * 1000.0 / self.info.rate.max(1) as f64;
                self.latency_sum += latency_ms;
                self.latency_count += 1;
                self.latency_max = self.latency_max.max(latency_ms);
            }
        }
        Ok(())
    }

    unsafe fn summary(&self, api: &Api) -> String {
        let xruns = (api.get_xrun_count)(self.info.native);
        let fw = (api.get_frames_written)(self.info.native);
        let latency_avg = if self.latency_count > 0 {
            self.latency_sum / self.latency_count as f64
        } else {
            -1.0
        };
        format!(
            "input_bytes={} frames={} frames_written={} rate={} channels={} device={} burst={} capacity={} buffer={} perf={} sharing={} latency_ms_avg={:.3} latency_ms_max={:.3} xruns={} elapsed_ms={}",
            self.input_bytes,
            self.written_frames,
            fw,
            self.info.rate,
            self.info.channels,
            self.info.device,
            self.info.burst,
            self.info.capacity,
            self.info.buffer,
            self.info.perf,
            self.info.sharing,
            latency_avg,
            self.latency_max,
            xruns,
            self.started.elapsed().as_millis()
        )
    }
}

unsafe fn handle_stream(api: &Api, stream: &mut UnixStream, line: &str) -> Result<(), String> {
    let fields = parse_kv(line);
    let requested_frames = parse_usize(&fields, "frames", 48_000)?;
    let rate = parse_i32(&fields, "rate", 48_000)?;
    let channels = parse_i32(&fields, "channels", 1)?;
    if requested_frames == 0 || requested_frames > MAX_FRAMES_PER_STREAM {
        return Err(format!("frames out of range: {requested_frames}"));
    }
    if rate <= 0 || rate > 384_000 || channels <= 0 || channels > 8 {
        return Err(format!(
            "bad stream format: rate={rate} channels={channels}"
        ));
    }

    let mut builder: *mut Builder = core::ptr::null_mut();
    let rc = (api.create_builder)(&mut builder);
    if rc != AAUDIO_OK || builder.is_null() {
        return Err(format!("AAudio_createStreamBuilder rc={rc}"));
    }
    (api.set_direction)(builder, AAUDIO_DIRECTION_OUTPUT);
    (api.set_format)(builder, AAUDIO_FORMAT_PCM_I16);
    (api.set_sample_rate)(builder, rate);
    (api.set_channel_count)(builder, channels);
    (api.set_sharing_mode)(builder, AAUDIO_SHARING_MODE_SHARED);
    (api.set_performance_mode)(builder, AAUDIO_PERFORMANCE_MODE_LOW_LATENCY);
    (api.set_usage)(builder, AAUDIO_USAGE_MEDIA);

    let mut native: *mut Stream = core::ptr::null_mut();
    let open_rc = (api.open)(builder, &mut native);
    (api.delete_builder)(builder);
    if open_rc != AAUDIO_OK || native.is_null() {
        return Err(format!("AAudioStreamBuilder_openStream rc={open_rc}"));
    }

    let actual_rate = (api.get_sample_rate)(native);
    let actual_channels = (api.get_channel_count)(native);
    let actual_format = (api.get_format)(native);
    let device_id = (api.get_device_id)(native);
    let burst = (api.get_frames_per_burst)(native);
    let capacity = (api.get_buffer_capacity)(native);
    let buffer_size = (api.get_buffer_size)(native);
    let perf = (api.get_performance_mode)(native);
    let sharing = (api.get_sharing_mode)(native);
    let xruns_at_open = (api.get_xrun_count)(native);
    crate::log(&format!(
        "audio|stream|open|rc={open_rc}|rate={actual_rate}|channels={actual_channels}|format={actual_format}|device={device_id}|burst={burst}|capacity={capacity}|buffer={buffer_size}|perf={perf}|sharing={sharing}|xruns={xruns_at_open}"
    ));

    let start_rc = (api.request_start)(native);
    if start_rc != AAUDIO_OK || !api.wait_started(native) {
        let state = (api.get_state)(native);
        let _ = (api.request_stop)(native);
        let _ = (api.close)(native);
        return Err(format!("AAudio start failed: rc={start_rc} state={state}"));
    }
    send_line(
        stream,
        &format!(
            "STREAM_READY rate={actual_rate} channels={actual_channels} format={actual_format} device={device_id} burst={burst} capacity={capacity} buffer={buffer_size} perf={perf} sharing={sharing}"
        ),
    )
    .map_err(|e| format!("send READY: {e}"))?;

    let frame_bytes = (actual_channels as usize).saturating_mul(2);
    if frame_bytes == 0 {
        let _ = (api.request_stop)(native);
        let _ = (api.close)(native);
        return Err("AAudio returned zero channels".into());
    }
    let total_bytes = requested_frames
        .checked_mul(frame_bytes)
        .ok_or_else(|| "PCM byte count overflow".to_string())?;
    let mut buf = vec![0u8; 4096.min(total_bytes.max(1))];
    let started = Instant::now();
    let mut input_bytes = 0usize;
    let mut written_frames = 0i64;
    let mut latency_sum = 0.0f64;
    let mut latency_count = 0usize;
    let mut latency_max = 0.0f64;
    let mut next_log_frames = actual_rate as usize;

    while input_bytes < total_bytes {
        let want = (total_bytes - input_bytes).min(buf.len());
        stream
            .read_exact(&mut buf[..want])
            .map_err(|e| format!("read PCM at byte {input_bytes}: {e}"))?;
        input_bytes += want;
        let mut frames_left = want / frame_bytes;
        let mut offset = 0usize;
        while frames_left > 0 {
            let ptr = buf[offset..].as_ptr() as *const c_void;
            let wrote = (api.write)(native, ptr, frames_left as i32, WRITE_TIMEOUT_NS);
            if wrote < 0 {
                let state = (api.get_state)(native);
                let _ = (api.request_stop)(native);
                let _ = (api.close)(native);
                return Err(format!(
                    "AAudioStream_write rc={wrote} state={state} input_bytes={input_bytes}"
                ));
            }
            if wrote == 0 {
                return Err(format!(
                    "AAudioStream_write returned 0 at frame {written_frames}"
                ));
            }
            frames_left -= wrote as usize;
            offset += wrote as usize * frame_bytes;
            written_frames += wrote as i64;

            let mut frame_pos = 0i64;
            let mut time_ns = 0i64;
            let ts_rc = (api.get_timestamp)(native, CLOCK_MONOTONIC, &mut frame_pos, &mut time_ns);
            let frames_written = (api.get_frames_written)(native);
            if ts_rc == AAUDIO_OK {
                let queued = (frames_written - frame_pos).max(0) as f64;
                let latency_ms = queued * 1000.0 / actual_rate.max(1) as f64;
                latency_sum += latency_ms;
                latency_count += 1;
                latency_max = latency_max.max(latency_ms);
            }
        }
        if written_frames as usize >= next_log_frames {
            crate::log(&format!(
                "audio|stream|progress|frames={written_frames}/{requested_frames}|input_bytes={input_bytes}/{total_bytes}"
            ));
            next_log_frames += actual_rate as usize;
        }
    }

    let playback_ms = (requested_frames as u64 * 1000 / actual_rate.max(1) as u64) + 250;
    std::thread::sleep(Duration::from_millis(playback_ms.min(3000)));
    let frames_written_final = (api.get_frames_written)(native);
    let xruns_final = (api.get_xrun_count)(native);
    let state_before_stop = (api.get_state)(native);
    let _ = (api.request_stop)(native);
    let _ = (api.close)(native);
    let elapsed_ms = started.elapsed().as_millis() as u64;
    let latency_avg = if latency_count > 0 {
        latency_sum / latency_count as f64
    } else {
        -1.0
    };
    let latency_max = if latency_count > 0 { latency_max } else { -1.0 };
    let pass = written_frames as usize == requested_frames
        && input_bytes == total_bytes
        && state_before_stop == AAUDIO_STREAM_STATE_STARTED;
    crate::log(&format!(
        "audio|stream|done|pass={pass}|input_bytes={input_bytes}|frames={written_frames}/{requested_frames}|frames_written={frames_written_final}|xruns={xruns_final}|latency_avg_ms={latency_avg:.3}|latency_max_ms={latency_max:.3}|elapsed_ms={elapsed_ms}|state={state_before_stop}"
    ));
    send_line(
        stream,
        &format!(
            "DONE status={} input_bytes={} frames={} requested_frames={} frames_written={} rate={} channels={} device={} burst={} capacity={} buffer={} perf={} sharing={} latency_ms_avg={:.3} latency_ms_max={:.3} xruns={} elapsed_ms={}",
            if pass { "PASS" } else { "FAIL" },
            input_bytes,
            written_frames,
            requested_frames,
            frames_written_final,
            actual_rate,
            actual_channels,
            device_id,
            burst,
            capacity,
            buffer_size,
            perf,
            sharing,
            latency_avg,
            latency_max,
            xruns_final,
            elapsed_ms
        ),
    )
    .map_err(|e| format!("send DONE: {e}"))?;
    Ok(())
}

fn handle_client(api: &Api, mut stream: UnixStream) {
    crate::log(&format!(
        "audio|client|connected|fd={}",
        std::os::fd::AsRawFd::as_raw_fd(&stream)
    ));
    let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
    let mut active: Option<ContinuousStream> = None;
    loop {
        let line = match read_line(&mut stream) {
            Ok(line) => line,
            Err(e) => {
                crate::log(&format!("audio|client|read|{e}"));
                break;
            }
        };
        let parts: Vec<&str> = line.split_whitespace().collect();
        let cmd = parts.first().copied().unwrap_or("").to_ascii_uppercase();
        let reply = match cmd.as_str() {
            "PING" => send_line(&mut stream, "PONG bridge=audio-v1"),
            "DEVICES" => match crate::surface::audio_devices() {
                Ok(devices) => {
                    let mut rc = send_line(
                        &mut stream,
                        &format!("DEVICES_BEGIN count={}", devices.len()),
                    );
                    for (i, dev) in devices.iter().enumerate() {
                        if rc.is_ok() {
                            rc = send_line(
                                &mut stream,
                                &format!(
                                    "DEVICE index={} id={} type={} type_name={} sink={} name=\"{}\" rates={} channels={}",
                                    i,
                                    dev.id,
                                    dev.ty,
                                    crate::surface::audio_device_type_name(dev.ty),
                                    dev.is_sink,
                                    dev.name,
                                    dev.sample_rates,
                                    dev.channel_counts
                                ),
                            );
                        }
                    }
                    if rc.is_ok() {
                        rc = send_line(&mut stream, "DEVICES_END");
                    }
                    rc
                }
                Err(e) => send_line(&mut stream, &format!("DEVICES_ERROR {e}")),
            },
            "VOLUME" => match parts
                .get(1)
                .copied()
                .unwrap_or("")
                .to_ascii_uppercase()
                .as_str()
            {
                "GET" => match crate::surface::audio_music_volume() {
                    Ok((index, max)) => {
                        send_line(&mut stream, &format!("VOLUME OK index={index} max={max}"))
                    }
                    Err(e) => send_line(&mut stream, &format!("VOLUME_ERROR {e}")),
                },
                "SET" => match parts.get(2).and_then(|v| v.parse::<u8>().ok()) {
                    Some(percent) if percent <= 100 => {
                        match crate::surface::audio_set_music_volume_percent(percent) {
                            Ok((index, max)) => send_line(
                                &mut stream,
                                &format!("VOLUME OK index={index} max={max} percent={percent}"),
                            ),
                            Err(e) => send_line(&mut stream, &format!("VOLUME_ERROR {e}")),
                        }
                    }
                    _ => send_line(&mut stream, "VOLUME_ERROR percent must be 0..100"),
                },
                "INDEX" => match parts.get(2).and_then(|v| v.parse::<i32>().ok()) {
                    Some(index) => match crate::surface::audio_set_music_volume_index(index) {
                        Ok((actual, max)) => {
                            send_line(&mut stream, &format!("VOLUME OK index={actual} max={max}"))
                        }
                        Err(e) => send_line(&mut stream, &format!("VOLUME_ERROR {e}")),
                    },
                    None => send_line(&mut stream, "VOLUME_ERROR index must be integer"),
                },
                _ => send_line(&mut stream, "VOLUME_ERROR expected GET, SET, or INDEX"),
            },
            // Long-lived session protocol. The client sends DATA frames=N, then exactly
            // N * channels * 2 bytes of PCM16LE, and waits for DATA_OK before the next chunk.
            "OPEN" => {
                if active.is_some() {
                    send_line(&mut stream, "OPEN_ERROR stream already open")
                } else {
                    let fields = parse_kv(&line);
                    let rate = match parse_i32(&fields, "rate", 48_000) {
                        Ok(v) => v,
                        Err(e) => {
                            let _ = send_line(&mut stream, &format!("OPEN_ERROR {e}"));
                            continue;
                        }
                    };
                    let channels = match parse_i32(&fields, "channels", 2) {
                        Ok(v) => v,
                        Err(e) => {
                            let _ = send_line(&mut stream, &format!("OPEN_ERROR {e}"));
                            continue;
                        }
                    };
                    match unsafe { OpenedStream::open(api, rate, channels) } {
                        Ok(info) => {
                            let ready = info.ready_line();
                            active = Some(ContinuousStream::new(info));
                            send_line(&mut stream, &ready)
                        }
                        Err(e) => {
                            crate::log(&format!("audio|session|open|FAIL|{e}"));
                            send_line(&mut stream, &format!("OPEN_ERROR {e}"))
                        }
                    }
                }
            }
            "DATA" => {
                let fields = parse_kv(&line);
                let frames = match parse_usize(&fields, "frames", 0) {
                    Ok(0) | Err(_) => {
                        let _ = send_line(&mut stream, "DATA_ERROR frames must be 1..1048576");
                        continue;
                    }
                    Ok(v) if v > MAX_DATA_FRAMES => {
                        let _ = send_line(&mut stream, "DATA_ERROR frames must be 1..1048576");
                        continue;
                    }
                    Ok(v) => v,
                };
                let Some(current) = active.as_mut() else {
                    let _ = send_line(&mut stream, "DATA_ERROR no stream; send OPEN first");
                    continue;
                };
                let bytes = match frames
                    .checked_mul(current.info.channels as usize)
                    .and_then(|n| n.checked_mul(2))
                {
                    Some(v) => v,
                    None => {
                        let _ = send_line(&mut stream, "DATA_ERROR PCM byte count overflow");
                        continue;
                    }
                };
                let mut pcm = vec![0u8; bytes];
                if let Err(e) = stream.read_exact(&mut pcm) {
                    crate::log(&format!("audio|session|read-pcm|{e}"));
                    break;
                }
                match unsafe { current.write_pcm(api, &pcm) } {
                    Ok(()) => {
                        let xruns = unsafe { (api.get_xrun_count)(current.info.native) };
                        send_line(
                            &mut stream,
                            &format!(
                                "DATA_OK frames={} input_bytes={} frames_written={} xruns={}",
                                frames, current.input_bytes, current.written_frames, xruns
                            ),
                        )
                    }
                    Err(e) => {
                        crate::log(&format!("audio|session|write|FAIL|{e}"));
                        send_line(&mut stream, &format!("DATA_ERROR {e}"))
                    }
                }
            }
            "CLOSE" => {
                if let Some(current) = active.take() {
                    let summary = unsafe { current.summary(api) };
                    crate::log(&format!("audio|session|close|{summary}"));
                    unsafe { current.info.close(api) };
                    send_line(&mut stream, &format!("CLOSE_OK {summary}"))
                } else {
                    send_line(&mut stream, "CLOSE_OK input_bytes=0 frames=0")
                }
            }
            "STREAM" => match unsafe { handle_stream(api, &mut stream, &line) } {
                Ok(()) => Ok(()),
                Err(e) => {
                    crate::log(&format!("audio|stream|FAIL|{e}"));
                    send_line(&mut stream, &format!("STREAM_ERROR {e}"))
                }
            },
            "QUIT" => {
                if let Some(current) = active.take() {
                    let summary = unsafe { current.summary(api) };
                    crate::log(&format!("audio|session|close|{summary}"));
                    unsafe { current.info.close(api) };
                }
                let _ = send_line(&mut stream, "BYE");
                break;
            }
            _ => send_line(&mut stream, "ERROR unknown-command"),
        };
        if let Err(e) = reply {
            crate::log(&format!("audio|client|write|{e}"));
            break;
        }
    }
    if let Some(current) = active.take() {
        let summary = unsafe { current.summary(api) };
        crate::log(&format!("audio|session|disconnect|{summary}"));
        unsafe { current.info.close(api) };
    }
    crate::log("audio|client|disconnected");
}
fn serve() {
    let Some(files) = files_dir() else {
        crate::log("audio|bridge|SKIP|no files dir");
        return;
    };
    let Some((_handle, api)) = (unsafe { Api::load().ok() }) else {
        crate::log("audio|bridge|FAIL|AAudio symbols unavailable");
        return;
    };
    let api = Arc::new(api);
    crate::log("audio|bridge|API|libaaudio.so");

    let marker = files.join("audio-bridge");
    let guest_marker = files.join("rootfs-full/tmp/husk-pulse/enabled");
    let enabled = || marker.exists() || guest_marker.exists();
    let mut announced_wait = false;
    loop {
        if !enabled() {
            if !announced_wait {
                crate::log(&format!(
                    "audio|bridge|WAIT|{} or {}",
                    marker.display(),
                    guest_marker.display()
                ));
                announced_wait = true;
            }
            std::thread::sleep(Duration::from_millis(500));
            continue;
        }
        announced_wait = false;
        // Two listeners, same protocol. `files/audio-bridge.sock` keeps the old
        // host-side scripts working; `rootfs-full/run/husk/audio-out.sock` is the
        // one a guest binary can actually open, because the path shim resolves
        // absolute paths inside the guest root (a socket outside the rootfs is
        // invisible to `audio-relay`). Same rule the capture socket follows.
        let paths = [
            files.join("audio-bridge.sock"),
            files.join("rootfs-full/run/husk/audio-out.sock"),
        ];
        let mut listeners = Vec::new();
        for path in &paths {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::remove_file(path);
            match UnixListener::bind(path) {
                Ok(listener) => {
                    let _ = listener.set_nonblocking(true);
                    crate::log(&format!("audio|bridge|LISTEN|{}", path.display()));
                    listeners.push(listener);
                }
                Err(e) => crate::log(&format!("audio|bridge|FAIL|bind {}: {e}", path.display())),
            }
        }
        if listeners.is_empty() {
            std::thread::sleep(Duration::from_secs(1));
            continue;
        }
        while enabled() {
            let mut any = false;
            for listener in &listeners {
                match listener.accept() {
                    Ok((stream, _)) => {
                        any = true;
                        spawn_client(Arc::clone(&api), stream, handle_client);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(e) => crate::log(&format!("audio|bridge|FAIL|accept {e}")),
                }
            }
            if !any {
                std::thread::sleep(Duration::from_millis(25));
            }
        }
        for path in &paths {
            let _ = std::fs::remove_file(path);
        }
        crate::log("audio|bridge|STOP|marker removed");
    }
}

/// Start the single audio bridge thread for this app process.
pub fn start_once() {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| {
        let _ = std::panic::catch_unwind(serve);
    });
    std::thread::spawn(|| {
        let _ = std::panic::catch_unwind(serve_capture);
    });
}

// ---------------------------------------------------------------------------
// Microphone capture (`untrusted_app` cannot open /dev/snd, but AAudio input
// reaches AudioFlinger and the vendor capture path).
//
// Direction is the only real difference from the output bridge: the guest asks
// for frames (`READ frames=N`) and the app answers with exactly N frames of
// interleaved PCM16, because the guest-side `module-pipe-source` has to be fed
// by a process we control. A separate marker and a separate socket keep this
// channel from ever being confused with playback.
// ---------------------------------------------------------------------------

const CAPTURE_SOCKET: &str = "rootfs-full/run/husk/audio-in.sock";
const CAPTURE_MARKER: &str = "audio-in";
const CAPTURE_GUEST_MARKER: &str = "rootfs-full/tmp/husk-pulse/enabled-in";
const CAPTURE_READ_TIMEOUT_NS: i64 = 1_000_000_000;

struct CaptureStream {
    native: *mut Stream,
    rate: i32,
    channels: i32,
    format: i32,
    device: i32,
    burst: i32,
    capacity: i32,
    frames_read: u64,
    silent_chunks: u64,
    peak: i32,
}

impl CaptureStream {
    unsafe fn open_input(api: &Api, rate: i32, channels: i32) -> Result<Self, String> {
        if rate <= 0 || rate > 384_000 || channels <= 0 || channels > 8 {
            return Err(format!(
                "bad capture format: rate={rate} channels={channels}"
            ));
        }
        let mut builder: *mut Builder = core::ptr::null_mut();
        let rc = (api.create_builder)(&mut builder);
        if rc != AAUDIO_OK || builder.is_null() {
            return Err(format!("AAudio_createStreamBuilder rc={rc}"));
        }
        (api.set_direction)(builder, AAUDIO_DIRECTION_INPUT);
        (api.set_format)(builder, AAUDIO_FORMAT_PCM_I16);
        (api.set_sample_rate)(builder, rate);
        (api.set_channel_count)(builder, channels);
        (api.set_sharing_mode)(builder, AAUDIO_SHARING_MODE_SHARED);
        (api.set_performance_mode)(builder, AAUDIO_PERFORMANCE_MODE_LOW_LATENCY);
        // GENERIC is the AAudio spelling of `MediaRecorder.AudioSource.MIC`, which
        // is what the prior art uses (DroidNative's MicBridge). UNPROCESSED was
        // tried first on this device and delivered all-zero samples while the
        // open/report path looked healthy, so it is now the fallback only.
        let mut preset = AAUDIO_INPUT_PRESET_GENERIC;
        (api.set_input_preset)(builder, preset);

        let mut native: *mut Stream = core::ptr::null_mut();
        let mut open_rc = (api.open)(builder, &mut native);
        if open_rc != AAUDIO_OK || native.is_null() {
            (api.set_input_preset)(builder, AAUDIO_INPUT_PRESET_UNPROCESSED);
            preset = AAUDIO_INPUT_PRESET_UNPROCESSED;
            open_rc = (api.open)(builder, &mut native);
        }
        (api.delete_builder)(builder);
        if open_rc != AAUDIO_OK || native.is_null() {
            return Err(format!("AAudio input openStream rc={open_rc}"));
        }
        let actual_rate = (api.get_sample_rate)(native);
        let actual_channels = (api.get_channel_count)(native);
        let actual_format = (api.get_format)(native);
        let device_id = (api.get_device_id)(native);
        let burst = (api.get_frames_per_burst)(native);
        let capacity = (api.get_buffer_capacity)(native);
        let start_rc = (api.request_start)(native);
        if start_rc != AAUDIO_OK || !api.wait_started(native) {
            let state = (api.get_state)(native);
            let _ = (api.request_stop)(native);
            let _ = (api.close)(native);
            return Err(format!(
                "AAudio input start failed: rc={start_rc} state={state}"
            ));
        }
        crate::log(&format!(
            "audio|capture|open|preset={preset}|rate={actual_rate}|channels={actual_channels}|format={actual_format}|device={device_id}|burst={burst}|capacity={capacity}"
        ));
        Ok(Self {
            native,
            rate: actual_rate,
            channels: actual_channels,
            format: actual_format,
            device: device_id,
            burst,
            capacity,
            frames_read: 0,
            silent_chunks: 0,
            peak: 0,
        })
    }

    fn ready_line(&self) -> String {
        format!(
            "CAPTURE_READY rate={} channels={} format={} device={} burst={} capacity={}",
            self.rate, self.channels, self.format, self.device, self.burst, self.capacity
        )
    }

    /// Blocking read of exactly `frames` interleaved PCM16 frames.
    unsafe fn read_frames(&mut self, api: &Api, frames: usize) -> Result<Vec<u8>, String> {
        let channels = self.channels.max(1) as usize;
        let mut pcm = vec![0u8; frames * channels * 2];
        let mut done = 0usize;
        while done < frames {
            let want = (frames - done) as i32;
            let offset = done * channels * 2;
            let got = (api.read)(
                self.native,
                pcm[offset..].as_mut_ptr().cast::<c_void>(),
                want,
                CAPTURE_READ_TIMEOUT_NS,
            );
            if got < 0 {
                return Err(format!("AAudioStream_read rc={got}"));
            }
            if got == 0 {
                return Err("AAudioStream_read timed out".into());
            }
            done += got as usize;
        }
        self.frames_read += frames as u64;
        let mut peak = 0i32;
        for sample in pcm.chunks_exact(2) {
            let value = i16::from_le_bytes([sample[0], sample[1]]) as i32;
            peak = peak.max(value.abs());
        }
        self.peak = self.peak.max(peak);
        if peak == 0 {
            self.silent_chunks += 1;
        }
        Ok(pcm)
    }

    fn summary(&self) -> String {
        format!(
            "frames_read={} silent_chunks={} peak={}",
            self.frames_read, self.silent_chunks, self.peak
        )
    }

    unsafe fn close(&self, api: &Api) {
        let _ = (api.request_stop)(self.native);
        let _ = (api.close)(self.native);
    }
}

fn handle_capture(api: &Api, mut stream: UnixStream) {
    let mut active: Option<CaptureStream> = None;
    loop {
        let line = match read_line(&mut stream) {
            Ok(line) => line,
            Err(e) => {
                crate::log(&format!("audio|capture|read|{e}"));
                break;
            }
        };
        let fields = parse_kv(&line);
        let cmd = line
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_ascii_uppercase();
        let reply = match cmd.as_str() {
            "OPEN" => {
                let rate = parse_i32(&fields, "rate", 48_000).unwrap_or(48_000);
                let channels = parse_i32(&fields, "channels", 2).unwrap_or(2);
                if active.is_some() {
                    send_line(&mut stream, "CAPTURE_ERROR already open")
                } else {
                    match unsafe { CaptureStream::open_input(api, rate, channels) } {
                        Ok(opened) => {
                            let ready = opened.ready_line();
                            let sent = send_line(&mut stream, &ready);
                            active = Some(opened);
                            sent
                        }
                        Err(e) => {
                            crate::log(&format!("audio|capture|OPEN_FAIL|{e}"));
                            send_line(&mut stream, &format!("CAPTURE_ERROR {e}"))
                        }
                    }
                }
            }
            "READ" => match active.as_mut() {
                Some(current) => {
                    let frames = parse_usize(&fields, "frames", 1024)
                        .unwrap_or(1024)
                        .clamp(1, 8192);
                    match unsafe { current.read_frames(api, frames) } {
                        Ok(pcm) => send_line(
                            &mut stream,
                            &format!("PCM_OK frames={frames} bytes={}", pcm.len()),
                        )
                        .and_then(|_| stream.write_all(&pcm))
                        .and_then(|_| stream.flush()),
                        Err(e) => send_line(&mut stream, &format!("PCM_ERROR {e}")),
                    }
                }
                None => send_line(&mut stream, "PCM_ERROR no open capture"),
            },
            "CLOSE" | "QUIT" => {
                let summary = match active.take() {
                    Some(current) => {
                        let summary = current.summary();
                        unsafe { current.close(api) };
                        summary
                    }
                    None => "frames_read=0 silent_chunks=0 peak=0".to_string(),
                };
                crate::log(&format!("audio|capture|close|{summary}"));
                let _ = send_line(&mut stream, &format!("CAPTURE_OK {summary}"));
                if cmd == "QUIT" {
                    break;
                }
                Ok(())
            }
            _ => send_line(&mut stream, "ERROR unknown-command"),
        };
        if let Err(e) = reply {
            crate::log(&format!("audio|capture|write|{e}"));
            break;
        }
    }
    if let Some(current) = active.take() {
        let summary = current.summary();
        crate::log(&format!("audio|capture|disconnect|{summary}"));
        unsafe { current.close(api) };
    }
}

fn serve_capture() {
    let Some(files) = files_dir() else {
        crate::log("audio|capture|SKIP|no files dir");
        return;
    };
    let Some((_handle, api)) = (unsafe { Api::load().ok() }) else {
        crate::log("audio|capture|FAIL|AAudio symbols unavailable");
        return;
    };
    let api = Arc::new(api);
    let marker = files.join(CAPTURE_MARKER);
    let guest_marker = files.join(CAPTURE_GUEST_MARKER);
    let enabled = || marker.exists() || guest_marker.exists();
    let mut announced_wait = false;
    loop {
        if !enabled() {
            if !announced_wait {
                crate::log(&format!(
                    "audio|capture|WAIT|{} or {}",
                    marker.display(),
                    guest_marker.display()
                ));
                announced_wait = true;
            }
            std::thread::sleep(Duration::from_millis(500));
            continue;
        }
        announced_wait = false;
        let path = files.join(CAPTURE_SOCKET);
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                crate::log(&format!("audio|capture|FAIL|mkdir {e}"));
                std::thread::sleep(Duration::from_secs(1));
                continue;
            }
        }
        let _ = std::fs::remove_file(&path);
        let listener = match UnixListener::bind(&path) {
            Ok(listener) => listener,
            Err(e) => {
                crate::log(&format!("audio|capture|FAIL|bind {e}"));
                std::thread::sleep(Duration::from_secs(1));
                continue;
            }
        };
        let _ = listener.set_nonblocking(true);
        crate::log(&format!("audio|capture|LISTEN|{}", path.display()));
        while enabled() {
            match listener.accept() {
                Ok((stream, _)) => spawn_client(Arc::clone(&api), stream, handle_capture),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(e) => {
                    crate::log(&format!("audio|capture|FAIL|accept {e}"));
                    break;
                }
            }
        }
        let _ = std::fs::remove_file(&path);
        crate::log("audio|capture|STOP|marker removed");
    }
}
