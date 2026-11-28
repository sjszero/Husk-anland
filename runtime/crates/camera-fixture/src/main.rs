//! Deterministic camera.sock fixture for guest runtime tests.
//!
//! This is deliberately not an Android camera implementation. It speaks the
//! same socket protocol as `app-rs/src/camera.rs` and serves patterned NV12 so
//! the loader, libv4l2 interposition, mmap, QBUF/DQBUF and GStreamer paths can
//! be tested without granting sensor permission.

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::io::{ErrorKind, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

struct Fixture {
    frames: u32,
    width: u32,
    height: u32,
    seq: u32,
}

impl Fixture {
    fn frame(&mut self) -> Vec<u8> {
        let w = self.width as usize;
        let h = self.height as usize;
        let y_bytes = w * h;
        let mut out = vec![0u8; y_bytes * 3 / 2];
        let phase = self.seq.wrapping_mul(11) as u8;
        for y in 0..h {
            for x in 0..w {
                out[y * w + x] = (x as u8)
                    .wrapping_add((y as u8).wrapping_mul(3))
                    .wrapping_add(phase);
            }
        }
        let u = 64u8.wrapping_add(self.seq as u8);
        let v = 192u8.wrapping_sub(self.seq as u8);
        for row in 0..h / 2 {
            let base = y_bytes + row * w;
            for col in (0..w).step_by(2) {
                out[base + col] = u;
                out[base + col + 1] = v;
            }
        }
        self.seq = self.seq.wrapping_add(1);
        out
    }
}

fn read_command(stream: &mut UnixStream) -> Result<String, String> {
    let mut out = Vec::with_capacity(64);
    let mut one = [0u8; 1];
    loop {
        stream.read_exact(&mut one).map_err(|e| e.to_string())?;
        match one[0] {
            b'\n' => return Ok(String::from_utf8_lossy(&out).into_owned()),
            b'\r' => {}
            b => {
                if out.len() >= 512 {
                    return Err("command too long".into());
                }
                out.push(b);
            }
        }
    }
}

fn field<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    line.split_whitespace()
        .filter_map(|part| part.split_once('='))
        .find_map(|(k, v)| (k == key).then_some(v))
}

fn serve_client(mut stream: UnixStream, fixture: &mut Fixture) -> Result<u32, String> {
    let mut sent = 0u32;
    let mut handshake_done = false;
    loop {
        let line = match read_command(&mut stream) {
            Ok(line) => line,
            // GStreamer probing may leave INFO-only connections open after it
            // has enough caps. Treat EOF/timeout after a completed handshake as
            // a normal short-lived probe, not as a fixture failure.
            Err(e) if handshake_done => {
                println!("FIXTURE|client_done|frames={sent}|reason={e}");
                break;
            }
            Err(e) => return Err(e),
        };
        let verb = line.split_whitespace().next().unwrap_or("");
        match verb {
            "INFO" => {
                handshake_done = true;
                let w = fixture.width;
                let h = fixture.height;
                // The real camera bridge returns every camera index in one
                // INFO reply. A single `k=0` reply makes GStreamer's second
                // /dev/videoN probe wait forever for `k=1`.
                let reply = format!(
                    "CAM k=0 id=fixture-0 facing=back name=FIXTURE-BACK sizes={w}x{h},640x480,1280x720,1920x1080\nCAM k=1 id=fixture-1 facing=front name=FIXTURE-FRONT sizes={w}x{h},640x480,1280x720,1920x1080\nOK\n"
                );
                stream
                    .write_all(reply.as_bytes())
                    .map_err(|e| e.to_string())?;
            }
            "OPEN" => {
                handshake_done = true;
                fixture.width = field(&line, "w")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(fixture.width);
                fixture.height = field(&line, "h")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(fixture.height);
                if fixture.width == 0
                    || fixture.height == 0
                    || fixture.width > 1920
                    || fixture.height > 1080
                    || fixture.width % 2 != 0
                    || fixture.height % 2 != 0
                {
                    stream
                        .write_all(b"ERR unsupported_size\n")
                        .map_err(|e| e.to_string())?;
                    continue;
                }
                fixture.seq = 0;
                let bytes = fixture.width * fixture.height * 3 / 2;
                let reply = format!(
                    "OK w={} h={} fourcc=NV12 bytes={bytes} id=fixture\n",
                    fixture.width, fixture.height,
                );
                stream
                    .write_all(reply.as_bytes())
                    .map_err(|e| e.to_string())?;
            }
            "FRAME" => {
                if sent >= fixture.frames {
                    stream.write_all(b"NOFRAME\n").map_err(|e| e.to_string())?;
                    continue;
                }
                let frame = fixture.frame();
                let header = format!(
                    "FRAME seq={} ts={} w={} h={} bytes={}\n",
                    sent,
                    sent as u64 * 33_333,
                    fixture.width,
                    fixture.height,
                    frame.len()
                );
                stream
                    .write_all(header.as_bytes())
                    .map_err(|e| e.to_string())?;
                stream.write_all(&frame).map_err(|e| e.to_string())?;
                sent += 1;
                println!("FIXTURE|frame|seq={sent}|bytes={}", frame.len());
            }
            "STOP" | "BYE" => {
                let _ = stream.write_all(b"BYE\n");
                break;
            }
            other => {
                let _ = stream.write_all(format!("ERR unknown {other}\n").as_bytes());
            }
        }
    }
    Ok(sent)
}

fn usage() -> ! {
    eprintln!("usage: camera-fixture <socket> [frames-per-client] [width] [height] [clients]");
    eprintln!("       camera-fixture --gst [width] [height] [frames]");
    std::process::exit(64);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--gst") {
        let width = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(640);
        let height = args.get(3).and_then(|v| v.parse().ok()).unwrap_or(480);
        let frames = args.get(4).and_then(|v| v.parse().ok()).unwrap_or(3);
        if let Err(e) = run_gst_audit(width, height, frames) {
            eprintln!("GST_AUDIT|FAIL|{e}");
            std::process::exit(1);
        }
        return;
    }
    if args.len() < 2 || args.len() > 6 {
        usage();
    }
    let socket = PathBuf::from(&args[1]);
    let frames = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(10);
    let width = args.get(3).and_then(|v| v.parse().ok()).unwrap_or(640);
    let height = args.get(4).and_then(|v| v.parse().ok()).unwrap_or(480);
    let clients = args.get(5).and_then(|v| v.parse().ok()).unwrap_or(1usize);
    if let Err(e) = run(&socket, frames, width, height, clients) {
        eprintln!("FIXTURE|FAIL|{e}");
        std::process::exit(1);
    }
}

#[repr(C)]
struct GError {
    domain: u32,
    code: c_int,
    message: *mut c_char,
}

struct GstApi {
    _handle: *mut c_void,
    init: unsafe extern "C" fn(*mut c_int, *mut *mut *mut c_char),
    parse_launch: unsafe extern "C" fn(*const c_char, *mut *mut GError) -> *mut c_void,
    set_state: unsafe extern "C" fn(*mut c_void, c_int) -> c_int,
    get_bus: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
    timed_pop_filtered: unsafe extern "C" fn(*mut c_void, u64, u32) -> *mut c_void,
    pop_filtered: unsafe extern "C" fn(*mut c_void, u32) -> *mut c_void,
    message_parse_error: unsafe extern "C" fn(*mut c_void, *mut *mut GError, *mut *mut c_char),
    error_free: unsafe extern "C" fn(*mut GError),
    free: unsafe extern "C" fn(*mut c_void),
    message_unref: unsafe extern "C" fn(*mut c_void),
    object_unref: unsafe extern "C" fn(*mut c_void),
}

impl GstApi {
    unsafe fn load() -> Result<Self, String> {
        let handle = libc::dlopen(
            c"libgstreamer-1.0.so.0".as_ptr(),
            libc::RTLD_NOW | libc::RTLD_GLOBAL,
        );
        if handle.is_null() {
            return Err(format!("dlopen(libgstreamer): {}", dlerror()));
        }
        macro_rules! sym {
            ($name:literal, $ty:ty) => {{
                let p = libc::dlsym(handle, concat!($name, "\0").as_ptr() as *const c_char);
                if p.is_null() {
                    return Err(format!("dlsym({}): {}", $name, dlerror()));
                }
                core::mem::transmute::<*mut c_void, $ty>(p)
            }};
        }
        Ok(Self {
            _handle: handle,
            init: sym!(
                "gst_init",
                unsafe extern "C" fn(*mut c_int, *mut *mut *mut c_char)
            ),
            parse_launch: sym!(
                "gst_parse_launch",
                unsafe extern "C" fn(*const c_char, *mut *mut GError) -> *mut c_void
            ),
            set_state: sym!(
                "gst_element_set_state",
                unsafe extern "C" fn(*mut c_void, c_int) -> c_int
            ),
            get_bus: sym!(
                "gst_element_get_bus",
                unsafe extern "C" fn(*mut c_void) -> *mut c_void
            ),
            timed_pop_filtered: sym!(
                "gst_bus_timed_pop_filtered",
                unsafe extern "C" fn(*mut c_void, u64, u32) -> *mut c_void
            ),
            pop_filtered: sym!(
                "gst_bus_pop_filtered",
                unsafe extern "C" fn(*mut c_void, u32) -> *mut c_void
            ),
            message_parse_error: sym!(
                "gst_message_parse_error",
                unsafe extern "C" fn(*mut c_void, *mut *mut GError, *mut *mut c_char)
            ),
            error_free: sym!("g_error_free", unsafe extern "C" fn(*mut GError)),
            free: sym!("g_free", unsafe extern "C" fn(*mut c_void)),
            message_unref: sym!("gst_message_unref", unsafe extern "C" fn(*mut c_void)),
            object_unref: sym!("gst_object_unref", unsafe extern "C" fn(*mut c_void)),
        })
    }
}

unsafe fn dlerror() -> String {
    let p = libc::dlerror();
    if p.is_null() {
        "unknown loader error".into()
    } else {
        CStr::from_ptr(p).to_string_lossy().into_owned()
    }
}

fn gst_pipeline(width: u32, height: u32, frames: u32) -> Result<(), String> {
    const EOS: u32 = 1 << 0;
    const ERROR: u32 = 1 << 1;
    const PLAYING: c_int = 4;
    const TIMEOUT_NS: u64 = 20_000_000_000;

    unsafe {
        let api = GstApi::load()?;
        (api.init)(core::ptr::null_mut(), core::ptr::null_mut());
        let desc = format!(
            "v4l2src device=/dev/video0 num-buffers={frames} ! video/x-raw,format=NV12,width={width},height={height} ! fakesink sync=false"
        );
        let desc = CString::new(desc).map_err(|e| e.to_string())?;
        let mut err: *mut GError = core::ptr::null_mut();
        println!("GST_AUDIT|pipeline|{width}x{height}|frames={frames}");
        let pipeline = (api.parse_launch)(desc.as_ptr(), &mut err);
        if pipeline.is_null() {
            let message = if err.is_null() {
                "gst_parse_launch returned NULL".to_string()
            } else {
                CStr::from_ptr((*err).message)
                    .to_string_lossy()
                    .into_owned()
            };
            if !err.is_null() {
                (api.error_free)(err);
            }
            return Err(format!("parse_launch: {message}"));
        }
        let rc = (api.set_state)(pipeline, PLAYING);
        if rc == 0 {
            (api.object_unref)(pipeline);
            return Err("set_state(PLAYING) failed".into());
        }
        let bus = (api.get_bus)(pipeline);
        if bus.is_null() {
            (api.object_unref)(pipeline);
            return Err("pipeline has no bus".into());
        }
        let message = (api.timed_pop_filtered)(bus, TIMEOUT_NS, EOS);
        if !message.is_null() {
            (api.message_unref)(message);
            (api.object_unref)(bus);
            (api.set_state)(pipeline, 1); // GST_STATE_NULL
            (api.object_unref)(pipeline);
            println!("GST_AUDIT|EOS|{width}x{height}");
            return Ok(());
        }
        let message = (api.pop_filtered)(bus, ERROR);
        (api.object_unref)(bus);
        (api.set_state)(pipeline, 1); // GST_STATE_NULL
        (api.object_unref)(pipeline);
        if message.is_null() {
            return Err("timed out waiting for EOS; no queued ERROR".into());
        }
        let mut gerr: *mut GError = core::ptr::null_mut();
        let mut debug: *mut c_char = core::ptr::null_mut();
        (api.message_parse_error)(message, &mut gerr, &mut debug);
        let text = if gerr.is_null() {
            "GStreamer ERROR message had no error payload".to_string()
        } else {
            CStr::from_ptr((*gerr).message)
                .to_string_lossy()
                .into_owned()
        };
        if !gerr.is_null() {
            (api.error_free)(gerr);
        }
        if !debug.is_null() {
            (api.free)(debug as *mut c_void);
        }
        (api.message_unref)(message);
        Err(text)
    }
}

fn run_gst_audit(width: u32, height: u32, frames: u32) -> Result<(), String> {
    let socket = std::env::var_os("NR_FIXTURE_SOCKET")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/run/husk/camera.sock"));
    if let Some(parent) = socket.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    let _ = std::fs::remove_file(&socket);
    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
    let socket_for_server = socket.clone();
    let stop = Arc::new(AtomicBool::new(false));
    let stop_server = Arc::clone(&stop);
    let server = std::thread::spawn(move || -> Result<u32, String> {
        let listener = UnixListener::bind(&socket_for_server)
            .map_err(|e| format!("bind {}: {e}", socket_for_server.display()))?;
        listener
            .set_nonblocking(true)
            .map_err(|e| format!("nonblocking fixture listener: {e}"))?;
        ready_tx.send(Ok(())).map_err(|e| e.to_string())?;
        let mut handlers = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(15);
        while !stop_server.load(Ordering::Acquire) && Instant::now() < deadline {
            match listener.accept() {
                Ok((stream, _)) => {
                    let handle = std::thread::spawn(move || -> Result<u32, String> {
                        stream
                            .set_read_timeout(Some(Duration::from_secs(5)))
                            .map_err(|e| format!("fixture read timeout: {e}"))?;
                        stream
                            .set_write_timeout(Some(Duration::from_secs(5)))
                            .map_err(|e| format!("fixture write timeout: {e}"))?;
                        let mut fixture = Fixture {
                            frames,
                            width,
                            height,
                            seq: 0,
                        };
                        serve_client(stream, &mut fixture)
                    });
                    handlers.push(handle);
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(e) => return Err(format!("accept: {e}")),
            }
        }
        let _ = std::fs::remove_file(&socket_for_server);
        let mut total = 0u32;
        for handle in handlers {
            total = total.saturating_add(
                handle
                    .join()
                    .map_err(|_| "fixture client thread panicked".to_string())??,
            );
        }
        Ok(total)
    });
    match ready_rx.recv_timeout(Duration::from_secs(2)) {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(e),
        Err(e) => return Err(format!("fixture did not become ready: {e}")),
    }
    let pipeline = gst_pipeline(width, height, frames);
    stop.store(true, Ordering::Release);
    let served = server
        .join()
        .map_err(|_| "fixture thread panicked".to_string())?;
    let _ = std::fs::remove_file(&socket);
    pipeline?;
    let served = served?;
    if served < frames {
        return Err(format!(
            "pipeline ended after {served}/{frames} fixture frames"
        ));
    }
    println!("GST_AUDIT|PASS|{width}x{height}|frames={served}");
    Ok(())
}

fn run(socket: &Path, frames: u32, width: u32, height: u32, clients: usize) -> Result<(), String> {
    if let Some(parent) = socket.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    let _ = std::fs::remove_file(socket);
    let listener =
        UnixListener::bind(socket).map_err(|e| format!("bind {}: {e}", socket.display()))?;
    println!("FIXTURE|READY|{}", socket.display());
    let mut fixture = Fixture {
        frames,
        width,
        height,
        seq: 0,
    };
    for _ in 0..clients {
        let (stream, _) = listener.accept().map_err(|e| format!("accept: {e}"))?;
        let sent = serve_client(stream, &mut fixture)?;
        println!("FIXTURE|client_done|frames={sent}");
    }
    let _ = std::fs::remove_file(socket);
    println!("FIXTURE|PASS|clients={clients}");
    Ok(())
}
