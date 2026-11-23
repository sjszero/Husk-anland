//! A minimal Rust VA-API driver for Husk.
//!
//! This is intentionally a system-layer driver, not a player.  It lets
//! libva, GStreamer and Chromium see the same standard VA-API surface while
//! the actual MediaCodec transport is added behind it.

#![allow(non_camel_case_types, non_snake_case, non_upper_case_globals)]

#[allow(dead_code)]
mod va_bindings;

use va_bindings::*;

const VENDOR: &[u8] = b"husk\0";
const PROFILE_LIST: [VAProfile; 2] = [VAProfile_VAProfileH264High, VAProfile_VAProfileHEVCMain];
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

unsafe extern "C" fn unimplemented() -> VAStatus {
    VA_STATUS_ERROR_UNIMPLEMENTED as VAStatus
}

unsafe extern "C" fn terminate(_ctx: VADriverContextP) -> VAStatus {
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
                (*a).value = VA_RT_FORMAT_YUV420 | VA_RT_FORMAT_YUV420_10;
            }
            VAConfigAttribType_VAConfigAttribMaxPictureWidth
            | VAConfigAttribType_VAConfigAttribMaxPictureHeight => {
                (*a).value = 1920;
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
    if config.is_null() {
        return VA_STATUS_ERROR_INVALID_PARAMETER as VAStatus;
    }
    *config = 1;
    VA_STATUS_SUCCESS as VAStatus
}

unsafe extern "C" fn destroy_config(_ctx: VADriverContextP, _config: VAConfigID) -> VAStatus {
    VA_STATUS_SUCCESS as VAStatus
}

unsafe extern "C" fn query_config_attributes(
    _ctx: VADriverContextP,
    _config: VAConfigID,
    profile: *mut VAProfile,
    entrypoint: *mut VAEntrypoint,
    attrs: *mut VAConfigAttrib,
    count: *mut i32,
) -> VAStatus {
    if !profile.is_null() {
        *profile = VAProfile_VAProfileH264High;
    }
    if !entrypoint.is_null() {
        *entrypoint = VAEntrypoint_VAEntrypointVLD;
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
    let v = (*ctx).vtable;

    (*v).vaTerminate = Some(terminate);
    (*v).vaQueryConfigProfiles = Some(query_profiles);
    (*v).vaQueryConfigEntrypoints = Some(query_entrypoints);
    (*v).vaGetConfigAttributes = Some(get_config_attributes);
    (*v).vaCreateConfig = Some(create_config);
    (*v).vaDestroyConfig = Some(destroy_config);
    (*v).vaQueryConfigAttributes = Some(query_config_attributes);
    (*v).vaQueryImageFormats = Some(query_image_formats);

    install_stub!(
        v,
        vaCreateSurfaces,
        vaDestroySurfaces,
        vaCreateContext,
        vaDestroyContext,
        vaCreateBuffer,
        vaBufferSetNumElements,
        vaMapBuffer,
        vaUnmapBuffer,
        vaDestroyBuffer,
        vaBeginPicture,
        vaRenderPicture,
        vaEndPicture,
        vaSyncSurface,
        vaQuerySurfaceStatus,
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
