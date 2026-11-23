//! libdrm 的替换层（只覆盖 Mesa loader 真正会调的那一小撮符号）。
//!
//! 为什么需要：Mesa 判断「这个 fd 是什么设备」是靠 libdrm 的 `drmGetVersion` /
//! `drmGetDevice2`。我们的假 fd 是 memfd，`st_rdev` 对不上任何 DRM 节点，libdrm
//! 枚举 /sys/class/drm 必然失败 → Mesa 只打印 "failed to retrieve device information"
//! 然后退回 llvmpipe。把这一组符号在 LD_PRELOAD 里实现掉，Mesa 就会认为手里是一个
//! panfrost render node。
//!
//! 语义照抄 `manoedinata/mesa-kbase-j4lte` 的 libdrm-kbase（docs/design/mali-kbase-prior-art.md）。

use core::ffi::{c_char, c_int, c_uint, c_void};

extern "C" {
    fn malloc(n: usize) -> *mut c_void;
    fn calloc(n: usize, m: usize) -> *mut c_void;
    fn free(p: *mut c_void);
    fn strlen(p: *const c_char) -> usize;
    fn __errno_location() -> *mut c_int;
    fn drm_shim_is_fake(fd: c_int) -> c_int;
}

const DRM_NODE_RENDER: usize = 2;
const DRM_NODE_MAX: usize = 3;
const DRM_BUS_PLATFORM: c_int = 2;
const DRM_PLATFORM_DEVICE_NAME_LEN: usize = 512;

const DRM_CAP_PRIME: u64 = 0x5;
const DRM_CAP_SYNCOBJ: u64 = 0x13;
const DRM_CAP_SYNCOBJ_TIMELINE: u64 = 0x14;
const DRM_PRIME_CAP_IMPORT: u64 = 0x2;

const EINVAL: c_int = 22;
const ENODEV: c_int = 19;

#[repr(C)]
pub struct DrmVersion {
    version_major: c_int,
    version_minor: c_int,
    version_patchlevel: c_int,
    name_len: c_int,
    name: *mut c_char,
    date_len: c_int,
    date: *mut c_char,
    desc_len: c_int,
    desc: *mut c_char,
}

#[repr(C)]
struct PlatformBusInfo {
    fullname: [c_char; DRM_PLATFORM_DEVICE_NAME_LEN],
}

#[repr(C)]
struct PlatformDeviceInfo {
    compatible: *mut *mut c_char,
}

#[repr(C)]
struct DrmDevice {
    nodes: *mut *mut c_char,
    available_nodes: c_int,
    bustype: c_int,
    businfo: *mut c_void,
    deviceinfo: *mut c_void,
}

unsafe fn dup_bytes(s: &[u8]) -> *mut c_char {
    let p = malloc(s.len() + 1) as *mut u8;
    if p.is_null() {
        return core::ptr::null_mut();
    }
    core::ptr::copy_nonoverlapping(s.as_ptr(), p, s.len());
    *p.add(s.len()) = 0;
    p as *mut c_char
}

fn set_errno(e: c_int) {
    unsafe { *__errno_location() = e }
}

fn dbg(tag: &[u8]) {
    unsafe {
        if !crate::debug_on_pub() {
            return;
        }
        let n = 64i64;
        core::arch::asm!("mov x8, #64", "svc 0", in("x0") 2i64, in("x1") tag.as_ptr(), in("x2") tag.len(), in("x3") 0i64, out("x8") _, options(nostack));
        let _ = n;
    }
}

unsafe fn is_fake(fd: c_int) -> bool {
    drm_shim_is_fake(fd) != 0
}

/// 调试用：打印 fd 指向的目标（NR_DRM_DEBUG=1 时）
unsafe fn dbg_fd_link(fd: c_int) {
    // 临时：无条件打印（定位 fake fd 识别问题）

    let mut p = *b"/proc/self/fd/000\0";
    let mut v = fd as u32;
    let mut d = [0u8; 3];
    let mut dn = 0;
    if v == 0 {
        d[0] = b'0';
        dn = 1;
    }
    while v > 0 {
        d[dn] = b'0' + (v % 10) as u8;
        v /= 10;
        dn += 1;
    }
    let mut i = 14;
    while dn > 0 {
        dn -= 1;
        p[i] = d[dn];
        i += 1;
    }
    p[i] = 0;
    let mut out = [0u8; 128];
    let r = crate::syscall_readlinkat(p.as_ptr(), out.as_mut_ptr(), out.len());
    if crate::debug_on_pub() {
        dbg(b"[drmapi] fd link = ");
        if r > 0 {
            dbg(&out[..r as usize]);
        }
        dbg(b"\n");
    }
}

unsafe fn make_device() -> *mut DrmDevice {
    let d = calloc(1, core::mem::size_of::<DrmDevice>()) as *mut DrmDevice;
    if d.is_null() {
        return core::ptr::null_mut();
    }
    let nodes = calloc(DRM_NODE_MAX, core::mem::size_of::<*mut c_char>()) as *mut *mut c_char;
    *nodes.add(DRM_NODE_RENDER) = dup_bytes(b"/dev/dri/renderD128");
    (*d).nodes = nodes;
    (*d).available_nodes = 1 << DRM_NODE_RENDER;
    (*d).bustype = DRM_BUS_PLATFORM;
    let bus = calloc(1, core::mem::size_of::<PlatformBusInfo>()) as *mut PlatformBusInfo;
    let name = b"platform:panfrost-kbase";
    for (i, c) in name.iter().enumerate() {
        (*bus).fullname[i] = *c as c_char;
    }
    (*d).businfo = bus as *mut c_void;
    let dev = calloc(1, core::mem::size_of::<PlatformDeviceInfo>()) as *mut PlatformDeviceInfo;
    let compat = calloc(2, core::mem::size_of::<*mut c_char>()) as *mut *mut c_char;
    *compat = dup_bytes(b"arm,mali-valhall");
    (*dev).compatible = compat;
    (*d).deviceinfo = dev as *mut c_void;
    d
}

#[no_mangle]
pub unsafe extern "C" fn drmGetVersion(fd: c_int) -> *mut DrmVersion {
    dbg(b"[drmapi] getversion\n");
    if !is_fake(fd) {
        dbg_fd_link(fd);
        set_errno(ENODEV);
        return core::ptr::null_mut();
    }
    let v = calloc(1, core::mem::size_of::<DrmVersion>()) as *mut DrmVersion;
    if v.is_null() {
        return core::ptr::null_mut();
    }
    let name = b"panfrost";
    let desc = b"panfrost on mali_kbase (husk)";
    (*v).name = dup_bytes(name);
    (*v).name_len = name.len() as c_int;
    (*v).date = dup_bytes(b"20261123");
    (*v).date_len = 8;
    (*v).desc = dup_bytes(desc);
    (*v).desc_len = desc.len() as c_int;
    (*v).version_major = 1;
    (*v).version_minor = 2;

    v
}

#[no_mangle]
pub unsafe extern "C" fn drmFreeVersion(v: *mut DrmVersion) {
    if v.is_null() {
        return;
    }
    free((*v).name as *mut c_void);
    free((*v).date as *mut c_void);
    free((*v).desc as *mut c_void);
    free(v as *mut c_void);
}

#[no_mangle]
pub unsafe extern "C" fn drmGetDevice2(fd: c_int, _flags: u32, out: *mut *mut DrmDevice) -> c_int {
    dbg(b"[drmapi] getdevice2\n");
    if !is_fake(fd) {
        return -ENODEV;
    }
    *out = make_device();
    if (*out).is_null() {
        -12
    } else {
        0
    }
}

#[no_mangle]
pub unsafe extern "C" fn drmGetDevice(fd: c_int, out: *mut *mut DrmDevice) -> c_int {
    drmGetDevice2(fd, 0, out)
}

#[no_mangle]
pub unsafe extern "C" fn drmGetDevices2(
    _flags: u32,
    devices: *mut *mut DrmDevice,
    max: c_int,
) -> c_int {
    dbg(b"[drmapi] devices2\n");
    if devices.is_null() || max < 1 {
        return 0;
    }
    let d = make_device();
    if d.is_null() {
        return -12;
    }
    *devices = d;
    1
}

#[no_mangle]
pub unsafe extern "C" fn drmGetDevices(devices: *mut *mut DrmDevice, max: c_int) -> c_int {
    drmGetDevices2(0, devices, max)
}

#[no_mangle]
pub unsafe extern "C" fn drmFreeDevice(dev: *mut *mut DrmDevice) {
    dbg(b"[drmapi] freedevice\n");
    if dev.is_null() || (*dev).is_null() {
        return;
    }
    let d = *dev;
    if !(*d).nodes.is_null() {
        let mut i = 0;
        while i < DRM_NODE_MAX {
            free(*(*d).nodes.add(i) as *mut c_void);
            i += 1;
        }
        free((*d).nodes as *mut c_void);
    }
    if !(*d).deviceinfo.is_null() {
        let info = (*d).deviceinfo as *mut PlatformDeviceInfo;
        if !(*info).compatible.is_null() {
            free(*(*info).compatible as *mut c_void);
            free((*info).compatible as *mut c_void);
        }
        free(info as *mut c_void);
    }
    free((*d).businfo);
    free(d as *mut c_void);
    *dev = core::ptr::null_mut();
}

#[no_mangle]
pub unsafe extern "C" fn drmFreeDevices(devices: *mut *mut DrmDevice, count: c_int) {
    dbg(b"[drmapi] freedevices\n");
    let mut i = 0;
    while i < count {
        drmFreeDevice(devices.add(i as usize));
        i += 1;
    }
}

#[no_mangle]
pub unsafe extern "C" fn drmDevicesEqual(a: *mut DrmDevice, b: *mut DrmDevice) -> c_int {
    dbg(b"[drmapi] deviceequal\n");
    if a.is_null() || b.is_null() {
        return 0;
    }
    if (*a).bustype != (*b).bustype {
        return 0;
    }
    let na = *(*a).nodes.add(DRM_NODE_RENDER);
    let nb = *(*b).nodes.add(DRM_NODE_RENDER);
    if na.is_null() || nb.is_null() {
        return 0;
    }
    if strlen(na) != strlen(nb) {
        return 0;
    }
    let mut i = 0;
    while *na.add(i) != 0 {
        if *na.add(i) != *nb.add(i) {
            return 0;
        }
        i += 1;
    }
    1
}

#[no_mangle]
pub unsafe extern "C" fn drmGetNodeTypeFromFd(fd: c_int) -> c_int {
    dbg(b"[drmapi] nodetype\n");
    if is_fake(fd) {
        DRM_NODE_RENDER as c_int
    } else {
        -1
    }
}

#[no_mangle]
pub unsafe extern "C" fn drmGetRenderDeviceNameFromFd(fd: c_int) -> *mut c_char {
    dbg(b"[drmapi] rendername\n");
    if is_fake(fd) {
        dup_bytes(b"/dev/dri/renderD128")
    } else {
        core::ptr::null_mut()
    }
}

#[no_mangle]
pub unsafe extern "C" fn drmGetPrimaryDeviceNameFromFd(_fd: c_int) -> *mut c_char {
    core::ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn drmGetCap(_fd: c_int, capability: u64, value: *mut u64) -> c_int {
    dbg(b"[drmapi] getcap\n");
    if value.is_null() {
        return -EINVAL;
    }
    *value = match capability {
        DRM_CAP_PRIME => DRM_PRIME_CAP_IMPORT,
        DRM_CAP_SYNCOBJ | DRM_CAP_SYNCOBJ_TIMELINE => 1,
        _ => 0,
    };
    0
}

#[no_mangle]
pub unsafe extern "C" fn drmIoctl(fd: c_int, request: u64, arg: *mut c_void) -> c_int {
    if !crate::debug_on_pub() {
        return crate::ioctl_impl(fd, request, arg as *mut u8) as c_int;
    }
    let mut b = [0u8; 48];
    let hex = b"0123456789abcdef";
    let pre = b"[drmapi] drmioctl fd=";
    let mut i = 0;
    while i < pre.len() {
        b[i] = pre[i];
        i += 1;
    }
    if fd >= 10 {
        b[i] = b'0' + (fd / 10) as u8;
        i += 1;
    }
    b[i] = b'0' + (fd % 10) as u8;
    i += 1;
    let pre2 = b" req=";
    let mut k = 0;
    while k < pre2.len() {
        b[i] = pre2[k];
        i += 1;
        k += 1;
    }
    let mut sh = 60;
    while sh >= 0 {
        b[i] = hex[((request >> sh) & 0xf) as usize];
        i += 1;
        sh -= 4;
    }
    b[i] = b'\n';
    i += 1;
    dbg(&b[..i]);
    dbg_fd_link(fd);
    crate::ioctl_impl(fd, request, arg as *mut u8) as c_int
}

#[no_mangle]
pub unsafe extern "C" fn drmCloseBufferHandle(fd: c_int, handle: u32) -> c_int {
    dbg(b"[drmapi] closehandle\n");
    let mut req = [0u8; 8];
    core::ptr::copy_nonoverlapping(&handle as *const u32 as *const u8, req.as_mut_ptr(), 4);
    crate::ioctl_impl(fd, 0x40086409, req.as_mut_ptr()) as c_int
}
