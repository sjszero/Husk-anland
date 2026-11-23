//! Receive an AHardwareBuffer native handle from the Husk app and import the
//! contained dma-buf into PanVK as external buffer memory.
//!
//! The host side sends through AHardwareBuffer_sendHandleToUnixSocket(), so the
//! guest receives a normal SCM_RIGHTS message. We intentionally do not try to
//! reconstruct the Android native_handle_t: the only object PanVK needs is the
//! dma-buf descriptor, which we locate by inspecting /proc/self/fd.

use super::*;
use core::ffi::{c_int, c_uint, c_void};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const SOL_SOCKET: c_int = 1;
const SCM_RIGHTS: c_int = 1;
const PROT_READ: c_int = 1;
const MAP_SHARED: c_int = 1;
const VK_EXTERNAL_MEMORY_HANDLE_TYPE_DMA_BUF_BIT_EXT: u32 = 0x200;
const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_EXTERNAL_BUFFER_INFO: u32 = 1_000_071_002;
const VK_STRUCTURE_TYPE_EXTERNAL_BUFFER_PROPERTIES: u32 = 1_000_071_003;
const VK_STRUCTURE_TYPE_EXTERNAL_MEMORY_BUFFER_CREATE_INFO: u32 = 1_000_072_000;
const VK_STRUCTURE_TYPE_IMPORT_MEMORY_FD_INFO_KHR: u32 = 1_000_074_000;
const VK_STRUCTURE_TYPE_BUFFER_MEMORY_REQUIREMENTS_INFO_2: u32 = 1_000_146_000;
const VK_STRUCTURE_TYPE_MEMORY_REQUIREMENTS_2: u32 = 1_000_146_003;
const AHB_FORMAT_YV12: u32 = 0x3231_5659;

#[repr(C)]
struct IoVec {
    base: *mut c_void,
    len: usize,
}

#[repr(C)]
struct MsgHdr {
    name: *mut c_void,
    name_len: u32,
    iov: *mut IoVec,
    iov_len: usize,
    control: *mut c_void,
    control_len: usize,
    flags: c_int,
}

#[repr(C)]
struct CmsgHdr {
    len: usize,
    level: c_int,
    ty: c_int,
}

struct AsyncCompletion {
    acquire_fd: c_int,
    join: Option<JoinHandle<()>>,
}

impl Default for AsyncCompletion {
    fn default() -> Self {
        Self {
            acquire_fd: -1,
            join: None,
        }
    }
}

struct RingFns {
    create_image: FnCreateImage,
    destroy_image: FnDestroyImage,
    get_image_requirements: FnGetImageRequirements2,
    bind_image: FnBindImageMemory,
    allocate_memory: FnAllocateMemory,
    free_memory: FnFreeMemory,
    get_memory_props: FnGetPhysicalDeviceMemoryProperties,
    create_buffer: FnCreateBuffer,
    destroy_buffer: FnDestroyBuffer,
    get_buffer_requirements: FnGetBufferMemoryRequirements,
    bind_buffer: FnBindBufferMemory,
    map_memory: FnMapMemory,
    unmap_memory: FnUnmapMemory,
    create_pool: FnCreateCommandPool,
    destroy_pool: FnDestroyCommandPool,
    allocate_commands: FnAllocateCommandBuffers,
    begin_command: FnBeginCommandBuffer,
    end_command: FnEndCommandBuffer,
    pipeline_barrier: FnCmdPipelineBarrier,
    clear_color: FnCmdClearColorImage,
    copy_image_to_buffer: FnCmdCopyImageToBuffer,
    create_fence: FnCreateFence,
    destroy_fence: FnDestroyFence,
    wait_fences: FnWaitForFences,
    queue_submit: FnQueueSubmit,
    reset_pool: FnResetCommandPool,
    reset_fences: FnResetFences,
    get_fence_fd: FnGetFenceFdKhr,
}

struct PooledSlot {
    image: VkImage,
    image_memory: VkDeviceMemory,
    readback: VkBuffer,
    readback_memory: VkDeviceMemory,
    command_pool: VkCommandPool,
    command: VkCommandBuffer,
    fence: VkFence,
    readback_size: u64,
    pitch: u64,
    width: u32,
    height: u32,
}

#[derive(Default)]
struct RingStats {
    export_pending: AtomicUsize,
    export_signaled: AtomicUsize,
    completion_ok: AtomicUsize,
    completion_fail: AtomicUsize,
    readback_ok: AtomicUsize,
    readback_fail: AtomicUsize,
    fallback: AtomicUsize,
}

struct PooledCompletion {
    device: usize,
    readback_memory: usize,
    readback_size: u64,
    pitch: u64,
    width: u32,
    height: u32,
    expected_pixel: u32,
    frame: usize,
    slot: usize,
    completion_fd: c_int,
    map_memory: FnMapMemory,
    unmap_memory: FnUnmapMemory,
    verbose: bool,
    stats: Arc<RingStats>,
}

impl Drop for PooledCompletion {
    fn drop(&mut self) {
        unsafe {
            if self.completion_fd >= 0 {
                close(self.completion_fd);
            }
        }
    }
}

struct CompletionResources {
    device: usize,
    fence: usize,
    command_pool: usize,
    readback_memory: usize,
    readback: usize,
    image_memory: usize,
    image: usize,
    completion_fd: c_int,
    write_fd: c_int,
    gpu_complete: bool,
    wait_fences: FnWaitForFences,
    destroy_fence: FnDestroyFence,
    destroy_pool: FnDestroyCommandPool,
    free_memory: FnFreeMemory,
    destroy_buffer: FnDestroyBuffer,
    destroy_image: FnDestroyImage,
    map_memory: FnMapMemory,
    unmap_memory: FnUnmapMemory,
    readback_size: u64,
    pitch: u64,
    width: u32,
    height: u32,
    expected_pixel: u32,
    frame: usize,
    slot: usize,
}

// All handles are Vulkan objects owned by the completion thread after submit.
// The thread is the only user of them until it waits for the fence.
unsafe impl Send for CompletionResources {}

impl CompletionResources {
    unsafe fn verify_readback(&self) {
        let device = self.device as VkDevice;
        let readback_memory = self.readback_memory as VkDeviceMemory;
        let mut mapped: *mut c_void = ptr::null_mut();
        let map_rc = (self.map_memory)(
            device,
            readback_memory,
            0,
            self.readback_size,
            0,
            &mut mapped,
        );
        if map_rc != VK_SUCCESS || mapped.is_null() {
            line(
                "dmabuf",
                "image_readback",
                format!(
                    "frame={} slot={} FAIL|async map {map_rc}",
                    self.frame, self.slot
                ),
            );
            return;
        }
        let base = mapped as *const u8;
        let mut checked = 0usize;
        let mut matched = 0usize;
        for (x, y) in probe_points(self.width, self.height) {
            let off = (y as usize * self.pitch as usize) + x as usize * 4;
            let pixel = (base.add(off) as *const u32).read_unaligned();
            checked += 1;
            if pixel == self.expected_pixel {
                matched += 1;
            }
        }
        line(
            "dmabuf",
            "image_readback",
            format!(
                "frame={} slot={} {}|match={matched}/{checked} expected={:#010x}",
                self.frame,
                self.slot,
                if checked == 9 && matched == 9 {
                    "PASS"
                } else {
                    "FAIL"
                },
                self.expected_pixel
            ),
        );
        (self.unmap_memory)(device, readback_memory);
    }
}

impl Drop for CompletionResources {
    fn drop(&mut self) {
        unsafe {
            let device = self.device as VkDevice;
            let fence = self.fence as VkFence;
            // This also covers a thread-spawn failure and a panic in the
            // completion path: never destroy objects while the GPU may use
            // them.
            if !self.gpu_complete {
                if self.completion_fd >= 0 {
                    let mut pfd = PollFd {
                        fd: self.completion_fd,
                        events: POLLIN,
                        revents: 0,
                    };
                    let _ = poll(&mut pfd, 1, 30_000);
                } else if !fence.is_null() {
                    let _ = (self.wait_fences)(device, 1, &fence, VK_TRUE, u64::MAX);
                }
            }
            if self.completion_fd >= 0 {
                close(self.completion_fd);
            }
            if !fence.is_null() {
                (self.destroy_fence)(device, fence, ptr::null());
            }
            if self.command_pool != 0 {
                (self.destroy_pool)(device, self.command_pool as VkCommandPool, ptr::null());
            }
            if self.readback_memory != 0 {
                (self.free_memory)(device, self.readback_memory as VkDeviceMemory, ptr::null());
            }
            if self.readback != 0 {
                (self.destroy_buffer)(device, self.readback as VkBuffer, ptr::null());
            }
            if self.image_memory != 0 {
                (self.free_memory)(device, self.image_memory as VkDeviceMemory, ptr::null());
            }
            if self.image != 0 {
                (self.destroy_image)(device, self.image as VkImage, ptr::null());
            }
            if self.write_fd >= 0 {
                close(self.write_fd);
                self.write_fd = -1;
            }
        }
    }
}

extern "C" {
    fn recvmsg(fd: c_int, msg: *mut MsgHdr, flags: c_int) -> isize;
    fn sendmsg(fd: c_int, msg: *const MsgHdr, flags: c_int) -> isize;
    fn close(fd: c_int) -> c_int;
    fn dup(fd: c_int) -> c_int;
    fn eventfd(initval: c_uint, flags: c_int) -> c_int;
    fn write(fd: c_int, buf: *const c_void, count: usize) -> isize;
    fn lseek(fd: c_int, offset: i64, whence: c_int) -> i64;
    fn mmap(
        addr: *mut c_void,
        len: usize,
        prot: c_int,
        flags: c_int,
        fd: c_int,
        offset: i64,
    ) -> *mut c_void;
    fn munmap(addr: *mut c_void, len: usize) -> c_int;
}

#[derive(Debug)]
struct AhbMeta {
    buffer_key: u64,
    dmabuf_key: u64,
    width: u32,
    height: u32,
    stride: u32,
    format: u32,
    usage: u64,
    cookies: [u32; 9],
    count: usize,
    frames: usize,
    plane_count: usize,
    planes: [PlaneMeta; 4],
    plane_fnv: [u64; 4],
    plane_bytes: [usize; 4],
    output_buffer_key: u64,
    output_dmabuf_key: u64,
    output_width: u32,
    output_height: u32,
    output_stride: u32,
    output_format: u32,
    output_offset: u64,
    handle_count: usize,
}

#[derive(Debug, Clone, Copy, Default)]
struct PlaneMeta {
    offset: u64,
    row_stride: u32,
    pixel_stride: u32,
}

fn parse_meta(text: &str) -> Result<AhbMeta, String> {
    let mut lines = text.lines();
    let magic = lines.next();
    if magic != Some("HUSK-AHB-1")
        && magic != Some("HUSK-AHB-2")
        && magic != Some("HUSK-VIDEO-AHB-1")
        && magic != Some("HUSK-VIDEO-AHB-2")
        && magic != Some("HUSK-VIDEO-AHB-3")
    {
        return Err("bad magic".to_string());
    }
    let mut width = None;
    let mut buffer_key = 0u64;
    let mut dmabuf_key = 0u64;
    let mut height = None;
    let mut stride = None;
    let mut format = None;
    let mut usage = None;
    let mut cookies = None;
    let mut count = 1usize;
    let mut frames = 1usize;
    let mut plane_count = 0usize;
    let mut planes = [PlaneMeta::default(); 4];
    let mut luma_fnv = 0u64;
    let mut luma_bytes = 0usize;
    let mut plane_fnv = [0u64; 4];
    let mut plane_bytes = [0usize; 4];
    let mut output_buffer_key = 0u64;
    let mut output_dmabuf_key = 0u64;
    let mut output_width = 0u32;
    let mut output_height = 0u32;
    let mut output_stride = 0u32;
    let mut output_format = 0u32;
    let mut output_offset = 0u64;
    let mut handle_count = 1usize;
    for line in lines {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key {
            "buffer_key" => {
                buffer_key = value
                    .strip_prefix("0x")
                    .and_then(|v| u64::from_str_radix(v, 16).ok())
                    .or_else(|| u64::from_str_radix(value, 16).ok())
                    .unwrap_or(0)
            }
            "dmabuf_key" => {
                dmabuf_key = value
                    .strip_prefix("0x")
                    .and_then(|v| u64::from_str_radix(v, 16).ok())
                    .or_else(|| u64::from_str_radix(value, 16).ok())
                    .unwrap_or(0)
            }
            "width" => width = value.parse().ok(),
            "height" => height = value.parse().ok(),
            "stride" => stride = value.parse().ok(),
            "format" => format = value.parse().ok(),
            "usage" => {
                usage = value
                    .strip_prefix("0x")
                    .and_then(|v| u64::from_str_radix(v, 16).ok())
            }
            "cookies" => {
                let parsed: Vec<u32> = value.split(',').filter_map(|v| v.parse().ok()).collect();
                if parsed.len() == 9 {
                    let mut out = [0u32; 9];
                    out.copy_from_slice(&parsed);
                    cookies = Some(out);
                }
            }
            "count" => count = value.parse().unwrap_or(1),
            "frames" => frames = value.parse().unwrap_or(1),
            "plane_count" => plane_count = value.parse().unwrap_or(0),
            "luma_fnv" => luma_fnv = value.parse().unwrap_or(0),
            "luma_bytes" => luma_bytes = value.parse().unwrap_or(0),
            "output_buffer_key" => {
                output_buffer_key = value
                    .strip_prefix("0x")
                    .and_then(|v| u64::from_str_radix(v, 16).ok())
                    .or_else(|| u64::from_str_radix(value, 16).ok())
                    .unwrap_or(0)
            }
            "output_dmabuf_key" => {
                output_dmabuf_key = value
                    .strip_prefix("0x")
                    .and_then(|v| u64::from_str_radix(v, 16).ok())
                    .or_else(|| u64::from_str_radix(value, 16).ok())
                    .unwrap_or(0)
            }
            "output_width" => output_width = value.parse().unwrap_or(0),
            "output_height" => output_height = value.parse().unwrap_or(0),
            "output_stride" => output_stride = value.parse().unwrap_or(0),
            "output_format" => output_format = value.parse().unwrap_or(0),
            "output_offset" => output_offset = value.parse().unwrap_or(0),
            "handle_count" => handle_count = value.parse().unwrap_or(1).clamp(1, 2),
            _ => {}
        }
        if let Some(rest) = key.strip_prefix("plane") {
            if let Some((index, field)) = rest.split_once('_') {
                if let Ok(index) = index.parse::<usize>() {
                    if index < planes.len() {
                        match field {
                            "offset" => planes[index].offset = value.parse().unwrap_or(0),
                            "row_stride" => planes[index].row_stride = value.parse().unwrap_or(0),
                            "pixel_stride" => {
                                planes[index].pixel_stride = value.parse().unwrap_or(0)
                            }
                            "fnv" => plane_fnv[index] = value.parse().unwrap_or(0),
                            "bytes" => plane_bytes[index] = value.parse().unwrap_or(0),
                            _ => {}
                        }
                    }
                }
            }
        }
    }
    if plane_fnv[0] == 0 {
        plane_fnv[0] = luma_fnv;
    }
    if plane_bytes[0] == 0 {
        plane_bytes[0] = luma_bytes;
    }
    Ok(AhbMeta {
        buffer_key,
        dmabuf_key,
        width: width.ok_or_else(|| "missing width".to_string())?,
        height: height.ok_or_else(|| "missing height".to_string())?,
        stride: stride.ok_or_else(|| "missing stride".to_string())?,
        format: format.ok_or_else(|| "missing format".to_string())?,
        usage: usage.ok_or_else(|| "missing usage".to_string())?,
        cookies: cookies.ok_or_else(|| "missing cookies".to_string())?,
        count: count.clamp(1, 8),
        frames: frames.clamp(1, 100_000),
        plane_count: plane_count.clamp(0, 4),
        planes,
        plane_fnv,
        plane_bytes,
        output_buffer_key,
        output_dmabuf_key,
        output_width,
        output_height,
        output_stride,
        output_format,
        output_offset,
        handle_count,
    })
}

fn probe_points(width: u32, height: u32) -> [(u32, u32); 9] {
    let cx = width / 2;
    let cy = height / 2;
    [
        (0, 0),
        (cx, 0),
        (width - 1, 0),
        (0, cy),
        (cx, cy),
        (width - 1, cy),
        (0, height - 1),
        (cx, height - 1),
        (width - 1, height - 1),
    ]
}

unsafe fn probe_linear_dmabuf(fd: c_int, meta: &AhbMeta) -> bool {
    if meta.width == 0 || meta.height == 0 || meta.stride < meta.width || meta.format != 1 {
        line(
            "dmabuf",
            "linear",
            format!(
                "SKIP|unsupported {}x{} stride={} format={}",
                meta.width, meta.height, meta.stride, meta.format
            ),
        );
        return false;
    }
    let pitch = meta.stride as usize * 4;
    let seek_size = lseek(fd, 0, 2);
    if seek_size < 0 {
        line(
            "dmabuf",
            "fd_size",
            format!(
                "FAIL|lseek_end={seek_size}: {}",
                std::io::Error::last_os_error()
            ),
        );
    } else {
        line("dmabuf", "fd_size", format!("PASS|lseek_end={seek_size}"));
    }
    let len = match pitch.checked_mul(meta.height as usize) {
        Some(v) if v > 0 => v,
        _ => {
            line("dmabuf", "linear", "FAIL|size overflow");
            return false;
        }
    };
    let map = mmap(ptr::null_mut(), len, PROT_READ, MAP_SHARED, fd, 0);
    if map as isize == -1 {
        line(
            "dmabuf",
            "linear",
            format!("FAIL|mmap len={len}: {}", std::io::Error::last_os_error()),
        );
        return false;
    }
    let base = map as *const u8;
    let mut matched = 0;
    let mut seen = [0u32; 9];
    for (i, (x, y)) in probe_points(meta.width, meta.height)
        .into_iter()
        .enumerate()
    {
        let off = y as usize * pitch + x as usize * 4;
        let got = (base.add(off) as *const u32).read_unaligned();
        seen[i] = got;
        if got == meta.cookies[i] {
            matched += 1;
        }
    }
    let unmap_rc = munmap(map, len);
    let ok = matched == 9 && unmap_rc == 0;
    line(
        "dmabuf",
        "linear",
        format!(
            "{}|match={matched}/9|{}x{} stride={} pitch={} len={} expected={:?} got={seen:?} unmap={unmap_rc}",
            if ok { "PASS" } else { "FAIL" },
            meta.width,
            meta.height,
            meta.stride,
            pitch,
            len,
            meta.cookies
        ),
    );
    ok
}

unsafe fn load_ring_fns(lib: *mut c_void, device: VkDevice) -> Option<RingFns> {
    let get_proc: FnGetDeviceProcAddr = symbol(lib, "vkGetDeviceProcAddr")?;
    let name = std::ffi::CString::new("vkGetFenceFdKHR").unwrap();
    let raw = get_proc(device, name.as_ptr());
    if raw.is_null() {
        line("ring", "FAIL", "vkGetDeviceProcAddr(vkGetFenceFdKHR)");
        return None;
    }
    Some(RingFns {
        create_image: symbol(lib, "vkCreateImage")?,
        destroy_image: symbol(lib, "vkDestroyImage")?,
        get_image_requirements: symbol(lib, "vkGetImageMemoryRequirements2")?,
        bind_image: symbol(lib, "vkBindImageMemory")?,
        allocate_memory: symbol(lib, "vkAllocateMemory")?,
        free_memory: symbol(lib, "vkFreeMemory")?,
        get_memory_props: symbol(lib, "vkGetPhysicalDeviceMemoryProperties")?,
        create_buffer: symbol(lib, "vkCreateBuffer")?,
        destroy_buffer: symbol(lib, "vkDestroyBuffer")?,
        get_buffer_requirements: symbol(lib, "vkGetBufferMemoryRequirements")?,
        bind_buffer: symbol(lib, "vkBindBufferMemory")?,
        map_memory: symbol(lib, "vkMapMemory")?,
        unmap_memory: symbol(lib, "vkUnmapMemory")?,
        create_pool: symbol(lib, "vkCreateCommandPool")?,
        destroy_pool: symbol(lib, "vkDestroyCommandPool")?,
        allocate_commands: symbol(lib, "vkAllocateCommandBuffers")?,
        begin_command: symbol(lib, "vkBeginCommandBuffer")?,
        end_command: symbol(lib, "vkEndCommandBuffer")?,
        pipeline_barrier: symbol(lib, "vkCmdPipelineBarrier")?,
        clear_color: symbol(lib, "vkCmdClearColorImage")?,
        copy_image_to_buffer: symbol(lib, "vkCmdCopyImageToBuffer")?,
        create_fence: symbol(lib, "vkCreateFence")?,
        destroy_fence: symbol(lib, "vkDestroyFence")?,
        wait_fences: symbol(lib, "vkWaitForFences")?,
        queue_submit: symbol(lib, "vkQueueSubmit")?,
        reset_pool: symbol(lib, "vkResetCommandPool")?,
        reset_fences: symbol(lib, "vkResetFences")?,
        get_fence_fd: std::mem::transmute_copy(&raw),
    })
}

unsafe fn destroy_pooled_slot(api: &RingFns, device: VkDevice, slot: &PooledSlot) {
    (api.destroy_fence)(device, slot.fence, ptr::null());
    (api.destroy_pool)(device, slot.command_pool, ptr::null());
    (api.destroy_buffer)(device, slot.readback, ptr::null());
    (api.free_memory)(device, slot.readback_memory, ptr::null());
    (api.destroy_image)(device, slot.image, ptr::null());
    (api.free_memory)(device, slot.image_memory, ptr::null());
}

unsafe fn create_pooled_slot(
    api: &RingFns,
    physical: VkPhysicalDevice,
    device: VkDevice,
    family_index: u32,
    fd: c_int,
    meta: &AhbMeta,
    slot_index: usize,
) -> Option<PooledSlot> {
    let pitch = meta.stride as u64 * 4;
    let image_bytes = pitch * meta.height as u64;
    let mut mem_props: VkPhysicalDeviceMemoryProperties = std::mem::zeroed();
    (api.get_memory_props)(physical, &mut mem_props);

    let layout = VkSubresourceLayout {
        offset: 0,
        size: image_bytes,
        row_pitch: pitch,
        array_pitch: 0,
        depth_pitch: 0,
    };
    let external = VkExternalMemoryImageCreateInfo {
        s_type: VK_STRUCTURE_TYPE_EXTERNAL_MEMORY_IMAGE_CREATE_INFO,
        p_next: ptr::null(),
        handle_types: VK_EXTERNAL_MEMORY_HANDLE_TYPE_DMA_BUF_BIT_EXT,
    };
    let explicit = VkImageDrmFormatModifierExplicitCreateInfo {
        s_type: VK_STRUCTURE_TYPE_IMAGE_DRM_FORMAT_MODIFIER_EXPLICIT_CREATE_INFO_EXT,
        p_next: &external as *const _ as *const c_void,
        drm_format_modifier: DRM_FORMAT_MOD_LINEAR,
        drm_format_modifier_plane_count: 1,
        p_plane_layouts: &layout,
    };
    let image_info = VkImageCreateInfo {
        s_type: VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO,
        p_next: &explicit as *const _ as *const c_void,
        flags: 0,
        image_type: VK_IMAGE_TYPE_2D,
        format: VK_FORMAT_R8G8B8A8_UNORM,
        extent: VkExtent3D {
            width: meta.width,
            height: meta.height,
            depth: 1,
        },
        mip_levels: 1,
        array_layers: 1,
        samples: 1,
        tiling: VK_IMAGE_TILING_DRM_FORMAT_MODIFIER_EXT,
        usage: VK_IMAGE_USAGE_TRANSFER_SRC_BIT | VK_IMAGE_USAGE_TRANSFER_DST_BIT,
        sharing_mode: VK_SHARING_MODE_EXCLUSIVE,
        queue_family_index_count: 0,
        p_queue_family_indices: ptr::null(),
        initial_layout: VK_IMAGE_LAYOUT_UNDEFINED,
    };
    let mut image: VkImage = ptr::null_mut();
    let r = (api.create_image)(device, &image_info, ptr::null(), &mut image);
    if r != VK_SUCCESS {
        line(
            "pool",
            "FAIL",
            format!("slot={slot_index} create image {r}"),
        );
        return None;
    }

    let req_info = VkImageMemoryRequirementsInfo2 {
        s_type: VK_STRUCTURE_TYPE_IMAGE_MEMORY_REQUIREMENTS_INFO_2,
        p_next: ptr::null(),
        image,
    };
    let mut req2 = VkMemoryRequirements2 {
        s_type: VK_STRUCTURE_TYPE_MEMORY_REQUIREMENTS_2,
        p_next: ptr::null_mut(),
        requirements: VkMemoryRequirements::default(),
    };
    (api.get_image_requirements)(device, &req_info, &mut req2);
    let req = req2.requirements;
    let Some(image_type_index) = choose_memory_type(&mem_props, req.memory_type_bits) else {
        line(
            "pool",
            "FAIL",
            format!("slot={slot_index} image memory type"),
        );
        (api.destroy_image)(device, image, ptr::null());
        return None;
    };
    let mut import = VkImportMemoryFdInfo {
        s_type: VK_STRUCTURE_TYPE_IMPORT_MEMORY_FD_INFO_KHR,
        p_next: ptr::null(),
        handle_type: VK_EXTERNAL_MEMORY_HANDLE_TYPE_DMA_BUF_BIT_EXT,
        fd: dup(fd),
    };
    if import.fd < 0 {
        line("pool", "FAIL", format!("slot={slot_index} dup image fd"));
        (api.destroy_image)(device, image, ptr::null());
        return None;
    }
    let alloc = VkMemoryAllocateInfo {
        s_type: VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO,
        p_next: &mut import as *mut _ as *const c_void,
        allocation_size: req.size,
        memory_type_index: image_type_index,
    };
    let mut image_memory: VkDeviceMemory = ptr::null_mut();
    let r = (api.allocate_memory)(device, &alloc, ptr::null(), &mut image_memory);
    close(import.fd);
    if r != VK_SUCCESS {
        line(
            "pool",
            "FAIL",
            format!("slot={slot_index} image import {r}"),
        );
        (api.destroy_image)(device, image, ptr::null());
        return None;
    }
    let r = (api.bind_image)(device, image, image_memory, 0);
    if r != VK_SUCCESS {
        line("pool", "FAIL", format!("slot={slot_index} bind image {r}"));
        (api.destroy_image)(device, image, ptr::null());
        (api.free_memory)(device, image_memory, ptr::null());
        return None;
    }

    let buffer_info = VkBufferCreateInfo {
        s_type: VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO,
        p_next: ptr::null(),
        flags: 0,
        size: image_bytes,
        usage: VK_BUFFER_USAGE_TRANSFER_DST_BIT,
        sharing_mode: VK_SHARING_MODE_EXCLUSIVE,
        queue_family_index_count: 0,
        p_queue_family_indices: ptr::null(),
    };
    let mut readback: VkBuffer = ptr::null_mut();
    let r = (api.create_buffer)(device, &buffer_info, ptr::null(), &mut readback);
    if r != VK_SUCCESS {
        line(
            "pool",
            "FAIL",
            format!("slot={slot_index} readback buffer {r}"),
        );
        (api.destroy_image)(device, image, ptr::null());
        (api.free_memory)(device, image_memory, ptr::null());
        return None;
    }
    let mut readback_req = VkMemoryRequirements::default();
    (api.get_buffer_requirements)(device, readback, &mut readback_req);
    let Some(readback_type_index) = choose_memory_type(&mem_props, readback_req.memory_type_bits)
    else {
        line(
            "pool",
            "FAIL",
            format!("slot={slot_index} readback memory type"),
        );
        (api.destroy_buffer)(device, readback, ptr::null());
        (api.destroy_image)(device, image, ptr::null());
        (api.free_memory)(device, image_memory, ptr::null());
        return None;
    };
    let readback_alloc = VkMemoryAllocateInfo {
        s_type: VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO,
        p_next: ptr::null(),
        allocation_size: readback_req.size,
        memory_type_index: readback_type_index,
    };
    let mut readback_memory: VkDeviceMemory = ptr::null_mut();
    let r = (api.allocate_memory)(device, &readback_alloc, ptr::null(), &mut readback_memory);
    if r != VK_SUCCESS || (api.bind_buffer)(device, readback, readback_memory, 0) != VK_SUCCESS {
        line(
            "pool",
            "FAIL",
            format!("slot={slot_index} readback allocation/bind"),
        );
        (api.destroy_buffer)(device, readback, ptr::null());
        if r == VK_SUCCESS {
            (api.free_memory)(device, readback_memory, ptr::null());
        }
        (api.destroy_image)(device, image, ptr::null());
        (api.free_memory)(device, image_memory, ptr::null());
        return None;
    }

    let pool_info = VkCommandPoolCreateInfo {
        s_type: VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO,
        p_next: ptr::null(),
        flags: 0,
        queue_family_index: family_index,
    };
    let mut command_pool: VkCommandPool = ptr::null_mut();
    let r = (api.create_pool)(device, &pool_info, ptr::null(), &mut command_pool);
    if r != VK_SUCCESS {
        line(
            "pool",
            "FAIL",
            format!("slot={slot_index} command pool {r}"),
        );
        (api.destroy_buffer)(device, readback, ptr::null());
        (api.free_memory)(device, readback_memory, ptr::null());
        (api.destroy_image)(device, image, ptr::null());
        (api.free_memory)(device, image_memory, ptr::null());
        return None;
    }
    let cmd_alloc = VkCommandBufferAllocateInfo {
        s_type: VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO,
        p_next: ptr::null(),
        command_pool,
        level: VK_COMMAND_BUFFER_LEVEL_PRIMARY,
        command_buffer_count: 1,
    };
    let mut command: VkCommandBuffer = ptr::null_mut();
    let r = (api.allocate_commands)(device, &cmd_alloc, &mut command);
    if r != VK_SUCCESS {
        line("pool", "FAIL", format!("slot={slot_index} command {r}"));
        (api.destroy_pool)(device, command_pool, ptr::null());
        (api.destroy_buffer)(device, readback, ptr::null());
        (api.free_memory)(device, readback_memory, ptr::null());
        (api.destroy_image)(device, image, ptr::null());
        (api.free_memory)(device, image_memory, ptr::null());
        return None;
    }

    let export_info = VkExportFenceCreateInfo {
        s_type: VK_STRUCTURE_TYPE_EXPORT_FENCE_CREATE_INFO,
        p_next: ptr::null(),
        handle_types: VK_EXTERNAL_FENCE_HANDLE_TYPE_SYNC_FD_BIT,
    };
    let fence_info = VkFenceCreateInfo {
        s_type: VK_STRUCTURE_TYPE_FENCE_CREATE_INFO,
        p_next: &export_info as *const _ as *const c_void,
        flags: 0,
    };
    let mut fence: VkFence = ptr::null_mut();
    let r = (api.create_fence)(device, &fence_info, ptr::null(), &mut fence);
    if r != VK_SUCCESS {
        line("pool", "FAIL", format!("slot={slot_index} fence {r}"));
        (api.destroy_pool)(device, command_pool, ptr::null());
        (api.destroy_buffer)(device, readback, ptr::null());
        (api.free_memory)(device, readback_memory, ptr::null());
        (api.destroy_image)(device, image, ptr::null());
        (api.free_memory)(device, image_memory, ptr::null());
        return None;
    }

    line(
        "pool",
        "INIT",
        format!(
            "slot={slot_index} fd={fd} image={:?} readback={:?} fence={:?}",
            image, readback, fence
        ),
    );
    Some(PooledSlot {
        image,
        image_memory,
        readback,
        readback_memory,
        command_pool,
        command,
        fence,
        readback_size: readback_req.size,
        pitch,
        width: meta.width,
        height: meta.height,
    })
}

unsafe fn verify_readback_region(
    map_memory: FnMapMemory,
    unmap_memory: FnUnmapMemory,
    device: VkDevice,
    memory: VkDeviceMemory,
    size: u64,
    pitch: u64,
    width: u32,
    height: u32,
    expected_pixel: u32,
    frame: usize,
    slot: usize,
    verbose: bool,
) -> bool {
    let mut mapped: *mut c_void = ptr::null_mut();
    let map_rc = map_memory(device, memory, 0, size, 0, &mut mapped);
    if map_rc != VK_SUCCESS || mapped.is_null() {
        line(
            "dmabuf",
            "image_readback",
            format!("frame={frame} slot={slot} FAIL|map {map_rc}"),
        );
        return false;
    }
    let base = mapped as *const u8;
    let mut checked = 0usize;
    let mut matched = 0usize;
    for (x, y) in probe_points(width, height) {
        let off = (y as usize * pitch as usize) + x as usize * 4;
        let pixel = (base.add(off) as *const u32).read_unaligned();
        checked += 1;
        if pixel == expected_pixel {
            matched += 1;
        }
    }
    let ok = checked == 9 && matched == 9;
    if verbose || !ok {
        line(
            "dmabuf",
            "image_readback",
            format!(
                "frame={frame} slot={slot} {}|match={matched}/{checked} expected={expected_pixel:#010x}",
                if ok { "PASS" } else { "FAIL" }
            ),
        );
    }
    unmap_memory(device, memory);
    ok
}

unsafe fn record_pooled_frame(
    api: &RingFns,
    device: VkDevice,
    slot: &PooledSlot,
    color: [f32; 4],
) -> bool {
    let r = (api.reset_pool)(device, slot.command_pool, 0);
    if r != VK_SUCCESS {
        line("pool", "FAIL", format!("reset command pool {r}"));
        return false;
    }
    let begin_info = VkCommandBufferBeginInfo {
        s_type: VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO,
        p_next: ptr::null(),
        flags: VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT,
        p_inheritance_info: ptr::null(),
    };
    let r = (api.begin_command)(slot.command, &begin_info);
    if r != VK_SUCCESS {
        line("pool", "FAIL", format!("begin command {r}"));
        return false;
    }
    let range = VkImageSubresourceRange {
        aspect_mask: VK_IMAGE_ASPECT_COLOR_BIT,
        base_mip_level: 0,
        level_count: 1,
        base_array_layer: 0,
        layer_count: 1,
    };
    let to_dst = VkImageMemoryBarrier {
        s_type: VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER,
        p_next: ptr::null(),
        src_access_mask: 0,
        dst_access_mask: VK_ACCESS_TRANSFER_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        new_layout: VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
        src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
        dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
        image: slot.image,
        subresource_range: range,
    };
    (api.pipeline_barrier)(
        slot.command,
        VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
        VK_PIPELINE_STAGE_TRANSFER_BIT,
        0,
        0,
        ptr::null(),
        0,
        ptr::null(),
        1,
        &to_dst,
    );
    let clear = VkClearColorValue { float32: color };
    (api.clear_color)(
        slot.command,
        slot.image,
        VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
        &clear,
        1,
        &range,
    );
    let to_src = VkImageMemoryBarrier {
        s_type: VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER,
        p_next: ptr::null(),
        src_access_mask: VK_ACCESS_TRANSFER_WRITE_BIT,
        dst_access_mask: VK_ACCESS_TRANSFER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
        src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
        dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
        image: slot.image,
        subresource_range: range,
    };
    (api.pipeline_barrier)(
        slot.command,
        VK_PIPELINE_STAGE_TRANSFER_BIT,
        VK_PIPELINE_STAGE_TRANSFER_BIT,
        0,
        0,
        ptr::null(),
        0,
        ptr::null(),
        1,
        &to_src,
    );
    let copy = VkBufferImageCopy {
        buffer_offset: 0,
        buffer_row_length: 0,
        buffer_image_height: 0,
        image_subresource: VkImageSubresourceLayers {
            aspect_mask: VK_IMAGE_ASPECT_COLOR_BIT,
            mip_level: 0,
            base_array_layer: 0,
            layer_count: 1,
        },
        image_offset: VkOffset3D::default(),
        image_extent: VkExtent3D {
            width: slot.width,
            height: slot.height,
            depth: 1,
        },
    };
    (api.copy_image_to_buffer)(
        slot.command,
        slot.image,
        VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
        slot.readback,
        1,
        &copy,
    );
    let host_barrier = VkMemoryBarrier {
        s_type: VK_STRUCTURE_TYPE_MEMORY_BARRIER,
        p_next: ptr::null(),
        src_access_mask: VK_ACCESS_TRANSFER_WRITE_BIT,
        dst_access_mask: VK_ACCESS_HOST_READ_BIT,
    };
    (api.pipeline_barrier)(
        slot.command,
        VK_PIPELINE_STAGE_TRANSFER_BIT,
        VK_PIPELINE_STAGE_HOST_BIT,
        0,
        1,
        &host_barrier as *const _ as *const c_void,
        0,
        ptr::null(),
        0,
        ptr::null(),
    );
    let r = (api.end_command)(slot.command);
    if r != VK_SUCCESS {
        line("pool", "FAIL", format!("end command {r}"));
        return false;
    }
    true
}

unsafe fn export_fence_sync_fd_via(
    get_fence_fd: FnGetFenceFdKhr,
    device: VkDevice,
    fence: VkFence,
) -> Result<c_int, String> {
    let info = VkFenceGetFdInfoKhr {
        s_type: VK_STRUCTURE_TYPE_FENCE_GET_FD_INFO_KHR,
        p_next: ptr::null(),
        fence,
        handle_type: VK_EXTERNAL_FENCE_HANDLE_TYPE_SYNC_FD_BIT,
    };
    let mut fd = -1;
    let rc = get_fence_fd(device, &info, &mut fd);
    if rc != VK_SUCCESS {
        return Err(format!("vkGetFenceFdKHR rc={rc}"));
    }
    Ok(fd)
}

unsafe fn render_pooled_frame(
    api: &RingFns,
    device: VkDevice,
    queue: VkQueue,
    slot: &PooledSlot,
    color: [f32; 4],
    expected_pixel: u32,
    frame: usize,
    slot_index: usize,
    verbose: bool,
    stats: &Arc<RingStats>,
) -> Option<AsyncCompletion> {
    if !record_pooled_frame(api, device, slot, color) {
        return None;
    }
    let reset_rc = (api.reset_fences)(device, 1, &slot.fence);
    if reset_rc != VK_SUCCESS {
        line(
            "pool",
            "FAIL",
            format!("frame={frame} slot={slot_index} reset fence {reset_rc}"),
        );
        return None;
    }
    let submit = VkSubmitInfo {
        s_type: VK_STRUCTURE_TYPE_SUBMIT_INFO,
        p_next: ptr::null(),
        wait_semaphore_count: 0,
        p_wait_semaphores: ptr::null(),
        p_wait_dst_stage_mask: ptr::null(),
        command_buffer_count: 1,
        p_command_buffers: &slot.command,
        signal_semaphore_count: 0,
        p_signal_semaphores: ptr::null(),
    };
    let submit_rc = (api.queue_submit)(queue, 1, &submit, slot.fence);
    if submit_rc != VK_SUCCESS {
        line(
            "pool",
            "FAIL",
            format!("frame={frame} slot={slot_index} submit {submit_rc}"),
        );
        return None;
    }

    match export_fence_sync_fd_via(api.get_fence_fd, device, slot.fence) {
        Ok(acquire_fd) if acquire_fd >= 0 => {
            if verbose {
                super::sync_file_info(
                    &format!("dmabuf|sync_file(export frame={frame} slot={slot_index})"),
                    acquire_fd,
                );
            }
            let mut pfd = PollFd {
                fd: acquire_fd,
                events: POLLIN,
                revents: 0,
            };
            let initial_rc = poll(&mut pfd, 1, 0);
            if initial_rc == 0 {
                stats.export_pending.fetch_add(1, Ordering::Relaxed);
                if verbose {
                    line("dmabuf", "sync_file(export poll)", "PENDING");
                }
            } else if initial_rc > 0 && pfd.revents & POLLIN != 0 {
                stats.export_signaled.fetch_add(1, Ordering::Relaxed);
                if verbose {
                    line("dmabuf", "sync_file(export poll)", "ALREADY_SIGNALED");
                }
            } else {
                line(
                    "dmabuf",
                    "FAIL",
                    format!(
                        "frame={frame} slot={slot_index} export poll rc={initial_rc} revents={:#x}",
                        pfd.revents
                    ),
                );
                close(acquire_fd);
                return None;
            }
            let completion_fd = dup(acquire_fd);
            if completion_fd < 0 {
                line(
                    "dmabuf",
                    "FAIL",
                    format!("frame={frame} slot={slot_index} dup sync_file"),
                );
                close(acquire_fd);
                return None;
            }
            let completion = PooledCompletion {
                device: device as usize,
                readback_memory: slot.readback_memory as usize,
                readback_size: slot.readback_size,
                pitch: slot.pitch,
                width: slot.width,
                height: slot.height,
                expected_pixel,
                frame,
                slot: slot_index,
                completion_fd,
                map_memory: api.map_memory,
                unmap_memory: api.unmap_memory,
                verbose,
                stats: Arc::clone(stats),
            };
            let join = std::thread::Builder::new()
                .name(format!("husk-pool-frame-{frame}"))
                .spawn(move || {
                    let completion = completion;
                    let mut pfd = PollFd {
                        fd: completion.completion_fd,
                        events: POLLIN,
                        revents: 0,
                    };
                    let started = std::time::Instant::now();
                    let poll_rc = poll(&mut pfd, 1, 30_000);
                    let complete = poll_rc > 0 && pfd.revents & POLLIN != 0;
                    if complete {
                        completion
                            .stats
                            .completion_ok
                            .fetch_add(1, Ordering::Relaxed);
                        if completion.verbose {
                            super::sync_file_info(
                                &format!(
                                    "dmabuf|sync_file(signaled frame={} slot={})",
                                    completion.frame, completion.slot
                                ),
                                completion.completion_fd,
                            );
                            line(
                                "dmabuf",
                                "image_complete",
                                format!(
                                    "frame={} slot={} sync_file=READABLE elapsed_us={}",
                                    completion.frame,
                                    completion.slot,
                                    started.elapsed().as_micros()
                                ),
                            );
                        }
                        let ok = verify_readback_region(
                            completion.map_memory,
                            completion.unmap_memory,
                            completion.device as VkDevice,
                            completion.readback_memory as VkDeviceMemory,
                            completion.readback_size,
                            completion.pitch,
                            completion.width,
                            completion.height,
                            completion.expected_pixel,
                            completion.frame,
                            completion.slot,
                            completion.verbose,
                        );
                        if ok {
                            completion.stats.readback_ok.fetch_add(1, Ordering::Relaxed);
                        } else {
                            completion
                                .stats
                                .readback_fail
                                .fetch_add(1, Ordering::Relaxed);
                        }
                    } else {
                        completion
                            .stats
                            .completion_fail
                            .fetch_add(1, Ordering::Relaxed);
                        line(
                            "dmabuf",
                            "FAIL",
                            format!(
                                "frame={} slot={} sync_file poll={poll_rc} revents={:#x}",
                                completion.frame, completion.slot, pfd.revents
                            ),
                        );
                    }
                });
            match join {
                Ok(join) => {
                    if verbose {
                        line(
                            "dmabuf",
                            "image_async",
                            format!(
                                "frame={frame} slot={slot_index} acquire_fd={acquire_fd} kind=SYNC_FD submit=PASS"
                            ),
                        );
                    }
                    Some(AsyncCompletion {
                        acquire_fd,
                        join: Some(join),
                    })
                }
                Err(e) => {
                    close(acquire_fd);
                    line(
                        "dmabuf",
                        "FAIL",
                        format!("frame={frame} slot={slot_index} spawn completion: {e}"),
                    );
                    None
                }
            }
        }
        Ok(_) | Err(_) => {
            stats.fallback.fetch_add(1, Ordering::Relaxed);
            if verbose {
                line(
                    "dmabuf",
                    "WARN",
                    format!(
                        "frame={frame} slot={slot_index} SYNC_FD unavailable; eventfd fallback"
                    ),
                );
            }
            let acquire_fd = eventfd(0, 0);
            let write_fd = if acquire_fd >= 0 { dup(acquire_fd) } else { -1 };
            if acquire_fd < 0 || write_fd < 0 {
                if acquire_fd >= 0 {
                    close(acquire_fd);
                }
                if write_fd >= 0 {
                    close(write_fd);
                }
                line(
                    "dmabuf",
                    "FAIL",
                    format!("frame={frame} slot={slot_index} eventfd fallback"),
                );
                return None;
            }
            let stats_thread = Arc::clone(stats);
            let device_addr = device as usize;
            let wait_fences = api.wait_fences;
            let map_memory = api.map_memory;
            let unmap_memory = api.unmap_memory;
            let fence_addr = slot.fence as usize;
            let readback_memory_addr = slot.readback_memory as usize;
            let readback_size = slot.readback_size;
            let pitch = slot.pitch;
            let width = slot.width;
            let height = slot.height;
            let join = std::thread::Builder::new()
                .name(format!("husk-pool-eventfd-{frame}"))
                .spawn(move || {
                    let device = device_addr as VkDevice;
                    let fence = fence_addr as VkFence;
                    let wait_rc = wait_fences(device, 1, &fence, VK_TRUE, u64::MAX);
                    if wait_rc == VK_SUCCESS {
                        let one = 1u64;
                        let wrote = write(write_fd, &one as *const u64 as *const c_void, 8);
                        close(write_fd);
                        if wrote == 8 {
                            let ok = verify_readback_region(
                                map_memory,
                                unmap_memory,
                                device,
                                readback_memory_addr as VkDeviceMemory,
                                readback_size,
                                pitch,
                                width,
                                height,
                                expected_pixel,
                                frame,
                                slot_index,
                                verbose,
                            );
                            if ok {
                                stats_thread.readback_ok.fetch_add(1, Ordering::Relaxed);
                            } else {
                                stats_thread.readback_fail.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    } else {
                        close(write_fd);
                        line(
                            "dmabuf",
                            "FAIL",
                            format!(
                                "frame={frame} slot={slot_index} fallback wait={}",
                                api_result(wait_rc)
                            ),
                        );
                    }
                });
            match join {
                Ok(join) => Some(AsyncCompletion {
                    acquire_fd,
                    join: Some(join),
                }),
                Err(e) => {
                    close(acquire_fd);
                    close(write_fd);
                    line(
                        "dmabuf",
                        "FAIL",
                        format!("frame={frame} slot={slot_index} spawn eventfd: {e}"),
                    );
                    None
                }
            }
        }
    }
}

unsafe fn render_linear_image(
    lib: *mut c_void,
    physical: VkPhysicalDevice,
    device: VkDevice,
    queue: VkQueue,
    family_index: u32,
    fd: c_int,
    meta: &AhbMeta,
    color: [f32; 4],
    expected_pixel: u32,
    frame: usize,
    slot: usize,
    mut async_out: Option<&mut AsyncCompletion>,
) -> bool {
    if let Some(out) = async_out.as_deref_mut() {
        *out = AsyncCompletion::default();
    }
    if meta.width == 0
        || meta.height == 0
        || meta.stride < meta.width
        || meta.format != 1
        || meta.width > i32::MAX as u32
        || meta.height > i32::MAX as u32
    {
        line("dmabuf", "image", "SKIP|unsupported AHB metadata");
        return false;
    }
    let pitch = meta.stride as u64 * 4;
    let image_bytes = pitch * meta.height as u64;

    let Some(create_image): Option<FnCreateImage> = symbol(lib, "vkCreateImage") else {
        line("dmabuf", "image", "FAIL|vkCreateImage missing");
        return false;
    };
    let Some(destroy_image): Option<FnDestroyImage> = symbol(lib, "vkDestroyImage") else {
        line("dmabuf", "image", "FAIL|vkDestroyImage missing");
        return false;
    };
    let Some(get_image_requirements): Option<FnGetImageRequirements2> =
        symbol(lib, "vkGetImageMemoryRequirements2")
    else {
        line(
            "dmabuf",
            "image",
            "FAIL|vkGetImageMemoryRequirements2 missing",
        );
        return false;
    };
    let Some(bind_image): Option<FnBindImageMemory> = symbol(lib, "vkBindImageMemory") else {
        line("dmabuf", "image", "FAIL|vkBindImageMemory missing");
        return false;
    };
    let Some(allocate_memory): Option<FnAllocateMemory> = symbol(lib, "vkAllocateMemory") else {
        line("dmabuf", "image", "FAIL|vkAllocateMemory missing");
        return false;
    };
    let Some(free_memory): Option<FnFreeMemory> = symbol(lib, "vkFreeMemory") else {
        line("dmabuf", "image", "FAIL|vkFreeMemory missing");
        return false;
    };
    let Some(get_memory_props): Option<FnGetPhysicalDeviceMemoryProperties> =
        symbol(lib, "vkGetPhysicalDeviceMemoryProperties")
    else {
        line("dmabuf", "image", "FAIL|memory properties missing");
        return false;
    };
    let Some(create_buffer): Option<FnCreateBuffer> = symbol(lib, "vkCreateBuffer") else {
        line("dmabuf", "image", "FAIL|vkCreateBuffer missing");
        return false;
    };
    let Some(destroy_buffer): Option<FnDestroyBuffer> = symbol(lib, "vkDestroyBuffer") else {
        line("dmabuf", "image", "FAIL|vkDestroyBuffer missing");
        return false;
    };
    let Some(get_buffer_requirements): Option<FnGetBufferMemoryRequirements> =
        symbol(lib, "vkGetBufferMemoryRequirements")
    else {
        line("dmabuf", "image", "FAIL|buffer requirements missing");
        return false;
    };
    let Some(bind_buffer): Option<FnBindBufferMemory> = symbol(lib, "vkBindBufferMemory") else {
        line("dmabuf", "image", "FAIL|vkBindBufferMemory missing");
        return false;
    };
    let Some(map_memory): Option<FnMapMemory> = symbol(lib, "vkMapMemory") else {
        line("dmabuf", "image", "FAIL|vkMapMemory missing");
        return false;
    };
    let Some(unmap_memory): Option<FnUnmapMemory> = symbol(lib, "vkUnmapMemory") else {
        line("dmabuf", "image", "FAIL|vkUnmapMemory missing");
        return false;
    };
    let Some(create_pool): Option<FnCreateCommandPool> = symbol(lib, "vkCreateCommandPool") else {
        line("dmabuf", "image", "FAIL|vkCreateCommandPool missing");
        return false;
    };
    let Some(destroy_pool): Option<FnDestroyCommandPool> = symbol(lib, "vkDestroyCommandPool")
    else {
        line("dmabuf", "image", "FAIL|vkDestroyCommandPool missing");
        return false;
    };
    let Some(allocate_commands): Option<FnAllocateCommandBuffers> =
        symbol(lib, "vkAllocateCommandBuffers")
    else {
        line("dmabuf", "image", "FAIL|vkAllocateCommandBuffers missing");
        return false;
    };
    let Some(begin_command): Option<FnBeginCommandBuffer> = symbol(lib, "vkBeginCommandBuffer")
    else {
        line("dmabuf", "image", "FAIL|vkBeginCommandBuffer missing");
        return false;
    };
    let Some(end_command): Option<FnEndCommandBuffer> = symbol(lib, "vkEndCommandBuffer") else {
        line("dmabuf", "image", "FAIL|vkEndCommandBuffer missing");
        return false;
    };
    let Some(pipeline_barrier): Option<FnCmdPipelineBarrier> = symbol(lib, "vkCmdPipelineBarrier")
    else {
        line("dmabuf", "image", "FAIL|vkCmdPipelineBarrier missing");
        return false;
    };
    let Some(clear_color): Option<FnCmdClearColorImage> = symbol(lib, "vkCmdClearColorImage")
    else {
        line("dmabuf", "image", "FAIL|vkCmdClearColorImage missing");
        return false;
    };
    let Some(copy_image_to_buffer): Option<FnCmdCopyImageToBuffer> =
        symbol(lib, "vkCmdCopyImageToBuffer")
    else {
        line("dmabuf", "image", "FAIL|vkCmdCopyImageToBuffer missing");
        return false;
    };
    let Some(create_fence): Option<FnCreateFence> = symbol(lib, "vkCreateFence") else {
        line("dmabuf", "image", "FAIL|vkCreateFence missing");
        return false;
    };
    let Some(destroy_fence): Option<FnDestroyFence> = symbol(lib, "vkDestroyFence") else {
        line("dmabuf", "image", "FAIL|vkDestroyFence missing");
        return false;
    };
    let Some(wait_fences): Option<FnWaitForFences> = symbol(lib, "vkWaitForFences") else {
        line("dmabuf", "image", "FAIL|vkWaitForFences missing");
        return false;
    };
    let Some(queue_submit): Option<FnQueueSubmit> = symbol(lib, "vkQueueSubmit") else {
        line("dmabuf", "image", "FAIL|vkQueueSubmit missing");
        return false;
    };

    let layout = VkSubresourceLayout {
        offset: 0,
        size: image_bytes,
        row_pitch: pitch,
        array_pitch: 0,
        depth_pitch: 0,
    };
    let external = VkExternalMemoryImageCreateInfo {
        s_type: VK_STRUCTURE_TYPE_EXTERNAL_MEMORY_IMAGE_CREATE_INFO,
        p_next: ptr::null(),
        handle_types: VK_EXTERNAL_MEMORY_HANDLE_TYPE_DMA_BUF_BIT_EXT,
    };
    let explicit = VkImageDrmFormatModifierExplicitCreateInfo {
        s_type: VK_STRUCTURE_TYPE_IMAGE_DRM_FORMAT_MODIFIER_EXPLICIT_CREATE_INFO_EXT,
        p_next: &external as *const _ as *const c_void,
        drm_format_modifier: DRM_FORMAT_MOD_LINEAR,
        drm_format_modifier_plane_count: 1,
        p_plane_layouts: &layout,
    };
    let image_info = VkImageCreateInfo {
        s_type: VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO,
        p_next: &explicit as *const _ as *const c_void,
        flags: 0,
        image_type: VK_IMAGE_TYPE_2D,
        format: VK_FORMAT_R8G8B8A8_UNORM,
        extent: VkExtent3D {
            width: meta.width,
            height: meta.height,
            depth: 1,
        },
        mip_levels: 1,
        array_layers: 1,
        samples: 1,
        tiling: VK_IMAGE_TILING_DRM_FORMAT_MODIFIER_EXT,
        usage: VK_IMAGE_USAGE_TRANSFER_SRC_BIT | VK_IMAGE_USAGE_TRANSFER_DST_BIT,
        sharing_mode: VK_SHARING_MODE_EXCLUSIVE,
        queue_family_index_count: 0,
        p_queue_family_indices: ptr::null(),
        initial_layout: VK_IMAGE_LAYOUT_UNDEFINED,
    };
    let mut image: VkImage = ptr::null_mut();
    let r = create_image(device, &image_info, ptr::null(), &mut image);
    line(
        "dmabuf",
        "image_create",
        format!(
            "{}|{}x{} pitch={} modifier=LINEAR",
            api_result(r),
            meta.width,
            meta.height,
            pitch
        ),
    );
    if r != VK_SUCCESS {
        return false;
    }

    let req_info = VkImageMemoryRequirementsInfo2 {
        s_type: VK_STRUCTURE_TYPE_IMAGE_MEMORY_REQUIREMENTS_INFO_2,
        p_next: ptr::null(),
        image,
    };
    let mut req2 = VkMemoryRequirements2 {
        s_type: VK_STRUCTURE_TYPE_MEMORY_REQUIREMENTS_2,
        p_next: ptr::null_mut(),
        requirements: VkMemoryRequirements::default(),
    };
    get_image_requirements(device, &req_info, &mut req2);
    let req = req2.requirements;
    line(
        "dmabuf",
        "image_requirements",
        format!(
            "size={} align={} bits={:#x}",
            req.size, req.alignment, req.memory_type_bits
        ),
    );
    let mut mem_props: VkPhysicalDeviceMemoryProperties = std::mem::zeroed();
    get_memory_props(physical, &mut mem_props);
    let Some(image_type_index) = choose_memory_type(&mem_props, req.memory_type_bits) else {
        line("dmabuf", "image", "FAIL|no memory type");
        destroy_image(device, image, ptr::null());
        return false;
    };
    let mut import = VkImportMemoryFdInfo {
        s_type: VK_STRUCTURE_TYPE_IMPORT_MEMORY_FD_INFO_KHR,
        p_next: ptr::null(),
        handle_type: VK_EXTERNAL_MEMORY_HANDLE_TYPE_DMA_BUF_BIT_EXT,
        fd: dup(fd),
    };
    if import.fd < 0 {
        line("dmabuf", "image_import", "FAIL|dup");
        destroy_image(device, image, ptr::null());
        return false;
    }
    let alloc = VkMemoryAllocateInfo {
        s_type: VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO,
        p_next: &mut import as *mut _ as *const c_void,
        allocation_size: req.size,
        memory_type_index: image_type_index,
    };
    let mut image_memory: VkDeviceMemory = ptr::null_mut();
    let r = allocate_memory(device, &alloc, ptr::null(), &mut image_memory);
    line(
        "dmabuf",
        "image_import",
        format!(
            "{}|type={} size={} fd={}",
            api_result(r),
            image_type_index,
            req.size,
            fd
        ),
    );
    close(import.fd);
    if r != VK_SUCCESS {
        destroy_image(device, image, ptr::null());
        return false;
    }
    let r = bind_image(device, image, image_memory, 0);
    line("dmabuf", "image_bind", api_result(r));
    if r != VK_SUCCESS {
        free_memory(device, image_memory, ptr::null());
        destroy_image(device, image, ptr::null());
        return false;
    }

    let buffer_info = VkBufferCreateInfo {
        s_type: VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO,
        p_next: ptr::null(),
        flags: 0,
        size: image_bytes,
        usage: VK_BUFFER_USAGE_TRANSFER_DST_BIT,
        sharing_mode: VK_SHARING_MODE_EXCLUSIVE,
        queue_family_index_count: 0,
        p_queue_family_indices: ptr::null(),
    };
    let mut readback: VkBuffer = ptr::null_mut();
    let r = create_buffer(device, &buffer_info, ptr::null(), &mut readback);
    if r != VK_SUCCESS {
        line("dmabuf", "image", format!("FAIL|readback buffer {r}"));
        free_memory(device, image_memory, ptr::null());
        destroy_image(device, image, ptr::null());
        return false;
    }
    let mut readback_req = VkMemoryRequirements::default();
    get_buffer_requirements(device, readback, &mut readback_req);
    let Some(readback_type_index) = choose_memory_type(&mem_props, readback_req.memory_type_bits)
    else {
        line("dmabuf", "image", "FAIL|no readback memory type");
        destroy_buffer(device, readback, ptr::null());
        free_memory(device, image_memory, ptr::null());
        destroy_image(device, image, ptr::null());
        return false;
    };
    let readback_alloc = VkMemoryAllocateInfo {
        s_type: VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO,
        p_next: ptr::null(),
        allocation_size: readback_req.size,
        memory_type_index: readback_type_index,
    };
    let mut readback_memory: VkDeviceMemory = ptr::null_mut();
    let r = allocate_memory(device, &readback_alloc, ptr::null(), &mut readback_memory);
    if r != VK_SUCCESS || bind_buffer(device, readback, readback_memory, 0) != VK_SUCCESS {
        line("dmabuf", "image", "FAIL|readback allocation/bind");
        if r == VK_SUCCESS {
            free_memory(device, readback_memory, ptr::null());
        }
        destroy_buffer(device, readback, ptr::null());
        free_memory(device, image_memory, ptr::null());
        destroy_image(device, image, ptr::null());
        return false;
    }

    let pool_info = VkCommandPoolCreateInfo {
        s_type: VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO,
        p_next: ptr::null(),
        flags: 0,
        queue_family_index: family_index,
    };
    let mut pool: VkCommandPool = ptr::null_mut();
    let r = create_pool(device, &pool_info, ptr::null(), &mut pool);
    if r != VK_SUCCESS {
        line("dmabuf", "image", format!("FAIL|command pool {r}"));
        free_memory(device, readback_memory, ptr::null());
        destroy_buffer(device, readback, ptr::null());
        free_memory(device, image_memory, ptr::null());
        destroy_image(device, image, ptr::null());
        return false;
    }
    let cmd_alloc = VkCommandBufferAllocateInfo {
        s_type: VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO,
        p_next: ptr::null(),
        command_pool: pool,
        level: VK_COMMAND_BUFFER_LEVEL_PRIMARY,
        command_buffer_count: 1,
    };
    let mut command: VkCommandBuffer = ptr::null_mut();
    let r = allocate_commands(device, &cmd_alloc, &mut command);
    if r != VK_SUCCESS {
        line("dmabuf", "image", format!("FAIL|allocate command {r}"));
        destroy_pool(device, pool, ptr::null());
        free_memory(device, readback_memory, ptr::null());
        destroy_buffer(device, readback, ptr::null());
        free_memory(device, image_memory, ptr::null());
        destroy_image(device, image, ptr::null());
        return false;
    }
    let begin_info = VkCommandBufferBeginInfo {
        s_type: VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO,
        p_next: ptr::null(),
        flags: VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT,
        p_inheritance_info: ptr::null(),
    };
    let r = begin_command(command, &begin_info);
    if r != VK_SUCCESS {
        line("dmabuf", "image", format!("FAIL|begin command {r}"));
        destroy_pool(device, pool, ptr::null());
        free_memory(device, readback_memory, ptr::null());
        destroy_buffer(device, readback, ptr::null());
        free_memory(device, image_memory, ptr::null());
        destroy_image(device, image, ptr::null());
        return false;
    }

    let range = VkImageSubresourceRange {
        aspect_mask: VK_IMAGE_ASPECT_COLOR_BIT,
        base_mip_level: 0,
        level_count: 1,
        base_array_layer: 0,
        layer_count: 1,
    };
    let to_dst = VkImageMemoryBarrier {
        s_type: VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER,
        p_next: ptr::null(),
        src_access_mask: 0,
        dst_access_mask: VK_ACCESS_TRANSFER_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        new_layout: VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
        src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
        dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
        image,
        subresource_range: range,
    };
    pipeline_barrier(
        command,
        VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
        VK_PIPELINE_STAGE_TRANSFER_BIT,
        0,
        0,
        ptr::null(),
        0,
        ptr::null(),
        1,
        &to_dst,
    );
    let clear = VkClearColorValue { float32: color };
    clear_color(
        command,
        image,
        VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
        &clear,
        1,
        &range,
    );
    let to_src = VkImageMemoryBarrier {
        s_type: VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER,
        p_next: ptr::null(),
        src_access_mask: VK_ACCESS_TRANSFER_WRITE_BIT,
        dst_access_mask: VK_ACCESS_TRANSFER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
        src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
        dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
        image,
        subresource_range: range,
    };
    pipeline_barrier(
        command,
        VK_PIPELINE_STAGE_TRANSFER_BIT,
        VK_PIPELINE_STAGE_TRANSFER_BIT,
        0,
        0,
        ptr::null(),
        0,
        ptr::null(),
        1,
        &to_src,
    );
    let copy = VkBufferImageCopy {
        buffer_offset: 0,
        buffer_row_length: 0,
        buffer_image_height: 0,
        image_subresource: VkImageSubresourceLayers {
            aspect_mask: VK_IMAGE_ASPECT_COLOR_BIT,
            mip_level: 0,
            base_array_layer: 0,
            layer_count: 1,
        },
        image_offset: VkOffset3D::default(),
        image_extent: VkExtent3D {
            width: meta.width,
            height: meta.height,
            depth: 1,
        },
    };
    copy_image_to_buffer(
        command,
        image,
        VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
        readback,
        1,
        &copy,
    );
    let host_barrier = VkMemoryBarrier {
        s_type: VK_STRUCTURE_TYPE_MEMORY_BARRIER,
        p_next: ptr::null(),
        src_access_mask: VK_ACCESS_TRANSFER_WRITE_BIT,
        dst_access_mask: VK_ACCESS_HOST_READ_BIT,
    };
    pipeline_barrier(
        command,
        VK_PIPELINE_STAGE_TRANSFER_BIT,
        VK_PIPELINE_STAGE_HOST_BIT,
        0,
        1,
        &host_barrier as *const _ as *const c_void,
        0,
        ptr::null(),
        0,
        ptr::null(),
    );
    let r = end_command(command);
    if r != VK_SUCCESS {
        line("dmabuf", "image", format!("FAIL|end command {r}"));
        destroy_pool(device, pool, ptr::null());
        free_memory(device, readback_memory, ptr::null());
        destroy_buffer(device, readback, ptr::null());
        free_memory(device, image_memory, ptr::null());
        destroy_image(device, image, ptr::null());
        return false;
    }

    let export_info = VkExportFenceCreateInfo {
        s_type: VK_STRUCTURE_TYPE_EXPORT_FENCE_CREATE_INFO,
        p_next: ptr::null(),
        handle_types: VK_EXTERNAL_FENCE_HANDLE_TYPE_SYNC_FD_BIT,
    };
    let fence_info = VkFenceCreateInfo {
        s_type: VK_STRUCTURE_TYPE_FENCE_CREATE_INFO,
        p_next: if async_out.is_some() {
            &export_info as *const _ as *const c_void
        } else {
            ptr::null()
        },
        flags: 0,
    };
    let mut fence: VkFence = ptr::null_mut();
    let r = create_fence(device, &fence_info, ptr::null(), &mut fence);
    if r != VK_SUCCESS {
        line("dmabuf", "image", format!("FAIL|fence {r}"));
        destroy_pool(device, pool, ptr::null());
        free_memory(device, readback_memory, ptr::null());
        destroy_buffer(device, readback, ptr::null());
        free_memory(device, image_memory, ptr::null());
        destroy_image(device, image, ptr::null());
        return false;
    }
    let submit = VkSubmitInfo {
        s_type: VK_STRUCTURE_TYPE_SUBMIT_INFO,
        p_next: ptr::null(),
        wait_semaphore_count: 0,
        p_wait_semaphores: ptr::null(),
        p_wait_dst_stage_mask: ptr::null(),
        command_buffer_count: 1,
        p_command_buffers: &command,
        signal_semaphore_count: 0,
        p_signal_semaphores: ptr::null(),
    };
    let submit_rc = queue_submit(queue, 1, &submit, fence);
    if submit_rc == VK_SUCCESS {
        if let Some(out) = async_out.as_deref_mut() {
            match export_fence_sync_fd(lib, device, fence) {
                Ok(acquire_fd) if acquire_fd >= 0 => {
                    super::sync_file_info(
                        &format!("dmabuf|sync_file(export frame={frame} slot={slot})"),
                        acquire_fd,
                    );
                    let mut initial_pfd = PollFd {
                        fd: acquire_fd,
                        events: POLLIN,
                        revents: 0,
                    };
                    let initial_rc = poll(&mut initial_pfd, 1, 0);
                    line(
                        "dmabuf",
                        "sync_file(export poll)",
                        if initial_rc == 0 {
                            "PENDING".to_string()
                        } else if initial_rc > 0 && initial_pfd.revents & POLLIN != 0 {
                            "ALREADY_SIGNALED".to_string()
                        } else {
                            format!("POLLERR rc={initial_rc} revents={:#x}", initial_pfd.revents)
                        },
                    );
                    let completion_fd = dup(acquire_fd);
                    if completion_fd >= 0 {
                        let resources = CompletionResources {
                            device: device as usize,
                            fence: fence as usize,
                            command_pool: pool as usize,
                            readback_memory: readback_memory as usize,
                            readback: readback as usize,
                            image_memory: image_memory as usize,
                            image: image as usize,
                            completion_fd,
                            write_fd: -1,
                            gpu_complete: false,
                            wait_fences,
                            destroy_fence,
                            destroy_pool,
                            free_memory,
                            destroy_buffer,
                            destroy_image,
                            map_memory,
                            unmap_memory,
                            readback_size: readback_req.size,
                            pitch,
                            width: meta.width,
                            height: meta.height,
                            expected_pixel,
                            frame,
                            slot,
                        };
                        let join = std::thread::Builder::new()
                            .name(format!("husk-gpu-frame-{frame}"))
                            .spawn(move || {
                                let mut resources = resources;
                                let mut pfd = PollFd {
                                    fd: resources.completion_fd,
                                    events: POLLIN,
                                    revents: 0,
                                };
                                let started = std::time::Instant::now();
                                let poll_rc = poll(&mut pfd, 1, 30_000);
                                let complete =
                                    poll_rc > 0 && pfd.revents & POLLIN != 0;
                                if complete {
                                    resources.gpu_complete = true;
                                    super::sync_file_info(
                                        &format!(
                                            "dmabuf|sync_file(signaled frame={} slot={})",
                                            resources.frame, resources.slot
                                        ),
                                        resources.completion_fd,
                                    );
                                    line(
                                        "dmabuf",
                                        "image_complete",
                                        format!(
                                            "frame={} slot={} sync_file=READABLE elapsed_us={}",
                                            resources.frame,
                                            resources.slot,
                                            started.elapsed().as_micros()
                                        ),
                                    );
                                    resources.verify_readback();
                                } else {
                                    line(
                                        "dmabuf",
                                        "image_complete",
                                        format!(
                                            "frame={} slot={} sync_file=FAIL poll={poll_rc} revents={:#x}",
                                            resources.frame,
                                            resources.slot,
                                            pfd.revents
                                        ),
                                    );
                                }
                                drop(resources);
                            });
                        match join {
                            Ok(join) => {
                                out.acquire_fd = acquire_fd;
                                out.join = Some(join);
                                line(
                                    "dmabuf",
                                    "image_async",
                                    format!(
                                        "frame={frame} slot={slot} acquire_fd={acquire_fd} kind=SYNC_FD submit=PASS"
                                    ),
                                );
                                return true;
                            }
                            Err(e) => {
                                close(completion_fd);
                                close(acquire_fd);
                                line(
                                    "dmabuf",
                                    "image_async",
                                    format!("frame={frame} slot={slot} FAIL|sync_file spawn: {e}"),
                                );
                                return false;
                            }
                        }
                    }
                    close(acquire_fd);
                    line(
                        "dmabuf",
                        "image_async",
                        format!("frame={frame} slot={slot} FAIL|dup sync_file"),
                    );
                    return false;
                }
                Ok(fd) => {
                    line(
                        "dmabuf",
                        "image_async",
                        format!(
                            "frame={frame} slot={slot} WARN|SYNC_FD already signaled fd={fd}; using eventfd fallback"
                        ),
                    );
                }
                Err(e) => {
                    line(
                        "dmabuf",
                        "image_async",
                        format!(
                            "frame={frame} slot={slot} WARN|SYNC_FD: {e}; using eventfd fallback"
                        ),
                    );
                }
            }
            let acquire_fd = eventfd(0, 0);
            let write_fd = if acquire_fd >= 0 { dup(acquire_fd) } else { -1 };
            if acquire_fd >= 0 && write_fd >= 0 {
                let resources = CompletionResources {
                    device: device as usize,
                    fence: fence as usize,
                    command_pool: pool as usize,
                    readback_memory: readback_memory as usize,
                    readback: readback as usize,
                    image_memory: image_memory as usize,
                    image: image as usize,
                    completion_fd: -1,
                    write_fd,
                    gpu_complete: false,
                    wait_fences,
                    destroy_fence,
                    destroy_pool,
                    free_memory,
                    destroy_buffer,
                    destroy_image,
                    map_memory,
                    unmap_memory,
                    readback_size: readback_req.size,
                    pitch,
                    width: meta.width,
                    height: meta.height,
                    expected_pixel,
                    frame,
                    slot,
                };
                let join = std::thread::Builder::new()
                    .name(format!("husk-gpu-frame-{frame}"))
                    .spawn(move || {
                        let mut resources = resources;
                        let device = resources.device as VkDevice;
                        let fence = resources.fence as VkFence;
                        let wait_rc = (resources.wait_fences)(device, 1, &fence, VK_TRUE, u64::MAX);
                        if wait_rc == VK_SUCCESS {
                            let one = 1u64;
                            let wrote =
                                write(resources.write_fd, &one as *const u64 as *const c_void, 8);
                            resources.gpu_complete = true;
                            line(
                                "dmabuf",
                                "image_complete",
                                format!(
                                    "frame={} slot={} wait={} write={wrote}",
                                    resources.frame,
                                    resources.slot,
                                    api_result(wait_rc)
                                ),
                            );
                            resources.verify_readback();
                        } else {
                            line(
                                "dmabuf",
                                "image_complete",
                                format!(
                                    "frame={} slot={} wait={} acquire_fd=HUP",
                                    resources.frame,
                                    resources.slot,
                                    api_result(wait_rc)
                                ),
                            );
                        }
                        drop(resources);
                    });
                match join {
                    Ok(join) => {
                        out.acquire_fd = acquire_fd;
                        out.join = Some(join);
                        line(
                            "dmabuf",
                            "image_async",
                            format!(
                                "frame={frame} slot={slot} acquire_fd={acquire_fd} submit=PASS"
                            ),
                        );
                        return true;
                    }
                    Err(e) => {
                        // The closure, and with it CompletionResources, was
                        // dropped by Builder::spawn on this failure. Its Drop
                        // waits for the GPU and performs the cleanup.
                        close(acquire_fd);
                        line(
                            "dmabuf",
                            "image_async",
                            format!("frame={frame} slot={slot} FAIL|spawn: {e}"),
                        );
                        return false;
                    }
                }
            }
            if acquire_fd >= 0 {
                close(acquire_fd);
            }
            if write_fd >= 0 {
                close(write_fd);
            }
            line(
                "dmabuf",
                "image_async",
                format!(
                    "frame={frame} slot={slot} WARN|eventfd: {}",
                    std::io::Error::last_os_error()
                ),
            );
        }
    }
    let wait_rc = if submit_rc == VK_SUCCESS {
        wait_fences(device, 1, &fence, VK_TRUE, u64::MAX)
    } else {
        submit_rc
    };
    line(
        "dmabuf",
        "image_submit",
        format!("submit={submit_rc} wait={wait_rc}"),
    );

    let mut mapped: *mut c_void = ptr::null_mut();
    let mut pixels_ok = false;
    let mut dma_view_ok = false;
    if wait_rc == VK_SUCCESS {
        let map_rc = map_memory(
            device,
            readback_memory,
            0,
            readback_req.size,
            0,
            &mut mapped,
        );
        if map_rc == VK_SUCCESS && !mapped.is_null() {
            let base = mapped as *const u8;
            let mut checked = 0;
            let mut matched = 0;
            for (x, y) in probe_points(meta.width, meta.height) {
                let off = (y as usize * pitch as usize) + x as usize * 4;
                let pixel = (base.add(off) as *const u32).read_unaligned();
                checked += 1;
                if pixel == expected_pixel {
                    matched += 1;
                }
            }
            pixels_ok = checked == 9 && matched == 9;
            line(
                "dmabuf",
                "image_readback",
                format!(
                    "{}|match={matched}/{checked} expected={expected_pixel:#010x} first={:#010x}",
                    if pixels_ok { "PASS" } else { "FAIL" },
                    (base as *const u32).read_unaligned()
                ),
            );
            unmap_memory(device, readback_memory);
        } else {
            line("dmabuf", "image_readback", format!("FAIL|map {map_rc}"));
        }

        let dma_map = mmap(
            ptr::null_mut(),
            image_bytes as usize,
            PROT_READ,
            MAP_SHARED,
            fd,
            0,
        );
        if dma_map as isize != -1 {
            let base = dma_map as *const u8;
            let mut dma_matched = 0;
            for (x, y) in probe_points(meta.width, meta.height) {
                let off = (y as usize * pitch as usize) + x as usize * 4;
                if (base.add(off) as *const u32).read_unaligned() == expected_pixel {
                    dma_matched += 1;
                }
            }
            dma_view_ok = dma_matched == 9;
            line(
                "dmabuf",
                "image_dma_buf_view",
                format!(
                    "{}|match={dma_matched}/9 expected={expected_pixel:#010x} first={:#010x}",
                    if dma_view_ok { "PASS" } else { "FAIL" },
                    (base as *const u32).read_unaligned()
                ),
            );
            munmap(dma_map, image_bytes as usize);
        } else {
            line(
                "dmabuf",
                "image_dma_buf_view",
                format!("FAIL|mmap: {}", std::io::Error::last_os_error()),
            );
        }
    }

    destroy_fence(device, fence, ptr::null());
    destroy_pool(device, pool, ptr::null());
    free_memory(device, readback_memory, ptr::null());
    destroy_buffer(device, readback, ptr::null());
    free_memory(device, image_memory, ptr::null());
    destroy_image(device, image, ptr::null());
    pixels_ok && dma_view_ok
}

#[repr(C)]
struct VkExternalMemoryProperties {
    features: u32,
    export_from_imported: u32,
    compatible: u32,
}

#[repr(C)]
struct VkPhysicalDeviceExternalBufferInfo {
    s_type: u32,
    p_next: *const c_void,
    flags: u32,
    usage: u32,
    handle_type: u32,
}

#[repr(C)]
struct VkExternalBufferProperties {
    s_type: u32,
    p_next: *mut c_void,
    properties: VkExternalMemoryProperties,
}

#[repr(C)]
struct VkExternalMemoryBufferCreateInfo {
    s_type: u32,
    p_next: *const c_void,
    handle_types: u32,
}

#[repr(C)]
struct VkImportMemoryFdInfo {
    s_type: u32,
    p_next: *const c_void,
    handle_type: u32,
    fd: c_int,
}

#[repr(C)]
struct VkBufferMemoryRequirementsInfo2 {
    s_type: u32,
    p_next: *const c_void,
    buffer: VkBuffer,
}

#[repr(C)]
struct VkMemoryRequirements2 {
    s_type: u32,
    p_next: *mut c_void,
    requirements: VkMemoryRequirements,
}

const VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO: u32 = 14;
const VK_STRUCTURE_TYPE_EXTERNAL_MEMORY_IMAGE_CREATE_INFO: u32 = 1_000_072_001;
const VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER: u32 = 45;
const VK_STRUCTURE_TYPE_MEMORY_BARRIER: u32 = 46;
const VK_STRUCTURE_TYPE_IMAGE_MEMORY_REQUIREMENTS_INFO_2: u32 = 1_000_146_001;
const VK_STRUCTURE_TYPE_IMAGE_DRM_FORMAT_MODIFIER_EXPLICIT_CREATE_INFO_EXT: u32 = 1_000_158_004;
const VK_FORMAT_R8G8B8A8_UNORM: u32 = 37;
const VK_FORMAT_G8_B8_R8_3PLANE_420_UNORM: u32 = 1_000_156_002;
const VK_IMAGE_TYPE_2D: u32 = 1;
const VK_IMAGE_TILING_DRM_FORMAT_MODIFIER_EXT: u32 = 1_000_158_000;
const VK_IMAGE_USAGE_TRANSFER_SRC_BIT: u32 = 1;
const VK_IMAGE_USAGE_TRANSFER_DST_BIT: u32 = 2;
const VK_IMAGE_LAYOUT_UNDEFINED: u32 = 0;
const VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL: u32 = 6;
const VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL: u32 = 7;
const VK_IMAGE_ASPECT_COLOR_BIT: u32 = 1;
const VK_IMAGE_ASPECT_PLANE_0_BIT: u32 = 0x10;
const VK_IMAGE_ASPECT_PLANE_1_BIT: u32 = 0x20;
const VK_IMAGE_ASPECT_PLANE_2_BIT: u32 = 0x40;
const VK_QUEUE_FAMILY_IGNORED: u32 = u32::MAX;
const VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT: u32 = 1;
const VK_PIPELINE_STAGE_TRANSFER_BIT: u32 = 1 << 12;
const VK_PIPELINE_STAGE_HOST_BIT: u32 = 1 << 14;
const VK_ACCESS_TRANSFER_READ_BIT: u32 = 1 << 11;
const VK_ACCESS_TRANSFER_WRITE_BIT: u32 = 1 << 12;
const VK_ACCESS_HOST_READ_BIT: u32 = 1 << 13;
const DRM_FORMAT_MOD_LINEAR: u64 = 0;

type VkImage = *mut c_void;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct VkExtent3D {
    width: u32,
    height: u32,
    depth: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct VkOffset3D {
    x: i32,
    y: i32,
    z: i32,
}

#[repr(C)]
struct VkSubresourceLayout {
    offset: u64,
    size: u64,
    row_pitch: u64,
    array_pitch: u64,
    depth_pitch: u64,
}

#[repr(C)]
struct VkExternalMemoryImageCreateInfo {
    s_type: u32,
    p_next: *const c_void,
    handle_types: u32,
}

#[repr(C)]
struct VkImageDrmFormatModifierExplicitCreateInfo {
    s_type: u32,
    p_next: *const c_void,
    drm_format_modifier: u64,
    drm_format_modifier_plane_count: u32,
    p_plane_layouts: *const VkSubresourceLayout,
}

#[repr(C)]
struct VkImageCreateInfo {
    s_type: u32,
    p_next: *const c_void,
    flags: u32,
    image_type: u32,
    format: u32,
    extent: VkExtent3D,
    mip_levels: u32,
    array_layers: u32,
    samples: u32,
    tiling: u32,
    usage: u32,
    sharing_mode: u32,
    queue_family_index_count: u32,
    p_queue_family_indices: *const u32,
    initial_layout: u32,
}

#[repr(C)]
struct VkImageMemoryRequirementsInfo2 {
    s_type: u32,
    p_next: *const c_void,
    image: VkImage,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct VkImageSubresourceRange {
    aspect_mask: u32,
    base_mip_level: u32,
    level_count: u32,
    base_array_layer: u32,
    layer_count: u32,
}

#[repr(C)]
struct VkImageMemoryBarrier {
    s_type: u32,
    p_next: *const c_void,
    src_access_mask: u32,
    dst_access_mask: u32,
    old_layout: u32,
    new_layout: u32,
    src_queue_family_index: u32,
    dst_queue_family_index: u32,
    image: VkImage,
    subresource_range: VkImageSubresourceRange,
}

#[repr(C)]
struct VkMemoryBarrier {
    s_type: u32,
    p_next: *const c_void,
    src_access_mask: u32,
    dst_access_mask: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct VkFormatProperties {
    linear_tiling_features: u32,
    optimal_tiling_features: u32,
    buffer_features: u32,
}

#[repr(C)]
union VkClearColorValue {
    float32: [f32; 4],
    uint32: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct VkImageSubresourceLayers {
    aspect_mask: u32,
    mip_level: u32,
    base_array_layer: u32,
    layer_count: u32,
}

#[repr(C)]
struct VkBufferImageCopy {
    buffer_offset: u64,
    buffer_row_length: u32,
    buffer_image_height: u32,
    image_subresource: VkImageSubresourceLayers,
    image_offset: VkOffset3D,
    image_extent: VkExtent3D,
}

type FnCreateImage = unsafe extern "C" fn(
    VkDevice,
    *const VkImageCreateInfo,
    *const c_void,
    *mut VkImage,
) -> VkResult;
type FnDestroyImage = unsafe extern "C" fn(VkDevice, VkImage, *const c_void);
type FnGetImageRequirements2 = unsafe extern "C" fn(
    VkDevice,
    *const VkImageMemoryRequirementsInfo2,
    *mut VkMemoryRequirements2,
);
type FnBindImageMemory =
    unsafe extern "C" fn(VkDevice, VkImage, VkDeviceMemory, VkDeviceSize) -> VkResult;
type FnCmdPipelineBarrier = unsafe extern "C" fn(
    VkCommandBuffer,
    u32,
    u32,
    u32,
    u32,
    *const c_void,
    u32,
    *const c_void,
    u32,
    *const VkImageMemoryBarrier,
);
type FnCmdClearColorImage = unsafe extern "C" fn(
    VkCommandBuffer,
    VkImage,
    u32,
    *const VkClearColorValue,
    u32,
    *const VkImageSubresourceRange,
);
type FnCmdCopyImageToBuffer =
    unsafe extern "C" fn(VkCommandBuffer, VkImage, u32, VkBuffer, u32, *const VkBufferImageCopy);
type FnResetCommandPool = unsafe extern "C" fn(VkDevice, VkCommandPool, u32) -> VkResult;
type FnResetFences = unsafe extern "C" fn(VkDevice, u32, *const VkFence) -> VkResult;

type FnGetExternalBufferProperties = unsafe extern "C" fn(
    VkPhysicalDevice,
    *const VkPhysicalDeviceExternalBufferInfo,
    *mut VkExternalBufferProperties,
);
type FnGetBufferRequirements2 = unsafe extern "C" fn(
    VkDevice,
    *const VkBufferMemoryRequirementsInfo2,
    *mut VkMemoryRequirements2,
);

fn cmsg_align(n: usize) -> usize {
    let a = core::mem::size_of::<usize>();
    (n + a - 1) & !(a - 1)
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for &byte in bytes {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

unsafe fn hash_plane_rows(data: *const u8, row_stride: usize, width: usize, height: usize) -> u64 {
    if data.is_null() || width == 0 || height == 0 || row_stride < width {
        return 0;
    }
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for row in 0..height {
        let bytes = core::slice::from_raw_parts(data.add(row * row_stride), width);
        for &byte in bytes {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    hash
}

unsafe fn recv_handle_fds(stream: &UnixStream) -> Result<Vec<c_int>, String> {
    let mut data = [0u8; 512];
    let mut control = [0u8; 512];
    let mut iov = IoVec {
        base: data.as_mut_ptr() as *mut c_void,
        len: data.len(),
    };
    let mut msg = MsgHdr {
        name: ptr::null_mut(),
        name_len: 0,
        iov: &mut iov,
        iov_len: 1,
        control: control.as_mut_ptr() as *mut c_void,
        control_len: control.len(),
        flags: 0,
    };
    let n = recvmsg(stream.as_raw_fd(), &mut msg, 0);
    if n < 0 {
        return Err(format!("recvmsg: {}", std::io::Error::last_os_error()));
    }
    line(
        "dmabuf",
        "recv",
        format!("bytes={n} control_len={}", msg.control_len),
    );

    let mut fds = Vec::new();
    let mut off = 0usize;
    let header = core::mem::size_of::<CmsgHdr>();
    while off + header <= msg.control_len {
        let h = control.as_ptr().add(off) as *const CmsgHdr;
        let len = (*h).len;
        if len < header || off + len > msg.control_len {
            break;
        }
        if (*h).level == SOL_SOCKET && (*h).ty == SCM_RIGHTS {
            let data_off = off + cmsg_align(header);
            let data_len = len.saturating_sub(cmsg_align(header));
            let count = data_len / core::mem::size_of::<c_int>();
            for i in 0..count {
                let p = control
                    .as_ptr()
                    .add(data_off + i * core::mem::size_of::<c_int>())
                    as *const c_int;
                fds.push(p.read_unaligned());
            }
        }
        off += cmsg_align(len);
    }
    if fds.is_empty() {
        return Err("SCM_RIGHTS message contained no fds".to_string());
    }
    line("dmabuf", "fd_count", format!("{}", fds.len()));
    Ok(fds)
}

unsafe fn connect_video(path: &str) -> Result<UnixStream, String> {
    let deadline = Instant::now() + Duration::from_secs(90);
    let mut retries = 0u32;
    loop {
        match UnixStream::connect(path) {
            Ok(stream) => {
                line("dmabuf", "socket", format!("PASS|{path}"));
                return Ok(stream);
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                ) && Instant::now() < deadline =>
            {
                retries += 1;
                if retries == 1 || retries % 20 == 0 {
                    line(
                        "dmabuf",
                        "socket_wait",
                        format!("retry={retries}|{path}|{error}"),
                    );
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(error) => return Err(format!("connect {path}: {error}")),
        }
    }
}

unsafe fn receive_handle_from_stream(
    stream: &mut UnixStream,
    verbose: bool,
) -> Result<(AhbMeta, Vec<Vec<c_int>>), String> {
    let mut frame_len = [0u8; 4];
    stream
        .read_exact(&mut frame_len)
        .map_err(|e| format!("read meta length: {e}"))?;
    let meta_len = u32::from_be_bytes(frame_len) as usize;
    if meta_len == 0 || meta_len > 4096 {
        return Err(format!("bad meta length {meta_len}"));
    }
    let mut meta_buf = vec![0u8; meta_len];
    stream
        .read_exact(&mut meta_buf)
        .map_err(|e| format!("read meta: {e}"))?;
    let meta_text = core::str::from_utf8(&meta_buf).map_err(|e| format!("meta utf8: {e}"))?;
    let meta = parse_meta(meta_text).map_err(|e| format!("meta: {e}"))?;
    // v2 and v3 both carry a one-byte NEW/REUSE flag before any SCM_RIGHTS
    // payload. Keep this in lockstep with the app sender: v3 adds the output
    // AHB metadata and a second handle group, but does not change framing.
    let has_handle_flag =
        meta_text.starts_with("HUSK-VIDEO-AHB-2") || meta_text.starts_with("HUSK-VIDEO-AHB-3");
    if verbose {
        line(
            "dmabuf",
            "meta",
            format!(
                "PASS|{}x{} stride={} format={} usage={:#x} count={} frames={}",
                meta.width,
                meta.height,
                meta.stride,
                meta.format,
                meta.usage,
                meta.count,
                meta.frames
            ),
        );
    }

    let mut handle_flag = b'N';
    if has_handle_flag {
        let mut flag = [0u8; 1];
        stream
            .read_exact(&mut flag)
            .map_err(|e| format!("read handle flag: {e}"))?;
        handle_flag = flag[0];
        if handle_flag != b'N' && handle_flag != b'R' {
            return Err(format!("bad handle flag 0x{:02x}", handle_flag));
        }
    }
    let mut handles = Vec::new();
    if handle_flag == b'N' {
        handles.reserve(meta.handle_count);
        for slot in 0..meta.handle_count {
            let fds = recv_handle_fds(stream)
                .map_err(|e| format!("receive handle {slot}/{}: {e}", meta.handle_count))?;
            handles.push(fds);
        }
    }
    if verbose {
        line(
            "dmabuf",
            "handoff",
            format!(
                "{}|key={:016x}|handles={}",
                if handle_flag == b'N' { "NEW" } else { "REUSE" },
                meta.buffer_key,
                handles.len()
            ),
        );
    }
    Ok((meta, handles))
}

unsafe fn receive_handle(path: &str) -> Result<(AhbMeta, Vec<Vec<c_int>>, UnixStream), String> {
    let mut stream = connect_video(path)?;
    let (meta, handles) = receive_handle_from_stream(&mut stream, true)?;
    Ok((meta, handles, stream))
}

unsafe fn find_dmabuf(fds: &[c_int]) -> Option<c_int> {
    for &fd in fds {
        let target = std::fs::read_link(format!("/proc/self/fd/{fd}"))
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|e| format!("<readlink: {e}>"));
        let is_dmabuf = target.contains("dmabuf") || target.contains("dma-buf");
        line(
            "dmabuf",
            if is_dmabuf { "found" } else { "fd" },
            format!("fd={fd} target={target}"),
        );
        if is_dmabuf {
            return Some(fd);
        }
    }
    None
}

fn dmabuf_key(fd: c_int) -> Option<u64> {
    let meta = std::fs::metadata(format!("/proc/self/fd/{fd}")).ok()?;
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&meta.dev().to_le_bytes());
    bytes[8..].copy_from_slice(&meta.ino().to_le_bytes());
    let mut key = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        key ^= byte as u64;
        key = key.wrapping_mul(0x0000_0100_0000_01b3);
    }
    Some(key)
}

fn ring_log_every() -> usize {
    std::env::args()
        .find_map(|arg| arg.strip_prefix("--log-every=")?.parse().ok())
        .unwrap_or(1)
}

fn ring_pace() -> std::time::Duration {
    let ms = std::env::args()
        .find_map(|arg| arg.strip_prefix("--pace-ms=")?.parse().ok())
        .unwrap_or(0u64);
    std::time::Duration::from_millis(ms)
}

fn frame_color(frame: usize) -> ([f32; 4], u32) {
    match frame % 3 {
        0 => ([1.0, 0.0, 0.0, 1.0], 0xff00_00ff),
        1 => ([0.0, 1.0, 0.0, 1.0], 0xff00_ff00),
        _ => ([0.0, 0.0, 1.0, 1.0], 0xffff_0000),
    }
}

unsafe fn export_fence_sync_fd(
    lib: *mut c_void,
    device: VkDevice,
    fence: VkFence,
) -> Result<c_int, String> {
    let get_proc: FnGetDeviceProcAddr = symbol(lib, "vkGetDeviceProcAddr")
        .ok_or_else(|| "vkGetDeviceProcAddr missing".to_string())?;
    let name = std::ffi::CString::new("vkGetFenceFdKHR").unwrap();
    let raw = get_proc(device, name.as_ptr());
    if raw.is_null() {
        return Err("vkGetDeviceProcAddr(vkGetFenceFdKHR) returned null".to_string());
    }
    let get_fence_fd: FnGetFenceFdKhr = std::mem::transmute(raw);
    let info = VkFenceGetFdInfoKhr {
        s_type: VK_STRUCTURE_TYPE_FENCE_GET_FD_INFO_KHR,
        p_next: ptr::null(),
        fence,
        handle_type: VK_EXTERNAL_FENCE_HANDLE_TYPE_SYNC_FD_BIT,
    };
    let mut fd = -1;
    let rc = get_fence_fd(device, &info, &mut fd);
    if rc != VK_SUCCESS {
        return Err(format!("vkGetFenceFdKHR rc={rc}"));
    }
    Ok(fd)
}

unsafe fn send_ready_with_fd(stream: &UnixStream, text: &str, acquire_fd: c_int) -> bool {
    let mut io = IoVec {
        base: text.as_ptr() as *mut c_void,
        len: text.len(),
    };
    let mut control = [0u8; 64];
    let header_len = cmsg_align(core::mem::size_of::<CmsgHdr>());
    let cmsg_len = header_len + core::mem::size_of::<c_int>();
    let control_len = cmsg_align(cmsg_len);
    let header = control.as_mut_ptr() as *mut CmsgHdr;
    (*header).len = cmsg_len;
    (*header).level = SOL_SOCKET;
    (*header).ty = SCM_RIGHTS;
    (control.as_mut_ptr().add(header_len) as *mut c_int).write_unaligned(acquire_fd);
    let msg = MsgHdr {
        name: ptr::null_mut(),
        name_len: 0,
        iov: &mut io,
        iov_len: 1,
        control: control.as_mut_ptr() as *mut c_void,
        control_len,
        flags: 0,
    };
    let sent = sendmsg(stream.as_raw_fd(), &msg, 0);
    if sent == text.len() as isize {
        line(
            "ring",
            "sendmsg",
            format!("len={} sent={sent} acquire_fd={acquire_fd}", text.len()),
        );
        true
    } else {
        line(
            "ring",
            "FAIL",
            format!(
                "sendmsg fd={acquire_fd}: sent={sent} errno={}",
                std::io::Error::last_os_error()
            ),
        );
        false
    }
}

unsafe fn send_text(stream: &UnixStream, text: &str) -> bool {
    let mut io = IoVec {
        base: text.as_ptr() as *mut c_void,
        len: text.len(),
    };
    let msg = MsgHdr {
        name: ptr::null_mut(),
        name_len: 0,
        iov: &mut io,
        iov_len: 1,
        control: ptr::null_mut(),
        control_len: 0,
        flags: 0,
    };
    let sent = sendmsg(stream.as_raw_fd(), &msg, 0);
    if sent == text.len() as isize {
        true
    } else {
        line(
            "ring",
            "FAIL",
            format!(
                "sendmsg text: sent={sent} len={} errno={}",
                text.len(),
                std::io::Error::last_os_error()
            ),
        );
        false
    }
}

unsafe fn run_ring(
    lib: *mut c_void,
    physical: VkPhysicalDevice,
    device: VkDevice,
    queue: VkQueue,
    family_index: u32,
    meta: &AhbMeta,
    handles: &[Vec<c_int>],
    stream: UnixStream,
) -> bool {
    let started = std::time::Instant::now();
    let log_every = ring_log_every().max(1);
    let pace = ring_pace();
    line(
        "ring",
        "BEGIN",
        format!(
            "buffers={} frames={} {}x{} log_every={log_every} pace_ms={}",
            meta.count,
            meta.frames,
            meta.width,
            meta.height,
            pace.as_millis()
        ),
    );
    if handles.len() != meta.count {
        line(
            "ring",
            "FAIL",
            format!("handle count {} != {}", handles.len(), meta.count),
        );
        return false;
    }
    let Some(api) = load_ring_fns(lib, device) else {
        line("ring", "FAIL", "load pooled Vulkan symbols");
        for fds in handles {
            for &fd in fds {
                close(fd);
            }
        }
        return false;
    };

    let mut dmabufs = Vec::with_capacity(handles.len());
    let mut linear_ok = true;
    for (slot, fds) in handles.iter().enumerate() {
        let Some(fd) = find_dmabuf(fds) else {
            line("ring", "FAIL", format!("slot {slot} has no dmabuf"));
            linear_ok = false;
            break;
        };
        let ok = probe_linear_dmabuf(fd, meta);
        line("ring", "linear", format!("slot={slot} fd={fd} ok={ok}"));
        linear_ok &= ok;
        dmabufs.push(fd);
    }
    if !linear_ok || dmabufs.len() != meta.count {
        for fds in handles {
            for &fd in fds {
                close(fd);
            }
        }
        return false;
    }

    let mut slots = Vec::with_capacity(meta.count);
    for (slot_index, &fd) in dmabufs.iter().enumerate() {
        match create_pooled_slot(&api, physical, device, family_index, fd, meta, slot_index) {
            Some(slot) => slots.push(slot),
            None => {
                line("pool", "FAIL", format!("create slot={slot_index}"));
                for slot in &slots {
                    destroy_pooled_slot(&api, device, slot);
                }
                for fds in handles {
                    for &fd in fds {
                        close(fd);
                    }
                }
                return false;
            }
        }
    }
    line(
        "pool",
        "SUMMARY",
        format!(
            "slots={} image_imports={} readback_allocations={}",
            slots.len(),
            slots.len(),
            slots.len()
        ),
    );

    let reader_stream = match stream.try_clone() {
        Ok(v) => v,
        Err(e) => {
            line("ring", "FAIL", format!("clone reader: {e}"));
            for slot in &slots {
                destroy_pooled_slot(&api, device, slot);
            }
            for fds in handles {
                for &fd in fds {
                    close(fd);
                }
            }
            return false;
        }
    };
    if let Err(e) = reader_stream.set_read_timeout(Some(std::time::Duration::from_secs(15))) {
        line("ring", "WARN", format!("read timeout: {e}"));
    }
    let mut reader = BufReader::new(reader_stream);
    let writer = stream;
    let stats = Arc::new(RingStats::default());
    let mut used = vec![false; meta.count];
    let mut slot_joins: Vec<Option<JoinHandle<()>>> = (0..meta.count).map(|_| None).collect();
    let mut rendered = 0usize;
    let mut releases = 0usize;
    let mut ok = true;

    for frame in 0..meta.frames {
        let slot_index = frame % meta.count;
        let verbose = frame % log_every == 0 || frame + 1 == meta.frames;
        if used[slot_index] {
            if verbose {
                line(
                    "ring",
                    "WAIT_RELEASE",
                    format!("frame={frame} slot={slot_index}"),
                );
            }
            loop {
                let mut response = String::new();
                match reader.read_line(&mut response) {
                    Ok(0) => {
                        line("ring", "FAIL", "app closed before releasing buffer");
                        ok = false;
                        break;
                    }
                    Ok(_) => {
                        let response = response.trim();
                        if response == format!("RELEASE {slot_index}") {
                            releases += 1;
                            if verbose {
                                line(
                                    "ring",
                                    "RELEASE",
                                    format!("frame={frame} slot={slot_index}"),
                                );
                            }
                            break;
                        }
                        line("ring", "RX", response);
                    }
                    Err(e) => {
                        line(
                            "ring",
                            "FAIL",
                            format!("wait release slot={slot_index}: {e}"),
                        );
                        ok = false;
                        break;
                    }
                }
            }
            if let Some(join) = slot_joins[slot_index].take() {
                if join.join().is_err() {
                    line(
                        "ring",
                        "FAIL",
                        format!("slot={slot_index} completion panic"),
                    );
                    ok = false;
                }
            }
        }
        if !ok {
            break;
        }

        let (color, expected) = frame_color(frame);
        if verbose {
            line(
                "ring",
                "RENDER",
                format!("frame={frame} slot={slot_index} expected={expected:#010x}"),
            );
        }
        let Some(mut completion) = render_pooled_frame(
            &api,
            device,
            queue,
            &slots[slot_index],
            color,
            expected,
            frame,
            slot_index,
            verbose,
            &stats,
        ) else {
            line(
                "ring",
                "FAIL",
                format!("frame={frame} slot={slot_index} pooled render"),
            );
            ok = false;
            break;
        };
        let acquire_fd = completion.acquire_fd;
        let ready = acquire_fd >= 0
            && send_ready_with_fd(
                &writer,
                &format!("READY {slot_index} {frame} {expected:08x}\n"),
                acquire_fd,
            );
        if acquire_fd >= 0 {
            close(acquire_fd);
            completion.acquire_fd = -1;
        }
        if let Some(join) = completion.join.take() {
            slot_joins[slot_index] = Some(join);
        }
        if verbose || !ready {
            line(
                "ring",
                "READY",
                format!(
                    "frame={frame} slot={slot_index} rendered=true acquire_fd={acquire_fd} sent={ready}"
                ),
            );
        }
        if !ready {
            ok = false;
            break;
        }
        used[slot_index] = true;
        rendered += 1;
        if !pace.is_zero() && frame + 1 < meta.frames {
            std::thread::sleep(pace);
        }
    }

    for (slot_index, join) in slot_joins.into_iter().enumerate() {
        if let Some(join) = join {
            if join.join().is_err() {
                line(
                    "ring",
                    "FAIL",
                    format!("slot={slot_index} completion panic"),
                );
                ok = false;
            }
        }
    }
    for slot in &slots {
        destroy_pooled_slot(&api, device, slot);
    }
    line("pool", "teardown", "all pooled Vulkan slots destroyed");

    let done = send_text(&writer, &format!("DONE {rendered}\n"));
    let elapsed_s = started.elapsed().as_secs_f64();
    let fps = if elapsed_s > 0.0 {
        rendered as f64 / elapsed_s
    } else {
        0.0
    };
    let pending = stats.export_pending.load(Ordering::Relaxed);
    let already_signaled = stats.export_signaled.load(Ordering::Relaxed);
    let completion_ok = stats.completion_ok.load(Ordering::Relaxed);
    let completion_fail = stats.completion_fail.load(Ordering::Relaxed);
    let readback_ok = stats.readback_ok.load(Ordering::Relaxed);
    let readback_fail = stats.readback_fail.load(Ordering::Relaxed);
    let fallback = stats.fallback.load(Ordering::Relaxed);
    let stats_ok = completion_ok == rendered
        && readback_ok == rendered
        && completion_fail == 0
        && readback_fail == 0
        && fallback == 0;
    let stats_line = format!(
        "pending={pending} already_signaled={already_signaled} completion_ok={completion_ok} completion_fail={completion_fail} readback_ok={readback_ok} readback_fail={readback_fail} fallback={fallback} stats_ok={stats_ok}"
    );
    line("ring", "STATS", stats_line);
    line(
        "ring",
        if ok && rendered == meta.frames && done && stats_ok {
            "PASS"
        } else {
            "FAIL"
        },
        format!(
            "rendered={rendered}/{} releases={releases} done={done} elapsed_s={elapsed_s:.3} fps={fps:.3} stats_ok={stats_ok}",
            meta.frames
        ),
    );
    std::thread::sleep(std::time::Duration::from_secs(1));
    for fds in handles {
        for &fd in fds {
            close(fd);
        }
    }
    line("ring", "teardown", "all received AHB fds closed");
    ok && rendered == meta.frames && done
}

pub unsafe fn run(
    lib: *mut c_void,
    physical: VkPhysicalDevice,
    device: VkDevice,
    queue: VkQueue,
    family_index: u32,
    path: &str,
) -> bool {
    let (meta, handles, mut stream) = match receive_handle(path) {
        Ok(v) => v,
        Err(e) => {
            line("dmabuf", "FAIL", format!("{e}"));
            return false;
        }
    };
    if meta.count > 1 || meta.frames > 1 {
        return run_ring(
            lib,
            physical,
            device,
            queue,
            family_index,
            &meta,
            &handles,
            stream,
        );
    }
    let fds = match handles.into_iter().next() {
        Some(v) => v,
        None => {
            line("dmabuf", "FAIL", "no AHB handle");
            return false;
        }
    };
    let Some(fd) = find_dmabuf(&fds) else {
        line("dmabuf", "FAIL", "no dmabuf fd in received handle");
        for &fd in &fds {
            close(fd);
        }
        return false;
    };
    let linear_ok = probe_linear_dmabuf(fd, &meta);

    let get_external_props: FnGetExternalBufferProperties =
        match symbol(lib, "vkGetPhysicalDeviceExternalBufferProperties") {
            Some(f) => f,
            None => {
                line(
                    "dmabuf",
                    "FAIL",
                    "vkGetPhysicalDeviceExternalBufferProperties missing",
                );
                for &fd in &fds {
                    close(fd);
                }
                return false;
            }
        };
    let usage = VK_BUFFER_USAGE_TRANSFER_SRC_BIT
        | VK_BUFFER_USAGE_TRANSFER_DST_BIT
        | VK_BUFFER_USAGE_STORAGE_BUFFER_BIT;
    let info = VkPhysicalDeviceExternalBufferInfo {
        s_type: VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_EXTERNAL_BUFFER_INFO,
        p_next: ptr::null(),
        flags: 0,
        usage,
        handle_type: VK_EXTERNAL_MEMORY_HANDLE_TYPE_DMA_BUF_BIT_EXT,
    };
    let mut props = VkExternalBufferProperties {
        s_type: VK_STRUCTURE_TYPE_EXTERNAL_BUFFER_PROPERTIES,
        p_next: ptr::null_mut(),
        properties: VkExternalMemoryProperties {
            features: 0,
            export_from_imported: 0,
            compatible: 0,
        },
    };
    get_external_props(physical, &info, &mut props);
    line(
        "dmabuf",
        "external_props",
        format!(
            "features={:#x} export={:#x} compatible={:#x}",
            props.properties.features,
            props.properties.export_from_imported,
            props.properties.compatible
        ),
    );
    if props.properties.compatible & VK_EXTERNAL_MEMORY_HANDLE_TYPE_DMA_BUF_BIT_EXT == 0 {
        line(
            "dmabuf",
            "FAIL",
            "DMA_BUF import not advertised for buffer usage",
        );
        for &fd in &fds {
            close(fd);
        }
        return false;
    }

    let Some(create_buffer): Option<FnCreateBuffer> = symbol(lib, "vkCreateBuffer") else {
        line("dmabuf", "FAIL", "vkCreateBuffer missing");
        for &fd in &fds {
            close(fd);
        }
        return false;
    };
    let Some(destroy_buffer): Option<FnDestroyBuffer> = symbol(lib, "vkDestroyBuffer") else {
        line("dmabuf", "FAIL", "vkDestroyBuffer missing");
        for &fd in &fds {
            close(fd);
        }
        return false;
    };
    let Some(allocate_memory): Option<FnAllocateMemory> = symbol(lib, "vkAllocateMemory") else {
        line("dmabuf", "FAIL", "vkAllocateMemory missing");
        for &fd in &fds {
            close(fd);
        }
        return false;
    };
    let Some(free_memory): Option<FnFreeMemory> = symbol(lib, "vkFreeMemory") else {
        line("dmabuf", "FAIL", "vkFreeMemory missing");
        for &fd in &fds {
            close(fd);
        }
        return false;
    };
    let Some(bind_buffer): Option<FnBindBufferMemory> = symbol(lib, "vkBindBufferMemory") else {
        line("dmabuf", "FAIL", "vkBindBufferMemory missing");
        for &fd in &fds {
            close(fd);
        }
        return false;
    };
    let Some(get_requirements2): Option<FnGetBufferRequirements2> =
        symbol(lib, "vkGetBufferMemoryRequirements2")
    else {
        line("dmabuf", "FAIL", "vkGetBufferMemoryRequirements2 missing");
        for &fd in &fds {
            close(fd);
        }
        return false;
    };
    let Some(get_memory_props): Option<FnGetPhysicalDeviceMemoryProperties> =
        symbol(lib, "vkGetPhysicalDeviceMemoryProperties")
    else {
        line(
            "dmabuf",
            "FAIL",
            "vkGetPhysicalDeviceMemoryProperties missing",
        );
        for &fd in &fds {
            close(fd);
        }
        return false;
    };

    let external = VkExternalMemoryBufferCreateInfo {
        s_type: VK_STRUCTURE_TYPE_EXTERNAL_MEMORY_BUFFER_CREATE_INFO,
        p_next: ptr::null(),
        handle_types: VK_EXTERNAL_MEMORY_HANDLE_TYPE_DMA_BUF_BIT_EXT,
    };
    let buffer_info = VkBufferCreateInfo {
        s_type: VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO,
        p_next: &external as *const _ as *const c_void,
        flags: 0,
        size: 32768,
        usage,
        sharing_mode: VK_SHARING_MODE_EXCLUSIVE,
        queue_family_index_count: 0,
        p_queue_family_indices: ptr::null(),
    };
    let mut buffer: VkBuffer = ptr::null_mut();
    let r = create_buffer(device, &buffer_info, ptr::null(), &mut buffer);
    line(
        "dmabuf",
        "buffer",
        format!("create {} ptr={buffer:p}", api_result(r)),
    );
    if r != VK_SUCCESS {
        for &fd in &fds {
            close(fd);
        }
        return false;
    }

    let req_info = VkBufferMemoryRequirementsInfo2 {
        s_type: VK_STRUCTURE_TYPE_BUFFER_MEMORY_REQUIREMENTS_INFO_2,
        p_next: ptr::null(),
        buffer,
    };
    let mut req2 = VkMemoryRequirements2 {
        s_type: VK_STRUCTURE_TYPE_MEMORY_REQUIREMENTS_2,
        p_next: ptr::null_mut(),
        requirements: VkMemoryRequirements::default(),
    };
    get_requirements2(device, &req_info, &mut req2);
    let req = req2.requirements;
    line(
        "dmabuf",
        "requirements",
        format!(
            "size={} align={} bits={:#x}",
            req.size, req.alignment, req.memory_type_bits
        ),
    );

    let mut mem_props: VkPhysicalDeviceMemoryProperties = std::mem::zeroed();
    get_memory_props(physical, &mut mem_props);
    let Some(memory_type_index) = choose_memory_type(&mem_props, req.memory_type_bits) else {
        line("dmabuf", "FAIL", "no memory type for imported buffer");
        destroy_buffer(device, buffer, ptr::null());
        for &fd in &fds {
            close(fd);
        }
        return false;
    };

    let import_fd = dup(fd);
    if import_fd < 0 {
        line(
            "dmabuf",
            "import_fd",
            format!("FAIL|dup: {}", std::io::Error::last_os_error()),
        );
    } else {
        line(
            "dmabuf",
            "import_fd",
            format!("PASS|fd={import_fd} source_size={}", lseek(fd, 0, 2)),
        );
    }
    if import_fd < 0 {
        destroy_buffer(device, buffer, ptr::null());
        for &fd in &fds {
            close(fd);
        }
        return false;
    }
    let mut import = VkImportMemoryFdInfo {
        s_type: VK_STRUCTURE_TYPE_IMPORT_MEMORY_FD_INFO_KHR,
        p_next: ptr::null(),
        handle_type: VK_EXTERNAL_MEMORY_HANDLE_TYPE_DMA_BUF_BIT_EXT,
        fd: import_fd,
    };
    let alloc = VkMemoryAllocateInfo {
        s_type: VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO,
        p_next: &mut import as *mut _ as *const c_void,
        allocation_size: req.size,
        memory_type_index,
    };
    let mut memory: VkDeviceMemory = ptr::null_mut();
    let r = allocate_memory(device, &alloc, ptr::null(), &mut memory);
    line(
        "dmabuf",
        "import",
        format!(
            "vkAllocateMemory {} type={} size={} fd={}",
            api_result(r),
            memory_type_index,
            req.size,
            fd
        ),
    );
    if r != VK_SUCCESS {
        close(import_fd);
        destroy_buffer(device, buffer, ptr::null());
        for &fd in &fds {
            close(fd);
        }
        return false;
    }
    close(import_fd);

    let r = bind_buffer(device, buffer, memory, 0);
    line("dmabuf", "bind", api_result(r));
    let gpu_ok = r == VK_SUCCESS
        && submit_compute(
            lib,
            device,
            queue,
            family_index,
            buffer,
            memory,
            FenceMode::Wait,
        );
    line(
        "dmabuf",
        "gpu_access",
        if gpu_ok {
            "PASS|compute wrote and read 0x4855534b"
        } else {
            "FAIL|GPU access to imported dma-buf did not complete"
        },
    );
    free_memory(device, memory, ptr::null());
    destroy_buffer(device, buffer, ptr::null());
    let image_ok = gpu_ok
        && render_linear_image(
            lib,
            physical,
            device,
            queue,
            family_index,
            fd,
            &meta,
            [1.0, 0.0, 0.0, 1.0],
            0xff00_00ff,
            0,
            0,
            None,
        );
    line(
        "dmabuf",
        "image_summary",
        if image_ok {
            "PASS|LINEAR 2D image imported, cleared by GPU, read back 9/9 red"
        } else {
            "FAIL|LINEAR 2D image path incomplete"
        },
    );
    let handoff_ok = image_ok && stream.write_all(b"D").is_ok();
    line(
        "dmabuf",
        "display_handoff",
        if handoff_ok {
            "PASS|guest GPU image ready; app may present"
        } else {
            "FAIL|guest did not signal app"
        },
    );
    for &fd in &fds {
        close(fd);
    }
    line("dmabuf", "teardown", "memory/buffer/fds released");
    r == VK_SUCCESS && gpu_ok && linear_ok && image_ok && handoff_ok
}

// The MTK producer pool is four buffers; the app still limits handoff to
// three in-flight AImages for backpressure. Cache all four imports so a
// repeated producer buffer never forces a Vulkan image/memory recreation.
const VIDEO_SLOT_LIMIT: usize = 16;

struct VideoSlot {
    key: u64,
    image: VkImage,
    image_memory: VkDeviceMemory,
    dmabuf_fd: c_int,
    converter: Option<crate::yuv::Converter>,
    destroy_image: FnDestroyImage,
    free_memory: FnFreeMemory,
}

struct VideoRing {
    slots: Vec<VideoSlot>,
    imports: u64,
    reuses: u64,
}

impl VideoRing {
    fn new() -> Self {
        Self {
            slots: Vec::with_capacity(VIDEO_SLOT_LIMIT),
            imports: 0,
            reuses: 0,
        }
    }

    fn find(&self, key: u64) -> Option<usize> {
        self.slots.iter().position(|slot| slot.key == key)
    }
}

unsafe fn create_video_slot(
    lib: *mut c_void,
    physical: VkPhysicalDevice,
    device: VkDevice,
    queue: VkQueue,
    family_index: u32,
    meta: &AhbMeta,
    fd: c_int,
    output_fd: Option<c_int>,
    key: u64,
    slot_index: usize,
) -> Result<VideoSlot, String> {
    let create_image: FnCreateImage =
        symbol(lib, "vkCreateImage").ok_or_else(|| "vkCreateImage missing".to_string())?;
    let destroy_image: FnDestroyImage =
        symbol(lib, "vkDestroyImage").ok_or_else(|| "vkDestroyImage missing".to_string())?;
    let get_image_requirements: FnGetImageRequirements2 =
        symbol(lib, "vkGetImageMemoryRequirements2")
            .ok_or_else(|| "vkGetImageMemoryRequirements2 missing".to_string())?;
    let allocate_memory: FnAllocateMemory =
        symbol(lib, "vkAllocateMemory").ok_or_else(|| "vkAllocateMemory missing".to_string())?;
    let free_memory: FnFreeMemory =
        symbol(lib, "vkFreeMemory").ok_or_else(|| "vkFreeMemory missing".to_string())?;
    let bind_image: FnBindImageMemory =
        symbol(lib, "vkBindImageMemory").ok_or_else(|| "vkBindImageMemory missing".to_string())?;
    let get_memory_props: FnGetPhysicalDeviceMemoryProperties =
        symbol(lib, "vkGetPhysicalDeviceMemoryProperties")
            .ok_or_else(|| "vkGetPhysicalDeviceMemoryProperties missing".to_string())?;

    let chroma_height = meta.height.div_ceil(2);
    let plane_layouts = [
        VkSubresourceLayout {
            offset: meta.planes[0].offset,
            size: meta.planes[0].row_stride as u64 * meta.height as u64,
            row_pitch: meta.planes[0].row_stride as u64,
            array_pitch: 0,
            depth_pitch: 0,
        },
        VkSubresourceLayout {
            offset: meta.planes[1].offset,
            size: meta.planes[1].row_stride as u64 * chroma_height as u64,
            row_pitch: meta.planes[1].row_stride as u64,
            array_pitch: 0,
            depth_pitch: 0,
        },
        VkSubresourceLayout {
            offset: meta.planes[2].offset,
            size: meta.planes[2].row_stride as u64 * chroma_height as u64,
            row_pitch: meta.planes[2].row_stride as u64,
            array_pitch: 0,
            depth_pitch: 0,
        },
    ];
    let external = VkExternalMemoryImageCreateInfo {
        s_type: VK_STRUCTURE_TYPE_EXTERNAL_MEMORY_IMAGE_CREATE_INFO,
        p_next: ptr::null(),
        handle_types: VK_EXTERNAL_MEMORY_HANDLE_TYPE_DMA_BUF_BIT_EXT,
    };
    let explicit = VkImageDrmFormatModifierExplicitCreateInfo {
        s_type: VK_STRUCTURE_TYPE_IMAGE_DRM_FORMAT_MODIFIER_EXPLICIT_CREATE_INFO_EXT,
        p_next: &external as *const _ as *const c_void,
        drm_format_modifier: DRM_FORMAT_MOD_LINEAR,
        drm_format_modifier_plane_count: plane_layouts.len() as u32,
        p_plane_layouts: plane_layouts.as_ptr(),
    };
    let image_info = VkImageCreateInfo {
        s_type: VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO,
        p_next: &explicit as *const _ as *const c_void,
        flags: 0,
        image_type: VK_IMAGE_TYPE_2D,
        format: VK_FORMAT_G8_B8_R8_3PLANE_420_UNORM,
        extent: VkExtent3D {
            width: meta.width,
            height: meta.height,
            depth: 1,
        },
        mip_levels: 1,
        array_layers: 1,
        samples: 1,
        tiling: VK_IMAGE_TILING_DRM_FORMAT_MODIFIER_EXT,
        usage: VK_IMAGE_USAGE_TRANSFER_SRC_BIT | VK_IMAGE_USAGE_TRANSFER_DST_BIT,
        sharing_mode: VK_SHARING_MODE_EXCLUSIVE,
        queue_family_index_count: 0,
        p_queue_family_indices: ptr::null(),
        initial_layout: VK_IMAGE_LAYOUT_UNDEFINED,
    };
    let mut image: VkImage = ptr::null_mut();
    let create_result = create_image(device, &image_info, ptr::null(), &mut image);
    line(
        "video-ahb",
        "ring_import_image",
        format!(
            "slot={slot_index}|key={key:016x}|{}|{}x{}",
            api_result(create_result),
            meta.width,
            meta.height
        ),
    );
    if create_result != VK_SUCCESS {
        return Err(format!("vkCreateImage {create_result}"));
    }

    let requirements_info = VkImageMemoryRequirementsInfo2 {
        s_type: VK_STRUCTURE_TYPE_IMAGE_MEMORY_REQUIREMENTS_INFO_2,
        p_next: ptr::null(),
        image,
    };
    let mut requirements = VkMemoryRequirements2 {
        s_type: VK_STRUCTURE_TYPE_MEMORY_REQUIREMENTS_2,
        p_next: ptr::null_mut(),
        requirements: VkMemoryRequirements::default(),
    };
    get_image_requirements(device, &requirements_info, &mut requirements);
    let required = requirements.requirements;
    let mut memory_properties: VkPhysicalDeviceMemoryProperties = std::mem::zeroed();
    get_memory_props(physical, &mut memory_properties);
    let Some(memory_type_index) = choose_memory_type(&memory_properties, required.memory_type_bits)
    else {
        destroy_image(device, image, ptr::null());
        return Err("no importable memory type".into());
    };

    let held_fd = dup(fd);
    if held_fd < 0 {
        destroy_image(device, image, ptr::null());
        return Err("dup held dma-buf".into());
    }
    let mut import = VkImportMemoryFdInfo {
        s_type: VK_STRUCTURE_TYPE_IMPORT_MEMORY_FD_INFO_KHR,
        p_next: ptr::null(),
        handle_type: VK_EXTERNAL_MEMORY_HANDLE_TYPE_DMA_BUF_BIT_EXT,
        fd: dup(fd),
    };
    if import.fd < 0 {
        close(held_fd);
        destroy_image(device, image, ptr::null());
        return Err("dup import dma-buf".into());
    }
    let allocation = VkMemoryAllocateInfo {
        s_type: VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO,
        p_next: &mut import as *mut _ as *const c_void,
        allocation_size: required.size,
        memory_type_index,
    };
    let mut image_memory: VkDeviceMemory = ptr::null_mut();
    let import_result = allocate_memory(device, &allocation, ptr::null(), &mut image_memory);
    close(import.fd);
    line(
        "video-ahb",
        "ring_import_memory",
        format!(
            "slot={slot_index}|key={key:016x}|{}|type={memory_type_index}|size={}",
            api_result(import_result),
            required.size
        ),
    );
    if import_result != VK_SUCCESS {
        close(held_fd);
        destroy_image(device, image, ptr::null());
        return Err(format!("vkAllocateMemory {import_result}"));
    }
    let bind_result = bind_image(device, image, image_memory, 0);
    if bind_result != VK_SUCCESS {
        close(held_fd);
        free_memory(device, image_memory, ptr::null());
        destroy_image(device, image, ptr::null());
        return Err(format!("vkBindImageMemory {bind_result}"));
    }
    let converter = match output_fd {
        Some(output_fd) => match crate::yuv::Converter::new(
            lib,
            physical,
            device,
            queue,
            family_index,
            held_fd,
            output_fd,
            meta.width,
            meta.height,
            [
                meta.planes[0].offset,
                meta.planes[1].offset,
                meta.planes[2].offset,
            ],
            [
                meta.planes[0].row_stride,
                meta.planes[1].row_stride,
                meta.planes[2].row_stride,
            ],
            meta.output_offset,
            meta.output_stride,
            slot_index,
        ) {
            Ok(converter) => Some(converter),
            Err(error) => {
                close(held_fd);
                free_memory(device, image_memory, ptr::null());
                destroy_image(device, image, ptr::null());
                return Err(format!("YUV converter: {error}"));
            }
        },
        None => None,
    };
    Ok(VideoSlot {
        key,
        image,
        image_memory,
        dmabuf_fd: held_fd,
        converter,
        destroy_image,
        free_memory,
    })
}

unsafe fn destroy_video_slot(device: VkDevice, slot: VideoSlot) {
    let mut slot = slot;
    drop(slot.converter.take());
    close(slot.dmabuf_fd);
    (slot.free_memory)(device, slot.image_memory, ptr::null());
    (slot.destroy_image)(device, slot.image, ptr::null());
}

unsafe fn teardown_video_ring(device: VkDevice, ring: &mut VideoRing) {
    while let Some(slot) = ring.slots.pop() {
        let key = slot.key;
        if let Some(converter) = slot.converter.as_ref() {
            line("yuv_conv", "SUMMARY", converter.summary());
        }
        destroy_video_slot(device, slot);
        line("video-ahb", "ring_drop", format!("key={key:016x}"));
    }
}

/// Import and verify one already-received MediaCodec YV12 AHB. The metadata
/// offsets come from `AHardwareBuffer_lockPlanes`, not from a guessed vendor
/// layout. The caller owns the stream and sends the per-frame acknowledgement.
unsafe fn run_video_core(
    lib: *mut c_void,
    physical: VkPhysicalDevice,
    device: VkDevice,
    queue: VkQueue,
    family_index: u32,
    meta: AhbMeta,
    handles: Vec<Vec<c_int>>,
    ring: &mut VideoRing,
    verify: bool,
    verbose: bool,
) -> bool {
    let mut handle_groups = handles.into_iter();
    let fds = handle_groups.next().unwrap_or_default();
    let output_fds = handle_groups.next().unwrap_or_default();
    let close_handles = || {
        for &fd in &fds {
            close(fd);
        }
        for &fd in &output_fds {
            close(fd);
        }
    };
    if meta.width == 0
        || meta.height == 0
        || meta.plane_count != 3
        || meta.planes[0].pixel_stride != 1
        || meta.planes[1].pixel_stride != 1
        || meta.planes[2].pixel_stride != 1
    {
        line("video-ahb", "FAIL", "unsupported YUV plane layout");
        close_handles();
        return false;
    }

    let observed_dma_key = find_dmabuf(&fds).and_then(dmabuf_key);
    let slot_key = if meta.buffer_key != 0 {
        meta.buffer_key
    } else {
        observed_dma_key.unwrap_or(0)
    };
    if slot_key == 0 {
        line("video-ahb", "FAIL", "no stable AHB/dma-buf slot key");
        close_handles();
        return false;
    }
    let slot_index = match ring.find(slot_key) {
        Some(index) => {
            if let Some(key) = observed_dma_key {
                if meta.dmabuf_key != 0 && key != meta.dmabuf_key {
                    line(
                        "video-ahb",
                        "FAIL",
                        format!(
                            "reuse dma-buf key mismatch|expected={:016x}|actual={key:016x}",
                            meta.dmabuf_key
                        ),
                    );
                    close_handles();
                    return false;
                }
            }
            close_handles();
            ring.reuses = ring.reuses.saturating_add(1);
            if verbose {
                line(
                    "video-ahb",
                    "ring_reuse",
                    format!(
                        "slot={index}|key={:016x}|slots={}",
                        ring.slots[index].key,
                        ring.slots.len()
                    ),
                );
            }
            index
        }
        None => {
            let Some(fd) = find_dmabuf(&fds) else {
                line("video-ahb", "FAIL", "no dma-buf fd in native handle");
                close_handles();
                return false;
            };
            let actual_dma_key = dmabuf_key(fd).unwrap_or(0);
            if actual_dma_key == 0 {
                line("video-ahb", "FAIL", "no stable dma-buf key");
                close_handles();
                return false;
            }
            if meta.dmabuf_key != 0 && actual_dma_key != meta.dmabuf_key {
                line(
                    "video-ahb",
                    "FAIL",
                    format!(
                        "metadata dmabuf_key mismatch|expected={:016x}|actual={actual_dma_key:016x}",
                        meta.dmabuf_key
                    ),
                );
                close_handles();
                return false;
            }
            if ring.slots.len() >= VIDEO_SLOT_LIMIT {
                line(
                    "video-ahb",
                    "ring_full",
                    format!("slots={}|key={slot_key:016x}", ring.slots.len()),
                );
                close_handles();
                return false;
            }
            let index = ring.slots.len();
            let output_fd = find_dmabuf(&output_fds);
            if meta.handle_count > 1 && output_fd.is_none() {
                line("video-ahb", "FAIL", "no output dma-buf in second handle");
                close_handles();
                return false;
            }
            if let Some(output_fd) = output_fd {
                if meta.output_format != 1
                    || meta.output_width != meta.width
                    || meta.output_height != meta.height
                    || meta.output_stride == 0
                {
                    line(
                        "video-ahb",
                        "FAIL",
                        format!(
                            "unsupported output AHB|{}x{}|stride={}|format={}",
                            meta.output_width,
                            meta.output_height,
                            meta.output_stride,
                            meta.output_format
                        ),
                    );
                    close_handles();
                    return false;
                }
                let output_key = dmabuf_key(output_fd).unwrap_or(0);
                if output_key == 0
                    || (meta.output_dmabuf_key != 0 && output_key != meta.output_dmabuf_key)
                {
                    line(
                        "video-ahb",
                        "FAIL",
                        format!(
                            "output dma-buf key mismatch|expected={:016x}|actual={output_key:016x}",
                            meta.output_dmabuf_key
                        ),
                    );
                    close_handles();
                    return false;
                }
                line(
                    "video-ahb",
                    "output_layout",
                    format!(
                        "slot={index}|key={:016x}|dmabuf={output_key:016x}|{}x{}|stride={}|format={}|offset={}",
                        meta.output_buffer_key,
                        meta.output_width,
                        meta.output_height,
                        meta.output_stride,
                        meta.output_format,
                        meta.output_offset
                    ),
                );
            }
            match create_video_slot(
                lib,
                physical,
                device,
                queue,
                family_index,
                &meta,
                fd,
                output_fd,
                slot_key,
                index,
            ) {
                Ok(slot) => {
                    ring.slots.push(slot);
                    ring.imports = ring.imports.saturating_add(1);
                    line(
                        "video-ahb",
                        "ring_import",
                        format!(
                            "slot={index}|key={slot_key:016x}|dmabuf={actual_dma_key:016x}|slots={}",
                            ring.slots.len()
                        ),
                    );
                }
                Err(error) => {
                    line(
                        "video-ahb",
                        "ring_import_fail",
                        format!("slot={index}|key={slot_key:016x}|{error}"),
                    );
                    close_handles();
                    return false;
                }
            }
            close_handles();
            index
        }
    };

    let imports = ring.imports;
    let reuses = ring.reuses;
    let slot = &mut ring.slots[slot_index];
    let fd = slot.dmabuf_fd;
    let image = slot.image;
    let dmabuf_size = lseek(fd, 0, 2);
    if verbose {
        line(
            "video-ahb",
            "layout",
            format!(
                "slot={slot_index}|size={}x{} stride={} format={:#x} dmabuf_size={} planes={}",
                meta.width, meta.height, meta.stride, meta.format, dmabuf_size, meta.plane_count
            ),
        );
        for index in 0..meta.plane_count {
            let plane = meta.planes[index];
            line(
                "video-ahb",
                "plane",
                format!(
                    "index={index} offset={} row_stride={} pixel_stride={}",
                    plane.offset, plane.row_stride, plane.pixel_stride
                ),
            );
        }
    }

    let yuv_ok = if meta.format != AHB_FORMAT_YV12 || meta.plane_count < 3 {
        line(
            "yuv_conv",
            "SKIP",
            format!(
                "slot={slot_index}|unsupported input format={:#x} planes={}",
                meta.format, meta.plane_count
            ),
        );
        true
    } else if let Some(converter) = slot.converter.as_mut() {
        converter.convert(verify)
    } else {
        line(
            "yuv_conv",
            "SKIP",
            format!("slot={slot_index}|no output AHB converter"),
        );
        true
    };

    let cpu_ok = if verify {
        let get_format_properties: unsafe extern "C" fn(
            VkPhysicalDevice,
            u32,
            *mut VkFormatProperties,
        ) = match symbol(lib, "vkGetPhysicalDeviceFormatProperties") {
            Some(function) => function,
            None => {
                line(
                    "video-ahb",
                    "FAIL",
                    "vkGetPhysicalDeviceFormatProperties missing",
                );
                return false;
            }
        };
        let mut format_properties = VkFormatProperties::default();
        get_format_properties(
            physical,
            VK_FORMAT_G8_B8_R8_3PLANE_420_UNORM,
            &mut format_properties,
        );
        line(
            "video-ahb",
            "format_properties",
            format!(
                "format=G8_B8_R8_3PLANE_420_UNORM linear={:#x} optimal={:#x} buffer={:#x}",
                format_properties.linear_tiling_features,
                format_properties.optimal_tiling_features,
                format_properties.buffer_features
            ),
        );
        let luma_bytes =
            meta.plane_bytes[0].min(meta.planes[0].row_stride as usize * meta.height as usize);
        let luma_map_bytes = meta.planes[0].row_stride as usize
            * meta.height.saturating_sub(1) as usize
            + meta.width as usize;
        let mmap_bytes = luma_map_bytes
            .max(luma_bytes)
            .min(meta.planes[0].row_stride as usize * meta.height as usize);
        let cpu_map = mmap(ptr::null_mut(), mmap_bytes, PROT_READ, MAP_SHARED, fd, 0);
        if cpu_map as isize == -1 {
            line(
                "video-ahb",
                "cpu_luma",
                format!("FAIL|mmap: {}", std::io::Error::last_os_error()),
            );
            false
        } else {
            let hash = hash_plane_rows(
                cpu_map as *const u8,
                meta.planes[0].row_stride as usize,
                meta.width as usize,
                meta.height as usize,
            );
            let ok = hash != 0 && hash == meta.plane_fnv[0];
            line(
                "video-ahb",
                "cpu_luma",
                format!(
                    "{}|slot={slot_index}|fnv={hash:#x}|expected={:#x}|bytes={luma_bytes}",
                    if ok { "PASS" } else { "FAIL" },
                    meta.plane_fnv[0]
                ),
            );
            munmap(cpu_map, mmap_bytes);
            ok
        }
    } else {
        true
    };

    let mut gpu_ok = true;
    if verify {
        for plane in 0..meta.plane_count.min(3) {
            gpu_ok &= probe_plane_with_gpu(
                lib,
                physical,
                device,
                queue,
                family_index,
                image,
                &meta,
                plane,
            );
        }
    }
    let ok = cpu_ok && gpu_ok && yuv_ok;
    if verify || !ok {
        line(
            "video-ahb",
            "ring_frame",
            format!(
                "{}|slot={slot_index}|key={:016x}|imports={}|reuses={}|verify={verify}|cpu={cpu_ok}|gpu={gpu_ok}|yuv={yuv_ok}",
                if ok { "PASS" } else { "FAIL" },
                slot.key,
                imports,
                reuses
            ),
        );
    }
    ok
}

unsafe fn run_video_on_stream(
    lib: *mut c_void,
    physical: VkPhysicalDevice,
    device: VkDevice,
    queue: VkQueue,
    family_index: u32,
    meta: AhbMeta,
    handles: Vec<Vec<c_int>>,
    ring: &mut VideoRing,
    stream: &mut UnixStream,
    verify: bool,
    verbose: bool,
) -> bool {
    let ok = run_video_core(
        lib,
        physical,
        device,
        queue,
        family_index,
        meta,
        handles,
        ring,
        verify,
        verbose,
    );
    let ack = if ok { b'D' } else { b'F' };
    let handoff = stream.write_all(&[ack]).is_ok();
    if verbose || !ok || !handoff {
        line(
            "video-ahb",
            "display_handoff",
            if ok && handoff { "PASS" } else { "FAIL" },
        );
    }
    ok && handoff
}

/// Receive and verify one AHB over a fresh one-shot connection.
pub unsafe fn run_video(
    lib: *mut c_void,
    physical: VkPhysicalDevice,
    device: VkDevice,
    queue: VkQueue,
    family_index: u32,
    path: &str,
) -> bool {
    let (meta, handles, mut stream) = match receive_handle(path) {
        Ok(value) => value,
        Err(error) => {
            line("video-ahb", "FAIL", format!("receive: {error}"));
            return false;
        }
    };
    let mut ring = VideoRing::new();
    let ok = run_video_on_stream(
        lib,
        physical,
        device,
        queue,
        family_index,
        meta,
        handles,
        &mut ring,
        &mut stream,
        true,
        true,
    );
    teardown_video_ring(device, &mut ring);
    ok
}

/// Receive and verify a bounded sequence over one persistent connection. The
/// app keeps the same socket open and only reuses an AHB after this side sends
/// the per-frame `D` acknowledgement.
pub unsafe fn run_video_many(
    lib: *mut c_void,
    physical: VkPhysicalDevice,
    device: VkDevice,
    queue: VkQueue,
    family_index: u32,
    path: &str,
    frames: usize,
    verify_every: usize,
) -> bool {
    if frames == 0 {
        line("video-ahb", "SUMMARY", "FAIL|frames=0");
        return false;
    }
    let mut stream = match connect_video(path) {
        Ok(stream) => stream,
        Err(error) => {
            line("video-ahb", "FAIL", format!("persistent connect: {error}"));
            return false;
        }
    };
    line(
        "video-ahb",
        "STREAM",
        format!("CONNECTED|path={path}|frames={frames}"),
    );
    let mut ring = VideoRing::new();
    let mut completed = 0usize;
    let verify_every = verify_every.max(1);
    let trace = std::env::var_os("HUSK_VIDEO_TRACE").is_some();
    let mut slow_frames = 0usize;
    let mut max_frame_ms = 0u128;
    let started = std::time::Instant::now();
    for index in 0..frames {
        let frame_started = std::time::Instant::now();
        let verify = index < 6 || index.is_multiple_of(verify_every);
        let verbose = trace || index < 6 || (index + 1).is_multiple_of(30) || index + 1 == frames;
        let (meta, handles) = match receive_handle_from_stream(&mut stream, verbose) {
            Ok(value) => value,
            Err(error) => {
                line(
                    "video-ahb",
                    "frame_summary",
                    format!("FAIL|index={index}|receive={error}"),
                );
                break;
            }
        };
        let receive_ms = frame_started.elapsed().as_millis();
        let work_started = std::time::Instant::now();
        let ok = run_video_on_stream(
            lib,
            physical,
            device,
            queue,
            family_index,
            meta,
            handles,
            &mut ring,
            &mut stream,
            verify,
            verbose,
        );
        let work_ms = work_started.elapsed().as_millis();
        let total_ms = receive_ms.saturating_add(work_ms);
        max_frame_ms = max_frame_ms.max(total_ms);
        if total_ms > 33 {
            slow_frames = slow_frames.saturating_add(1);
        }
        if trace || !ok || index < 3 || (index + 1).is_multiple_of(30) || index + 1 == frames {
            line(
                "video-ahb",
                "frame_summary",
                format!(
                    "{}|index={}|elapsed_ms={total_ms}|receive_ms={receive_ms}|work_ms={work_ms}",
                    if ok { "PASS" } else { "FAIL" },
                    index,
                ),
            );
        }
        if !ok {
            break;
        }
        completed += 1;
    }
    line(
        "video-ahb",
        "ring_summary",
        format!(
            "slots={}|imports={}|reuses={}",
            ring.slots.len(),
            ring.imports,
            ring.reuses
        ),
    );
    teardown_video_ring(device, &mut ring);
    line(
        "video-ahb",
        "STREAM",
        format!("CLOSED|completed={completed}/{frames}"),
    );
    let elapsed = started.elapsed();
    let dropped = frames.saturating_sub(completed);
    let fps = if elapsed.is_zero() {
        0.0
    } else {
        completed as f64 / elapsed.as_secs_f64()
    };
    line(
        "video-ahb",
        "THROUGHPUT",
        format!(
            "expected={frames}|completed={completed}|dropped={dropped}|elapsed_ms={}|fps={fps:.2}|verify_every={verify_every}|slow_gt_33ms={slow_frames}|max_frame_ms={max_frame_ms}",
            elapsed.as_millis(),
        ),
    );
    let ok = completed == frames;
    line(
        "video-ahb",
        "SUMMARY",
        format!(
            "{}|expected={frames}|completed={completed}/{frames}|dropped={dropped}",
            if ok { "PASS" } else { "FAIL" }
        ),
    );
    ok
}

unsafe fn probe_plane_with_gpu(
    lib: *mut c_void,
    physical: VkPhysicalDevice,
    device: VkDevice,
    queue: VkQueue,
    family_index: u32,
    image: VkImage,
    meta: &AhbMeta,
    plane_index: usize,
) -> bool {
    let label = match plane_index {
        0 => "gpu_luma",
        1 => "gpu_plane1",
        2 => "gpu_plane2",
        _ => return false,
    };
    let width = if plane_index == 0 {
        meta.width as usize
    } else {
        (meta.width as usize).div_ceil(2)
    };
    let height = if plane_index == 0 {
        meta.height as usize
    } else {
        (meta.height as usize).div_ceil(2)
    };
    let plane_bytes = meta.plane_bytes[plane_index];
    let expected_fnv = meta.plane_fnv[plane_index];
    let aspect_mask = match plane_index {
        0 => VK_IMAGE_ASPECT_PLANE_0_BIT,
        1 => VK_IMAGE_ASPECT_PLANE_1_BIT,
        2 => VK_IMAGE_ASPECT_PLANE_2_BIT,
        _ => return false,
    };
    if plane_bytes == 0 || expected_fnv == 0 || width == 0 || height == 0 {
        line(
            "video-ahb",
            label,
            format!("SKIP|plane={plane_index}|missing metadata"),
        );
        return false;
    }

    let create_buffer: FnCreateBuffer = match symbol(lib, "vkCreateBuffer") {
        Some(function) => function,
        None => {
            line("video-ahb", label, "FAIL|vkCreateBuffer missing");
            return false;
        }
    };
    let destroy_buffer: FnDestroyBuffer = match symbol(lib, "vkDestroyBuffer") {
        Some(function) => function,
        None => {
            line("video-ahb", label, "FAIL|vkDestroyBuffer missing");
            return false;
        }
    };
    let get_buffer_requirements: FnGetBufferMemoryRequirements =
        match symbol(lib, "vkGetBufferMemoryRequirements") {
            Some(function) => function,
            None => {
                line(
                    "video-ahb",
                    label,
                    "FAIL|vkGetBufferMemoryRequirements missing",
                );
                return false;
            }
        };
    let allocate_memory: FnAllocateMemory = match symbol(lib, "vkAllocateMemory") {
        Some(function) => function,
        None => {
            line("video-ahb", label, "FAIL|vkAllocateMemory missing");
            return false;
        }
    };
    let free_memory: FnFreeMemory = match symbol(lib, "vkFreeMemory") {
        Some(function) => function,
        None => {
            line("video-ahb", label, "FAIL|vkFreeMemory missing");
            return false;
        }
    };
    let bind_buffer_memory: FnBindBufferMemory = match symbol(lib, "vkBindBufferMemory") {
        Some(function) => function,
        None => {
            line("video-ahb", label, "FAIL|vkBindBufferMemory missing");
            return false;
        }
    };
    let map_memory: FnMapMemory = match symbol(lib, "vkMapMemory") {
        Some(function) => function,
        None => {
            line("video-ahb", label, "FAIL|vkMapMemory missing");
            return false;
        }
    };
    let unmap_memory: FnUnmapMemory = match symbol(lib, "vkUnmapMemory") {
        Some(function) => function,
        None => {
            line("video-ahb", label, "FAIL|vkUnmapMemory missing");
            return false;
        }
    };
    let get_memory_props: FnGetPhysicalDeviceMemoryProperties =
        match symbol(lib, "vkGetPhysicalDeviceMemoryProperties") {
            Some(function) => function,
            None => return false,
        };
    let create_pool: FnCreateCommandPool = match symbol(lib, "vkCreateCommandPool") {
        Some(function) => function,
        None => return false,
    };
    let destroy_pool: FnDestroyCommandPool = match symbol(lib, "vkDestroyCommandPool") {
        Some(function) => function,
        None => return false,
    };
    let allocate_commands: FnAllocateCommandBuffers = match symbol(lib, "vkAllocateCommandBuffers")
    {
        Some(function) => function,
        None => return false,
    };
    let begin_command: FnBeginCommandBuffer = match symbol(lib, "vkBeginCommandBuffer") {
        Some(function) => function,
        None => return false,
    };
    let end_command: FnEndCommandBuffer = match symbol(lib, "vkEndCommandBuffer") {
        Some(function) => function,
        None => return false,
    };
    let pipeline_barrier: FnCmdPipelineBarrier = match symbol(lib, "vkCmdPipelineBarrier") {
        Some(function) => function,
        None => return false,
    };
    let copy_image_to_buffer: FnCmdCopyImageToBuffer = match symbol(lib, "vkCmdCopyImageToBuffer") {
        Some(function) => function,
        None => return false,
    };
    let create_fence: FnCreateFence = match symbol(lib, "vkCreateFence") {
        Some(function) => function,
        None => return false,
    };
    let destroy_fence: FnDestroyFence = match symbol(lib, "vkDestroyFence") {
        Some(function) => function,
        None => return false,
    };
    let queue_submit: FnQueueSubmit = match symbol(lib, "vkQueueSubmit") {
        Some(function) => function,
        None => return false,
    };
    let wait_fences: FnWaitForFences = match symbol(lib, "vkWaitForFences") {
        Some(function) => function,
        None => return false,
    };

    let buffer_info = VkBufferCreateInfo {
        s_type: VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO,
        p_next: ptr::null(),
        flags: 0,
        size: plane_bytes as u64,
        usage: VK_BUFFER_USAGE_TRANSFER_DST_BIT,
        sharing_mode: VK_SHARING_MODE_EXCLUSIVE,
        queue_family_index_count: 0,
        p_queue_family_indices: ptr::null(),
    };
    let mut readback: VkBuffer = ptr::null_mut();
    let r = create_buffer(device, &buffer_info, ptr::null(), &mut readback);
    if r != VK_SUCCESS {
        line("video-ahb", label, format!("FAIL|create buffer {r}"));
        return false;
    }
    let mut requirements = VkMemoryRequirements::default();
    get_buffer_requirements(device, readback, &mut requirements);
    let mut memory_properties: VkPhysicalDeviceMemoryProperties = std::mem::zeroed();
    get_memory_props(physical, &mut memory_properties);
    let Some(memory_type_index) =
        choose_memory_type(&memory_properties, requirements.memory_type_bits)
    else {
        line("video-ahb", label, "FAIL|no host memory type");
        destroy_buffer(device, readback, ptr::null());
        return false;
    };
    let allocation = VkMemoryAllocateInfo {
        s_type: VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO,
        p_next: ptr::null(),
        allocation_size: requirements.size,
        memory_type_index,
    };
    let mut readback_memory: VkDeviceMemory = ptr::null_mut();
    let r = allocate_memory(device, &allocation, ptr::null(), &mut readback_memory);
    if r != VK_SUCCESS {
        line("video-ahb", label, format!("FAIL|allocate {r}"));
        destroy_buffer(device, readback, ptr::null());
        return false;
    }
    let r = bind_buffer_memory(device, readback, readback_memory, 0);
    if r != VK_SUCCESS {
        line("video-ahb", label, format!("FAIL|bind {r}"));
        free_memory(device, readback_memory, ptr::null());
        destroy_buffer(device, readback, ptr::null());
        return false;
    }

    let pool_info = VkCommandPoolCreateInfo {
        s_type: VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO,
        p_next: ptr::null(),
        flags: 0,
        queue_family_index: family_index,
    };
    let mut pool: VkCommandPool = ptr::null_mut();
    let r = create_pool(device, &pool_info, ptr::null(), &mut pool);
    if r != VK_SUCCESS {
        line("video-ahb", label, format!("FAIL|command pool {r}"));
        free_memory(device, readback_memory, ptr::null());
        destroy_buffer(device, readback, ptr::null());
        return false;
    }
    let command_info = VkCommandBufferAllocateInfo {
        s_type: VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO,
        p_next: ptr::null(),
        command_pool: pool,
        level: VK_COMMAND_BUFFER_LEVEL_PRIMARY,
        command_buffer_count: 1,
    };
    let mut command: VkCommandBuffer = ptr::null_mut();
    let r = allocate_commands(device, &command_info, &mut command);
    if r != VK_SUCCESS {
        line("video-ahb", label, format!("FAIL|command buffer {r}"));
        destroy_pool(device, pool, ptr::null());
        free_memory(device, readback_memory, ptr::null());
        destroy_buffer(device, readback, ptr::null());
        return false;
    }
    let begin_info = VkCommandBufferBeginInfo {
        s_type: VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO,
        p_next: ptr::null(),
        flags: VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT,
        p_inheritance_info: ptr::null(),
    };
    if begin_command(command, &begin_info) != VK_SUCCESS {
        line("video-ahb", label, "FAIL|begin command");
        destroy_pool(device, pool, ptr::null());
        free_memory(device, readback_memory, ptr::null());
        destroy_buffer(device, readback, ptr::null());
        return false;
    }
    let barrier = VkImageMemoryBarrier {
        s_type: VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER,
        p_next: ptr::null(),
        src_access_mask: 0,
        dst_access_mask: VK_ACCESS_TRANSFER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        new_layout: VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
        src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
        dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
        image,
        subresource_range: VkImageSubresourceRange {
            aspect_mask,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        },
    };
    pipeline_barrier(
        command,
        VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
        VK_PIPELINE_STAGE_TRANSFER_BIT,
        0,
        0,
        ptr::null(),
        0,
        ptr::null(),
        1,
        &barrier,
    );
    let region = VkBufferImageCopy {
        buffer_offset: 0,
        buffer_row_length: 0,
        buffer_image_height: 0,
        image_subresource: VkImageSubresourceLayers {
            aspect_mask,
            mip_level: 0,
            base_array_layer: 0,
            layer_count: 1,
        },
        image_offset: VkOffset3D::default(),
        image_extent: VkExtent3D {
            width: width as u32,
            height: height as u32,
            depth: 1,
        },
    };
    copy_image_to_buffer(
        command,
        image,
        VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
        readback,
        1,
        &region,
    );
    let host_barrier = VkMemoryBarrier {
        s_type: VK_STRUCTURE_TYPE_MEMORY_BARRIER,
        p_next: ptr::null(),
        src_access_mask: VK_ACCESS_TRANSFER_WRITE_BIT,
        dst_access_mask: VK_ACCESS_HOST_READ_BIT,
    };
    pipeline_barrier(
        command,
        VK_PIPELINE_STAGE_TRANSFER_BIT,
        VK_PIPELINE_STAGE_HOST_BIT,
        0,
        1,
        &host_barrier as *const _ as *const c_void,
        0,
        ptr::null(),
        0,
        ptr::null(),
    );
    if end_command(command) != VK_SUCCESS {
        line("video-ahb", label, "FAIL|end command");
        destroy_pool(device, pool, ptr::null());
        free_memory(device, readback_memory, ptr::null());
        destroy_buffer(device, readback, ptr::null());
        return false;
    }

    let fence_info = VkFenceCreateInfo {
        s_type: VK_STRUCTURE_TYPE_FENCE_CREATE_INFO,
        p_next: ptr::null(),
        flags: 0,
    };
    let mut fence: VkFence = ptr::null_mut();
    let r = create_fence(device, &fence_info, ptr::null(), &mut fence);
    if r != VK_SUCCESS {
        line("video-ahb", label, format!("FAIL|fence {r}"));
        destroy_pool(device, pool, ptr::null());
        free_memory(device, readback_memory, ptr::null());
        destroy_buffer(device, readback, ptr::null());
        return false;
    }
    let submit = VkSubmitInfo {
        s_type: VK_STRUCTURE_TYPE_SUBMIT_INFO,
        p_next: ptr::null(),
        wait_semaphore_count: 0,
        p_wait_semaphores: ptr::null(),
        p_wait_dst_stage_mask: ptr::null(),
        command_buffer_count: 1,
        p_command_buffers: &command,
        signal_semaphore_count: 0,
        p_signal_semaphores: ptr::null(),
    };
    let r = queue_submit(queue, 1, &submit, fence);
    let wait_rc = if r == VK_SUCCESS {
        wait_fences(device, 1, &fence, 1, 2_000_000_000)
    } else {
        r
    };
    let mut hash = 0u64;
    let mut mapped = ptr::null_mut();
    if wait_rc == VK_SUCCESS {
        let r = map_memory(
            device,
            readback_memory,
            0,
            requirements.size,
            0,
            &mut mapped,
        );
        if r == VK_SUCCESS && !mapped.is_null() {
            let bytes = core::slice::from_raw_parts(mapped as *const u8, plane_bytes);
            hash = fnv1a(bytes);
            unmap_memory(device, readback_memory);
        } else {
            line("video-ahb", label, format!("FAIL|map {r}"));
        }
    } else {
        line(
            "video-ahb",
            label,
            format!("FAIL|submit/wait submit={r} wait={wait_rc}"),
        );
    }
    destroy_fence(device, fence, ptr::null());
    destroy_pool(device, pool, ptr::null());
    free_memory(device, readback_memory, ptr::null());
    destroy_buffer(device, readback, ptr::null());

    let ok = hash != 0 && hash == expected_fnv;
    line(
        "video-ahb",
        label,
        format!(
            "{}|plane={plane_index}|fnv={hash:#x}|expected={expected_fnv:#x}|bytes={plane_bytes}|size={width}x{height}",
            if ok { "PASS" } else { "FAIL" },
        ),
    );
    ok
}

pub fn socket_arg() -> Option<String> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--ahb-socket" {
            return args.next();
        }
        if let Some(path) = arg.strip_prefix("--ahb-socket=") {
            return Some(path.to_string());
        }
    }
    None
}

pub fn video_socket_arg() -> Option<String> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--video-ahb-socket" {
            return args.next();
        }
        if let Some(path) = arg.strip_prefix("--video-ahb-socket=") {
            return Some(path.to_string());
        }
    }
    None
}

pub fn video_frames_arg() -> Option<usize> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--video-ahb-frames" {
            return args.next().and_then(|value| value.parse().ok());
        }
        if let Some(value) = arg.strip_prefix("--video-ahb-frames=") {
            return value.parse().ok();
        }
    }
    None
}

pub fn video_verify_every_arg() -> Option<usize> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--video-verify-every" {
            return args.next().and_then(|value| value.parse().ok());
        }
        if let Some(value) = arg.strip_prefix("--video-verify-every=") {
            return value.parse().ok();
        }
    }
    None
}
