//! Embedded X11 desktop bridge.
//!
//! Husk keeps the X server inside its own Android process. The platform gives
//! us one `ANativeWindow`; Xlorie owns that window through its native renderer
//! and exposes an X11 socket in the guest rootfs' `/tmp`. No Java Activity,
//! SurfaceView, or second application is involved.

use core::ffi::{c_char, c_int, c_void};
use std::ffi::{CStr, CString};
use std::fs::OpenOptions;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};
use std::sync::OnceLock;

static STARTED: AtomicBool = AtomicBool::new(false);
static ACTIVE: AtomicBool = AtomicBool::new(false);
static READY: AtomicBool = AtomicBool::new(false);
static X11_FD: AtomicI32 = AtomicI32::new(-1);
static INPUT_LOG: AtomicU32 = AtomicU32::new(0);

type StartFn = unsafe extern "C" fn(*mut c_void, c_int, *mut *mut c_char) -> c_int;
type ConnectFn = unsafe extern "C" fn() -> c_int;
type SetWindowFn = unsafe extern "C" fn(*mut c_void, c_int, c_int);
type SendWindowFn = unsafe extern "C" fn(c_int, c_int, c_int, *const c_char);
type SendTouchFn = unsafe extern "C" fn(c_int, c_int, c_int, c_int);
type SendMouseFn = unsafe extern "C" fn(f32, f32, c_int, c_int, c_int);
type SendKeyFn = unsafe extern "C" fn(c_int, c_int);

struct Api {
    _handle: *mut c_void,
    start: StartFn,
    connect: ConnectFn,
    set_window: SetWindowFn,
    send_window: SendWindowFn,
    send_touch: SendTouchFn,
    send_mouse: SendMouseFn,
    send_key: SendKeyFn,
}

unsafe impl Send for Api {}
unsafe impl Sync for Api {}

static API: OnceLock<Option<Api>> = OnceLock::new();

extern "C" {
    fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    fn fork() -> c_int;
    fn execve(path: *const c_char, argv: *const *const c_char, envp: *const *const c_char)
        -> c_int;
    fn _exit(status: c_int) -> !;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlerror() -> *mut c_char;
    fn setsid() -> c_int;
    fn dup2(oldfd: c_int, newfd: c_int) -> c_int;
}

const RTLD_NOW: c_int = 2;

fn dlerror_text() -> String {
    unsafe {
        let p = dlerror();
        if p.is_null() {
            "unknown dlopen error".into()
        } else {
            CStr::from_ptr(p).to_string_lossy().into_owned()
        }
    }
}

unsafe fn symbol<T: Copy>(handle: *mut c_void, name: &CStr) -> Result<T, String> {
    let p = dlsym(handle, name.as_ptr());
    if p.is_null() {
        Err(format!("dlsym({name:?}): {}", dlerror_text()))
    } else {
        Ok(core::mem::transmute_copy::<*mut c_void, T>(&p))
    }
}

/// Fork/exec the desktop launcher through the app-private guest loader.
///
/// The loader must be the first executable in the child: this ROM rejects a
/// guest shell that later re-execs the app-private loader. Once the loader has
/// entered the guest bash, the launcher stays inside the path shim and starts
/// D-Bus/GNOME with direct guest execs (`NR_DESKTOP_GUEST=1`). This avoids both
/// failures at once without making the app use a separate Android Activity.
fn spawn_desktop(
    loader: &Path,
    _session: &Path,
    rootfs: &Path,
    _tmp: &Path,
    _xkb: &Path,
    guest_libs: &str,
) -> Result<i32, String> {
    let bash = rootfs.join("usr/bin/bash");
    let argv_s = vec![
        "libld.so".to_string(),
        "--library-path".to_string(),
        guest_libs.to_string(),
        bash.display().to_string(),
        "/usr/local/share/linuxhub/husk-desktop.sh".to_string(),
        "/".to_string(),
    ];
    let env_s = vec![
        format!("NR_ROOTFS={}", rootfs.display()),
        format!("NR_HOST_ROOTFS={}", rootfs.display()),
        format!("NR_LD={}", loader.display()),
        format!("NR_LIBRARY_PATH={guest_libs}"),
        format!("NR_SIGSYS_LOG={}/tmp/nr-sigsys.log", rootfs.display()),
        "NR_DESKTOP_GUEST=1".to_string(),
        "HOME=/root".to_string(),
        "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".to_string(),
        "SHELL=/usr/bin/bash".to_string(),
        "USER=root".to_string(),
        "TERM=xterm-256color".to_string(),
        "GLIBC_TUNABLES=glibc.pthread.rseq=0".to_string(),
        "TMPDIR=/tmp".to_string(),
        "XKB_CONFIG_ROOT=/usr/share/X11/xkb".to_string(),
        "DISPLAY=:0".to_string(),
    ];
    let argv_c: Vec<CString> = argv_s
        .iter()
        .map(|s| CString::new(s.as_str()).map_err(|_| "NUL in desktop argv".to_string()))
        .collect::<Result<_, _>>()?;
    let env_c: Vec<CString> = env_s
        .iter()
        .map(|s| CString::new(s.as_str()).map_err(|_| "NUL in desktop env".to_string()))
        .collect::<Result<_, _>>()?;
    let mut argv_p: Vec<*const c_char> = argv_c.iter().map(|s| s.as_ptr()).collect();
    let mut env_p: Vec<*const c_char> = env_c.iter().map(|s| s.as_ptr()).collect();
    argv_p.push(core::ptr::null());
    env_p.push(core::ptr::null());
    let loader_c =
        CString::new(loader.display().to_string()).map_err(|_| "NUL in loader path".to_string())?;
    // Keep the launcher's diagnostics even when Android starts it from a
    // multithreaded NativeActivity. A plain fork/exec previously lost the
    // shell output and made a 200 ms SIGTERM look like a silent exec failure.
    let log_path = rootfs.join("tmp/husk-desktop.log");
    let _ = std::fs::remove_file(&log_path);
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .ok();
    let log_fd = log.as_ref().map(|f| f.as_raw_fd()).unwrap_or(-1);
    let pid = unsafe { fork() };
    if pid < 0 {
        return Err(format!(
            "fork desktop: errno {}",
            std::io::Error::last_os_error()
        ));
    }
    if pid == 0 {
        unsafe {
            // Android may deliver process-group signals while the NativeActivity
            // is transitioning surfaces. The desktop is a separate login
            // session; detach it before entering the guest loader.
            let _ = setsid();
            if log_fd >= 0 {
                let _ = dup2(log_fd, 1);
                let _ = dup2(log_fd, 2);
            }
            execve(loader_c.as_ptr(), argv_p.as_ptr(), env_p.as_ptr());
            _exit(127);
        }
    }
    Ok(pid)
}

fn api() -> Result<&'static Api, String> {
    API.get_or_init(|| unsafe {
        let handle = dlopen(c"libXlorie.so".as_ptr(), RTLD_NOW);
        if handle.is_null() {
            crate::log(&format!(
                "desktop: dlopen(libXlorie.so): {}",
                dlerror_text()
            ));
            return None;
        }
        let loaded: Result<Api, String> = (|| {
            Ok(Api {
                _handle: handle,
                start: symbol(handle, c"husk_xlorie_start")?,
                connect: symbol(handle, c"husk_xlorie_connect")?,
                set_window: symbol(handle, c"husk_xlorie_set_window")?,
                send_window: symbol(handle, c"husk_xlorie_send_window_size")?,
                send_touch: symbol(handle, c"husk_xlorie_send_touch")?,
                send_mouse: symbol(handle, c"husk_xlorie_send_mouse")?,
                send_key: symbol(handle, c"husk_xlorie_send_key")?,
            })
        })();
        match loaded {
            Ok(v) => Some(v),
            Err(e) => {
                crate::log(&format!("desktop: Xlorie API unavailable: {e}"));
                None
            }
        }
    })
    .as_ref()
    .ok_or_else(|| "embedded Xlorie API unavailable".to_string())
}

fn rootfs_from(activity: *mut crate::sys::ANativeActivity) -> Result<PathBuf, String> {
    if activity.is_null() {
        return Err("missing activity".into());
    }
    let a = unsafe { &*activity };
    if a.internalDataPath.is_null() {
        return Err("missing internalDataPath".into());
    }
    let files = unsafe { CStr::from_ptr(a.internalDataPath) }
        .to_string_lossy()
        .into_owned();
    crate::guest_rootfs::resolve_desktop(Path::new(&files))
}

fn prepare_socket(rootfs: &Path) -> Result<(PathBuf, PathBuf), String> {
    let tmp = rootfs.join("tmp");
    let sockets = tmp.join(".X11-unix");
    std::fs::create_dir_all(&sockets).map_err(|e| format!("mkdir {}: {e}", sockets.display()))?;
    let _ = std::fs::remove_file(sockets.join("X0"));
    let xkb = rootfs.join("usr/share/X11/xkb");
    if !xkb.is_dir() {
        return Err(format!("XKB data missing: {}", xkb.display()));
    }
    Ok((tmp, xkb))
}

fn guest_library_path(rootfs: &Path) -> String {
    const DIRS: &[&str] = &[
        "lib/aarch64-linux-gnu",
        "usr/lib/aarch64-linux-gnu",
        "usr/lib/aarch64-linux-gnu/mutter-14",
        "usr/lib/gnome-shell",
        "usr/lib/aarch64-linux-gnu/gvfs",
        "usr/lib/aarch64-linux-gnu/NetworkManager/1.46.0",
        "usr/lib/aarch64-linux-gnu/libproxy",
        "usr/lib/aarch64-linux-gnu/libunity",
        "usr/lib/aarch64-linux-gnu/pipewire-0.3",
        "usr/lib/aarch64-linux-gnu/pulseaudio",
        "usr/lib/aarch64-linux-gnu/samba",
        "usr/lib/aarch64-linux-gnu/systemd",
        "usr/lib/aarch64-linux-gnu/tracker-3.0",
        "usr/lib/aarch64-linux-gnu/tracker-miners-3.0",
        "usr/lib/gnome-settings-daemon-46",
        "usr/lib/pulse-16.1+dfsg1/modules",
    ];
    let mut dirs: Vec<String> = DIRS
        .iter()
        .map(|rel| rootfs.join(rel))
        .filter(|p| p.is_dir())
        .map(|p| p.display().to_string())
        .collect();
    if let Ok(extra) = std::fs::read_to_string(rootfs.join("etc/ld.so.husk-path")) {
        for line in extra.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || !line.starts_with('/') {
                continue;
            }
            let host = rootfs.join(line.trim_start_matches('/'));
            if host.is_dir() {
                let s = host.display().to_string();
                if !dirs.contains(&s) {
                    dirs.push(s);
                }
            }
        }
    }
    dirs.join(":")
}

/// Starts the embedded X11 server and the guest desktop session. Must be called
/// on the NativeActivity UI thread so Xlorie can attach its renderer fd to that
/// thread's Android Looper.
pub unsafe fn start(activity: *mut crate::sys::ANativeActivity, window: *mut c_void) {
    if STARTED.load(Ordering::SeqCst) {
        window_resized(window);
        return;
    }
    if STARTED.swap(true, Ordering::SeqCst) {
        window_resized(window);
        return;
    }
    let result = (|| -> Result<(), String> {
        if activity.is_null() || window.is_null() {
            return Err("missing activity/window".into());
        }
        let rootfs = rootfs_from(activity)?;
        let (tmp, xkb) = prepare_socket(&rootfs)?;
        // The APK native library directory is the location from which the
        // Android kernel lets untrusted_app exec the loader. A copy next to
        // the rootfs works for one `run-as` launch, but a guest bash cannot
        // re-exec it there (EACCES on this ROM).
        let loader =
            PathBuf::from(crate::native_lib_dir().ok_or("cannot resolve native library dir")?)
                .join("libld.so");
        let session = rootfs.join("usr/local/share/linuxhub/husk-desktop.sh");
        if !loader.is_file() {
            return Err(format!("loader missing: {}", loader.display()));
        }
        if !session.is_file() {
            return Err(format!("desktop launcher missing: {}", session.display()));
        }

        std::env::set_var("TMPDIR", &tmp);
        std::env::set_var("XKB_CONFIG_ROOT", &xkb);
        std::env::set_var("DISPLAY", ":0");

        // The X server runs on bionic while guest clients use the path shim's
        // synthetic SysV IPC. They cannot exchange a real MIT-SHM segment, and
        // Mesa otherwise treats XShmAttach failure as the only presentation
        // path (`Failed to attach to x11 shm`) and produces a black desktop.
        // Disable the extension so clients use the ordinary X11 image protocol
        // until a shared, side-effect-free buffer path replaces it.
        let args = [
            ":0",
            "-ac",
            "-noreset",
            "-nolisten",
            "tcp",
            "-extension",
            "MIT-SHM",
        ];
        let mut cargs: Vec<CString> = args.iter().map(|s| CString::new(*s).unwrap()).collect();
        let mut argv: Vec<*mut c_char> = cargs
            .iter_mut()
            .map(|s| s.as_ptr() as *mut c_char)
            .collect();

        let api = api()?;
        let env = unsafe { (*activity).env };
        if env.is_null() {
            return Err("NativeActivity env is null".into());
        }
        let rc = unsafe { (api.start)(env, argv.len() as c_int, argv.as_mut_ptr()) };
        if rc != 0 {
            return Err(format!("husk_xlorie_start rc={rc}"));
        }

        let fd = unsafe { (api.connect)() };
        if fd < 0 {
            return Err(format!("husk_xlorie_connect rc={fd}"));
        }
        X11_FD.store(fd, Ordering::SeqCst);

        let (ww, wh) = unsafe {
            (
                crate::sys::ANativeWindow_getWidth(window),
                crate::sys::ANativeWindow_getHeight(window),
            )
        };
        unsafe { (api.set_window)(window, ww, wh) };

        let name = CString::new("Husk X11").unwrap();
        unsafe { (api.send_window)(ww, wh, 60, name.as_ptr()) };

        let _ = std::fs::remove_file(rootfs.join("tmp/husk-shell.log"));
        let status = rootfs.join("tmp/husk-desktop.status");
        let _ = std::fs::remove_file(&status);
        READY.store(false, Ordering::SeqCst);
        let guest_libs = guest_library_path(&rootfs);
        let child_pid = spawn_desktop(&loader, &session, &rootfs, &tmp, &xkb, &guest_libs)?;
        crate::log(&format!(
            "desktop: X11 ready fd={fd} window={ww}x{wh} mit_shm=disabled rootfs={} launcher_pid={}",
            rootfs.display(),
            child_pid
        ));
        ACTIVE.store(true, Ordering::SeqCst);
        monitor_readiness(status, child_pid);
        Ok(())
    })();
    if let Err(e) = result {
        STARTED.store(false, Ordering::SeqCst);
        ACTIVE.store(false, Ordering::SeqCst);
        crate::log(&format!("desktop: start failed: {e}"));
    }
}

pub fn active() -> bool {
    ACTIVE.load(Ordering::SeqCst)
}

pub fn ready() -> bool {
    READY.load(Ordering::SeqCst)
}

fn monitor_readiness(status: PathBuf, launcher_pid: i32) {
    std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        loop {
            if let Ok(text) = std::fs::read_to_string(&status) {
                if text.starts_with("ready\n") {
                    READY.store(true, Ordering::SeqCst);
                    crate::log(&format!(
                        "desktop: ready launcher_pid={launcher_pid} status={}",
                        text.trim().replace('\n', ";")
                    ));
                    return;
                }
                if text.starts_with("failed\n") {
                    ACTIVE.store(false, Ordering::SeqCst);
                    crate::log(&format!(
                        "desktop: launcher failed pid={launcher_pid} status={}",
                        text.trim().replace('\n', ";")
                    ));
                    return;
                }
            }
            if std::time::Instant::now() >= deadline {
                crate::log(&format!(
                    "desktop: readiness timeout pid={launcher_pid} status={}",
                    status.display()
                ));
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
    });
}

pub fn window_resized(window: *mut c_void) {
    if !active() || window.is_null() {
        return;
    }
    let Ok(api) = api() else { return };
    let (w, h) = unsafe {
        (
            crate::sys::ANativeWindow_getWidth(window),
            crate::sys::ANativeWindow_getHeight(window),
        )
    };
    unsafe { (api.set_window)(window, w, h) };
    let name = CString::new("Husk X11").unwrap();
    unsafe { (api.send_window)(w, h, 60, name.as_ptr()) };
}

pub fn key(linux_keycode: i32, down: bool) {
    if !active() {
        return;
    }
    if let Ok(api) = api() {
        unsafe { (api.send_key)(linux_keycode, down as c_int) };
        if INPUT_LOG.fetch_add(1, Ordering::Relaxed) < 40 {
            crate::log(&format!("desktop|input|key|code={linux_keycode}|down={}", down as u8));
        }
    }
}

pub fn touch(action: i32, id: i32, x: i32, y: i32) {
    if !active() {
        return;
    }
    if let Ok(api) = api() {
        unsafe { (api.send_touch)(action, id, x, y) };
        if INPUT_LOG.fetch_add(1, Ordering::Relaxed) < 40 {
            crate::log(&format!(
                "desktop|input|touch|action={action}|id={id}|x={x}|y={y}"
            ));
        }
    }
}

pub fn mouse(x: f32, y: f32, button: i32, down: bool, relative: bool) {
    if !active() {
        return;
    }
    if let Ok(api) = api() {
        unsafe { (api.send_mouse)(x, y, button, down as c_int, relative as c_int) };
        if INPUT_LOG.fetch_add(1, Ordering::Relaxed) < 40 {
            crate::log(&format!(
                "desktop|input|mouse|x={x:.1}|y={y:.1}|button={button}|down={}|relative={}",
                down as u8, relative as u8
            ));
        }
    }
}

pub fn x11_fd() -> i32 {
    X11_FD.load(Ordering::SeqCst)
}
