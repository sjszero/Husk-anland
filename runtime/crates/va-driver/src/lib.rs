//! A minimal Rust VA-API driver for Husk.
//!
//! This is intentionally a system-layer driver, not a player.  It lets
//! libva, GStreamer and Chromium see the same standard VA-API surface while
//! the actual MediaCodec transport is added behind it.

#![allow(non_camel_case_types, non_snake_case, non_upper_case_globals)]

#[allow(dead_code)]
mod va_bindings;

use va_bindings::*;

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

#[allow(dead_code)]
struct ContextInfo {
    config: VAConfigID,
    width: u32,
    height: u32,
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
    configs: HashMap<VAConfigID, (VAProfile, VAEntrypoint)>,
    surfaces: HashMap<VASurfaceID, SurfaceInfo>,
    contexts: HashMap<VAContextID, ContextInfo>,
    buffers: HashMap<VABufferID, BufferInfo>,
    active_picture: Option<ActivePicture>,
}

struct DriverContext {
    state: Mutex<ContextState>,
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
        if !state.configs.contains_key(&config) {
            return VA_STATUS_ERROR_INVALID_CONFIG as VAStatus;
        }
        let id = alloc_id(&mut state.next_context);
        state.contexts.insert(
            id,
            ContextInfo {
                config,
                width: width as u32,
                height: height as u32,
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
    let Some(rc) = with_state(ctx, |state| {
        if context != 0 && !state.contexts.contains_key(&context) {
            return VA_STATUS_ERROR_INVALID_CONTEXT as VAStatus;
        }
        let id = alloc_id(&mut state.next_buffer);
        state.buffers.insert(
            id,
            BufferInfo {
                type_,
                unit_size: size,
                num_elements,
                data: payload,
            },
        );
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
        *out = info.data.as_mut_ptr().cast();
        VA_STATUS_SUCCESS as VAStatus
    }) else {
        return VA_STATUS_ERROR_INVALID_CONTEXT as VAStatus;
    };
    rc
}

unsafe extern "C" fn unmap_buffer(ctx: VADriverContextP, buffer: VABufferID) -> VAStatus {
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
    *status = VASurfaceStatus_VASurfaceReady;
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
    let Some(picture) = with_state(ctx, |state| {
        let Some(picture) = state.active_picture.take() else {
            return None;
        };
        (picture.context == context).then_some(picture)
    }) else {
        return VA_STATUS_ERROR_INVALID_CONTEXT as VAStatus;
    };
    let Some(picture) = picture else {
        return VA_STATUS_ERROR_INVALID_CONTEXT as VAStatus;
    };

    let trace = std::env::var_os("HUSK_VA_TRACE").is_some();
    let mut access_unit = Vec::new();
    let mut slice_buffers = 0usize;
    let mut buffer_types = Vec::new();
    if let Some(context) = driver_context(ctx) {
        if let Ok(state) = context.state.lock() {
            for id in &picture.buffers {
                let Some(buffer) = state.buffers.get(id) else {
                    continue;
                };
                buffer_types.push(format!("{}:{}", buffer.type_, buffer.data.len()));
                if buffer.type_ != VABufferType_VASliceDataBufferType || buffer.data.is_empty() {
                    continue;
                }
                slice_buffers += 1;
                if !buffer.data.starts_with(&[0, 0, 0, 1]) && !buffer.data.starts_with(&[0, 0, 1]) {
                    access_unit.extend_from_slice(&[0, 0, 0, 1]);
                }
                access_unit.extend_from_slice(&buffer.data);
            }
        }
    }
    if trace {
        let prefix = access_unit
            .iter()
            .take(32)
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        eprintln!(
            "HUSK_VA_ACCESS_UNIT context={} surface={} slices={} bytes={} buffers={} prefix={}",
            picture.context,
            picture.surface,
            slice_buffers,
            access_unit.len(),
            buffer_types.join(","),
            prefix
        );
    }

    // The state machine now captures the real compressed access unit.  Until
    // the app-side MediaCodec transport is connected, report the missing data
    // plane instead of pretending the surface was decoded.
    VA_STATUS_ERROR_UNIMPLEMENTED as VAStatus
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
    (*v).vaSyncSurface = Some(sync_surface);
    (*v).vaQuerySurfaceStatus = Some(query_surface_status);
    (*v).vaBeginPicture = Some(begin_picture);
    (*v).vaRenderPicture = Some(render_picture);
    (*v).vaEndPicture = Some(end_picture);

    install_stub!(
        v,
        vaCreateImage,
        vaDeriveImage,
        vaDestroyImage,
        vaSetImagePalette,
        vaGetImage,
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
