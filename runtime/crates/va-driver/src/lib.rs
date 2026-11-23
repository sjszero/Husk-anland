//! A minimal Rust VA-API driver for Husk.
//!
//! This is intentionally a system-layer driver, not a player.  It lets
//! libva, GStreamer and Chromium see the same standard VA-API surface while
//! the actual MediaCodec transport is added behind it.

#![allow(non_camel_case_types, non_snake_case, non_upper_case_globals)]

mod h264;
#[allow(dead_code)]
mod va_bindings;
mod video_client;

use va_bindings::*;
use video_client::VideoClient;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

const VENDOR: &[u8] = b"husk\0";
const PROFILE_LIST: [VAProfile; 5] = [
    VAProfile_VAProfileH264Baseline,
    VAProfile_VAProfileH264Main,
    VAProfile_VAProfileH264High,
    VAProfile_VAProfileH264ConstrainedBaseline,
    VAProfile_VAProfileHEVCMain,
];
const NV12_FORMAT: VAImageFormat = VAImageFormat {
    fourcc: VA_FOURCC_NV12,
    byte_order: VA_LSB_FIRST,
    bits_per_pixel: 12,
    depth: 12,
    red_mask: 0,
    green_mask: 0,
    blue_mask: 0,
    alpha_mask: 0,
    va_reserved: [0; 4],
};

const MAX_SURFACE_DIMENSION: u32 = 8192;
const SURFACE_ATTRIBUTE_COUNT: u32 = 6;

#[allow(dead_code)]
struct SurfaceInfo {
    width: u32,
    height: u32,
    format: u32,
}

#[derive(Clone, Copy)]
struct ContextInfo {
    width: u32,
    height: u32,
    profile: VAProfile,
}

struct BufferInfo {
    type_: VABufferType,
    unit_size: u32,
    num_elements: u32,
    data: Vec<u8>,
}

struct ActivePicture {
    context: VAContextID,
    surface: VASurfaceID,
    buffers: Vec<VABufferID>,
}

#[derive(Default)]
struct ContextState {
    next_config: VAConfigID,
    next_surface: VASurfaceID,
    next_context: VAContextID,
    next_buffer: VABufferID,
    next_image: VAImageID,
    next_pts: i64,
    configs: HashMap<VAConfigID, (VAProfile, VAEntrypoint)>,
    surfaces: HashMap<VASurfaceID, SurfaceInfo>,
    contexts: HashMap<VAContextID, ContextInfo>,
    buffers: HashMap<VABufferID, BufferInfo>,
    picture_params: HashMap<VAContextID, Vec<u8>>,
    active_picture: Option<ActivePicture>,
    /// Tight NV12 for the last decoded picture of each surface.
    surface_frames: HashMap<VASurfaceID, SurfaceFrame>,
    /// PTS of the picture most recently submitted to each surface.
    surface_pts: HashMap<VASurfaceID, i64>,
    /// Images are backed by an ordinary buffer so `vaMapBuffer` needs no
    /// second code path; this map exists to validate destroy/map by image id.
    images: HashMap<VAImageID, ImageInfo>,
}

struct ImageInfo {
    buffer: VABufferID,
    width: u32,
    height: u32,
}

struct SurfaceFrame {
    nv12: Vec<u8>,
    width: u32,
    height: u32,
}

struct DriverContext {
    state: Mutex<ContextState>,
    video: Mutex<Option<VideoClient>>,
}

static CONTEXTS: OnceLock<Mutex<HashMap<usize, Arc<DriverContext>>>> = OnceLock::new();

fn contexts() -> &'static Mutex<HashMap<usize, Arc<DriverContext>>> {
    CONTEXTS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn register_context(ctx: VADriverContextP) -> bool {
    let Ok(mut map) = contexts().lock() else {
        return false;
    };
    map.insert(
        ctx as usize,
        Arc::new(DriverContext {
            state: Mutex::new(ContextState::default()),
            video: Mutex::new(None),
        }),
    );
    true
}

fn unregister_context(ctx: VADriverContextP) {
    if let Ok(mut map) = contexts().lock() {
        map.remove(&(ctx as usize));
    }
}

fn driver_context(ctx: VADriverContextP) -> Option<Arc<DriverContext>> {
    contexts()
        .lock()
        .ok()
        .and_then(|map| map.get(&(ctx as usize)).cloned())
}

fn with_state<T>(ctx: VADriverContextP, f: impl FnOnce(&mut ContextState) -> T) -> Option<T> {
    let context = driver_context(ctx)?;
    let mut state = context.state.lock().ok()?;
    Some(f(&mut state))
}

fn alloc_id(next: &mut u32) -> u32 {
    let id = (*next).max(1);
    *next = id.wrapping_add(1);
    if *next == 0 {
        *next = 1;
    }
    id
}

fn int_attr(type_: VASurfaceAttribType, flags: u32, value: i32) -> VASurfaceAttrib {
    VASurfaceAttrib {
        type_,
        flags,
        value: VAGenericValue {
            type_: VAGenericValueType_VAGenericValueTypeInteger,
            value: _VAGenericValue__bindgen_ty_1 { i: value },
        },
    }
}

unsafe extern "C" fn unimplemented() -> VAStatus {
    VA_STATUS_ERROR_UNIMPLEMENTED as VAStatus
}

unsafe extern "C" fn terminate(ctx: VADriverContextP) -> VAStatus {
    unregister_context(ctx);
    VA_STATUS_SUCCESS as VAStatus
}

unsafe extern "C" fn query_profiles(
    _ctx: VADriverContextP,
    profiles: *mut VAProfile,
    count: *mut i32,
) -> VAStatus {
    if count.is_null() {
        return VA_STATUS_ERROR_INVALID_PARAMETER as VAStatus;
    }
    *count = PROFILE_LIST.len() as i32;
    if !profiles.is_null() {
        for (i, p) in PROFILE_LIST.iter().enumerate() {
            *profiles.add(i) = *p;
        }
    }
    VA_STATUS_SUCCESS as VAStatus
}

unsafe extern "C" fn query_entrypoints(
    _ctx: VADriverContextP,
    profile: VAProfile,
    entrypoints: *mut VAEntrypoint,
    count: *mut i32,
) -> VAStatus {
    if count.is_null() {
        return VA_STATUS_ERROR_INVALID_PARAMETER as VAStatus;
    }
    if !PROFILE_LIST.contains(&profile) {
        *count = 0;
        return VA_STATUS_ERROR_UNSUPPORTED_PROFILE as VAStatus;
    }
    *count = 1;
    if !entrypoints.is_null() {
        *entrypoints = VAEntrypoint_VAEntrypointVLD;
    }
    VA_STATUS_SUCCESS as VAStatus
}

unsafe extern "C" fn query_image_formats(
    _ctx: VADriverContextP,
    formats: *mut VAImageFormat,
    count: *mut i32,
) -> VAStatus {
    if count.is_null() {
        return VA_STATUS_ERROR_INVALID_PARAMETER as VAStatus;
    }
    *count = 1;
    if !formats.is_null() {
        *formats = NV12_FORMAT;
    }
    VA_STATUS_SUCCESS as VAStatus
}

unsafe extern "C" fn get_config_attributes(
    _ctx: VADriverContextP,
    _profile: VAProfile,
    _entrypoint: VAEntrypoint,
    attrs: *mut VAConfigAttrib,
    count: i32,
) -> VAStatus {
    if attrs.is_null() && count > 0 {
        return VA_STATUS_ERROR_INVALID_PARAMETER as VAStatus;
    }
    for i in 0..count.max(0) as usize {
        let a = attrs.add(i);
        match (*a).type_ {
            VAConfigAttribType_VAConfigAttribRTFormat => {
                (*a).value = VA_RT_FORMAT_YUV420;
            }
            VAConfigAttribType_VAConfigAttribMaxPictureWidth
            | VAConfigAttribType_VAConfigAttribMaxPictureHeight => {
                (*a).value = MAX_SURFACE_DIMENSION;
            }
            _ => {
                (*a).value = 0;
            }
        }
    }
    VA_STATUS_SUCCESS as VAStatus
}

unsafe extern "C" fn create_config(
    _ctx: VADriverContextP,
    _profile: VAProfile,
    _entrypoint: VAEntrypoint,
    _attrs: *mut VAConfigAttrib,
    _count: i32,
    config: *mut VAConfigID,
) -> VAStatus {
    if config.is_null() || !PROFILE_LIST.contains(&_profile) {
        return VA_STATUS_ERROR_INVALID_PARAMETER as VAStatus;
    }
    if _entrypoint != VAEntrypoint_VAEntrypointVLD {
        return VA_STATUS_ERROR_UNSUPPORTED_ENTRYPOINT as VAStatus;
    }
    let Some(rc) = with_state(_ctx, |state| {
        let id = alloc_id(&mut state.next_config);
        state.configs.insert(id, (_profile, _entrypoint));
        *config = id;
        VA_STATUS_SUCCESS as VAStatus
    }) else {
        return VA_STATUS_ERROR_INVALID_CONTEXT as VAStatus;
    };
    rc
}

unsafe extern "C" fn destroy_config(ctx: VADriverContextP, config: VAConfigID) -> VAStatus {
    let Some(found) = with_state(ctx, |state| state.configs.remove(&config).is_some()) else {
        return VA_STATUS_ERROR_INVALID_CONTEXT as VAStatus;
    };
    if found {
        VA_STATUS_SUCCESS as VAStatus
    } else {
        VA_STATUS_ERROR_INVALID_CONFIG as VAStatus
    }
}

unsafe extern "C" fn query_config_attributes(
    _ctx: VADriverContextP,
    _config: VAConfigID,
    profile: *mut VAProfile,
    entrypoint: *mut VAEntrypoint,
    attrs: *mut VAConfigAttrib,
    count: *mut i32,
) -> VAStatus {
    let Some(config_info) = with_state(_ctx, |state| state.configs.get(&_config).copied()) else {
        return VA_STATUS_ERROR_INVALID_CONTEXT as VAStatus;
    };
    let Some((stored_profile, stored_entrypoint)) = config_info else {
        return VA_STATUS_ERROR_INVALID_CONFIG as VAStatus;
    };
    if !profile.is_null() {
        *profile = stored_profile;
    }
    if !entrypoint.is_null() {
        *entrypoint = stored_entrypoint;
    }
    if !attrs.is_null() {
        let n = count.as_ref().copied().unwrap_or(0).max(0) as usize;
        for i in 0..n {
            (*attrs.add(i)).value = 0;
        }
    }
    if !count.is_null() {
        *count = 0;
    }
    VA_STATUS_SUCCESS as VAStatus
}

unsafe extern "C" fn query_surface_attributes(
    ctx: VADriverContextP,
    config: VAConfigID,
    attrs: *mut VASurfaceAttrib,
    count: *mut u32,
) -> VAStatus {
    if count.is_null() {
        return VA_STATUS_ERROR_INVALID_PARAMETER as VAStatus;
    }
    let Some(valid_config) = with_state(ctx, |state| state.configs.contains_key(&config)) else {
        return VA_STATUS_ERROR_INVALID_CONTEXT as VAStatus;
    };
    if !valid_config {
        return VA_STATUS_ERROR_INVALID_CONFIG as VAStatus;
    }

    let capacity = *count;
    *count = SURFACE_ATTRIBUTE_COUNT;
    if attrs.is_null() || capacity == 0 {
        return VA_STATUS_SUCCESS as VAStatus;
    }
    if capacity < SURFACE_ATTRIBUTE_COUNT {
        return VA_STATUS_ERROR_MAX_NUM_EXCEEDED as VAStatus;
    }

    let values = [
        int_attr(
            VASurfaceAttribType_VASurfaceAttribPixelFormat,
            VA_SURFACE_ATTRIB_GETTABLE | VA_SURFACE_ATTRIB_SETTABLE,
            VA_FOURCC_NV12 as i32,
        ),
        int_attr(
            VASurfaceAttribType_VASurfaceAttribMinWidth,
            VA_SURFACE_ATTRIB_GETTABLE,
            1,
        ),
        int_attr(
            VASurfaceAttribType_VASurfaceAttribMaxWidth,
            VA_SURFACE_ATTRIB_GETTABLE,
            MAX_SURFACE_DIMENSION as i32,
        ),
        int_attr(
            VASurfaceAttribType_VASurfaceAttribMinHeight,
            VA_SURFACE_ATTRIB_GETTABLE,
            1,
        ),
        int_attr(
            VASurfaceAttribType_VASurfaceAttribMaxHeight,
            VA_SURFACE_ATTRIB_GETTABLE,
            MAX_SURFACE_DIMENSION as i32,
        ),
        int_attr(
            VASurfaceAttribType_VASurfaceAttribMemoryType,
            VA_SURFACE_ATTRIB_GETTABLE,
            VA_SURFACE_ATTRIB_MEM_TYPE_VA as i32,
        ),
    ];
    for (index, value) in values.into_iter().enumerate() {
        *attrs.add(index) = value;
    }
    VA_STATUS_SUCCESS as VAStatus
}

unsafe fn create_surfaces_impl(
    ctx: VADriverContextP,
    format: u32,
    width: u32,
    height: u32,
    surfaces: *mut VASurfaceID,
    num_surfaces: u32,
    attrs: *mut VASurfaceAttrib,
    num_attrs: u32,
) -> VAStatus {
    if surfaces.is_null()
        || num_surfaces == 0
        || width == 0
        || height == 0
        || width > MAX_SURFACE_DIMENSION
        || height > MAX_SURFACE_DIMENSION
    {
        return VA_STATUS_ERROR_INVALID_PARAMETER as VAStatus;
    }
    if format & VA_RT_FORMAT_YUV420 == 0 {
        return VA_STATUS_ERROR_UNSUPPORTED_RT_FORMAT as VAStatus;
    }
    if num_attrs > 0 && attrs.is_null() {
        return VA_STATUS_ERROR_INVALID_PARAMETER as VAStatus;
    }
    for index in 0..num_attrs as usize {
        let attr = &*attrs.add(index);
        match attr.type_ {
            VASurfaceAttribType_VASurfaceAttribPixelFormat => {
                if attr.value.type_ != VAGenericValueType_VAGenericValueTypeInteger
                    || attr.value.value.i as u32 != VA_FOURCC_NV12
                {
                    return VA_STATUS_ERROR_INVALID_IMAGE_FORMAT as VAStatus;
                }
            }
            VASurfaceAttribType_VASurfaceAttribUsageHint
            | VASurfaceAttribType_VASurfaceAttribMinWidth
            | VASurfaceAttribType_VASurfaceAttribMaxWidth
            | VASurfaceAttribType_VASurfaceAttribMinHeight
            | VASurfaceAttribType_VASurfaceAttribMaxHeight => {}
            VASurfaceAttribType_VASurfaceAttribMemoryType => {
                if attr.value.type_ != VAGenericValueType_VAGenericValueTypeInteger
                    || attr.value.value.i as u32 & VA_SURFACE_ATTRIB_MEM_TYPE_VA == 0
                {
                    return VA_STATUS_ERROR_UNSUPPORTED_MEMORY_TYPE as VAStatus;
                }
            }
            _ => return VA_STATUS_ERROR_ATTR_NOT_SUPPORTED as VAStatus,
        }
    }

    let Some(ids) = with_state(ctx, |state| {
        let mut ids = Vec::with_capacity(num_surfaces as usize);
        for _ in 0..num_surfaces {
            let id = alloc_id(&mut state.next_surface);
            state.surfaces.insert(
                id,
                SurfaceInfo {
                    width,
                    height,
                    format,
                },
            );
            ids.push(id);
        }
        ids
    }) else {
        return VA_STATUS_ERROR_INVALID_CONTEXT as VAStatus;
    };
    for (index, id) in ids.into_iter().enumerate() {
        *surfaces.add(index) = id;
    }
    VA_STATUS_SUCCESS as VAStatus
}

unsafe extern "C" fn create_surfaces(
    ctx: VADriverContextP,
    width: i32,
    height: i32,
    format: i32,
    num_surfaces: i32,
    surfaces: *mut VASurfaceID,
) -> VAStatus {
    if width <= 0 || height <= 0 || format < 0 || num_surfaces <= 0 {
        return VA_STATUS_ERROR_INVALID_PARAMETER as VAStatus;
    }
    create_surfaces_impl(
        ctx,
        format as u32,
        width as u32,
        height as u32,
        surfaces,
        num_surfaces as u32,
        core::ptr::null_mut(),
        0,
    )
}

unsafe extern "C" fn create_surfaces2(
    ctx: VADriverContextP,
    format: u32,
    width: u32,
    height: u32,
    surfaces: *mut VASurfaceID,
    num_surfaces: u32,
    attrs: *mut VASurfaceAttrib,
    num_attrs: u32,
) -> VAStatus {
    create_surfaces_impl(
        ctx,
        format,
        width,
        height,
        surfaces,
        num_surfaces,
        attrs,
        num_attrs,
    )
}

unsafe extern "C" fn destroy_surfaces(
    ctx: VADriverContextP,
    surfaces: *mut VASurfaceID,
    num_surfaces: i32,
) -> VAStatus {
    if surfaces.is_null() || num_surfaces <= 0 {
        return VA_STATUS_ERROR_INVALID_PARAMETER as VAStatus;
    }
    let Some(all_found) = with_state(ctx, |state| {
        let mut all_found = true;
        for index in 0..num_surfaces as usize {
            all_found &= state.surfaces.remove(&*surfaces.add(index)).is_some();
        }
        all_found
    }) else {
        return VA_STATUS_ERROR_INVALID_CONTEXT as VAStatus;
    };
    if all_found {
        VA_STATUS_SUCCESS as VAStatus
    } else {
        VA_STATUS_ERROR_INVALID_SURFACE as VAStatus
    }
}

unsafe extern "C" fn create_context(
    ctx: VADriverContextP,
    config: VAConfigID,
    width: i32,
    height: i32,
    _flags: i32,
    _render_targets: *mut VASurfaceID,
    _num_render_targets: i32,
    context: *mut VAContextID,
) -> VAStatus {
    if context.is_null() || width <= 0 || height <= 0 {
        return VA_STATUS_ERROR_INVALID_PARAMETER as VAStatus;
    }
    let Some(rc) = with_state(ctx, |state| {
        let Some((profile, _entrypoint)) = state.configs.get(&config).copied() else {
            return VA_STATUS_ERROR_INVALID_CONFIG as VAStatus;
        };
        let id = alloc_id(&mut state.next_context);
        state.contexts.insert(
            id,
            ContextInfo {
                width: width as u32,
                height: height as u32,
                profile,
            },
        );
        *context = id;
        VA_STATUS_SUCCESS as VAStatus
    }) else {
        return VA_STATUS_ERROR_INVALID_CONTEXT as VAStatus;
    };
    rc
}

unsafe extern "C" fn destroy_context(ctx: VADriverContextP, context: VAContextID) -> VAStatus {
    let Some(found) = with_state(ctx, |state| state.contexts.remove(&context).is_some()) else {
        return VA_STATUS_ERROR_INVALID_CONTEXT as VAStatus;
    };
    if found {
        VA_STATUS_SUCCESS as VAStatus
    } else {
        VA_STATUS_ERROR_INVALID_CONTEXT as VAStatus
    }
}

unsafe extern "C" fn create_buffer(
    ctx: VADriverContextP,
    context: VAContextID,
    type_: VABufferType,
    size: u32,
    num_elements: u32,
    data: *mut core::ffi::c_void,
    buffer: *mut VABufferID,
) -> VAStatus {
    if buffer.is_null() {
        return VA_STATUS_ERROR_INVALID_PARAMETER as VAStatus;
    }
    let Some(bytes) = (size as usize).checked_mul(num_elements as usize) else {
        return VA_STATUS_ERROR_ALLOCATION_FAILED as VAStatus;
    };
    if bytes > 64 * 1024 * 1024 {
        return VA_STATUS_ERROR_ALLOCATION_FAILED as VAStatus;
    }
    let mut payload = vec![0u8; bytes];
    if !data.is_null() && bytes > 0 {
        core::ptr::copy_nonoverlapping(data.cast::<u8>(), payload.as_mut_ptr(), bytes);
    }
    // Guest-side writes to the shared run directory are not reliable, so the
    // picture parameter buffer is dumped on stdout when tracing. It is 672
    // bytes, which fits in one log line.
    if std::env::var_os("HUSK_VA_TRACE").is_some()
        && type_ == VABufferType_VAPictureParameterBufferType
    {
        let mut hex = String::with_capacity(payload.len() * 2);
        for byte in &payload {
            hex.push_str(&format!("{byte:02x}"));
        }
        eprintln!("HUSK_VA_PP bytes={} hex={hex}", payload.len());
    }
    if std::env::var_os("HUSK_VA_TRACE").is_some() {
        let prefix = payload
            .iter()
            .take(16)
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        eprintln!(
            "HUSK_VA_BUFFER context={context} type={type_} size={size} elements={num_elements} bytes={bytes} data_null={} prefix={prefix}",
            data.is_null()
        );
    }
    let Some(rc) = with_state(ctx, |state| {
        if context != 0 && !state.contexts.contains_key(&context) {
            return VA_STATUS_ERROR_INVALID_CONTEXT as VAStatus;
        }
        let id = alloc_id(&mut state.next_buffer);
        let picture_param =
            (type_ == VABufferType_VAPictureParameterBufferType).then(|| payload.clone());
        state.buffers.insert(
            id,
            BufferInfo {
                type_,
                unit_size: size,
                num_elements,
                data: payload,
            },
        );
        if let Some(data) = picture_param {
            state.picture_params.insert(context, data);
        }
        *buffer = id;
        VA_STATUS_SUCCESS as VAStatus
    }) else {
        return VA_STATUS_ERROR_INVALID_CONTEXT as VAStatus;
    };
    rc
}

unsafe extern "C" fn buffer_set_num_elements(
    ctx: VADriverContextP,
    buffer: VABufferID,
    num_elements: u32,
) -> VAStatus {
    let Some(rc) = with_state(ctx, |state| {
        let Some(info) = state.buffers.get_mut(&buffer) else {
            return VA_STATUS_ERROR_INVALID_BUFFER as VAStatus;
        };
        let Some(bytes) = (info.unit_size as usize).checked_mul(num_elements as usize) else {
            return VA_STATUS_ERROR_ALLOCATION_FAILED as VAStatus;
        };
        if bytes > 64 * 1024 * 1024 {
            return VA_STATUS_ERROR_ALLOCATION_FAILED as VAStatus;
        }
        info.data.resize(bytes, 0);
        info.num_elements = num_elements;
        VA_STATUS_SUCCESS as VAStatus
    }) else {
        return VA_STATUS_ERROR_INVALID_CONTEXT as VAStatus;
    };
    rc
}

unsafe extern "C" fn map_buffer(
    ctx: VADriverContextP,
    buffer: VABufferID,
    out: *mut *mut core::ffi::c_void,
) -> VAStatus {
    if out.is_null() {
        return VA_STATUS_ERROR_INVALID_PARAMETER as VAStatus;
    }
    let Some(rc) = with_state(ctx, |state| {
        let Some(info) = state.buffers.get_mut(&buffer) else {
            return VA_STATUS_ERROR_INVALID_BUFFER as VAStatus;
        };
        if std::env::var_os("HUSK_VA_TRACE").is_some() {
            eprintln!(
                "HUSK_VA_MAP_BUFFER id={buffer} type={} bytes={}",
                info.type_,
                info.data.len()
            );
        }
        *out = info.data.as_mut_ptr().cast();
        VA_STATUS_SUCCESS as VAStatus
    }) else {
        return VA_STATUS_ERROR_INVALID_CONTEXT as VAStatus;
    };
    rc
}

unsafe extern "C" fn unmap_buffer(ctx: VADriverContextP, buffer: VABufferID) -> VAStatus {
    if std::env::var_os("HUSK_VA_TRACE").is_some() {
        let _ = with_state(ctx, |state| {
            if let Some(info) = state.buffers.get(&buffer) {
                let prefix = info
                    .data
                    .iter()
                    .take(24)
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>();
                eprintln!(
                    "HUSK_VA_UNMAP id={buffer} type={} bytes={} prefix={prefix}",
                    info.type_,
                    info.data.len()
                );
            }
        });
    }
    let Some(found) = with_state(ctx, |state| state.buffers.contains_key(&buffer)) else {
        return VA_STATUS_ERROR_INVALID_CONTEXT as VAStatus;
    };
    if found {
        VA_STATUS_SUCCESS as VAStatus
    } else {
        VA_STATUS_ERROR_INVALID_BUFFER as VAStatus
    }
}

unsafe extern "C" fn destroy_buffer(ctx: VADriverContextP, buffer: VABufferID) -> VAStatus {
    let Some(found) = with_state(ctx, |state| state.buffers.remove(&buffer).is_some()) else {
        return VA_STATUS_ERROR_INVALID_CONTEXT as VAStatus;
    };
    if found {
        VA_STATUS_SUCCESS as VAStatus
    } else {
        VA_STATUS_ERROR_INVALID_BUFFER as VAStatus
    }
}

unsafe extern "C" fn buffer_info(
    ctx: VADriverContextP,
    buffer: VABufferID,
    type_: *mut VABufferType,
    size: *mut u32,
    num_elements: *mut u32,
) -> VAStatus {
    if type_.is_null() || size.is_null() || num_elements.is_null() {
        return VA_STATUS_ERROR_INVALID_PARAMETER as VAStatus;
    }
    let Some(info) = with_state(ctx, |state| {
        state
            .buffers
            .get(&buffer)
            .map(|info| (info.type_, info.unit_size, info.num_elements))
    }) else {
        return VA_STATUS_ERROR_INVALID_CONTEXT as VAStatus;
    };
    let Some((stored_type, stored_size, stored_elements)) = info else {
        return VA_STATUS_ERROR_INVALID_BUFFER as VAStatus;
    };
    *type_ = stored_type;
    *size = stored_size;
    *num_elements = stored_elements;
    VA_STATUS_SUCCESS as VAStatus
}

unsafe extern "C" fn sync_surface(ctx: VADriverContextP, surface: VASurfaceID) -> VAStatus {
    let Some(found) = with_state(ctx, |state| state.surfaces.contains_key(&surface)) else {
        return VA_STATUS_ERROR_INVALID_CONTEXT as VAStatus;
    };
    if found {
        VA_STATUS_SUCCESS as VAStatus
    } else {
        VA_STATUS_ERROR_INVALID_SURFACE as VAStatus
    }
}

unsafe extern "C" fn query_surface_status(
    ctx: VADriverContextP,
    surface: VASurfaceID,
    status: *mut VASurfaceStatus,
) -> VAStatus {
    if status.is_null() {
        return VA_STATUS_ERROR_INVALID_PARAMETER as VAStatus;
    }
    let Some(found) = with_state(ctx, |state| state.surfaces.contains_key(&surface)) else {
        return VA_STATUS_ERROR_INVALID_CONTEXT as VAStatus;
    };
    if !found {
        return VA_STATUS_ERROR_INVALID_SURFACE as VAStatus;
    }
    let decoded = with_state(ctx, |state| state.surface_frames.contains_key(&surface)).unwrap_or(false);
    *status = if decoded {
        VASurfaceStatus_VASurfaceReady
    } else {
        VASurfaceStatus_VASurfaceRendering
    };
    VA_STATUS_SUCCESS as VAStatus
}

unsafe extern "C" fn begin_picture(
    ctx: VADriverContextP,
    context: VAContextID,
    surface: VASurfaceID,
) -> VAStatus {
    let Some(rc) = with_state(ctx, |state| {
        if !state.contexts.contains_key(&context) {
            return VA_STATUS_ERROR_INVALID_CONTEXT as VAStatus;
        }
        if !state.surfaces.contains_key(&surface) {
            return VA_STATUS_ERROR_INVALID_SURFACE as VAStatus;
        }
        if state.active_picture.is_some() {
            return VA_STATUS_ERROR_OPERATION_FAILED as VAStatus;
        }
        state.active_picture = Some(ActivePicture {
            context,
            surface,
            buffers: Vec::new(),
        });
        VA_STATUS_SUCCESS as VAStatus
    }) else {
        return VA_STATUS_ERROR_INVALID_CONTEXT as VAStatus;
    };
    rc
}

unsafe extern "C" fn render_picture(
    ctx: VADriverContextP,
    context: VAContextID,
    buffers: *mut VABufferID,
    num_buffers: i32,
) -> VAStatus {
    if num_buffers < 0 || (num_buffers > 0 && buffers.is_null()) {
        return VA_STATUS_ERROR_INVALID_PARAMETER as VAStatus;
    }
    let Some(rc) = with_state(ctx, |state| {
        let Some(picture) = state.active_picture.as_mut() else {
            return VA_STATUS_ERROR_INVALID_CONTEXT as VAStatus;
        };
        if picture.context != context {
            return VA_STATUS_ERROR_INVALID_CONTEXT as VAStatus;
        }
        for index in 0..num_buffers as usize {
            let id = *buffers.add(index);
            if !state.buffers.contains_key(&id) {
                return VA_STATUS_ERROR_INVALID_BUFFER as VAStatus;
            }
            picture.buffers.push(id);
        }
        VA_STATUS_SUCCESS as VAStatus
    }) else {
        return VA_STATUS_ERROR_INVALID_CONTEXT as VAStatus;
    };
    rc
}

unsafe extern "C" fn end_picture(ctx: VADriverContextP, context: VAContextID) -> VAStatus {
    let trace = std::env::var_os("HUSK_VA_TRACE").is_some();
    if trace {
        eprintln!("HUSK_VA_END_ENTER ctx={ctx:p} context={context}");
    }
    let Some(driver) = driver_context(ctx) else {
        if trace {
            eprintln!("HUSK_VA_END_NO_DRIVER ctx={ctx:p} context={context}");
        }
        return VA_STATUS_ERROR_INVALID_CONTEXT as VAStatus;
    };

    let (picture, profile, width, height, access_unit, csd0, csd1) = {
        let mut state = match driver.state.lock() {
            Ok(state) => state,
            Err(_) => return VA_STATUS_ERROR_OPERATION_FAILED as VAStatus,
        };
        let Some(active_context) = state.active_picture.as_ref().map(|picture| picture.context)
        else {
            if trace {
                eprintln!("HUSK_VA_END_NO_ACTIVE context={context}");
            }
            return VA_STATUS_ERROR_INVALID_CONTEXT as VAStatus;
        };
        if active_context != context {
            if trace {
                eprintln!(
                    "HUSK_VA_END_CONTEXT_MISMATCH active={active_context} requested={context}"
                );
            }
            return VA_STATUS_ERROR_INVALID_CONTEXT as VAStatus;
        }
        let picture = state
            .active_picture
            .take()
            .expect("active picture checked immediately above");
        let Some(info) = state.contexts.get(&context).copied() else {
            if trace {
                eprintln!("HUSK_VA_END_NO_CONTEXT_INFO context={context}");
            }
            return VA_STATUS_ERROR_INVALID_CONTEXT as VAStatus;
        };
        if trace {
            eprintln!(
                "HUSK_VA_END_ACTIVE context={context} surface={} buffers={} size={}x{} profile={}",
                picture.surface,
                picture.buffers.len(),
                info.width,
                info.height,
                info.profile
            );
        }
        if !matches!(
            info.profile,
            VAProfile_VAProfileH264Baseline
                | VAProfile_VAProfileH264Main
                | VAProfile_VAProfileH264High
                | VAProfile_VAProfileH264ConstrainedBaseline
        ) {
            if trace {
                eprintln!("HUSK_VA_END_UNSUPPORTED_PROFILE profile={}", info.profile);
            }
            return VA_STATUS_ERROR_UNSUPPORTED_PROFILE as VAStatus;
        }

        let mut access_unit = Vec::new();
        let mut slice_buffers = 0usize;
        let mut buffer_types = Vec::new();
        let mut picture_params: Option<Vec<u8>> = None;
        let mut slice_params: Option<Vec<u8>> = None;
        for id in &picture.buffers {
            let Some(buffer) = state.buffers.get(id) else {
                continue;
            };
            buffer_types.push(format!("{}:{}", buffer.type_, buffer.data.len()));
            match buffer.type_ {
                VABufferType_VAPictureParameterBufferType => {
                    if picture_params.is_none() {
                        picture_params = Some(buffer.data.clone());
                    }
                }
                VABufferType_VASliceParameterBufferType => {
                    if slice_params.is_none() {
                        slice_params = Some(buffer.data.clone());
                    }
                }
                VABufferType_VASliceDataBufferType if !buffer.data.is_empty() => {
                    slice_buffers += 1;
                    if !buffer.data.starts_with(&[0, 0, 0, 1])
                        && !buffer.data.starts_with(&[0, 0, 1])
                    {
                        access_unit.extend_from_slice(&[0, 0, 0, 1]);
                    }
                    access_unit.extend_from_slice(&buffer.data);
                }
                _ => {}
            }
        }

        if picture_params.is_none() {
            // gst-vaapi submits the picture parameter buffer in its own
            // vaRenderPicture call, so it is not always part of this picture's
            // buffer list. Taking the create-time copy is wrong twice over: the
            // client fills the struct after vaMapBuffer, so that copy is zeros.
            // Use the newest non-empty picture buffer instead.
            picture_params = state
                .buffers
                .iter()
                .filter(|(_, buffer)| {
                    buffer.type_ == VABufferType_VAPictureParameterBufferType
                        && buffer.data.iter().any(|byte| *byte != 0)
                })
                .max_by_key(|(id, _)| **id)
                .map(|(_, buffer)| buffer.data.clone())
                .or_else(|| state.picture_params.get(&context).cloned());
        }
        if trace {
            static DUMPED: std::sync::atomic::AtomicBool =
                std::sync::atomic::AtomicBool::new(false);
            if !DUMPED.swap(true, std::sync::atomic::Ordering::SeqCst) {
                if let Some(data) = picture_params.as_ref() {
                    let mut hex = String::with_capacity(data.len() * 2);
                    for byte in data {
                        hex.push_str(&format!("{byte:02x}"));
                    }
                    eprintln!("HUSK_VA_PP_AT_END bytes={} hex={hex}", data.len());
                }
            }
        }
        if trace {
            eprintln!(
                "HUSK_VA_END_GATHER slices={slice_buffers} access_bytes={} picture_params={} slice_params={} types={}",
                access_unit.len(),
                picture_params.as_ref().map(Vec::len).unwrap_or(0),
                slice_params.as_ref().map(Vec::len).unwrap_or(0),
                buffer_types.join(",")
            );
        }
        if slice_buffers == 0 || access_unit.is_empty() {
            return VA_STATUS_ERROR_DECODING_ERROR as VAStatus;
        }
        let Some(picture_params) = picture_params.as_deref() else {
            if trace {
                eprintln!("HUSK_VA_END_NO_PICTURE_PARAMS context={context}");
            }
            return VA_STATUS_ERROR_INVALID_PARAMETER as VAStatus;
        };
        let csd = match h264::build_csd(
            info.profile,
            picture_params,
            slice_params.as_deref(),
            info.width,
            info.height,
        ) {
            Ok(csd) => csd,
            Err(error) => {
                eprintln!("HUSK_VA_CSD_FAILED context={context} {error}");
                return VA_STATUS_ERROR_INVALID_PARAMETER as VAStatus;
            }
        };
        if trace {
            let prefix = access_unit
                .iter()
                .take(32)
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            eprintln!(
                "HUSK_VA_ACCESS_UNIT context={} surface={} profile={} size={}x{} slices={} bytes={} buffers={} prefix={}",
                picture.context,
                picture.surface,
                info.profile,
                info.width,
                info.height,
                slice_buffers,
                access_unit.len(),
                buffer_types.join(","),
                prefix
            );
        }
        (
            picture,
            info.profile,
            info.width,
            info.height,
            access_unit,
            csd.sps,
            csd.pps,
        )
    };

    let mut video = match driver.video.lock() {
        Ok(video) => video,
        Err(_) => return VA_STATUS_ERROR_OPERATION_FAILED as VAStatus,
    };
    if video
        .as_ref()
        .is_none_or(|client| !client.matches(width, height))
    {
        *video = None;
        match VideoClient::connect(width, height, &csd0, &csd1) {
            Ok(client) => {
                if trace {
                    eprintln!(
                        "HUSK_VA_BRIDGE_OPEN context={} size={}x{} codec=c2.mtk.avc.decoder",
                        picture.context, width, height
                    );
                }
                *video = Some(client);
            }
            Err(error) => {
                eprintln!(
                    "HUSK_VA_BRIDGE_OPEN_FAILED context={} size={}x{} {error}",
                    picture.context, width, height
                );
                return VA_STATUS_ERROR_OPERATION_FAILED as VAStatus;
            }
        }
    }

    // VA-API does not carry a presentation timestamp in vaEndPicture. Keep the
    // bridge's PTS neutral until the display path supplies a real clock.
    let pts = with_state(ctx, |state| {
        let pts = state.next_pts.max(1);
        state.next_pts = pts + 1;
        state.surface_pts.insert(picture.surface, pts);
        state.surface_frames.remove(&picture.surface);
        pts
    })
    .unwrap_or(1);

    let result = video
        .as_mut()
        .expect("video client inserted above")
        .submit(&access_unit, pts, 0);
    if let Err(error) = result {
        eprintln!(
            "HUSK_VA_BRIDGE_SUBMIT_FAILED context={} surface={} {error}",
            picture.context, picture.surface
        );
        *video = None;
        return VA_STATUS_ERROR_DECODING_ERROR as VAStatus;
    }
    drop(video);
    if trace {
        eprintln!(
            "HUSK_VA_BRIDGE_SUBMIT context={} surface={} profile={} bytes={} pts={pts}",
            picture.context,
            picture.surface,
            profile,
            access_unit.len()
        );
    }
    VA_STATUS_SUCCESS as VAStatus
}

// ------------------------------------------------------------- images ---
//
// gst-vaapi builds its output pool from VA images, not surfaces, so a driver
// without `vaCreateImage` cannot produce a src pad no matter how well decoding
// works. An image here is an ordinary buffer holding tight NV12, which keeps a
// single mapping path in `vaMapBuffer`.

fn nv12_len(width: u32, height: u32) -> usize {
    let luma = width as usize * height as usize;
    let chroma = luma.div_ceil(4) * 2;
    luma + chroma
}

unsafe fn push_image(
    state: &mut ContextState,
    width: u32,
    height: u32,
    nv12: Vec<u8>,
    image: *mut VAImage,
) -> VAStatus {
    let data_size = nv12.len();
    let buffer = alloc_id(&mut state.next_buffer);
    state.buffers.insert(
        buffer,
        BufferInfo {
            type_: VABufferType_VAImageBufferType,
            unit_size: data_size as u32,
            num_elements: 1,
            data: nv12,
        },
    );
    let image_id = alloc_id(&mut state.next_image);
    state.images.insert(
        image_id,
        ImageInfo {
            buffer,
            width,
            height,
        },
    );
    let luma = width as u32 * height as u32;
    *image = VAImage {
        image_id,
        format: NV12_FORMAT,
        buf: buffer,
        width: width as u16,
        height: height as u16,
        data_size: data_size as u32,
        num_planes: 2,
        pitches: [width, width, 0],
        offsets: [0, luma, 0],
        num_palette_entries: 0,
        entry_bytes: 0,
        component_order: [0; 4],
        va_reserved: [0; 4],
    };
    VA_STATUS_SUCCESS as VAStatus
}

/// Fetch and cache the decoded picture for `surface`. Returns
/// `VA_STATUS_ERROR_INVALID_SURFACE` when nothing has been decoded for it yet,
/// so callers can report "no pixels" instead of inventing them.
unsafe fn ensure_frame(ctx: VADriverContextP, surface: VASurfaceID) -> Result<(), VAStatus> {
    let Some((cached, pts)) = with_state(ctx, |state| {
        (
            state.surface_frames.contains_key(&surface),
            state.surface_pts.get(&surface).copied(),
        )
    }) else {
        return Err(VA_STATUS_ERROR_INVALID_CONTEXT as VAStatus);
    };
    if cached {
        return Ok(());
    }
    let Some(pts) = pts else {
        return Err(VA_STATUS_ERROR_INVALID_SURFACE as VAStatus);
    };
    let Some(driver) = driver_context(ctx) else {
        return Err(VA_STATUS_ERROR_INVALID_CONTEXT as VAStatus);
    };
    let frame = {
        let mut video = match driver.video.lock() {
            Ok(video) => video,
            Err(_) => return Err(VA_STATUS_ERROR_OPERATION_FAILED as VAStatus),
        };
        let Some(client) = video.as_mut() else {
            return Err(VA_STATUS_ERROR_INVALID_SURFACE as VAStatus);
        };
        match client.fetch_frame(pts) {
            Ok(Some(frame)) => frame,
            Ok(None) => return Err(VA_STATUS_ERROR_INVALID_SURFACE as VAStatus),
            Err(error) => {
                eprintln!("HUSK_VA_FRAME_FAILED surface={surface} pts={pts} {error}");
                return Err(VA_STATUS_ERROR_DECODING_ERROR as VAStatus);
            }
        }
    };
    if std::env::var_os("HUSK_VA_TRACE").is_some() {
        // A coarse luma statistic, so "we got a frame" cannot be confused with
        // "we got a black frame". Sampling every 997th byte keeps it cheap.
        let luma_len = (frame.width as usize * frame.height as usize).min(frame.nv12.len());
        let mut sum = 0u64;
        let mut min = 255u8;
        let mut max = 0u8;
        let mut count = 0u64;
        let mut index = 0usize;
        while index < luma_len {
            let value = frame.nv12[index];
            sum += value as u64;
            min = min.min(value);
            max = max.max(value);
            count += 1;
            index += 997;
        }
        let mean = if count == 0 { 0 } else { sum / count };
        eprintln!(
            "HUSK_VA_FRAME surface={surface} pts={pts} size={}x{} bytes={} luma_mean={mean} min={min} max={max}",
            frame.width,
            frame.height,
            frame.nv12.len()
        );
    }
    let _ = with_state(ctx, |state| {
        state.surface_frames.insert(
            surface,
            SurfaceFrame {
                nv12: frame.nv12,
                width: frame.width,
                height: frame.height,
            },
        );
    });
    Ok(())
}

unsafe extern "C" fn create_image(
    ctx: VADriverContextP,
    format: *mut VAImageFormat,
    width: core::ffi::c_int,
    height: core::ffi::c_int,
    image: *mut VAImage,
) -> VAStatus {
    if image.is_null() || width <= 0 || height <= 0 {
        return VA_STATUS_ERROR_INVALID_PARAMETER as VAStatus;
    }
    let (w, h) = (width as u32, height as u32);
    if w > MAX_SURFACE_DIMENSION || h > MAX_SURFACE_DIMENSION {
        return VA_STATUS_ERROR_INVALID_PARAMETER as VAStatus;
    }
    // The client may ask for another fourcc; the driver decides what it can
    // actually produce and writes the real format back, which is what a real
    // VA driver does for an unsupported request.
    if !format.is_null() {
        (*format) = NV12_FORMAT;
    }
    let mut nv12 = vec![0u8; nv12_len(w, h)];
    let rc = match with_state(ctx, |state| push_image(state, w, h, core::mem::take(&mut nv12), image)) {
        Some(rc) => rc,
        None => return VA_STATUS_ERROR_INVALID_CONTEXT as VAStatus,
    };
    rc
}

unsafe extern "C" fn derive_image(
    ctx: VADriverContextP,
    surface: VASurfaceID,
    image: *mut VAImage,
) -> VAStatus {
    if image.is_null() {
        return VA_STATUS_ERROR_INVALID_PARAMETER as VAStatus;
    }
    if let Err(status) = ensure_frame(ctx, surface) {
        return status;
    }
    let Some(payload) = with_state(ctx, |state| {
        let Some(frame) = state.surface_frames.get(&surface) else {
            return None;
        };
        Some((frame.width, frame.height, frame.nv12.clone()))
    }) else {
        return VA_STATUS_ERROR_INVALID_CONTEXT as VAStatus;
    };
    let Some((width, height, nv12)) = payload else {
        // No decoded pixels for this surface yet. Returning an empty image here
        // would hand the caller a blank frame dressed up as decoded output.
        return VA_STATUS_ERROR_INVALID_SURFACE as VAStatus;
    };
    match with_state(ctx, |state| push_image(state, width, height, nv12, image)) {
        Some(rc) => rc,
        None => VA_STATUS_ERROR_INVALID_CONTEXT as VAStatus,
    }
}

unsafe extern "C" fn destroy_image(ctx: VADriverContextP, image: VAImageID) -> VAStatus {
    let Some(found) = with_state(ctx, |state| {
        let Some(info) = state.images.remove(&image) else {
            return false;
        };
        state.buffers.remove(&info.buffer);
        true
    }) else {
        return VA_STATUS_ERROR_INVALID_CONTEXT as VAStatus;
    };
    if found {
        VA_STATUS_SUCCESS as VAStatus
    } else {
        VA_STATUS_ERROR_INVALID_IMAGE as VAStatus
    }
}

unsafe extern "C" fn get_image(
    ctx: VADriverContextP,
    surface: VASurfaceID,
    x: core::ffi::c_int,
    y: core::ffi::c_int,
    width: core::ffi::c_uint,
    height: core::ffi::c_uint,
    image: VAImageID,
) -> VAStatus {
    if let Err(status) = ensure_frame(ctx, surface) {
        if std::env::var_os("HUSK_VA_TRACE").is_some() {
            eprintln!("HUSK_VA_GET_IMAGE surface={surface} ensure_frame status={status}");
        }
        return status;
    }
    let Some(result) = with_state(ctx, |state| {
        let Some(frame) = state.surface_frames.get(&surface) else {
            return VA_STATUS_ERROR_INVALID_SURFACE as VAStatus;
        };
        if x != 0 || y != 0 || width < frame.width || height < frame.height {
            // Cropping would need a real resampler, and a smaller image cannot
            // hold the frame. Say so instead of copying the wrong region.
            if std::env::var_os("HUSK_VA_TRACE").is_some() {
                eprintln!(
                    "HUSK_VA_GET_IMAGE_REJECT surface={surface} rect={}x{}+{x}+{y} frame={}x{}",
                    width, height, frame.width, frame.height
                );
            }
            return VA_STATUS_ERROR_INVALID_PARAMETER as VAStatus;
        }
        let Some(info) = state.images.get(&image) else {
            return VA_STATUS_ERROR_INVALID_IMAGE as VAStatus;
        };
        let (buffer, image_width, image_height) = (info.buffer, info.width, info.height);
        let Some(dest) = state.buffers.get_mut(&buffer) else {
            return VA_STATUS_ERROR_INVALID_IMAGE as VAStatus;
        };
        // NV12 with pitch = image width. Rows the frame does not cover (a
        // 1080-line frame in a 1088-line coded image) are zeroed, not left with
        // whatever the pool held.
        let image_width = image_width as usize;
        let image_height = image_height as usize;
        let frame_width = frame.width as usize;
        let frame_height = frame.height as usize;
        let luma_bytes = image_width * image_height;
        if dest.data.len() < luma_bytes * 3 / 2 {
            return VA_STATUS_ERROR_INVALID_PARAMETER as VAStatus;
        }
        dest.data.fill(0);
        let source = &frame.nv12;
        let copy_width = frame_width.min(image_width);
        let copy_height = frame_height.min(image_height);
        for row in 0..copy_height {
            dest.data[row * image_width..row * image_width + copy_width]
                .copy_from_slice(&source[row * frame_width..row * frame_width + copy_width]);
        }
        let chroma_width = copy_width.min(image_width);
        let chroma_height = (frame_height / 2).min(image_height / 2);
        let source_uv = frame_width * frame_height;
        let dest_uv = luma_bytes;
        for row in 0..chroma_height {
            dest.data[dest_uv + row * image_width..dest_uv + row * image_width + chroma_width]
                .copy_from_slice(
                    &source[source_uv + row * frame_width..source_uv + row * frame_width + chroma_width],
                );
        }
        VA_STATUS_SUCCESS as VAStatus
    }) else {
        return VA_STATUS_ERROR_INVALID_CONTEXT as VAStatus;
    };
    result

}

macro_rules! install_stub {
    ($v:ident, $($field:ident),+ $(,)?) => {
        $(
            if (*$v).$field.is_none() {
                (*$v).$field = Some(core::mem::transmute(
                    unimplemented as *const ()
                ));
            }
        )+
    };
}

/// libva's backend entry point.  libva probes this symbol by name.
#[no_mangle]
pub unsafe extern "C" fn __vaDriverInit_1_0(ctx: VADriverContextP) -> VAStatus {
    let trace = std::env::var_os("HUSK_VA_TRACE").is_some();
    if trace {
        eprintln!("HUSK_VA_INIT enter ctx={ctx:p}");
    }
    if ctx.is_null() {
        if trace {
            eprintln!("HUSK_VA_INIT null context");
        }
        return VA_STATUS_ERROR_INVALID_DISPLAY as VAStatus;
    }
    (*ctx).version_major = 1;
    (*ctx).version_minor = 20;
    (*ctx).max_profiles = PROFILE_LIST.len() as i32;
    (*ctx).max_entrypoints = 1;
    (*ctx).max_attributes = 8;
    (*ctx).max_image_formats = 1;
    (*ctx).max_subpic_formats = 1;
    (*ctx).max_display_attributes = 1;
    (*ctx).str_vendor = VENDOR.as_ptr().cast();

    if (*ctx).vtable.is_null() {
        if trace {
            eprintln!("HUSK_VA_INIT null vtable");
        }
        return VA_STATUS_ERROR_ALLOCATION_FAILED as VAStatus;
    }
    if !register_context(ctx) {
        return VA_STATUS_ERROR_ALLOCATION_FAILED as VAStatus;
    }
    let v = (*ctx).vtable;

    (*v).vaTerminate = Some(terminate);
    (*v).vaQueryConfigProfiles = Some(query_profiles);
    (*v).vaQueryConfigEntrypoints = Some(query_entrypoints);
    (*v).vaGetConfigAttributes = Some(get_config_attributes);
    (*v).vaCreateConfig = Some(create_config);
    (*v).vaDestroyConfig = Some(destroy_config);
    (*v).vaQueryConfigAttributes = Some(query_config_attributes);
    (*v).vaQueryImageFormats = Some(query_image_formats);
    (*v).vaCreateSurfaces = Some(create_surfaces);
    (*v).vaCreateSurfaces2 = Some(create_surfaces2);
    (*v).vaDestroySurfaces = Some(destroy_surfaces);
    (*v).vaQuerySurfaceAttributes = Some(query_surface_attributes);
    (*v).vaCreateContext = Some(create_context);
    (*v).vaDestroyContext = Some(destroy_context);
    (*v).vaCreateBuffer = Some(create_buffer);
    (*v).vaBufferSetNumElements = Some(buffer_set_num_elements);
    (*v).vaMapBuffer = Some(map_buffer);
    (*v).vaUnmapBuffer = Some(unmap_buffer);
    (*v).vaDestroyBuffer = Some(destroy_buffer);
    (*v).vaBufferInfo = Some(buffer_info);
    (*v).vaCreateImage = Some(create_image);
    (*v).vaDeriveImage = Some(derive_image);
    (*v).vaDestroyImage = Some(destroy_image);
    (*v).vaGetImage = Some(get_image);
    (*v).vaSyncSurface = Some(sync_surface);
    (*v).vaQuerySurfaceStatus = Some(query_surface_status);
    (*v).vaBeginPicture = Some(begin_picture);
    (*v).vaRenderPicture = Some(render_picture);
    (*v).vaEndPicture = Some(end_picture);

    install_stub!(
        v,
        vaSetImagePalette,
        vaPutImage,
        vaQuerySubpictureFormats,
        vaCreateSubpicture,
        vaDestroySubpicture,
        vaSetSubpictureImage,
        vaSetSubpictureChromakey,
        vaSetSubpictureGlobalAlpha,
        vaAssociateSubpicture,
        vaDeassociateSubpicture,
        vaQueryDisplayAttributes,
        vaGetDisplayAttributes,
        vaSetDisplayAttributes,
        vaMFSubmit,
    );

    if trace {
        eprintln!(
            "HUSK_VA_INIT done profiles={} entrypoints={} attrs={} images={} subpics={} vendor={:p} vtable={:p}",
            (*ctx).max_profiles,
            (*ctx).max_entrypoints,
            (*ctx).max_attributes,
            (*ctx).max_image_formats,
            (*ctx).max_subpic_formats,
            (*ctx).str_vendor,
            (*ctx).vtable,
        );
    }

    VA_STATUS_SUCCESS as VAStatus
}

/// Used by the on-device direct probe before a real VA display is available.
pub fn self_test() -> i32 {
    let mut ctx = VADriverContext {
        pDriverData: core::ptr::null_mut(),
        vtable: core::ptr::null_mut(),
        vtable_glx: core::ptr::null_mut(),
        vtable_egl: core::ptr::null_mut(),
        vtable_tpi: core::ptr::null_mut(),
        native_dpy: core::ptr::null_mut(),
        x11_screen: 0,
        version_major: 0,
        version_minor: 0,
        max_profiles: 0,
        max_entrypoints: 0,
        max_attributes: 0,
        max_image_formats: 0,
        max_subpic_formats: 0,
        max_display_attributes: 0,
        str_vendor: core::ptr::null(),
        handle: core::ptr::null_mut(),
        drm_state: core::ptr::null_mut(),
        glx: core::ptr::null_mut(),
        display_type: 0,
        vtable_wayland: core::ptr::null_mut(),
        vtable_vpp: core::ptr::null_mut(),
        override_driver_name: core::ptr::null_mut(),
        pDisplayContext: core::ptr::null_mut(),
        error_callback: None,
        info_callback: None,
        vtable_prot: core::ptr::null_mut(),
        reserved: [0; 37],
    };
    let mut vtable: VADriverVTable = unsafe { core::mem::zeroed() };
    ctx.vtable = &mut vtable;
    let rc = unsafe { __vaDriverInit_1_0(&mut ctx) };
    if rc != VA_STATUS_SUCCESS as VAStatus {
        println!("INIT_FAIL status={rc}");
        return 1;
    }
    let mut profiles = [0; PROFILE_LIST.len()];
    let mut count = profiles.len() as i32;
    let rc = unsafe {
        (vtable.vaQueryConfigProfiles.unwrap())(&mut ctx, profiles.as_mut_ptr(), &mut count)
    };
    if rc != VA_STATUS_SUCCESS as VAStatus {
        println!("PROFILES_FAIL status={rc}");
        return 2;
    }
    println!("HUSK_VA vendor={} profiles={count}", unsafe {
        core::ffi::CStr::from_ptr(ctx.str_vendor).to_string_lossy()
    });
    for p in profiles.iter().take(count.max(0) as usize) {
        let mut ep = 0;
        let mut ec = 0;
        let rc =
            unsafe { (vtable.vaQueryConfigEntrypoints.unwrap())(&mut ctx, *p, &mut ep, &mut ec) };
        if rc != VA_STATUS_SUCCESS as VAStatus || ec == 0 {
            println!("ENTRYPOINT_FAIL profile={p} status={rc} count={ec}");
            return 4;
        }
        println!("PROFILE id={p} entrypoints={ec} first={ep}");
    }
    let mut image_format: VAImageFormat = unsafe { core::mem::zeroed() };
    let mut image_format_count = 0;
    let rc = unsafe {
        (vtable.vaQueryImageFormats.unwrap())(&mut ctx, &mut image_format, &mut image_format_count)
    };
    if rc != VA_STATUS_SUCCESS as VAStatus || image_format_count != 1 {
        println!("IMAGE_FORMAT_FAIL status={rc} count={image_format_count}");
        return 5;
    }
    println!(
        "IMAGE_FORMAT count={image_format_count} fourcc=0x{:08x} bpp={} depth={}",
        image_format.fourcc, image_format.bits_per_pixel, image_format.depth
    );
    0
}
