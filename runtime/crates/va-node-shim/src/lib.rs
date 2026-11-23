//! Test-only display shim for headless VA-API validation.
//!
//! The production guest uses an X11/Wayland display.  On this device the
//! unprivileged process has `/dev/mali0` but no DRM render node, while libva's
//! DRM transport insists on a DRM node type before it will load any driver.
//! This shim only lets `vainfo` reach `husk_drv_video.so` in a headless probe.
//! It does not emulate DRM and is not part of the product path.

#[no_mangle]
pub extern "C" fn drmGetNodeTypeFromFd(_fd: i32) -> i32 {
    // DRM_NODE_RENDER in libdrm is 2.  The VA driver itself does not use the fd.
    2
}
