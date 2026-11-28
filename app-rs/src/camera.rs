//! Camera bridge — stage 1: pure-Rust Camera2 NDK capture.
//!
//! Prior art (read before writing this, see docs/design/camera-prior-art.md):
//! `DroidNative/DroidNativeApp` `devices/CameraBridge.kt` — Camera2 + ImageReader
//! in `YUV_420_888`, converted to NV12 / YUYV, frames handed to a guest
//! `/dev/videoN` that their PRoot extension emulates.
//!
//! This module is the *app* half. It is deliberately independent of the guest
//! protocol so it can be verified on its own: with the marker
//! `files/camera-selftest` present it captures N frames, converts them to NV12
//! and writes them under `files/camera-selftest/`, logging peak/mean so a fake
//! or blank stream cannot pass.
//!
//! Why the NDK and not `/dev/video*`: `untrusted_app` cannot open the V4L2
//! nodes; Camera2 is the only legal hardware path, and the NDK C ABI keeps the
//! app free of Java.

use core::ffi::{c_char, c_int, c_void};
use core::sync::atomic::{AtomicBool, AtomicI32, AtomicPtr, AtomicUsize, Ordering};
use std::ffi::CString;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

type CameraStatus = i32;
const ACAMERA_OK: CameraStatus = 0;

const AIMAGE_FORMAT_YUV_420_888: i32 = 0x23;
const AMEDIA_OK: i32 = 0;

const ACAMERA_TEMPLATE_PREVIEW: u32 = 1;
const ACAMERA_TEMPLATE_RECORD: u32 = 3;

// Tag encoding is `section << 16 | offset` (NdkCameraMetadataTags.h).
const TAG_LENS_FACING: u32 = (8 << 16) + 5;
const TAG_SCALER_AVAILABLE_STREAM_CONFIGURATIONS: u32 = (13 << 16) + 10;
const TAG_SENSOR_ORIENTATION: u32 = (14 << 16) + 14;
const TAG_CONTROL_AE_MODE: u32 = (1 << 16) + 3;
const TAG_CONTROL_MODE: u32 = (1 << 16) + 15;
const TAG_CONTROL_AE_STATE: u32 = (1 << 16) + 31;
const TAG_FLASH_MODE: u32 = (4 << 16) + 2;
const TAG_SENSOR_EXPOSURE_TIME: u32 = 14 << 16;
const TAG_SENSOR_FRAME_DURATION: u32 = (14 << 16) + 1;
const TAG_SENSOR_SENSITIVITY: u32 = (14 << 16) + 2;
const TAG_SENSOR_TIMESTAMP: u32 = (14 << 16) + 3;

const LENS_FACING_BACK: u8 = 1;
const LENS_FACING_FRONT: u8 = 0;
const CONTROL_MODE_AUTO: u8 = 1;
const CONTROL_AE_MODE_ON: u8 = 1;
const FLASH_MODE_TORCH: u8 = 2;
const TORCH_MODE_OFF: u32 = 0;
const TORCH_MODE_ON: u32 = 1;

const ALOOPER_PREPARE_ALLOW_NON_CALLBACKS: c_int = 1;

type ACameraManager = c_void;
type ACameraDevice = c_void;
type ACameraMetadata = c_void;
type ACaptureRequest = c_void;
type ACaptureSessionOutput = c_void;
type ACaptureSessionOutputContainer = c_void;
type ACameraOutputTarget = c_void;
type ACameraCaptureSession = c_void;
type AImageReader = c_void;
type AImage = c_void;
type ANativeWindow = c_void;
type ALooper = c_void;

type CaptureStartCallback =
    unsafe extern "C" fn(*mut c_void, *mut ACameraCaptureSession, *const ACaptureRequest, i64);
type CaptureResultCallback = unsafe extern "C" fn(
    *mut c_void,
    *mut ACameraCaptureSession,
    *mut ACaptureRequest,
    *const ACameraMetadata,
);
type CaptureFailedCallback = unsafe extern "C" fn(
    *mut c_void,
    *mut ACameraCaptureSession,
    *mut ACaptureRequest,
    *mut CaptureFailure,
);
type CaptureSequenceEndCallback =
    unsafe extern "C" fn(*mut c_void, *mut ACameraCaptureSession, c_int, i64);
type CaptureSequenceAbortCallback =
    unsafe extern "C" fn(*mut c_void, *mut ACameraCaptureSession, c_int);
type CaptureBufferLostCallback = unsafe extern "C" fn(
    *mut c_void,
    *mut ACameraCaptureSession,
    *mut ACaptureRequest,
    *mut ANativeWindow,
    i64,
);
type ImageAvailableCallback = unsafe extern "C" fn(*mut c_void, *mut AImageReader);

#[repr(C)]
struct CaptureFailure {
    frame_number: i64,
    reason: c_int,
    sequence_id: c_int,
    // C `bool` is one byte; keep it as u8 so we never construct an invalid Rust bool.
    was_image_captured: u8,
}

#[repr(C)]
struct CaptureCallbacks {
    context: *mut c_void,
    on_capture_started: Option<CaptureStartCallback>,
    on_capture_progressed: Option<CaptureResultCallback>,
    on_capture_completed: Option<CaptureResultCallback>,
    on_capture_failed: Option<CaptureFailedCallback>,
    on_capture_sequence_completed: Option<CaptureSequenceEndCallback>,
    on_capture_sequence_aborted: Option<CaptureSequenceAbortCallback>,
    on_capture_buffer_lost: Option<CaptureBufferLostCallback>,
}

#[repr(C)]
struct ImageListener {
    context: *mut c_void,
    on_image_available: Option<ImageAvailableCallback>,
}

#[repr(C)]
struct CameraIdList {
    num_cameras: c_int,
    _pad: u32,
    camera_ids: *mut *mut c_char,
}

#[repr(C)]
struct CameraMetadataEntry {
    tag: u32,
    type_: u8,
    _pad0: [u8; 3],
    count: u32,
    _pad1: u32,
    data: *const c_void,
}

type DeviceStateCallback = unsafe extern "C" fn(*mut c_void, *mut ACameraDevice);
type DeviceErrorCallback = unsafe extern "C" fn(*mut c_void, *mut ACameraDevice, c_int);
type SessionStateCallback = unsafe extern "C" fn(*mut c_void, *mut ACameraCaptureSession);

#[repr(C)]
struct DeviceCallbacks {
    context: *mut c_void,
    on_disconnected: Option<DeviceStateCallback>,
    on_error: Option<DeviceErrorCallback>,
    // Reserved: newer platforms read an `onIdle` slot at the same offset.
    _reserved: *mut c_void,
}

#[repr(C)]
struct SessionCallbacks {
    context: *mut c_void,
    on_closed: Option<SessionStateCallback>,
    on_ready: Option<SessionStateCallback>,
    on_active: Option<SessionStateCallback>,
}

unsafe fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void {
    extern "C" {
        fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    }
    dlopen(filename, flags)
}

unsafe fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void {
    extern "C" {
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    }
    dlsym(handle, symbol)
}

unsafe fn dlerror_text() -> String {
    extern "C" {
        fn dlerror() -> *mut c_char;
    }
    let message = dlerror();
    if message.is_null() {
        "unknown dlerror".to_string()
    } else {
        core::ffi::CStr::from_ptr(message)
            .to_string_lossy()
            .into_owned()
    }
}

unsafe fn symbol(handle: *mut c_void, name: &core::ffi::CStr) -> Result<*mut c_void, String> {
    let p = dlsym(handle, name.as_ptr());
    if p.is_null() {
        return Err(format!(
            "dlsym({:?}): {}",
            name.to_string_lossy(),
            dlerror_text()
        ));
    }
    Ok(p)
}

/// Standalone helpers do not inherit an Android Runtime thread that owns the
/// binder thread pool. CameraService can accept a request in that process but
/// never deliver capture/image callbacks. Start the NDK binder pool explicitly
/// before creating the Camera2 manager; native activities never take this path.
unsafe fn start_helper_binder_thread_pool() -> Result<(), String> {
    let handle = dlopen(c"libbinder_ndk.so".as_ptr(), 2 /* RTLD_NOW */);
    if handle.is_null() {
        return Err(format!("dlopen(libbinder_ndk.so): {}", dlerror_text()));
    }
    let raw = dlsym(handle, c"ABinderProcess_startThreadPool".as_ptr());
    if raw.is_null() {
        return Err(format!(
            "dlsym(ABinderProcess_startThreadPool): {}",
            dlerror_text()
        ));
    }
    let start: unsafe extern "C" fn() = core::mem::transmute(raw);
    start();
    Ok(())
}

struct CameraApi {
    _camera_handle: *mut c_void,
    _media_handle: *mut c_void,
    _android_handle: *mut c_void,
    manager_create: unsafe extern "C" fn(*mut *mut ACameraManager) -> CameraStatus,
    manager_delete: unsafe extern "C" fn(*mut ACameraManager),
    manager_set_torch_mode:
        Option<unsafe extern "C" fn(*mut ACameraManager, *const c_char, u32) -> CameraStatus>,
    manager_get_id_list:
        unsafe extern "C" fn(*mut ACameraManager, *mut *mut CameraIdList) -> CameraStatus,
    manager_delete_id_list: unsafe extern "C" fn(*mut CameraIdList),
    manager_get_characteristics: unsafe extern "C" fn(
        *mut ACameraManager,
        *const c_char,
        *mut *mut ACameraMetadata,
    ) -> CameraStatus,
    metadata_get_const_entry:
        unsafe extern "C" fn(*const ACameraMetadata, u32, *mut CameraMetadataEntry) -> CameraStatus,
    metadata_free: unsafe extern "C" fn(*mut ACameraMetadata),
    open_camera: unsafe extern "C" fn(
        *mut ACameraManager,
        *const c_char,
        *mut DeviceCallbacks,
        *mut *mut ACameraDevice,
    ) -> CameraStatus,
    device_close: unsafe extern "C" fn(*mut ACameraDevice) -> CameraStatus,
    device_create_request:
        unsafe extern "C" fn(*mut ACameraDevice, u32, *mut *mut ACaptureRequest) -> CameraStatus,
    request_add_target:
        unsafe extern "C" fn(*mut ACaptureRequest, *const ACameraOutputTarget) -> CameraStatus,
    // camera_status_t ACaptureRequest_setEntry_u8(request, tag, count, data)
    request_set_u8:
        Option<unsafe extern "C" fn(*mut ACaptureRequest, u32, u32, *const u8) -> CameraStatus>,
    request_free: unsafe extern "C" fn(*mut ACaptureRequest),
    output_target_create:
        unsafe extern "C" fn(*mut ANativeWindow, *mut *mut ACameraOutputTarget) -> CameraStatus,
    output_target_free: unsafe extern "C" fn(*mut ACameraOutputTarget),
    session_output_create:
        unsafe extern "C" fn(*mut ANativeWindow, *mut *mut ACaptureSessionOutput) -> CameraStatus,
    session_output_free: unsafe extern "C" fn(*mut ACaptureSessionOutput),
    container_create:
        unsafe extern "C" fn(*mut *mut ACaptureSessionOutputContainer) -> CameraStatus,
    container_add: unsafe extern "C" fn(
        *mut ACaptureSessionOutputContainer,
        *mut ACaptureSessionOutput,
    ) -> CameraStatus,
    container_free: unsafe extern "C" fn(*mut ACaptureSessionOutputContainer),
    session_create: unsafe extern "C" fn(
        *mut ACameraDevice,
        *const ACaptureSessionOutputContainer,
        *const SessionCallbacks,
        *mut *mut ACameraCaptureSession,
    ) -> CameraStatus,
    session_close: unsafe extern "C" fn(*mut ACameraCaptureSession),
    session_set_repeating: unsafe extern "C" fn(
        *mut ACameraCaptureSession,
        *mut CaptureCallbacks,
        c_int,
        *mut *mut ACaptureRequest,
        *mut c_int,
    ) -> CameraStatus,
    session_capture: unsafe extern "C" fn(
        *mut ACameraCaptureSession,
        *mut CaptureCallbacks,
        c_int,
        *mut *mut ACaptureRequest,
        *mut c_int,
    ) -> CameraStatus,
    session_stop_repeating: unsafe extern "C" fn(*mut ACameraCaptureSession) -> CameraStatus,
    reader_new: unsafe extern "C" fn(i32, i32, i32, i32, *mut *mut AImageReader) -> i32,
    reader_delete: unsafe extern "C" fn(*mut AImageReader),
    reader_get_window: unsafe extern "C" fn(*mut AImageReader, *mut *mut ANativeWindow) -> i32,
    reader_set_image_listener: unsafe extern "C" fn(*mut AImageReader, *mut ImageListener) -> i32,
    reader_acquire_next: unsafe extern "C" fn(*mut AImageReader, *mut *mut AImage) -> i32,
    image_delete: unsafe extern "C" fn(*mut AImage),
    image_get_timestamp: unsafe extern "C" fn(*const AImage, *mut i64) -> i32,
    image_get_plane_data:
        unsafe extern "C" fn(*const AImage, c_int, *mut *mut u8, *mut usize) -> i32,
    image_get_plane_row_stride: unsafe extern "C" fn(*const AImage, c_int, *mut i32) -> i32,
    image_get_plane_pixel_stride: unsafe extern "C" fn(*const AImage, c_int, *mut i32) -> i32,
    image_get_width: unsafe extern "C" fn(*const AImage, *mut i32) -> i32,
    image_get_height: unsafe extern "C" fn(*const AImage, *mut i32) -> i32,
    image_get_format: unsafe extern "C" fn(*const AImage, *mut c_int) -> i32,
    image_get_number_of_planes: unsafe extern "C" fn(*const AImage, *mut c_int) -> i32,
    looper_prepare: unsafe extern "C" fn(c_int) -> *mut ALooper,
    // int ALooper_pollOnce(int timeoutMillis, int* outFd, int* outEvents, void** outData)
    // There is no looper argument: it always polls the calling thread's looper.
    looper_poll_once:
        unsafe extern "C" fn(c_int, *mut c_int, *mut c_int, *mut *mut c_void) -> c_int,
    window_get_width: unsafe extern "C" fn(*mut ANativeWindow) -> i32,
    window_get_height: unsafe extern "C" fn(*mut ANativeWindow) -> i32,
    window_get_format: unsafe extern "C" fn(*mut ANativeWindow) -> i32,
    window_set_buffers_geometry: unsafe extern "C" fn(*mut ANativeWindow, i32, i32, i32) -> i32,
}

impl CameraApi {
    unsafe fn load() -> Result<Self, String> {
        let camera_handle = dlopen(c"libcamera2ndk.so".as_ptr(), 2 /* RTLD_NOW */);
        if camera_handle.is_null() {
            return Err(format!("dlopen(libcamera2ndk.so): {}", dlerror_text()));
        }
        let media_handle = dlopen(c"libmediandk.so".as_ptr(), 2);
        if media_handle.is_null() {
            return Err(format!("dlopen(libmediandk.so): {}", dlerror_text()));
        }
        let android_handle = dlopen(c"libandroid.so".as_ptr(), 2);
        if android_handle.is_null() {
            return Err(format!("dlopen(libandroid.so): {}", dlerror_text()));
        }
        Ok(Self {
            _camera_handle: camera_handle,
            _media_handle: media_handle,
            _android_handle: android_handle,
            manager_create: core::mem::transmute(symbol(camera_handle, c"ACameraManager_create")?),
            manager_delete: core::mem::transmute(symbol(camera_handle, c"ACameraManager_delete")?),
            manager_set_torch_mode: symbol(camera_handle, c"ACameraManager_setTorchMode")
                .ok()
                .map(|p| core::mem::transmute(p)),
            manager_get_id_list: core::mem::transmute(symbol(
                camera_handle,
                c"ACameraManager_getCameraIdList",
            )?),
            manager_delete_id_list: core::mem::transmute(symbol(
                camera_handle,
                c"ACameraManager_deleteCameraIdList",
            )?),
            manager_get_characteristics: core::mem::transmute(symbol(
                camera_handle,
                c"ACameraManager_getCameraCharacteristics",
            )?),
            metadata_get_const_entry: core::mem::transmute(symbol(
                camera_handle,
                c"ACameraMetadata_getConstEntry",
            )?),
            metadata_free: core::mem::transmute(symbol(camera_handle, c"ACameraMetadata_free")?),
            open_camera: core::mem::transmute(symbol(camera_handle, c"ACameraManager_openCamera")?),
            device_close: core::mem::transmute(symbol(camera_handle, c"ACameraDevice_close")?),
            device_create_request: core::mem::transmute(symbol(
                camera_handle,
                c"ACameraDevice_createCaptureRequest",
            )?),
            request_add_target: core::mem::transmute(symbol(
                camera_handle,
                c"ACaptureRequest_addTarget",
            )?),
            // The exported spelling is `_u8` on Android 13; `_uint8` shows up in
            // newer headers. Accept either, and tolerate neither.
            request_set_u8: symbol(camera_handle, c"ACaptureRequest_setEntry_u8")
                .or_else(|_| symbol(camera_handle, c"ACaptureRequest_setEntry_uint8"))
                .ok()
                .map(|p| core::mem::transmute(p)),
            request_free: core::mem::transmute(symbol(camera_handle, c"ACaptureRequest_free")?),
            output_target_create: core::mem::transmute(symbol(
                camera_handle,
                c"ACameraOutputTarget_create",
            )?),
            output_target_free: core::mem::transmute(symbol(
                camera_handle,
                c"ACameraOutputTarget_free",
            )?),
            session_output_create: core::mem::transmute(symbol(
                camera_handle,
                c"ACaptureSessionOutput_create",
            )?),
            session_output_free: core::mem::transmute(symbol(
                camera_handle,
                c"ACaptureSessionOutput_free",
            )?),
            container_create: core::mem::transmute(symbol(
                camera_handle,
                c"ACaptureSessionOutputContainer_create",
            )?),
            container_add: core::mem::transmute(symbol(
                camera_handle,
                c"ACaptureSessionOutputContainer_add",
            )?),
            container_free: core::mem::transmute(symbol(
                camera_handle,
                c"ACaptureSessionOutputContainer_free",
            )?),
            session_create: core::mem::transmute(symbol(
                camera_handle,
                c"ACameraDevice_createCaptureSession",
            )?),
            session_close: core::mem::transmute(symbol(
                camera_handle,
                c"ACameraCaptureSession_close",
            )?),
            session_set_repeating: core::mem::transmute(symbol(
                camera_handle,
                c"ACameraCaptureSession_setRepeatingRequest",
            )?),
            session_capture: core::mem::transmute(symbol(
                camera_handle,
                c"ACameraCaptureSession_capture",
            )?),
            session_stop_repeating: core::mem::transmute(symbol(
                camera_handle,
                c"ACameraCaptureSession_stopRepeating",
            )?),
            reader_new: core::mem::transmute(symbol(media_handle, c"AImageReader_new")?),
            reader_delete: core::mem::transmute(symbol(media_handle, c"AImageReader_delete")?),
            reader_get_window: core::mem::transmute(symbol(
                media_handle,
                c"AImageReader_getWindow",
            )?),
            reader_set_image_listener: core::mem::transmute(symbol(
                media_handle,
                c"AImageReader_setImageListener",
            )?),
            reader_acquire_next: core::mem::transmute(symbol(
                media_handle,
                c"AImageReader_acquireNextImage",
            )?),
            image_delete: core::mem::transmute(symbol(media_handle, c"AImage_delete")?),
            image_get_timestamp: core::mem::transmute(symbol(
                media_handle,
                c"AImage_getTimestamp",
            )?),
            image_get_plane_data: core::mem::transmute(symbol(
                media_handle,
                c"AImage_getPlaneData",
            )?),
            image_get_plane_row_stride: core::mem::transmute(symbol(
                media_handle,
                c"AImage_getPlaneRowStride",
            )?),
            image_get_plane_pixel_stride: core::mem::transmute(symbol(
                media_handle,
                c"AImage_getPlanePixelStride",
            )?),
            image_get_width: core::mem::transmute(symbol(media_handle, c"AImage_getWidth")?),
            image_get_height: core::mem::transmute(symbol(media_handle, c"AImage_getHeight")?),
            image_get_format: core::mem::transmute(symbol(media_handle, c"AImage_getFormat")?),
            image_get_number_of_planes: core::mem::transmute(symbol(
                media_handle,
                c"AImage_getNumberOfPlanes",
            )?),
            looper_prepare: core::mem::transmute(symbol(android_handle, c"ALooper_prepare")?),
            looper_poll_once: core::mem::transmute(symbol(android_handle, c"ALooper_pollOnce")?),
            window_get_width: core::mem::transmute(symbol(
                android_handle,
                c"ANativeWindow_getWidth",
            )?),
            window_get_height: core::mem::transmute(symbol(
                android_handle,
                c"ANativeWindow_getHeight",
            )?),
            window_get_format: core::mem::transmute(symbol(
                android_handle,
                c"ANativeWindow_getFormat",
            )?),
            window_set_buffers_geometry: core::mem::transmute(symbol(
                android_handle,
                c"ANativeWindow_setBuffersGeometry",
            )?),
        })
    }
}

// ---------------------------------------------------------------------------
// Static hand-off between the looper thread and everyone else.
// ---------------------------------------------------------------------------

static STARTED: AtomicBool = AtomicBool::new(false);
static SESSION_READY: AtomicBool = AtomicBool::new(false);
static SESSION_ACTIVE: AtomicBool = AtomicBool::new(false);
/// A dead or half-open camera client must never prevent another V4L2 device
/// from being accepted. Keep a hard bound so a broken client loop cannot grow
/// one thread per connection without limit.
static ACTIVE_CAMERA_CLIENTS: AtomicUsize = AtomicUsize::new(0);
/// Camera2 has process-global per-device session constraints and the callbacks
/// use shared readiness state. INFO may run concurrently, but only one capture
/// session may own the shared state at a time.
static CAMERA_SESSION_OWNER: Mutex<()> = Mutex::new(());
const MAX_CAMERA_CLIENTS: usize = 16;
const CAMERA_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(3);
const CAMERA_WRITE_TIMEOUT: Duration = Duration::from_secs(5);
static STOP: AtomicBool = AtomicBool::new(false);
static SESSION: AtomicPtr<ACameraCaptureSession> = AtomicPtr::new(core::ptr::null_mut());
static REQUEST: AtomicPtr<ACaptureRequest> = AtomicPtr::new(core::ptr::null_mut());
static DEVICE_ERROR: AtomicI32 = AtomicI32::new(0);
static FRAMES: AtomicUsize = AtomicUsize::new(0);
static IMAGE_AVAILABLE: AtomicUsize = AtomicUsize::new(0);
static CAPTURE_STARTED: AtomicUsize = AtomicUsize::new(0);
static CAPTURE_PROGRESSED: AtomicUsize = AtomicUsize::new(0);
static CAPTURE_COMPLETED: AtomicUsize = AtomicUsize::new(0);
static CAPTURE_FAILED: AtomicUsize = AtomicUsize::new(0);
static CAPTURE_BUFFER_LOST: AtomicUsize = AtomicUsize::new(0);
static CAMERA_API_FOR_CALLBACKS: AtomicPtr<CameraApi> = AtomicPtr::new(core::ptr::null_mut());

unsafe extern "C" fn on_device_disconnected(_ctx: *mut c_void, _device: *mut ACameraDevice) {
    crate::log("camera|device|disconnected");
}

unsafe extern "C" fn on_device_error(_ctx: *mut c_void, _device: *mut ACameraDevice, error: c_int) {
    DEVICE_ERROR.store(error, Ordering::SeqCst);
    crate::log(&format!("camera|device|error|{error}"));
}

unsafe extern "C" fn on_session_closed(_ctx: *mut c_void, _session: *mut ACameraCaptureSession) {
    SESSION_READY.store(false, Ordering::SeqCst);
    SESSION_ACTIVE.store(false, Ordering::SeqCst);
}

unsafe extern "C" fn on_session_ready(_ctx: *mut c_void, _session: *mut ACameraCaptureSession) {
    SESSION_READY.store(true, Ordering::SeqCst);
    crate::log("camera|callback|session_ready");
}

unsafe extern "C" fn on_session_active(_ctx: *mut c_void, _session: *mut ACameraCaptureSession) {
    SESSION_ACTIVE.store(true, Ordering::SeqCst);
    crate::log("camera|callback|session_active");
}

unsafe extern "C" fn on_image_available(_ctx: *mut c_void, reader: *mut AImageReader) {
    let count = IMAGE_AVAILABLE.fetch_add(1, Ordering::SeqCst) + 1;
    if count == 1 || count % 30 == 0 {
        crate::log(&format!(
            "camera|image_reader|available|count={count}|reader_null={}",
            reader.is_null()
        ));
    }
}

unsafe extern "C" fn on_capture_started(
    _ctx: *mut c_void,
    _session: *mut ACameraCaptureSession,
    _request: *const ACaptureRequest,
    timestamp: i64,
) {
    let count = CAPTURE_STARTED.fetch_add(1, Ordering::SeqCst) + 1;
    if count == 1 || count % 30 == 0 {
        crate::log(&format!(
            "camera|capture|started|count={count}|timestamp={timestamp}"
        ));
    }
}

unsafe extern "C" fn on_capture_progressed(
    _ctx: *mut c_void,
    _session: *mut ACameraCaptureSession,
    _request: *mut ACaptureRequest,
    _result: *const ACameraMetadata,
) {
    let count = CAPTURE_PROGRESSED.fetch_add(1, Ordering::SeqCst) + 1;
    if count == 1 || count % 30 == 0 {
        crate::log(&format!("camera|capture|progressed|count={count}"));
        log_capture_result_metadata(_result, "progressed");
    }
}

unsafe extern "C" fn on_capture_completed(
    _ctx: *mut c_void,
    _session: *mut ACameraCaptureSession,
    _request: *mut ACaptureRequest,
    _result: *const ACameraMetadata,
) {
    let count = CAPTURE_COMPLETED.fetch_add(1, Ordering::SeqCst) + 1;
    if count == 1 || count % 30 == 0 {
        crate::log(&format!("camera|capture|completed|count={count}"));
        log_capture_result_metadata(_result, "completed");
    }
}

unsafe extern "C" fn on_capture_failed(
    _ctx: *mut c_void,
    _session: *mut ACameraCaptureSession,
    _request: *mut ACaptureRequest,
    failure: *mut CaptureFailure,
) {
    let count = CAPTURE_FAILED.fetch_add(1, Ordering::SeqCst) + 1;
    let (frame, reason, sequence, image) = if failure.is_null() {
        (-1, -1, -1, 0)
    } else {
        let f = &*failure;
        (
            f.frame_number,
            f.reason,
            f.sequence_id,
            f.was_image_captured,
        )
    };
    if count == 1 || count % 30 == 0 {
        crate::log(&format!(
            "camera|capture|failed|count={count}|frame={frame}|reason={reason}|sequence={sequence}|image_captured={image}"
        ));
    }
}

unsafe extern "C" fn on_capture_sequence_completed(
    _ctx: *mut c_void,
    _session: *mut ACameraCaptureSession,
    sequence: c_int,
    frame: i64,
) {
    crate::log(&format!(
        "camera|capture|sequence_completed|sequence={sequence}|frame={frame}"
    ));
}

unsafe extern "C" fn on_capture_sequence_aborted(
    _ctx: *mut c_void,
    _session: *mut ACameraCaptureSession,
    sequence: c_int,
) {
    crate::log(&format!(
        "camera|capture|sequence_aborted|sequence={sequence}"
    ));
}

unsafe extern "C" fn on_capture_buffer_lost(
    _ctx: *mut c_void,
    _session: *mut ACameraCaptureSession,
    _request: *mut ACaptureRequest,
    _window: *mut ANativeWindow,
    frame: i64,
) {
    let count = CAPTURE_BUFFER_LOST.fetch_add(1, Ordering::SeqCst) + 1;
    if count == 1 || count % 30 == 0 {
        crate::log(&format!(
            "camera|capture|buffer_lost|count={count}|frame={frame}"
        ));
    }
}

// ---------------------------------------------------------------------------
// Metadata helpers
// ---------------------------------------------------------------------------

unsafe fn entry(
    api: &CameraApi,
    meta: *const ACameraMetadata,
    tag: u32,
) -> Option<CameraMetadataEntry> {
    let mut out = CameraMetadataEntry {
        tag: 0,
        type_: 0,
        _pad0: [0; 3],
        count: 0,
        _pad1: 0,
        data: core::ptr::null(),
    };
    if (api.metadata_get_const_entry)(meta, tag, &mut out) != ACAMERA_OK || out.data.is_null() {
        return None;
    }
    Some(out)
}

unsafe fn log_capture_result_metadata(result: *const ACameraMetadata, stage: &str) {
    if result.is_null() || std::env::var_os("HUSK_CAMERA_PLANE_AUDIT").is_none() {
        return;
    }
    let api_ptr = CAMERA_API_FOR_CALLBACKS.load(Ordering::SeqCst);
    if api_ptr.is_null() {
        return;
    }
    let api = &*api_ptr;
    let tags = [
        ("sensor_exposure", TAG_SENSOR_EXPOSURE_TIME, "i64"),
        ("sensor_frame_duration", TAG_SENSOR_FRAME_DURATION, "i64"),
        ("sensor_sensitivity", TAG_SENSOR_SENSITIVITY, "i32"),
        ("sensor_timestamp", TAG_SENSOR_TIMESTAMP, "i64"),
        ("ae_state", TAG_CONTROL_AE_STATE, "u8"),
        ("ae_mode", TAG_CONTROL_AE_MODE, "u8"),
        ("control_mode", TAG_CONTROL_MODE, "u8"),
    ];
    let mut fields = Vec::new();
    for (name, tag, kind) in tags {
        let mut e = CameraMetadataEntry {
            tag: 0,
            type_: 0,
            _pad0: [0; 3],
            count: 0,
            _pad1: 0,
            data: core::ptr::null(),
        };
        let rc = (api.metadata_get_const_entry)(result, tag, &mut e);
        if rc != ACAMERA_OK || e.data.is_null() || e.count == 0 {
            fields.push(format!("{name}=missing(rc={rc})"));
            continue;
        }
        let value = match (kind, e.type_) {
            ("i64", 3) => *(e.data as *const i64),
            ("i32", 1) => *(e.data as *const i32) as i64,
            ("u8", 0) => *(e.data as *const u8) as i64,
            _ => i64::MIN,
        };
        fields.push(format!("{name}={value}"));
    }
    crate::log(&format!(
        "camera|capture|result|{stage}|{}",
        fields.join("|")
    ));
}

unsafe fn lens_facing(api: &CameraApi, meta: *const ACameraMetadata) -> Option<u8> {
    let e = entry(api, meta, TAG_LENS_FACING)?;
    Some(*(e.data as *const u8))
}

unsafe fn orientation(api: &CameraApi, meta: *const ACameraMetadata) -> Option<i32> {
    let e = entry(api, meta, TAG_SENSOR_ORIENTATION)?;
    Some(*(e.data as *const i32))
}

/// YUV_420_888 output sizes, largest first, even dimensions only, ≤ 1920x1080.
unsafe fn yuv_sizes(api: &CameraApi, meta: *const ACameraMetadata) -> Vec<(i32, i32)> {
    let mut out = Vec::new();
    let Some(e) = entry(api, meta, TAG_SCALER_AVAILABLE_STREAM_CONFIGURATIONS) else {
        return out;
    };
    let data = e.data as *const i32;
    let quads = e.count as usize / 4;
    for i in 0..quads {
        let format = *data.add(i * 4);
        let width = *data.add(i * 4 + 1);
        let height = *data.add(i * 4 + 2);
        let input = *data.add(i * 4 + 3);
        if format != AIMAGE_FORMAT_YUV_420_888 || input != 0 {
            continue;
        }
        if width > 1920 || height > 1080 || width % 2 != 0 || height % 2 != 0 {
            continue;
        }
        if !out.contains(&(width, height)) {
            out.push((width, height));
        }
    }
    out.sort_by_key(|(w, h)| -(w * h));
    out.truncate(16);
    out
}

struct CameraInfo {
    id: String,
    facing: u8,
    orientation: i32,
    sizes: Vec<(i32, i32)>,
}

unsafe fn enumerate(api: &CameraApi, manager: *mut ACameraManager) -> Vec<CameraInfo> {
    let mut list: *mut CameraIdList = core::ptr::null_mut();
    if (api.manager_get_id_list)(manager, &mut list) != ACAMERA_OK || list.is_null() {
        crate::log("camera|list|FAIL|getCameraIdList");
        return Vec::new();
    }
    let mut out = Vec::new();
    for i in 0..(*list).num_cameras as usize {
        let raw = *(*list).camera_ids.add(i);
        if raw.is_null() {
            continue;
        }
        let id = core::ffi::CStr::from_ptr(raw)
            .to_string_lossy()
            .into_owned();
        let mut meta: *mut ACameraMetadata = core::ptr::null_mut();
        let cid = match CString::new(id.clone()) {
            Ok(c) => c,
            Err(_) => continue,
        };
        if (api.manager_get_characteristics)(manager, cid.as_ptr(), &mut meta) != ACAMERA_OK
            || meta.is_null()
        {
            crate::log(&format!("camera|list|id={id}|FAIL|characteristics"));
            continue;
        }
        let facing = lens_facing(api, meta).unwrap_or(2);
        let orient = orientation(api, meta).unwrap_or(-1);
        let sizes = yuv_sizes(api, meta);
        (api.metadata_free)(meta);
        let facing_name = match facing {
            LENS_FACING_BACK => "BACK",
            LENS_FACING_FRONT => "FRONT",
            _ => "EXTERNAL",
        };
        let size_text = sizes
            .iter()
            .take(6)
            .map(|(w, h)| format!("{w}x{h}"))
            .collect::<Vec<_>>()
            .join(",");
        crate::log(&format!(
            "camera|list|id={id}|facing={facing_name}|orientation={orient}|yuv={size_text}|n={}",
            sizes.len()
        ));
        out.push(CameraInfo {
            id,
            facing,
            orientation: orient,
            sizes,
        });
    }
    (api.manager_delete_id_list)(list);
    out
}

// ---------------------------------------------------------------------------
// Capture
// ---------------------------------------------------------------------------

/// Sample only the meaningful pixels of an AImage plane, excluding row padding.
unsafe fn plane_sample_stats(
    data: *const u8,
    len: usize,
    rows: usize,
    cols: usize,
    row_stride: usize,
    pixel_stride: usize,
) -> (u8, u8, f64) {
    if data.is_null() || rows == 0 || cols == 0 {
        return (0, 0, 0.0);
    }
    let mut min = 255u8;
    let mut max = 0u8;
    let mut sum = 0u64;
    let mut count = 0u64;
    for row in 0..rows {
        let row_base = row.saturating_mul(row_stride);
        for col in 0..cols {
            let offset = row_base.saturating_add(col.saturating_mul(pixel_stride));
            if offset >= len {
                continue;
            }
            let value = *data.add(offset);
            min = min.min(value);
            max = max.max(value);
            sum += value as u64;
            count += 1;
        }
    }
    if count == 0 {
        (0, 0, 0.0)
    } else {
        (min, max, sum as f64 / count as f64)
    }
}

/// Copy `YUV_420_888` into NV12 (Y plane, then interleaved UV at half size).
unsafe fn to_nv12(
    api: &CameraApi,
    image: *const AImage,
    out: &mut Vec<u8>,
) -> Result<(i32, i32), String> {
    let mut width = 0i32;
    let mut height = 0i32;
    if (api.image_get_width)(image, &mut width) != AMEDIA_OK
        || (api.image_get_height)(image, &mut height) != AMEDIA_OK
    {
        return Err("AImage_getWidth/Height failed".into());
    }
    let mut planes = [(core::ptr::null_mut::<u8>(), 0usize, 0i32, 0i32); 3];
    for (i, slot) in planes.iter_mut().enumerate() {
        let mut data = core::ptr::null_mut();
        let mut len = 0usize;
        let mut row_stride = 0i32;
        let mut pixel_stride = 0i32;
        if (api.image_get_plane_data)(image, i as c_int, &mut data, &mut len) != AMEDIA_OK {
            return Err(format!("AImage_getPlaneData({i}) failed"));
        }
        if (api.image_get_plane_row_stride)(image, i as c_int, &mut row_stride) != AMEDIA_OK
            || (api.image_get_plane_pixel_stride)(image, i as c_int, &mut pixel_stride) != AMEDIA_OK
        {
            return Err(format!("AImage_getPlane*Stride({i}) failed"));
        }
        *slot = (data, len, row_stride, pixel_stride.max(1));
    }

    if std::env::var_os("HUSK_CAMERA_PLANE_AUDIT").is_some() {
        let mut fmt = -1;
        let mut plane_count = -1;
        let fmt_rc = (api.image_get_format)(image, &mut fmt);
        let count_rc = (api.image_get_number_of_planes)(image, &mut plane_count);
        let (y_min, y_max, y_mean) = plane_sample_stats(
            planes[0].0,
            planes[0].1,
            height.max(0) as usize,
            width.max(0) as usize,
            planes[0].2.max(0) as usize,
            planes[0].3.max(1) as usize,
        );
        let (u_min, u_max, u_mean) = plane_sample_stats(
            planes[1].0,
            planes[1].1,
            (height.max(0) / 2) as usize,
            (width.max(0) / 2) as usize,
            planes[1].2.max(0) as usize,
            planes[1].3.max(1) as usize,
        );
        let (v_min, v_max, v_mean) = plane_sample_stats(
            planes[2].0,
            planes[2].1,
            (height.max(0) / 2) as usize,
            (width.max(0) / 2) as usize,
            planes[2].2.max(0) as usize,
            planes[2].3.max(1) as usize,
        );
        crate::log(&format!(
            "camera|image|audit|{width}x{height}|fmt={fmt}|fmt_rc={fmt_rc}|planes={plane_count}|count_rc={count_rc}|p0_len={}|p0_row={}|p0_pix={}|y={y_min}/{y_max}/{y_mean:.2}|p1_len={}|p1_row={}|p1_pix={}|u={u_min}/{u_max}/{u_mean:.2}|p2_len={}|p2_row={}|p2_pix={}|v={v_min}/{v_max}/{v_mean:.2}",
            planes[0].1,
            planes[0].2,
            planes[0].3,
            planes[1].1,
            planes[1].2,
            planes[1].3,
            planes[2].1,
            planes[2].2,
            planes[2].3,
        ));
    }

    let (y_ptr, _, y_stride, _) = planes[0];
    let (u_ptr, _, u_stride, u_pixel) = planes[1];
    let (v_ptr, _, v_stride, v_pixel) = planes[2];

    let y_bytes = (width * height) as usize;
    let uv_bytes = y_bytes / 2;
    out.clear();
    out.resize(y_bytes + uv_bytes, 0);

    for row in 0..height as usize {
        let src = y_ptr.add(row * y_stride as usize);
        core::ptr::copy_nonoverlapping(
            src,
            out.as_mut_ptr().add(row * width as usize),
            width as usize,
        );
    }
    let mut o = y_bytes;
    for row in 0..(height as usize) / 2 {
        let u_row = u_ptr.add(row * u_stride as usize);
        let v_row = v_ptr.add(row * v_stride as usize);
        for col in 0..(width as usize) / 2 {
            out[o] = *u_row.add(col * u_pixel as usize);
            out[o + 1] = *v_row.add(col * v_pixel as usize);
            o += 2;
        }
    }
    Ok((width, height))
}

fn stats(frame: &[u8]) -> (u8, u8, f64) {
    if frame.is_empty() {
        return (0, 0, 0.0);
    }
    let mut min = 255u8;
    let mut max = 0u8;
    let mut sum = 0u64;
    for &b in frame {
        min = min.min(b);
        max = max.max(b);
        sum += b as u64;
    }
    (min, max, sum as f64 / frame.len() as f64)
}

struct Selftest {
    dir: std::path::PathBuf,
    want: usize,
    saved: usize,
    /// Camera id chosen by the marker (`camera=1`); None picks the back camera.
    camera: Option<String>,
    /// Diagnostic-only torch mode inside the active Camera2 session.
    torch: bool,
}

/// `AImage_getTimestamp` reports nanoseconds since boot. Return the frame
/// cadence and timestamp span without ever dividing by a zero/invalid span.
fn selftest_timing(frame_count: usize, first_ts_ns: i64, last_ts_ns: i64) -> (f64, f64) {
    let span_ns = last_ts_ns.saturating_sub(first_ts_ns);
    if frame_count < 2 || span_ns <= 0 {
        return (0.0, 0.0);
    }
    let fps = (frame_count - 1) as f64 * 1_000_000_000.0 / span_ns as f64;
    (fps, span_ns as f64 / 1_000_000.0)
}

/// One open camera pipeline: device, capture session, ImageReader and the
/// repeating request. `next_frame` is the only frame source both the self-test
/// and the socket server use, so there is exactly one conversion path.
struct CameraSession {
    device: *mut ACameraDevice,
    session: *mut ACameraCaptureSession,
    reader: *mut AImageReader,
    image_listener: ImageListener,
    capture_cbs: CaptureCallbacks,
    window: *mut ANativeWindow,
    request: *mut ACaptureRequest,
    target: *mut ACameraOutputTarget,
    output: *mut ACaptureSessionOutput,
    container: *mut ACaptureSessionOutputContainer,
    looper: *mut ALooper,
    width: i32,
    height: i32,
    seq: u64,
    last_acquire_rc: i32,
}

unsafe fn submit_repeating(
    api: &CameraApi,
    session: *mut ACameraCaptureSession,
    request: *mut ACaptureRequest,
    callbacks: *mut CaptureCallbacks,
) -> (i32, c_int) {
    let mut requests = [request];
    let mut sequence = -1;
    let rc =
        (api.session_set_repeating)(session, callbacks, 1, requests.as_mut_ptr(), &mut sequence);
    (rc, sequence)
}

impl CameraSession {
    unsafe fn open(
        api: &CameraApi,
        manager: *mut ACameraManager,
        camera_id: &str,
        width: i32,
        height: i32,
        torch: bool,
    ) -> Result<Self, String> {
        CAMERA_API_FOR_CALLBACKS.store(api as *const CameraApi as *mut CameraApi, Ordering::SeqCst);
        let legacy_session = std::env::var_os("HUSK_CAMERA_LEGACY_SESSION").is_some();
        let looper = (api.looper_prepare)(ALOOPER_PREPARE_ALLOW_NON_CALLBACKS);
        let mut reader: *mut AImageReader = core::ptr::null_mut();
        let rc = (api.reader_new)(width, height, AIMAGE_FORMAT_YUV_420_888, 3, &mut reader);
        if rc != AMEDIA_OK || reader.is_null() {
            return Err(format!("AImageReader_new({width}x{height}) rc={rc}"));
        }
        let mut window: *mut ANativeWindow = core::ptr::null_mut();
        if (api.reader_get_window)(reader, &mut window) != AMEDIA_OK || window.is_null() {
            (api.reader_delete)(reader);
            return Err("AImageReader_getWindow".into());
        }
        let geometry_before = (
            (api.window_get_width)(window),
            (api.window_get_height)(window),
            (api.window_get_format)(window),
        );
        let geometry_rc = if legacy_session {
            -1
        } else {
            (api.window_set_buffers_geometry)(window, width, height, AIMAGE_FORMAT_YUV_420_888)
        };
        let geometry_after = (
            (api.window_get_width)(window),
            (api.window_get_height)(window),
            (api.window_get_format)(window),
        );
        crate::log(&format!(
            "camera|reader|geometry|before={}x{}/{}|set_rc={geometry_rc}|after={}x{}/{}",
            geometry_before.0,
            geometry_before.1,
            geometry_before.2,
            geometry_after.0,
            geometry_after.1,
            geometry_after.2
        ));
        let mut image_listener = ImageListener {
            context: core::ptr::null_mut(),
            on_image_available: Some(on_image_available),
        };
        let listener_rc = if legacy_session {
            -1
        } else {
            (api.reader_set_image_listener)(reader, &mut image_listener)
        };
        crate::log(&format!(
            "camera|reader|listener|rc={listener_rc}|legacy_session={legacy_session}"
        ));
        let mut capture_cbs = CaptureCallbacks {
            context: core::ptr::null_mut(),
            on_capture_started: Some(on_capture_started),
            on_capture_progressed: Some(on_capture_progressed),
            on_capture_completed: Some(on_capture_completed),
            on_capture_failed: Some(on_capture_failed),
            on_capture_sequence_completed: Some(on_capture_sequence_completed),
            on_capture_sequence_aborted: Some(on_capture_sequence_aborted),
            on_capture_buffer_lost: Some(on_capture_buffer_lost),
        };
        let mut callbacks = DeviceCallbacks {
            context: core::ptr::null_mut(),
            on_disconnected: Some(on_device_disconnected),
            on_error: Some(on_device_error),
            _reserved: core::ptr::null_mut(),
        };
        let cid = CString::new(camera_id).map_err(|e| format!("camera id: {e}"))?;
        let mut device: *mut ACameraDevice = core::ptr::null_mut();
        let open_rc = (api.open_camera)(manager, cid.as_ptr(), &mut callbacks, &mut device);
        if open_rc != ACAMERA_OK || device.is_null() {
            (api.reader_delete)(reader);
            return Err(format!("openCamera({camera_id}) rc={open_rc}"));
        }
        let mut container: *mut ACaptureSessionOutputContainer = core::ptr::null_mut();
        let mut output: *mut ACaptureSessionOutput = core::ptr::null_mut();
        let container_rc = (api.container_create)(&mut container);
        let output_rc = (api.session_output_create)(window, &mut output);
        let add_rc = (api.container_add)(container, output);
        crate::log(&format!(
            "camera|session|output_setup|container_rc={container_rc}|output_rc={output_rc}|add_rc={add_rc}|container_null={}|output_null={}",
            container.is_null(),
            output.is_null()
        ));
        let mut session_cbs = SessionCallbacks {
            context: core::ptr::null_mut(),
            on_closed: Some(on_session_closed),
            on_ready: Some(on_session_ready),
            on_active: Some(on_session_active),
        };
        let mut session: *mut ACameraCaptureSession = core::ptr::null_mut();
        // This ROM never delivers on_ready to the NDK state callback (proved in
        // evidence 89 and reproduced again in evidence 367). Pump once only to
        // give an immediate callback a chance, then follow the historical
        // successful path: submit directly and let the repeating-request return
        // code plus real frame acquisition decide whether the session works.
        SESSION_READY.store(false, Ordering::SeqCst);
        let session_rc = (api.session_create)(device, container, &mut session_cbs, &mut session);
        if session_rc != ACAMERA_OK || session.is_null() {
            (api.session_output_free)(output);
            (api.container_free)(container);
            (api.device_close)(device);
            (api.reader_delete)(reader);
            return Err(format!("createCaptureSession rc={session_rc}"));
        }
        // Match the historical success path: give the looper a short chance
        // to deliver session_active, but never make that callback a hard gate.
        // This ROM does not deliver on_ready, so do not wait for it here.
        SESSION_ACTIVE.store(false, Ordering::SeqCst);
        if !legacy_session {
            let active_deadline = Instant::now() + Duration::from_millis(200);
            while !SESSION_ACTIVE.load(Ordering::SeqCst) && Instant::now() < active_deadline {
                (api.looper_poll_once)(
                    16,
                    core::ptr::null_mut(),
                    core::ptr::null_mut(),
                    core::ptr::null_mut(),
                );
            }
        }
        let ready_observed = SESSION_READY.load(Ordering::SeqCst);
        let active_observed = SESSION_ACTIVE.load(Ordering::SeqCst);
        crate::log(&format!(
            "camera|session|state_observed|ready={ready_observed}|active={active_observed}|legacy_session={legacy_session}"
        ));
        let use_preview_template = std::env::var_os("HUSK_CAMERA_TEMPLATE_PREVIEW").is_some();
        let request_template = if use_preview_template {
            ACAMERA_TEMPLATE_PREVIEW
        } else {
            ACAMERA_TEMPLATE_RECORD
        };
        let mut request: *mut ACaptureRequest = core::ptr::null_mut();
        let request_rc = (api.device_create_request)(device, request_template, &mut request);
        let mut target: *mut ACameraOutputTarget = core::ptr::null_mut();
        let target_rc = (api.output_target_create)(window, &mut target);
        let add_target_rc = (api.request_add_target)(request, target);
        crate::log(&format!(
            "camera|session|request_setup|template={request_template}|request_rc={request_rc}|target_rc={target_rc}|add_target_rc={add_target_rc}|request_null={}|target_null={}",
            request.is_null(),
            target.is_null()
        ));
        let set_mode_rc;
        let set_ae_rc;
        let set_flash_rc;
        if let Some(set_u8) = api.request_set_u8 {
            let auto = [CONTROL_MODE_AUTO];
            set_mode_rc = set_u8(request, TAG_CONTROL_MODE, 1, auto.as_ptr());
            let ae = [CONTROL_AE_MODE_ON];
            set_ae_rc = set_u8(request, TAG_CONTROL_AE_MODE, 1, ae.as_ptr());
            let flash = [if torch { FLASH_MODE_TORCH } else { 0 }];
            set_flash_rc = set_u8(request, TAG_FLASH_MODE, 1, flash.as_ptr());
        } else {
            set_mode_rc = -1;
            set_ae_rc = -1;
            set_flash_rc = -1;
        }
        crate::log(&format!(
            "camera|session|request_entries|control_mode_rc={set_mode_rc}|ae_mode_rc={set_ae_rc}|flash_mode_rc={set_flash_rc}|torch={torch}"
        ));
        let oneshot = std::env::var_os("HUSK_CAMERA_ONESHOT").is_some();
        let (repeat_rc, sequence) = if oneshot {
            let mut requests = [request];
            let mut sequence = -1;
            let callbacks_ptr = if legacy_session {
                core::ptr::null_mut()
            } else {
                &mut capture_cbs
            };
            let rc = (api.session_capture)(
                session,
                callbacks_ptr,
                1,
                requests.as_mut_ptr(),
                &mut sequence,
            );
            (rc, sequence)
        } else {
            let callbacks_ptr = if legacy_session {
                core::ptr::null_mut()
            } else {
                &mut capture_cbs
            };
            submit_repeating(api, session, request, callbacks_ptr)
        };
        crate::log(&format!(
            "camera|session|open|id={camera_id}|{width}x{height}|mode={}|repeat_rc={repeat_rc}|sequence={sequence}",
            if oneshot { "oneshot" } else { "repeating" }
        ));
        Ok(Self {
            device,
            session,
            reader,
            image_listener,
            capture_cbs,
            window,
            request,
            target,
            output,
            container,
            looper,
            width,
            height,
            seq: 0,
            last_acquire_rc: i32::MIN,
        })
    }

    /// Pump the looper once and try to take a frame. `Ok(None)` means "nothing
    /// yet"; the caller decides how long to keep trying.
    unsafe fn try_frame(
        &mut self,
        api: &CameraApi,
        frame: &mut Vec<u8>,
    ) -> Result<Option<(i32, i32, i64)>, String> {
        if !self.looper.is_null() {
            (api.looper_poll_once)(
                8,
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                core::ptr::null_mut(),
            );
        }
        let mut image: *mut AImage = core::ptr::null_mut();
        let rc = (api.reader_acquire_next)(self.reader, &mut image);
        if rc != AMEDIA_OK || image.is_null() {
            if rc != self.last_acquire_rc {
                crate::log(&format!(
                    "camera|frame|acquire|rc={rc}|image_null={}",
                    image.is_null()
                ));
                self.last_acquire_rc = rc;
            }
            return Ok(None);
        }
        if self.last_acquire_rc != AMEDIA_OK {
            crate::log("camera|frame|acquire|recovered");
            self.last_acquire_rc = AMEDIA_OK;
        }
        let mut ts = 0i64;
        (api.image_get_timestamp)(image, &mut ts);
        let result = to_nv12(api, image, frame);
        (api.image_delete)(image);
        let (width, height) = result?;
        self.seq += 1;
        if self.seq == 1 || self.seq % 30 == 0 {
            let (min, max, mean) = stats(frame);
            let head = &frame[..frame.len().min(4096)];
            let (head_min, head_max, head_mean) = stats(head);
            let head_sum = (head_mean * head.len() as f64) as u64;
            crate::log(&format!(
                "camera|frame|ready|seq={}|{width}x{height}|ts={ts}|bytes={}|min={min}|max={max}|mean={mean:.2}|head_min={head_min}|head_max={head_max}|head_sum={head_sum}",
                self.seq,
                frame.len()
            ));
        }
        Ok(Some((width, height, ts)))
    }

    unsafe fn next_frame(
        &mut self,
        api: &CameraApi,
        frame: &mut Vec<u8>,
        timeout: Duration,
    ) -> Result<Option<(i32, i32, i64)>, String> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(got) = self.try_frame(api, frame)? {
                return Ok(Some(got));
            }
            if Instant::now() > deadline {
                return Ok(None);
            }
        }
    }

    unsafe fn close(self, api: &CameraApi) {
        (api.session_stop_repeating)(self.session);
        (api.session_close)(self.session);
        (api.request_free)(self.request);
        (api.output_target_free)(self.target);
        (api.session_output_free)(self.output);
        (api.container_free)(self.container);
        (api.device_close)(self.device);
        (api.reader_delete)(self.reader);
        crate::log("camera|session|closed");
    }
}

unsafe fn run_selftest(
    api: &CameraApi,
    manager: *mut ACameraManager,
    cams: &[CameraInfo],
    selftest: &mut Selftest,
) {
    let picked = selftest
        .camera
        .as_deref()
        .and_then(|id| cams.iter().find(|c| c.id == id))
        .or_else(|| cams.iter().find(|c| c.facing == LENS_FACING_BACK))
        .or_else(|| cams.first());
    let Some(cam) = picked else {
        crate::log("camera|selftest|FAIL|no camera");
        return;
    };
    let (want_w, want_h) = cam
        .sizes
        .iter()
        .copied()
        .find(|(w, h)| *w <= 1280 && *h <= 720)
        .unwrap_or((640, 480));
    let mut session =
        match CameraSession::open(api, manager, &cam.id, want_w, want_h, selftest.torch) {
            Ok(session) => session,
            Err(e) => {
                crate::log(&format!("camera|selftest|FAIL|{e}"));
                return;
            }
        };
    let mut frame = Vec::new();
    let mut first_ts: i64 = 0;
    let mut last_ts: i64 = 0;
    let started = Instant::now();
    while selftest.saved < selftest.want && started.elapsed() < Duration::from_secs(12) {
        match session.next_frame(api, &mut frame, Duration::from_millis(500)) {
            Ok(Some((w, h, ts))) => {
                let (min, max, mean) = stats(&frame);
                if first_ts == 0 {
                    first_ts = ts;
                }
                last_ts = ts;
                let path = selftest
                    .dir
                    .join(format!("frame-{:02}.nv12", selftest.saved));
                let wrote = std::fs::write(&path, &frame).is_ok();
                crate::log(&format!(
                    "camera|selftest|frame={}|{}x{}|bytes={}|min={min}|max={max}|mean={mean:.2}|ts={ts}|saved={wrote}",
                    selftest.saved, w, h, frame.len()
                ));
                selftest.saved += 1;
            }
            Ok(None) => crate::log("camera|selftest|no_frame"),
            Err(e) => {
                crate::log(&format!("camera|selftest|convert_fail|{e}"));
                break;
            }
        }
    }
    let (fps, span_ms) = selftest_timing(selftest.saved, first_ts, last_ts);
    crate::log(&format!(
        "camera|selftest|DONE|frames={}|fps={fps:.2}|span_ms={:.2}|elapsed_ms={}",
        selftest.saved,
        span_ms,
        started.elapsed().as_millis()
    ));
    session.close(api);
}

#[cfg(test)]
mod tests {
    use super::selftest_timing;

    #[test]
    fn selftest_timing_uses_nanosecond_timestamps() {
        let (fps, span_ms) = selftest_timing(10, 1_000_000_000, 1_300_000_000);
        assert!((fps - 30.0).abs() < 0.001, "fps={fps}");
        assert!((span_ms - 300.0).abs() < 0.001, "span_ms={span_ms}");
    }

    #[test]
    fn selftest_timing_rejects_invalid_span() {
        assert_eq!(selftest_timing(10, 2_000, 2_000), (0.0, 0.0));
        assert_eq!(selftest_timing(1, 1_000, 2_000), (0.0, 0.0));
    }
}

// ---------------------------------------------------------------------------
// Guest-facing socket. Protocol (one client at a time):
//
//   OPEN w=1280 h=720 [id=1]   -> OK w=W h=H fourcc=NV12 bytes=B
//   FRAME                      -> FRAME seq=N ts=US bytes=B  + B raw NV12 bytes
//                              -> NOFRAME (if nothing within 1 s)
//   STOP                       -> BYE
// ---------------------------------------------------------------------------

const CAMERA_SOCKET_REL: &str = "run/husk/camera.sock";

fn read_command(stream: &mut std::os::unix::net::UnixStream) -> Result<String, String> {
    use std::io::Read;
    let mut out = Vec::with_capacity(64);
    let mut one = [0u8; 1];
    loop {
        stream
            .read_exact(&mut one)
            .map_err(|e| format!("read: {e}"))?;
        match one[0] {
            b'\n' => return Ok(String::from_utf8_lossy(&out).into_owned()),
            b'\r' => {}
            _ => {
                if out.len() > 512 {
                    return Err("command too long".into());
                }
                out.push(one[0]);
            }
        }
    }
}

fn field_value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    line.split_whitespace()
        .filter_map(|f| f.split_once('='))
        .find_map(|(k, v)| (k == key).then_some(v))
}

/// Shared handles used by the per-client camera threads. `ACameraManager` and
/// the NDK function table are process-wide; Camera2 itself serializes access
/// to each physical camera. The socket server must still accept independently
/// because WirePlumber keeps one persistent client open per `/dev/videoN`.
struct CameraRuntime {
    api: CameraApi,
    manager: *mut ACameraManager,
    cams: Vec<CameraInfo>,
    deny: bool,
}

// SAFETY: the function pointers and dlopen handles are immutable after startup.
// ACameraManager is documented as usable across camera client calls; sessions
// remain owned by the client thread that opens them.
unsafe impl Send for CameraRuntime {}
unsafe impl Sync for CameraRuntime {}

struct CameraClientGuard;

impl CameraClientGuard {
    fn acquire() -> Option<Self> {
        loop {
            let active = ACTIVE_CAMERA_CLIENTS.load(Ordering::SeqCst);
            if active >= MAX_CAMERA_CLIENTS {
                return None;
            }
            if ACTIVE_CAMERA_CLIENTS
                .compare_exchange(active, active + 1, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                return Some(Self);
            }
        }
    }
}

impl Drop for CameraClientGuard {
    fn drop(&mut self) {
        ACTIVE_CAMERA_CLIENTS.fetch_sub(1, Ordering::SeqCst);
    }
}

unsafe fn handle_client(
    api: &CameraApi,
    manager: *mut ACameraManager,
    cams: &[CameraInfo],
    deny: bool,
    mut stream: std::os::unix::net::UnixStream,
) {
    use std::io::Write;
    let mut session: Option<CameraSession> = None;
    let mut session_owner: Option<std::sync::MutexGuard<'static, ()>> = None;
    let mut frame = Vec::new();
    // Initial handshake gets a bounded wait. A stale accepted socket used to
    // block the single-threaded accept loop forever; once a valid command has
    // been served, a persistent WirePlumber client may remain idle indefinitely.
    let _ = stream.set_read_timeout(Some(CAMERA_HANDSHAKE_TIMEOUT));
    let _ = stream.set_write_timeout(Some(CAMERA_WRITE_TIMEOUT));
    let mut handshake_done = false;
    // A STOP acknowledgement is a lifecycle barrier, not just a socket reply.
    // V4L2 consumers such as PipeWire tear down one probe stream and immediately
    // start the real stream. Replying before CameraSession::close() releases the
    // global owner makes the next OPEN observe a stale busy state.
    let mut stop_requested = false;
    loop {
        let line = match read_command(&mut stream) {
            Ok(line) => line,
            Err(e) => {
                crate::log(&format!("camera|client|{e}"));
                break;
            }
        };
        let verb = line.split_whitespace().next().unwrap_or("");
        match verb {
            "INFO" if deny || cams.is_empty() => {
                handshake_done = true;
                let _ = stream.set_read_timeout(None);
                // Explicit degradation, not a silent empty list: the guest must
                // be able to tell "no permission" from "no camera".
                crate::log("camera|DENY|no permission (guest asked INFO)");
                let _ = stream.write_all(b"ERR no_permission\n");
            }
            "INFO" => {
                handshake_done = true;
                let _ = stream.set_read_timeout(None);
                crate::log("camera|client|INFO");
                let mut reply = String::new();
                for (k, cam) in ordered(cams).iter().enumerate() {
                    let facing = if cam.facing == LENS_FACING_BACK {
                        "back"
                    } else {
                        "front"
                    };
                    reply.push_str(&format!(
                        "CAM k={k} id={} facing={facing} name={} sizes=",
                        cam.id,
                        if cam.facing == LENS_FACING_BACK {
                            "BACK"
                        } else {
                            "FRONT"
                        }
                    ));
                    for (i, (w, h)) in cam.sizes.iter().enumerate() {
                        if i > 0 {
                            reply.push(',');
                        }
                        reply.push_str(&format!("{w}x{h}"));
                    }
                    reply.push('\n');
                }
                reply.push_str("OK\n");
                match stream.write_all(reply.as_bytes()) {
                    Ok(()) => crate::log("camera|client|INFO|ok"),
                    Err(e) => crate::log(&format!("camera|client|INFO|write_fail|{e}")),
                }
            }
            "OPEN" => {
                handshake_done = true;
                let _ = stream.set_read_timeout(None);
                if session.is_some() {
                    let _ = stream.write_all(b"ERR busy\n");
                    continue;
                }
                let owner = match CAMERA_SESSION_OWNER.try_lock() {
                    Ok(owner) => owner,
                    Err(_) => {
                        let _ = stream.write_all(b"ERR busy\n");
                        continue;
                    }
                };
                let want_w: i32 = field_value(&line, "w")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(1280);
                let want_h: i32 = field_value(&line, "h")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(720);
                let id = field_value(&line, "id").map(|v| v.to_string());
                let picked = id
                    .as_deref()
                    .and_then(|id| cams.iter().find(|c| c.id == id))
                    .or_else(|| cams.iter().find(|c| c.facing == LENS_FACING_BACK))
                    .or_else(|| cams.first());
                let Some(cam) = picked else {
                    let _ = stream.write_all(b"ERR no_camera\n");
                    continue;
                };
                // Snap the request to a size the device actually offers.
                let (w, h) = cam
                    .sizes
                    .iter()
                    .copied()
                    .find(|(w, h)| *w == want_w && *h == want_h)
                    .or_else(|| {
                        cam.sizes
                            .iter()
                            .copied()
                            .find(|(w, h)| *w <= want_w && *h <= want_h)
                    })
                    .unwrap_or((640, 480));
                match CameraSession::open(api, manager, &cam.id, w, h, false) {
                    Ok(open) => {
                        let bytes = w * h * 3 / 2;
                        let reply =
                            format!("OK w={w} h={h} fourcc=NV12 bytes={bytes} id={}\n", cam.id);
                        let _ = stream.write_all(reply.as_bytes());
                        session = Some(open);
                        session_owner = Some(owner);
                    }
                    Err(e) => {
                        crate::log(&format!("camera|client|open_fail|{e}"));
                        let _ = stream.write_all(format!("ERR {e}\n").as_bytes());
                    }
                }
            }
            "FRAME" => {
                let Some(open) = session.as_mut() else {
                    let _ = stream.write_all(b"ERR not_open\n");
                    continue;
                };
                match open.next_frame(api, &mut frame, Duration::from_millis(1000)) {
                    Ok(Some((w, h, ts))) => {
                        let header = format!(
                            "FRAME seq={} ts={} w={w} h={h} bytes={}\n",
                            open.seq,
                            ts,
                            frame.len()
                        );
                        if stream.write_all(header.as_bytes()).is_err()
                            || stream.write_all(&frame).is_err()
                        {
                            break;
                        }
                    }
                    Ok(None) => {
                        let _ = stream.write_all(b"NOFRAME\n");
                    }
                    Err(e) => {
                        let _ = stream.write_all(format!("ERR convert {e}\n").as_bytes());
                    }
                }
            }
            "STOP" => {
                stop_requested = true;
                break;
            }
            other => {
                let _ = stream.write_all(format!("ERR unknown {other}\n").as_bytes());
            }
        }
    }
    if let Some(open) = session.take() {
        open.close(api);
    }
    drop(session_owner);
    // Only now may another OPEN claim the process-wide Camera2 session owner.
    // Send BYE after the owner is released so STREAMOFF's caller cannot race
    // ahead into an EBUSY-starting probe/real-stream handoff.
    if stop_requested {
        let _ = stream.write_all(b"BYE\n");
    }
    crate::log(&format!(
        "camera|client|done|handshake={}",
        if handshake_done {
            "ok"
        } else {
            "timeout_or_eof"
        }
    ));
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Video device order: back cameras first (DroidNative does the same), so
/// `/dev/video0` is the main camera. Everything the guest can see (INFO lines,
/// sysfs tree) uses this order.
fn ordered(cams: &[CameraInfo]) -> Vec<&CameraInfo> {
    let mut v: Vec<&CameraInfo> = cams
        .iter()
        .filter(|c| c.facing == LENS_FACING_BACK)
        .collect();
    v.extend(cams.iter().filter(|c| c.facing != LENS_FACING_BACK));
    v
}

/// Build the guest V4L2 sysfs facade. The implementation lives in a host-
/// testable runtime crate so the real-root and stale-shadow regressions are
/// covered without compiling Android-only camera symbols on the build host.
fn prepare_v4l2_tree(rootfs: &std::path::Path, cams: &[CameraInfo]) {
    let ordered = ordered(cams);
    let labels: Vec<&str> = ordered
        .iter()
        .map(|cam| {
            if cam.facing == LENS_FACING_BACK {
                "BACK"
            } else {
                "FRONT"
            }
        })
        .collect();
    match camera_v4l2_tree::prepare(rootfs, &labels) {
        Ok(()) => crate::log(&format!("camera|v4l2|tree|devices={}", labels.len())),
        Err(e) => crate::log(&format!("camera|v4l2|tree|FAIL|{e}")),
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

/// Start the camera bridge thread once per process.
pub fn start_once() {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| {
        let _ = std::panic::catch_unwind(run);
    });
}

/// Same bridge with caller-supplied paths; used by the standalone
/// `husk-bridges` helper (no `ANativeActivity` in that process).
pub fn start_once_at(files: std::path::PathBuf, rootfs: std::path::PathBuf) {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    match unsafe { start_helper_binder_thread_pool() } {
        Ok(()) => crate::log("camera|binder|thread_pool_started"),
        Err(e) => crate::log(&format!("camera|binder|thread_pool_FAIL|{e}")),
    }
    std::thread::spawn(move || {
        let _ = std::panic::catch_unwind(|| run_at(files, rootfs));
    });
}

fn run() {
    let Some(files) = files_dir() else {
        crate::log("camera|SKIP|no files dir");
        return;
    };
    let rootfs = match crate::guest_rootfs::resolve(&files) {
        Ok(rootfs) => rootfs,
        Err(e) => {
            crate::log(&format!("camera|SKIP|{e}"));
            return;
        }
    };
    run_at(files, rootfs);
}

fn run_at(files: std::path::PathBuf, rootfs: std::path::PathBuf) {
    crate::log(&format!("camera|ROOTFS|{}", rootfs.display()));
    let api = unsafe {
        match CameraApi::load() {
            Ok(api) => api,
            Err(e) => {
                crate::log(&format!("camera|FAIL|{e}"));
                return;
            }
        }
    };
    let mut manager: *mut ACameraManager = core::ptr::null_mut();
    unsafe {
        // The NDK camera2 stack wants a Looper on the calling thread.
        let looper = (api.looper_prepare)(ALOOPER_PREPARE_ALLOW_NON_CALLBACKS);
        crate::log(&format!("camera|looper|{looper:p}"));
        // Two ABI shapes exist for `ACameraManager_create`: the documented
        // `camera_status_t f(ACameraManager**)` and a pointer-returning variant
        // that this ROM's libcamera2ndk.so uses. Try the documented one and fall
        // back to the returned pointer, logging which shape was taken.
        let raw_create: unsafe extern "C" fn(*mut *mut ACameraManager) -> u64 =
            core::mem::transmute(api.manager_create);
        let raw = raw_create(&mut manager);
        if manager.is_null() && raw != 0 {
            manager = raw as usize as *mut ACameraManager;
            crate::log("camera|manager|pointer-return-abi");
        }
        crate::log(&format!("camera|manager|{manager:p}|raw=0x{raw:x}"));
        if manager.is_null() {
            crate::log("camera|FAIL|ACameraManager_create produced no manager");
            return;
        }
        let cams = enumerate(&api, manager);
        crate::log(&format!("camera|list|count={}", cams.len()));
        // An empty list on a phone with cameras means the CAMERA permission is
        // not held (this ROM refuses `pm revoke`/`appops set` from adb, so the
        // marker is the only way to exercise the path without wiping app data).
        // `camera-deny` is a test hook; it never invents frames.
        let deny = cams.is_empty() || files.join("camera-deny").exists();
        if deny {
            crate::log(&format!(
                "camera|DENY|reason={}",
                if cams.is_empty() {
                    "no_permission"
                } else {
                    "marker"
                }
            ));
        }
        if let Ok(mode) = std::env::var("HUSK_CAMERA_TORCH") {
            if let Some(set_torch) = api.manager_set_torch_mode {
                if let Some(cam) = cams
                    .iter()
                    .find(|c| c.facing == LENS_FACING_BACK)
                    .or_else(|| cams.first())
                {
                    if let Ok(id) = CString::new(cam.id.as_str()) {
                        let on = mode != "0";
                        let rc = set_torch(
                            manager,
                            id.as_ptr(),
                            if on { TORCH_MODE_ON } else { TORCH_MODE_OFF },
                        );
                        crate::log(&format!("camera|torch|id={}|on={on}|rc={rc}", cam.id));
                    }
                }
            } else {
                crate::log("camera|torch|setTorchMode unavailable");
            }
        }
        prepare_v4l2_tree(&rootfs, &cams);

        // One-shot self test, only when its marker exists.
        let selftest_marker = files.join("camera-selftest");
        if selftest_marker.exists() {
            let dir = files.join("camera-frames");
            let _ = std::fs::create_dir_all(&dir);
            let marker_text = std::fs::read_to_string(&selftest_marker).unwrap_or_default();
            let want = marker_text
                .split_whitespace()
                .find_map(|w| w.strip_prefix("frames=").and_then(|v| v.parse().ok()))
                .unwrap_or(10usize);
            let camera = marker_text
                .split_whitespace()
                .find_map(|w| w.strip_prefix("camera=").map(|v| v.to_string()));
            let torch = marker_text
                .split_whitespace()
                .find_map(|w| w.strip_prefix("torch="))
                .is_some_and(|v| v != "0");
            let mut selftest = Selftest {
                dir,
                want,
                saved: 0,
                camera,
                torch,
            };
            run_selftest(&api, manager, &cams, &mut selftest);
        }

        // Guest-facing server: camera is a standard Linux device, not an
        // opt-in test feature. Bind for every session. If permission is absent,
        // OPEN still receives the explicit denial instead of a dead socket.
        let runtime = Arc::new(CameraRuntime {
            api,
            manager,
            cams,
            deny,
        });
        loop {
            let path = rootfs.join(CAMERA_SOCKET_REL);
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::remove_file(&path);
            let listener = match std::os::unix::net::UnixListener::bind(&path) {
                Ok(listener) => listener,
                Err(e) => {
                    crate::log(&format!("camera|FAIL|bind {e}"));
                    std::thread::sleep(Duration::from_secs(1));
                    continue;
                }
            };
            crate::log(&format!("camera|LISTEN|{}", path.display()));
            for stream in listener.incoming() {
                let stream = match stream {
                    Ok(stream) => stream,
                    Err(e) => {
                        crate::log(&format!("camera|FAIL|accept {e}"));
                        break;
                    }
                };
                let Some(guard) = CameraClientGuard::acquire() else {
                    crate::log(&format!("camera|client|reject|active={MAX_CAMERA_CLIENTS}"));
                    // Drop the stream immediately; the guest gets EOF and can
                    // surface EBUSY instead of hanging forever.
                    continue;
                };
                let runtime = Arc::clone(&runtime);
                if let Err(e) = std::thread::Builder::new()
                    .name("husk-camera-client".into())
                    .spawn(move || {
                        let _guard = guard;
                        unsafe {
                            (runtime.api.looper_prepare)(ALOOPER_PREPARE_ALLOW_NON_CALLBACKS);
                        }
                        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
                            handle_client(
                                &runtime.api,
                                runtime.manager,
                                &runtime.cams,
                                runtime.deny,
                                stream,
                            )
                        }));
                    })
                {
                    crate::log(&format!("camera|FAIL|client thread: {e}"));
                }
            }
            let _ = std::fs::remove_file(&path);
            crate::log("camera|REBIND|listener closed");
        }
    }
}
