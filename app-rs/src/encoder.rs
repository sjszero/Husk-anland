//! Hardware video encoder bridge (app half) — MediaCodec, pure Rust, no Java.
//!
//! Prior art (`docs/design/prior-art-video.md`): lfdevs/termux-va does encoding
//! over the *same* kind of daemon socket it uses for decoding, with an NV12
//! uplink and a compact per-access-unit downlink, and it synthesises CSD from
//! the parsed parameter sets because the VA frontend never hands them over.
//! Camera2/CameraX in DroidNativeApp never encodes at all — the guest OS is
//! expected to.
//!
//! What this is: the guest sends raw NV12 frames, the app runs them through the
//! SoC encoder (`c2.mtk.avc.encoder` on this device) and streams H.264
//! Annex-B access units back. No software encoder anywhere: if MediaCodec
//! cannot open the codec, the session fails loudly.
//!
//! Protocol on `<active-rootfs>/run/husk/encoder.sock` (one client):
//!
//!   INFO                              -> ENC avc=<0|1> hevc=<0|1>\nOK\n
//!   OPEN w=640 h=480 fps=15 kbps=2000 codec=avc
//!                                     -> OK\n              (codec is started)
//!   FRAME <bytes>\n<nv12 bytes>       -> 0..n records of
//!                                        AU <flags> <bytes>\n<annex-b bytes>
//!   STOP                              -> trailing AU records, then BYE\n
//!
//! Every AU record carries the MediaCodec flags (1 = key frame, 2 = CSD), so
//! the guest can write the CSD to the file exactly once and the muxer sees a
//! well-formed stream.

use core::ffi::{c_char, c_int, c_void};
use core::sync::atomic::{AtomicBool, Ordering};
use std::ffi::CString;

const MEDIA_OK: i32 = 0;
const INFO_TRY_AGAIN_LATER: isize = -1;
const INFO_OUTPUT_FORMAT_CHANGED: isize = -2;
const INFO_OUTPUT_BUFFERS_CHANGED: isize = -3;

const FLAG_KEY_FRAME: u32 = 1;
const FLAG_CODEC_CONFIG: u32 = 2;
const FLAG_END_OF_STREAM: u32 = 4;

const COLOR_FormatYUV420SemiPlanar: i32 = 21;

const ENCODER_SOCKET: &str = "run/husk/encoder.sock";
const ENCODER_MARKER: &str = "encoder";
const ENCODER_GUEST_MARKER: &str = "tmp/husk-encoder/enabled";

static STARTED: AtomicBool = AtomicBool::new(false);

extern "C" {
    fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlerror() -> *mut c_char;
}

unsafe fn dlerror_text() -> String {
    let message = dlerror();
    if message.is_null() {
        "unknown dlerror".to_string()
    } else {
        core::ffi::CStr::from_ptr(message)
            .to_string_lossy()
            .into_owned()
    }
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct BufferInfo {
    offset: i32,
    size: i32,
    pts_us: i64,
    flags: u32,
}

struct Codec {
    handle: *mut c_void,
    start: unsafe extern "C" fn(*mut c_void) -> i32,
    stop: unsafe extern "C" fn(*mut c_void) -> i32,
    delete: unsafe extern "C" fn(*mut c_void),
    deq_in: unsafe extern "C" fn(*mut c_void, i64) -> isize,
    get_in: unsafe extern "C" fn(*mut c_void, usize, *mut usize) -> *mut u8,
    queue_in: unsafe extern "C" fn(*mut c_void, usize, usize, usize, u64, u32) -> i32,
    deq_out: unsafe extern "C" fn(*mut c_void, *mut BufferInfo, i64) -> isize,
    get_out: unsafe extern "C" fn(*mut c_void, usize, *mut usize) -> *mut u8,
    release_out: unsafe extern "C" fn(*mut c_void, usize, bool) -> i32,
    get_out_format: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
    fmt_get_buf:
        unsafe extern "C" fn(*mut c_void, *const c_char, *mut *mut c_void, *mut usize) -> bool,
    fmt_delete: unsafe extern "C" fn(*mut c_void),
}

type CreateByType = unsafe extern "C" fn(*const c_char) -> *mut c_void;
type Configure =
    unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void, u32) -> i32;
type FmtNew = unsafe extern "C" fn() -> *mut c_void;
type FmtSetStr = unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> i32;
type FmtSetI32 = unsafe extern "C" fn(*mut c_void, *const c_char, i32) -> i32;

struct Api {
    _handle: *mut c_void,
    create: CreateByType,
    configure: Configure,
    fmt_new: FmtNew,
    fmt_set_str: FmtSetStr,
    fmt_set_i32: FmtSetI32,
}

impl Api {
    unsafe fn load() -> Result<Self, String> {
        let handle = dlopen(c"libmediandk.so".as_ptr(), 2);
        if handle.is_null() {
            return Err(format!("dlopen(libmediandk.so): {}", dlerror_text()));
        }
        Ok(Self {
            _handle: handle,
            create: core::mem::transmute(sym(handle, c"AMediaCodec_createEncoderByType")?),
            configure: core::mem::transmute(sym(handle, c"AMediaCodec_configure")?),
            fmt_new: core::mem::transmute(sym(handle, c"AMediaFormat_new")?),
            fmt_set_str: core::mem::transmute(sym(handle, c"AMediaFormat_setString")?),
            fmt_set_i32: core::mem::transmute(sym(handle, c"AMediaFormat_setInt32")?),
        })
    }

    unsafe fn codec(&self, mime: &CString) -> Option<Codec> {
        let h = (self.create)(mime.as_ptr());
        if h.is_null() {
            return None;
        }
        // Symbols come from the *library* handle; AMediaCodec_* are not per
        // instance symbols (dlsym on the codec returns null — measured).
        let lib = self._handle;
        Some(Codec {
            handle: h,
            start: core::mem::transmute(sym(lib, c"AMediaCodec_start").ok()?),
            stop: core::mem::transmute(sym(lib, c"AMediaCodec_stop").ok()?),
            delete: core::mem::transmute(sym(lib, c"AMediaCodec_delete").ok()?),
            deq_in: core::mem::transmute(sym(lib, c"AMediaCodec_dequeueInputBuffer").ok()?),
            get_in: core::mem::transmute(sym(lib, c"AMediaCodec_getInputBuffer").ok()?),
            queue_in: core::mem::transmute(sym(lib, c"AMediaCodec_queueInputBuffer").ok()?),
            deq_out: core::mem::transmute(sym(lib, c"AMediaCodec_dequeueOutputBuffer").ok()?),
            get_out: core::mem::transmute(sym(lib, c"AMediaCodec_getOutputBuffer").ok()?),
            release_out: core::mem::transmute(sym(lib, c"AMediaCodec_releaseOutputBuffer").ok()?),
            get_out_format: core::mem::transmute(sym(lib, c"AMediaCodec_getOutputFormat").ok()?),
            fmt_get_buf: core::mem::transmute(sym(lib, c"AMediaFormat_getBuffer").ok()?),
            fmt_delete: core::mem::transmute(sym(lib, c"AMediaFormat_delete").ok()?),
        })
    }
}

unsafe fn sym(handle: *mut c_void, name: &core::ffi::CStr) -> Result<*mut c_void, String> {
    let p = dlsym(handle, name.as_ptr());
    if p.is_null() {
        Err(format!(
            "dlsym({:?}): {}",
            name.to_string_lossy(),
            dlerror_text()
        ))
    } else {
        Ok(p)
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
        unsafe { core::ffi::CStr::from_ptr(path) }
            .to_string_lossy()
            .into_owned(),
    ))
}

pub fn start_once() {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| {
        let _ = std::panic::catch_unwind(run);
    });
}

fn run() {
    let Some(files) = files_dir() else {
        crate::log("encoder|SKIP|no files dir");
        return;
    };
    let rootfs = match crate::guest_rootfs::resolve(&files) {
        Ok(rootfs) => rootfs,
        Err(e) => {
            crate::log(&format!("encoder|SKIP|{e}"));
            return;
        }
    };
    let marker = files.join(ENCODER_MARKER);
    let guest_marker = rootfs.join(ENCODER_GUEST_MARKER);
    let enabled = || marker.exists() || guest_marker.exists();
    loop {
        if !enabled() {
            std::thread::sleep(std::time::Duration::from_millis(500));
            continue;
        }
        let path = rootfs.join(ENCODER_SOCKET);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::remove_file(&path);
        let listener = match std::os::unix::net::UnixListener::bind(&path) {
            Ok(l) => l,
            Err(e) => {
                crate::log(&format!("encoder|FAIL|bind {e}"));
                std::thread::sleep(std::time::Duration::from_secs(1));
                continue;
            }
        };
        crate::log(&format!("encoder|LISTEN|{}", path.display()));
        for stream in listener.incoming() {
            match stream {
                Ok(s) => {
                    if let Err(e) = unsafe { session(s) } {
                        crate::log(&format!("encoder|session|{e}"));
                    }
                }
                Err(e) => {
                    crate::log(&format!("encoder|FAIL|accept {e}"));
                    break;
                }
            }
            if !enabled() {
                break;
            }
        }
        let _ = std::fs::remove_file(&path);
        crate::log("encoder|STOP|marker removed");
    }
}

struct Params {
    w: i32,
    h: i32,
    fps: i32,
    kbps: i32,
    hevc: bool,
}

fn parse_kv(line: &str, key: &str) -> Option<i32> {
    line.split_whitespace()
        .find_map(|f| f.strip_prefix(key)?.parse().ok())
}

unsafe fn probe(api: &Api) -> String {
    let avc = api
        .codec(&CString::new("video/avc").unwrap())
        .map(|c| {
            (c.delete)(c.handle);
            true
        })
        .unwrap_or(false);
    let hevc = api
        .codec(&CString::new("video/hevc").unwrap())
        .map(|c| {
            (c.delete)(c.handle);
            true
        })
        .unwrap_or(false);
    format!("ENC avc={} hevc={}\nOK\n", avc as u8, hevc as u8)
}

unsafe fn session(stream: std::os::unix::net::UnixStream) -> Result<(), String> {
    use std::io::{Read, Write};
    let mut stream = stream;
    let api = Api::load()?;
    let mut codec: Option<Codec> = None;
    let mut params: Option<Params> = None;
    let mut buf = Vec::with_capacity(64);

    loop {
        // One command line.
        buf.clear();
        let mut one = [0u8; 1];
        loop {
            match stream.read(&mut one) {
                Ok(0) => return Ok(()),
                Ok(_) => {
                    if one[0] == b'\n' {
                        break;
                    }
                    if one[0] != b'\r' {
                        buf.push(one[0]);
                        if buf.len() > 512 {
                            return Err("command too long".into());
                        }
                    }
                }
                Err(e) => return Err(format!("read: {e}")),
            }
        }
        let line = String::from_utf8_lossy(&buf).into_owned();
        let verb = line.split_whitespace().next().unwrap_or("");
        crate::log(&format!("encoder|cmd|{line}"));
        match verb {
            "INFO" => {
                let _ = stream.write_all(probe(&api).as_bytes());
            }
            "OPEN" => {
                let p = Params {
                    w: parse_kv(&line, "w=").unwrap_or(640).max(16),
                    h: parse_kv(&line, "h=").unwrap_or(480).max(16),
                    fps: parse_kv(&line, "fps=").unwrap_or(15).max(1),
                    kbps: parse_kv(&line, "kbps=").unwrap_or(2000).max(64),
                    hevc: line.split_whitespace().any(|w| w == "codec=hevc"),
                };
                let mime = if p.hevc { "video/hevc" } else { "video/avc" };
                let name = CString::new(mime).map_err(|e| e.to_string())?;
                let Some(c) = api.codec(&name) else {
                    let _ = stream.write_all(b"ERR no_codec\n");
                    continue;
                };
                let fmt = (api.fmt_new)();
                (api.fmt_set_str)(fmt, c"mime".as_ptr(), name.as_ptr());
                (api.fmt_set_i32)(fmt, c"width".as_ptr(), p.w);
                (api.fmt_set_i32)(fmt, c"height".as_ptr(), p.h);
                (api.fmt_set_i32)(fmt, c"color-format".as_ptr(), COLOR_FormatYUV420SemiPlanar);
                (api.fmt_set_i32)(fmt, c"bitrate".as_ptr(), p.kbps * 1000);
                (api.fmt_set_i32)(fmt, c"frame-rate".as_ptr(), p.fps);
                (api.fmt_set_i32)(fmt, c"i-frame-interval".as_ptr(), 1);
                let rc = (api.configure)(
                    c.handle,
                    fmt,
                    core::ptr::null_mut(),
                    core::ptr::null_mut(),
                    1,
                );
                (c.fmt_delete)(fmt);
                if rc != MEDIA_OK {
                    crate::log(&format!("encoder|configure|{rc}"));
                    (c.delete)(c.handle);
                    let _ = stream.write_all(b"ERR configure\n");
                    continue;
                }
                if (c.start)(c.handle) != MEDIA_OK {
                    (c.delete)(c.handle);
                    let _ = stream.write_all(b"ERR start\n");
                    continue;
                }
                crate::log(&format!(
                    "encoder|OPEN|{}x{}|{}fps|{}kbps|{}|color=21",
                    p.w, p.h, p.fps, p.kbps, mime
                ));
                let _ = stream.write_all(b"OK\n");
                codec = Some(c);
                params = Some(p);
            }
            "FRAME" => {
                let size: usize = line
                    .split_whitespace()
                    .nth(1)
                    .and_then(|v| v.parse().ok())
                    .ok_or("FRAME without size")?;
                let mut frame = vec![0u8; size];
                stream
                    .read_exact(&mut frame)
                    .map_err(|e| format!("frame body: {e}"))?;
                let (Some(c), Some(p)) = (codec.as_ref(), params.as_ref()) else {
                    let _ = stream.write_all(b"ERR not_open\n");
                    continue;
                };
                queue_frame(c, p, &frame, &mut stream)?;
            }
            "STOP" => {
                if let (Some(c), Some(p)) = (codec.as_ref(), params.as_ref()) {
                    finish(c, p, &mut stream)?;
                }
                let _ = stream.write_all(b"BYE\n");
                if let Some(c) = codec.take() {
                    (c.stop)(c.handle);
                    (c.delete)(c.handle);
                }
                params = None;
            }
            "BYE" => {
                if let Some(c) = codec.take() {
                    (c.stop)(c.handle);
                    (c.delete)(c.handle);
                }
                return Ok(());
            }
            "" => {}
            other => {
                let _ = stream.write_all(format!("ERR unknown {other}\n").as_bytes());
            }
        }
    }
}

/// Queue one NV12 frame, writing every access unit the encoder releases.
unsafe fn queue_frame(
    c: &Codec,
    p: &Params,
    frame: &[u8],
    out: &mut std::os::unix::net::UnixStream,
) -> Result<(), String> {
    let need = (p.w * p.h * 3 / 2) as usize;
    if frame.len() < need {
        return Err(format!("frame {} < {}", frame.len(), need));
    }
    // Wait for an input buffer, draining output in the meantime: the encoder
    // only frees an input slot once it has produced output.
    let mut tries = 0;
    loop {
        let idx = (c.deq_in)(c.handle, 1000);
        if idx >= 0 {
            let mut cap = 0usize;
            let dst = (c.get_in)(c.handle, idx as usize, &mut cap);
            if dst.is_null() {
                return Err("input buffer null".into());
            }
            let n = need.min(cap);
            core::ptr::copy_nonoverlapping(frame.as_ptr(), dst, n);
            let rc = (c.queue_in)(c.handle, idx as usize, 0, n, 0, 0);
            if rc != MEDIA_OK {
                return Err(format!("queueInputBuffer {rc}"));
            }
            pump(c, out, 0)?;
            return Ok(());
        }
        pump(c, out, 0)?;
        tries += 1;
        if tries > 2000 {
            return Err("no input buffer after 2 s".into());
        }
    }
}

/// Drain whatever the encoder has ready. `wait_us` is the dequeue timeout.
unsafe fn pump(
    c: &Codec,
    out: &mut std::os::unix::net::UnixStream,
    wait_us: i64,
) -> Result<bool, String> {
    use std::io::Write;
    let mut wrote = false;
    loop {
        let mut info = BufferInfo::default();
        let idx = (c.deq_out)(c.handle, &mut info, wait_us);
        if idx == INFO_OUTPUT_FORMAT_CHANGED {
            let fmt = (c.get_out_format)(c.handle);
            if !fmt.is_null() {
                for key in [c"csd-0", c"csd-1"] {
                    let mut data: *mut c_void = core::ptr::null_mut();
                    let mut len = 0usize;
                    if (c.fmt_get_buf)(fmt, key.as_ptr(), &mut data, &mut len)
                        && !data.is_null()
                        && len > 0
                    {
                        let bytes = core::slice::from_raw_parts(data as *const u8, len);
                        write_au(out, FLAG_CODEC_CONFIG, bytes)?;
                        wrote = true;
                    }
                }
                (c.fmt_delete)(fmt);
            }
            continue;
        }
        if idx == INFO_OUTPUT_BUFFERS_CHANGED || idx == INFO_TRY_AGAIN_LATER {
            return Ok(wrote);
        }
        if idx < 0 {
            return Ok(wrote);
        }
        let mut cap = 0usize;
        let src = (c.get_out)(c.handle, idx as usize, &mut cap);
        if src.is_null() {
            (c.release_out)(c.handle, idx as usize, false);
            return Err("output buffer null".into());
        }
        let off = info.offset as usize;
        let size = info.size as usize;
        if size > 0 && off + size <= cap {
            let bytes = core::slice::from_raw_parts(src.add(off), size);
            write_au(out, info.flags, bytes)?;
            wrote = true;
        }
        let eos = info.flags & FLAG_END_OF_STREAM != 0;
        (c.release_out)(c.handle, idx as usize, false);
        if eos {
            return Ok(wrote);
        }
        // Keep draining without waiting.
    }
}

unsafe fn write_au(
    out: &mut std::os::unix::net::UnixStream,
    flags: u32,
    bytes: &[u8],
) -> Result<(), String> {
    use std::io::Write;
    let header = format!("AU {} {}\n", flags, bytes.len());
    out.write_all(header.as_bytes())
        .and_then(|_| out.write_all(bytes))
        .map_err(|e| format!("au write: {e}"))
}

/// End of stream: mark EOS on an empty input buffer and drain to the EOS flag.
unsafe fn finish(
    c: &Codec,
    _p: &Params,
    out: &mut std::os::unix::net::UnixStream,
) -> Result<(), String> {
    for _ in 0..500 {
        let idx = (c.deq_in)(c.handle, 10_000);
        if idx >= 0 {
            (c.queue_in)(c.handle, idx as usize, 0, 0, 0, FLAG_END_OF_STREAM);
            break;
        }
        pump(c, out, 5_000)?;
    }
    for _ in 0..500 {
        let wrote = pump(c, out, 20_000)?;
        if !wrote {
            // Nothing came back in 20 ms: assume the encoder is drained.
            break;
        }
    }
    Ok(())
}
