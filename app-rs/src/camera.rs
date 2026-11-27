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

const LENS_FACING_BACK: u8 = 1;
const LENS_FACING_FRONT: u8 = 0;
const CONTROL_MODE_AUTO: u8 = 1;
const CONTROL_AE_MODE_ON: u8 = 1;

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

struct CameraApi {
    _camera_handle: *mut c_void,
    _media_handle: *mut c_void,
    _android_handle: *mut c_void,
    manager_create: unsafe extern "C" fn(*mut *mut ACameraManager) -> CameraStatus,
    manager_delete: unsafe extern "C" fn(*mut ACameraManager),
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
        *mut c_void,
        c_int,
        *mut *mut ACaptureRequest,
        *mut c_int,
    ) -> CameraStatus,
    session_stop_repeating: unsafe extern "C" fn(*mut ACameraCaptureSession) -> CameraStatus,
    reader_new: unsafe extern "C" fn(i32, i32, i32, i32, *mut *mut AImageReader) -> i32,
    reader_delete: unsafe extern "C" fn(*mut AImageReader),
    reader_get_window: unsafe extern "C" fn(*mut AImageReader, *mut *mut ANativeWindow) -> i32,
    reader_acquire_next: unsafe extern "C" fn(*mut AImageReader, *mut *mut AImage) -> i32,
    image_delete: unsafe extern "C" fn(*mut AImage),
    image_get_timestamp: unsafe extern "C" fn(*const AImage, *mut i64) -> i32,
    image_get_plane_data:
        unsafe extern "C" fn(*const AImage, c_int, *mut *mut u8, *mut usize) -> i32,
    image_get_plane_row_stride: unsafe extern "C" fn(*const AImage, c_int, *mut i32) -> i32,
    image_get_plane_pixel_stride: unsafe extern "C" fn(*const AImage, c_int, *mut i32) -> i32,
    image_get_width: unsafe extern "C" fn(*const AImage, *mut i32) -> i32,
    image_get_height: unsafe extern "C" fn(*const AImage, *mut i32) -> i32,
    looper_prepare: unsafe extern "C" fn(c_int) -> *mut ALooper,
    // int ALooper_pollOnce(int timeoutMillis, int* outFd, int* outEvents, void** outData)
    // There is no looper argument: it always polls the calling thread's looper.
    looper_poll_once:
        unsafe extern "C" fn(c_int, *mut c_int, *mut c_int, *mut *mut c_void) -> c_int,
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
            looper_prepare: core::mem::transmute(symbol(android_handle, c"ALooper_prepare")?),
            looper_poll_once: core::mem::transmute(symbol(android_handle, c"ALooper_pollOnce")?),
        })
    }
}

// ---------------------------------------------------------------------------
// Static hand-off between the looper thread and everyone else.
// ---------------------------------------------------------------------------

static STARTED: AtomicBool = AtomicBool::new(false);
static SESSION_READY: AtomicBool = AtomicBool::new(false);
static STOP: AtomicBool = AtomicBool::new(false);
static SESSION: AtomicPtr<ACameraCaptureSession> = AtomicPtr::new(core::ptr::null_mut());
static REQUEST: AtomicPtr<ACaptureRequest> = AtomicPtr::new(core::ptr::null_mut());
static DEVICE_ERROR: AtomicI32 = AtomicI32::new(0);
static FRAMES: AtomicUsize = AtomicUsize::new(0);

unsafe extern "C" fn on_device_disconnected(_ctx: *mut c_void, _device: *mut ACameraDevice) {
    crate::log("camera|device|disconnected");
}

unsafe extern "C" fn on_device_error(_ctx: *mut c_void, _device: *mut ACameraDevice, error: c_int) {
    DEVICE_ERROR.store(error, Ordering::SeqCst);
    crate::log(&format!("camera|device|error|{error}"));
}

unsafe extern "C" fn on_session_closed(_ctx: *mut c_void, _session: *mut ACameraCaptureSession) {
    SESSION_READY.store(false, Ordering::SeqCst);
}

unsafe extern "C" fn on_session_ready(_ctx: *mut c_void, _session: *mut ACameraCaptureSession) {
    SESSION_READY.store(true, Ordering::SeqCst);
    crate::log("camera|callback|session_ready");
}

unsafe extern "C" fn on_session_active(_ctx: *mut c_void, _session: *mut ACameraCaptureSession) {}

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
    window: *mut ANativeWindow,
    request: *mut ACaptureRequest,
    target: *mut ACameraOutputTarget,
    output: *mut ACaptureSessionOutput,
    container: *mut ACaptureSessionOutputContainer,
    looper: *mut ALooper,
    width: i32,
    height: i32,
    seq: u64,
}

impl CameraSession {
    unsafe fn open(
        api: &CameraApi,
        manager: *mut ACameraManager,
        camera_id: &str,
        width: i32,
        height: i32,
    ) -> Result<Self, String> {
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
        (api.container_create)(&mut container);
        (api.session_output_create)(window, &mut output);
        (api.container_add)(container, output);
        let mut session_cbs = SessionCallbacks {
            context: core::ptr::null_mut(),
            on_closed: Some(on_session_closed),
            on_ready: Some(on_session_ready),
            on_active: Some(on_session_active),
        };
        let mut session: *mut ACameraCaptureSession = core::ptr::null_mut();
        let session_rc = (api.session_create)(device, container, &mut session_cbs, &mut session);
        if session_rc != ACAMERA_OK || session.is_null() {
            (api.session_output_free)(output);
            (api.container_free)(container);
            (api.device_close)(device);
            (api.reader_delete)(reader);
            return Err(format!("createCaptureSession rc={session_rc}"));
        }
        let mut request: *mut ACaptureRequest = core::ptr::null_mut();
        (api.device_create_request)(device, ACAMERA_TEMPLATE_RECORD, &mut request);
        let mut target: *mut ACameraOutputTarget = core::ptr::null_mut();
        (api.output_target_create)(window, &mut target);
        (api.request_add_target)(request, target);
        if let Some(set_u8) = api.request_set_u8 {
            let auto = [CONTROL_MODE_AUTO];
            set_u8(request, TAG_CONTROL_MODE, 1, auto.as_ptr());
            let ae = [CONTROL_AE_MODE_ON];
            set_u8(request, TAG_CONTROL_AE_MODE, 1, ae.as_ptr());
        }
        let mut requests = [request];
        let repeat_rc = (api.session_set_repeating)(
            session,
            core::ptr::null_mut(),
            1,
            requests.as_mut_ptr(),
            core::ptr::null_mut(),
        );
        crate::log(&format!(
            "camera|session|open|id={camera_id}|{width}x{height}|repeat_rc={repeat_rc}"
        ));
        Ok(Self {
            device,
            session,
            reader,
            window,
            request,
            target,
            output,
            container,
            looper,
            width,
            height,
            seq: 0,
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
            return Ok(None);
        }
        let mut ts = 0i64;
        (api.image_get_timestamp)(image, &mut ts);
        let result = to_nv12(api, image, frame);
        (api.image_delete)(image);
        let (width, height) = result?;
        self.seq += 1;
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
    let mut session = match CameraSession::open(api, manager, &cam.id, want_w, want_h) {
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

unsafe fn handle_client(
    api: &CameraApi,
    manager: *mut ACameraManager,
    cams: &[CameraInfo],
    deny: bool,
    mut stream: std::os::unix::net::UnixStream,
) {
    use std::io::Write;
    let mut session: Option<CameraSession> = None;
    let mut frame = Vec::new();
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
                // Explicit degradation, not a silent empty list: the guest must
                // be able to tell "no permission" from "no camera".
                crate::log("camera|DENY|no permission (guest asked INFO)");
                let _ = stream.write_all(b"ERR no_permission\n");
            }
            "INFO" => {
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
                let _ = stream.write_all(reply.as_bytes());
            }
            "OPEN" => {
                if session.is_some() {
                    let _ = stream.write_all(b"ERR busy\n");
                    continue;
                }
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
                match CameraSession::open(api, manager, &cam.id, w, h) {
                    Ok(open) => {
                        let bytes = w * h * 3 / 2;
                        let reply =
                            format!("OK w={w} h={h} fourcc=NV12 bytes={bytes} id={}\n", cam.id);
                        let _ = stream.write_all(reply.as_bytes());
                        session = Some(open);
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
                let _ = stream.write_all(b"BYE\n");
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
    crate::log("camera|client|done");
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

/// The guest's `/sys` is Android's real sysfs (the kernel has no V4L2), so the
/// shim redirects `/sys/class/video4linux*` here. Same layout DroidNative binds:
/// class entry, the `/sys/subsystem` mirror eudev prefers, and the udev db entry.
fn prepare_v4l2_tree(rootfs: &std::path::Path, cams: &[CameraInfo]) {
    let class = rootfs.join("sys-v4l2/class");
    let devices = rootfs.join("sys-v4l2/subsystem/devices");
    let udev = rootfs.join("run/udev/data");
    // Camera permission can be revoked between launches. Rebuild the shadow
    // tree instead of leaving video nodes from an earlier permission state.
    let _ = std::fs::remove_dir_all(&class);
    let _ = std::fs::remove_dir_all(&devices);
    for d in [&class, &devices, &udev] {
        if let Err(e) = std::fs::create_dir_all(d) {
            crate::log(&format!("camera|v4l2|mkdir {d:?}: {e}"));
            return;
        }
    }
    if let Ok(entries) = std::fs::read_dir(&udev) {
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().starts_with("c81:") {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    for (k, cam) in ordered(cams).iter().enumerate() {
        let label = if cam.facing == LENS_FACING_BACK {
            "BACK"
        } else {
            "FRONT"
        };
        let uevent = format!("MAJOR=81\nMINOR={k}\nDEVNAME=video{k}\nDEVTYPE=video4linux\n");
        let dev = format!("81:{k}\n");
        let name = format!("{label}\n");
        let index = format!("{k}\n");
        for dir in [
            class.join(format!("video{k}")),
            devices.join(format!("video{k}")),
        ] {
            if let Err(e) = std::fs::create_dir_all(&dir) {
                crate::log(&format!("camera|v4l2|mkdir {dir:?}: {e}"));
                continue;
            }
            for (n, v) in [
                ("uevent", &uevent),
                ("dev", &dev),
                ("name", &name),
                ("index", &index),
            ] {
                let _ = std::fs::write(dir.join(n), v);
            }
        }
        let db = format!(
            "I:1\nE:ID_V4L_CAPABILITIES=:capture:\nE:ID_V4L_PRODUCT={label}\nE:DEVNAME=/dev/video{k}\nE:DEVTYPE=video4linux\n"
        );
        let _ = std::fs::write(udev.join(format!("c81:{k}")), db);
    }
    crate::log(&format!("camera|v4l2|tree|devices={}", cams.len()));
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
            let mut selftest = Selftest {
                dir,
                want,
                saved: 0,
                camera,
            };
            run_selftest(&api, manager, &cams, &mut selftest);
        }

        // Guest-facing server: camera is a standard Linux device, not an
        // opt-in test feature. Bind for every session. If permission is absent,
        // OPEN still receives the explicit denial instead of a dead socket.
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
                match stream {
                    Ok(stream) => handle_client(&api, manager, &cams, deny, stream),
                    Err(e) => {
                        crate::log(&format!("camera|FAIL|accept {e}"));
                        break;
                    }
                }
            }
            let _ = std::fs::remove_file(&path);
            crate::log("camera|REBIND|listener closed");
        }
    }
}
