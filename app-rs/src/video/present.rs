//! Streaming presenter for MediaCodec `AHardwareBuffer` outputs.
//!
//! This is the first display milestone after hardware decode: every decoded
//! AHB is submitted directly to a persistent SurfaceControl child layer.  The
//! presenter owns one reference per queued buffer and releases the previous one
//! only after SurfaceFlinger reports its release fence.  No pixels are copied.

use core::ffi::{c_char, c_int, c_void};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const RTLD_NOW: c_int = 2;
const POLLIN: i16 = 1;

type Ahb = c_void;
type SurfaceControl = *mut c_void;
type SurfaceTransaction = *mut c_void;

type FnAcquire = unsafe extern "C" fn(*mut Ahb);
type FnRelease = unsafe extern "C" fn(*mut Ahb);
type FnCreateChild = unsafe extern "C" fn(SurfaceControl, *const c_char) -> SurfaceControl;
type FnReleaseControl = unsafe extern "C" fn(SurfaceControl);
type FnCreateTransaction = unsafe extern "C" fn() -> SurfaceTransaction;
type FnDeleteTransaction = unsafe extern "C" fn(SurfaceTransaction);
type FnSetBuffer = unsafe extern "C" fn(SurfaceTransaction, SurfaceControl, *mut Ahb, c_int);
type FnSetGeometry =
    unsafe extern "C" fn(SurfaceTransaction, SurfaceControl, *const ARect, *const ARect, c_int);
type FnSetZOrder = unsafe extern "C" fn(SurfaceTransaction, SurfaceControl, c_int);
type FnApply = unsafe extern "C" fn(SurfaceTransaction);
type FnSetOnComplete = unsafe extern "C" fn(
    SurfaceTransaction,
    *mut c_void,
    unsafe extern "C" fn(*mut c_void, *mut c_void),
);
type FnGetPreviousReleaseFenceFd = unsafe extern "C" fn(*mut c_void, SurfaceControl) -> c_int;

#[repr(C)]
struct ARect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[repr(C)]
struct PollFd {
    fd: c_int,
    events: i16,
    revents: i16,
}

unsafe extern "C" {
    fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlclose(handle: *mut c_void) -> c_int;
    fn poll(fds: *mut PollFd, nfds: usize, timeout: c_int) -> c_int;
    fn close(fd: c_int) -> c_int;
}

struct Symbols {
    acquire: FnAcquire,
    release: FnRelease,
    create_child: FnCreateChild,
    release_control: FnReleaseControl,
    create_transaction: FnCreateTransaction,
    delete_transaction: FnDeleteTransaction,
    set_buffer: FnSetBuffer,
    set_geometry: FnSetGeometry,
    set_z_order: FnSetZOrder,
    apply: FnApply,
    set_on_complete: FnSetOnComplete,
    get_previous_release_fence: FnGetPreviousReleaseFenceFd,
}

#[derive(Default)]
struct Stats {
    submitted: AtomicU64,
    completed: AtomicU64,
    released: AtomicU64,
    release_fence_missing: AtomicU64,
    release_fence_timeout: AtomicU64,
}

struct Presenter {
    lib: *mut c_void,
    s: Symbols,
    parent: usize,
    control: SurfaceControl,
    pending: Arc<Mutex<Option<*mut Ahb>>>,
    stats: Arc<Stats>,
    callbacks: Arc<AtomicU64>,
    retired: Vec<RetiredControl>,
    last_geometry: (i32, i32, u32, u32),
}

unsafe impl Send for Presenter {}

struct RetiredControl {
    control: SurfaceControl,
    release_control: FnReleaseControl,
    callbacks: Arc<AtomicU64>,
}

struct CallbackDone(Arc<AtomicU64>);

impl Drop for CallbackDone {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

static PRESENTER: Mutex<Option<Presenter>> = Mutex::new(None);
static INIT_FAIL_LOGGED: AtomicBool = AtomicBool::new(false);
static PRESENT_FAIL_LOGGED: AtomicBool = AtomicBool::new(false);

fn load_symbol(handle: *mut c_void, name: &str) -> Option<*mut c_void> {
    let name = std::ffi::CString::new(name).ok()?;
    let value = unsafe { dlsym(handle, name.as_ptr()) };
    (!value.is_null()).then_some(value)
}

unsafe fn load() -> Option<(*mut c_void, Symbols)> {
    let lib = dlopen(c"libandroid.so".as_ptr(), RTLD_NOW);
    if lib.is_null() {
        return None;
    }
    let acquire = load_symbol(lib, "AHardwareBuffer_acquire")?;
    let release = load_symbol(lib, "AHardwareBuffer_release")?;
    let create_child = load_symbol(lib, "ASurfaceControl_create")?;
    let release_control = load_symbol(lib, "ASurfaceControl_release")?;
    let create_transaction = load_symbol(lib, "ASurfaceTransaction_create")?;
    let delete_transaction = load_symbol(lib, "ASurfaceTransaction_delete")?;
    let set_buffer = load_symbol(lib, "ASurfaceTransaction_setBuffer")?;
    let set_geometry = load_symbol(lib, "ASurfaceTransaction_setGeometry")?;
    let set_z_order = load_symbol(lib, "ASurfaceTransaction_setZOrder")?;
    let apply = load_symbol(lib, "ASurfaceTransaction_apply")?;
    let set_on_complete = load_symbol(lib, "ASurfaceTransaction_setOnComplete")?;
    let get_previous_release_fence =
        load_symbol(lib, "ASurfaceTransactionStats_getPreviousReleaseFenceFd")?;
    Some((
        lib,
        Symbols {
            acquire: core::mem::transmute(acquire),
            release: core::mem::transmute(release),
            create_child: core::mem::transmute(create_child),
            release_control: core::mem::transmute(release_control),
            create_transaction: core::mem::transmute(create_transaction),
            delete_transaction: core::mem::transmute(delete_transaction),
            set_buffer: core::mem::transmute(set_buffer),
            set_geometry: core::mem::transmute(set_geometry),
            set_z_order: core::mem::transmute(set_z_order),
            apply: core::mem::transmute(apply),
            set_on_complete: core::mem::transmute(set_on_complete),
            get_previous_release_fence: core::mem::transmute(get_previous_release_fence),
        },
    ))
}

unsafe fn wait_release_fence(ctx: &CallbackCtx) -> bool {
    if ctx.previous.is_none() {
        return true;
    }
    let fd = (ctx.get_previous_release_fence)(ctx.stats, ctx.control);
    if fd < 0 {
        ctx.shared
            .release_fence_missing
            .fetch_add(1, Ordering::Relaxed);
        return true;
    }
    let mut pfd = PollFd {
        fd,
        events: POLLIN,
        revents: 0,
    };
    let rc = loop {
        let rc = poll(&mut pfd, 1, 2_000);
        if rc >= 0 || std::io::Error::last_os_error().raw_os_error() != Some(4) {
            break rc;
        }
    };
    let signaled = rc > 0 && (pfd.revents & POLLIN) != 0;
    if !signaled {
        ctx.shared
            .release_fence_timeout
            .fetch_add(1, Ordering::Relaxed);
    }
    close(fd);
    signaled
}

struct CallbackCtx {
    previous: Option<*mut Ahb>,
    previous_slot: u64,
    release: FnRelease,
    get_previous_release_fence: FnGetPreviousReleaseFenceFd,
    control: SurfaceControl,
    stats: *mut c_void,
    shared: Arc<Stats>,
    callbacks: Arc<AtomicU64>,
}

unsafe extern "C" fn on_complete(context: *mut c_void, stats: *mut c_void) {
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if context.is_null() {
            crate::log("video|present|complete|FAIL|null context");
            return;
        }
        let mut ctx = Box::from_raw(context as *mut CallbackCtx);
        let _callback_done = CallbackDone(Arc::clone(&ctx.callbacks));
        ctx.stats = stats;
        let released = wait_release_fence(&ctx);
        if let Some(buffer) = ctx.previous.take() {
            (ctx.release)(buffer);
            ctx.shared.released.fetch_add(1, Ordering::Relaxed);
        }
        let completed = ctx.shared.completed.fetch_add(1, Ordering::Relaxed) + 1;
        if completed <= 3 || completed % 30 == 0 || !released {
            crate::log(&format!(
                "video|present|complete|n={completed}|slot={}|released={released}|submitted={}|released_count={}",
                ctx.previous_slot,
                ctx.shared.submitted.load(Ordering::Relaxed),
                ctx.shared.released.load(Ordering::Relaxed)
            ));
        }
    }));
}

impl Presenter {
    unsafe fn new() -> Option<Self> {
        let Some(parent) = crate::surface::wait_parent(Duration::from_secs(3)) else {
            return None;
        };
        let (lib, s) = load()?;
        let control = (s.create_child)(parent as SurfaceControl, c"husk-video".as_ptr());
        if control.is_null() {
            dlclose(lib);
            return None;
        }
        crate::log(&format!(
            "video|present|control|PASS|parent={parent:#x}|control={control:p}"
        ));
        Some(Self {
            lib,
            s,
            parent,
            control,
            pending: Arc::new(Mutex::new(None)),
            stats: Arc::new(Stats::default()),
            callbacks: Arc::new(AtomicU64::new(0)),
            retired: Vec::new(),
            last_geometry: (0, 0, 0, 0),
        })
    }

    unsafe fn drain_retired(&mut self) {
        let mut index = 0;
        while index < self.retired.len() {
            if self.retired[index].callbacks.load(Ordering::Acquire) != 0 {
                index += 1;
                continue;
            }
            let retired = self.retired.swap_remove(index);
            (retired.release_control)(retired.control);
        }
    }

    unsafe fn rebuild(&mut self, parent: usize) -> bool {
        let control = (self.s.create_child)(parent as SurfaceControl, c"husk-video".as_ptr());
        if control.is_null() {
            crate::log(&format!(
                "video|present|rebuild|FAIL|parent={parent:#x}|create_child"
            ));
            return false;
        }

        self.clear();
        let old_parent = self.parent;
        let old_control = self.control;
        let old_callbacks = core::mem::replace(&mut self.callbacks, Arc::new(AtomicU64::new(0)));
        self.retired.push(RetiredControl {
            control: old_control,
            release_control: self.s.release_control,
            callbacks: old_callbacks,
        });
        self.parent = parent;
        self.control = control;
        self.last_geometry = (0, 0, 0, 0);
        self.drain_retired();
        crate::log(&format!(
            "video|present|rebuild|PASS|old_parent={old_parent:#x}|new_parent={parent:#x}|control={control:p}|retired={}",
            self.retired.len()
        ));
        true
    }

    unsafe fn submit(&mut self, buffer: *mut Ahb, width: u32, height: u32) -> bool {
        let current_parent =
            crate::surface::wait_parent(Duration::from_millis(200)).unwrap_or(self.parent);
        if current_parent != self.parent && !self.rebuild(current_parent) {
            crate::log(&format!(
                "video|present|parent_changed|old={:#x}|new={current_parent:#x}",
                self.parent
            ));
            return false;
        }
        self.drain_retired();
        let Some(window) = crate::WINDOW.load(Ordering::SeqCst).as_mut() else {
            return false;
        };
        let win_w = crate::sys::ANativeWindow_getWidth(window);
        let win_h = crate::sys::ANativeWindow_getHeight(window);
        if win_w <= 0 || win_h <= 0 {
            return false;
        }

        let scale = (win_w as f64 / width.max(1) as f64).min(win_h as f64 / height.max(1) as f64);
        let draw_w = (width as f64 * scale).round().max(1.0) as i32;
        let draw_h = (height as f64 * scale).round().max(1.0) as i32;
        let left = (win_w - draw_w) / 2;
        let top = (win_h - draw_h) / 2;
        let source = ARect {
            left: 0,
            top: 0,
            right: width as i32,
            bottom: height as i32,
        };
        let destination = ARect {
            left,
            top,
            right: left + draw_w,
            bottom: top + draw_h,
        };

        (self.s.acquire)(buffer);
        self.callbacks.fetch_add(1, Ordering::AcqRel);
        let previous = self
            .pending
            .lock()
            .ok()
            .and_then(|mut slot| slot.replace(buffer));
        let submitted = self.stats.submitted.fetch_add(1, Ordering::Relaxed) + 1;
        let transaction = (self.s.create_transaction)();
        if transaction.is_null() {
            if let Some(buffer) = previous {
                (self.s.release)(buffer);
            }
            (self.s.release)(buffer);
            self.callbacks.fetch_sub(1, Ordering::AcqRel);
            return false;
        }
        (self.s.set_geometry)(transaction, self.control, &source, &destination, 0);
        (self.s.set_z_order)(transaction, self.control, 100);
        (self.s.set_buffer)(transaction, self.control, buffer, -1);
        let context = Box::new(CallbackCtx {
            previous,
            previous_slot: submitted.saturating_sub(1),
            release: self.s.release,
            get_previous_release_fence: self.s.get_previous_release_fence,
            control: self.control,
            stats: core::ptr::null_mut(),
            shared: Arc::clone(&self.stats),
            callbacks: Arc::clone(&self.callbacks),
        });
        (self.s.set_on_complete)(
            transaction,
            Box::into_raw(context) as *mut c_void,
            on_complete,
        );
        (self.s.apply)(transaction);
        (self.s.delete_transaction)(transaction);
        if self.last_geometry != (win_w, win_h, width, height) {
            self.last_geometry = (win_w, win_h, width, height);
            crate::log(&format!(
                "video|present|geometry|screen={win_w}x{win_h}|video={width}x{height}|dest=({left},{top})-({},{})",
                left + draw_w,
                top + draw_h
            ));
        }
        if submitted <= 3 || submitted % 30 == 0 {
            crate::log(&format!(
                "video|present|submit|n={submitted}|{width}x{height}|previous={}|completed={}",
                previous.is_some(),
                self.stats.completed.load(Ordering::Relaxed)
            ));
        }
        true
    }

    unsafe fn clear(&mut self) {
        let transaction = (self.s.create_transaction)();
        if transaction.is_null() {
            return;
        }
        (self.s.set_buffer)(transaction, self.control, core::ptr::null_mut(), -1);
        (self.s.apply)(transaction);
        (self.s.delete_transaction)(transaction);
        if let Some(buffer) = self.pending.lock().ok().and_then(|mut slot| slot.take()) {
            (self.s.release)(buffer);
            self.stats.released.fetch_add(1, Ordering::Relaxed);
        }
        self.drain_retired();
        crate::log(&format!(
            "video|present|clear|submitted={}|completed={}|released={}",
            self.stats.submitted.load(Ordering::Relaxed),
            self.stats.completed.load(Ordering::Relaxed),
            self.stats.released.load(Ordering::Relaxed)
        ));
    }
}

impl Drop for Presenter {
    fn drop(&mut self) {
        unsafe {
            for retired in self.retired.drain(..) {
                if retired.callbacks.load(Ordering::Acquire) == 0 {
                    (retired.release_control)(retired.control);
                }
            }
            (self.s.release_control)(self.control);
            dlclose(self.lib);
        }
    }
}

fn enabled(files: &Path) -> bool {
    files.join("rootfs-full/run/husk/present").exists()
}

pub(super) unsafe fn present_if_enabled(
    files: &Path,
    buffer: *mut c_void,
    width: u32,
    height: u32,
) -> bool {
    if !enabled(files) {
        return false;
    }
    let mut slot = match PRESENTER.lock() {
        Ok(v) => v,
        Err(_) => return false,
    };
    if slot.is_none() {
        match Presenter::new() {
            Some(presenter) => *slot = Some(presenter),
            None => {
                if !INIT_FAIL_LOGGED.swap(true, Ordering::Relaxed) {
                    crate::log("video|present|INIT|FAIL|SurfaceControl unavailable");
                }
                return false;
            }
        }
    }
    let ok = slot
        .as_mut()
        .is_some_and(|p| p.submit(buffer, width, height));
    if !ok && !PRESENT_FAIL_LOGGED.swap(true, Ordering::Relaxed) {
        crate::log("video|present|SUBMIT|FAIL");
    }
    ok
}

pub(super) unsafe fn finish_session() {
    if let Ok(mut slot) = PRESENTER.lock() {
        if let Some(presenter) = slot.as_mut() {
            presenter.clear();
            for _ in 0..20 {
                presenter.drain_retired();
                if presenter.retired.is_empty() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

pub(super) unsafe fn begin_session() {
    if let Ok(mut slot) = PRESENTER.lock() {
        if let Some(presenter) = slot.as_mut() {
            presenter.clear();
            presenter.stats.submitted.store(0, Ordering::Relaxed);
            presenter.stats.completed.store(0, Ordering::Relaxed);
            presenter.stats.released.store(0, Ordering::Relaxed);
            presenter
                .stats
                .release_fence_missing
                .store(0, Ordering::Relaxed);
            presenter
                .stats
                .release_fence_timeout
                .store(0, Ordering::Relaxed);
            presenter.last_geometry = (0, 0, 0, 0);
            presenter.drain_retired();
            crate::log("video|present|session|BEGIN");
        }
    }
}
