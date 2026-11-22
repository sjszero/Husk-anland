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
use std::os::unix::net::UnixStream;
use std::thread::JoinHandle;

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

struct CompletionResources {
    device: usize,
    fence: usize,
    command_pool: usize,
    readback_memory: usize,
    readback: usize,
    image_memory: usize,
    image: usize,
    write_fd: c_int,
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
            if !fence.is_null() {
                let _ = (self.wait_fences)(device, 1, &fence, VK_TRUE, u64::MAX);
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
    width: u32,
    height: u32,
    stride: u32,
    format: u32,
    usage: u64,
    cookies: [u32; 9],
    count: usize,
    frames: usize,
}

fn parse_meta(text: &str) -> Result<AhbMeta, String> {
    let mut lines = text.lines();
    let magic = lines.next();
    if magic != Some("HUSK-AHB-1") && magic != Some("HUSK-AHB-2") {
        return Err("bad magic".to_string());
    }
    let mut width = None;
    let mut height = None;
    let mut stride = None;
    let mut format = None;
    let mut usage = None;
    let mut cookies = None;
    let mut count = 1usize;
    let mut frames = 1usize;
    for line in lines {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key {
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
            _ => {}
        }
    }
    Ok(AhbMeta {
        width: width.ok_or_else(|| "missing width".to_string())?,
        height: height.ok_or_else(|| "missing height".to_string())?,
        stride: stride.ok_or_else(|| "missing stride".to_string())?,
        format: format.ok_or_else(|| "missing format".to_string())?,
        usage: usage.ok_or_else(|| "missing usage".to_string())?,
        cookies: cookies.ok_or_else(|| "missing cookies".to_string())?,
        count: count.clamp(1, 8),
        frames: frames.clamp(1, 100_000),
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

    let fence_info = VkFenceCreateInfo {
        s_type: VK_STRUCTURE_TYPE_FENCE_CREATE_INFO,
        p_next: ptr::null(),
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
                    write_fd,
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
                        let device = resources.device as VkDevice;
                        let fence = resources.fence as VkFence;
                        let wait_rc =
                            (resources.wait_fences)(device, 1, &fence, VK_TRUE, u64::MAX);
                        if wait_rc == VK_SUCCESS {
                            let one = 1u64;
                            let wrote = write(
                                resources.write_fd,
                                &one as *const u64 as *const c_void,
                                8,
                            );
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
const VK_IMAGE_TYPE_2D: u32 = 1;
const VK_IMAGE_TILING_DRM_FORMAT_MODIFIER_EXT: u32 = 1_000_158_000;
const VK_IMAGE_USAGE_TRANSFER_SRC_BIT: u32 = 1;
const VK_IMAGE_USAGE_TRANSFER_DST_BIT: u32 = 2;
const VK_IMAGE_LAYOUT_UNDEFINED: u32 = 0;
const VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL: u32 = 6;
const VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL: u32 = 7;
const VK_IMAGE_ASPECT_COLOR_BIT: u32 = 1;
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

unsafe fn receive_handle(path: &str) -> Result<(AhbMeta, Vec<Vec<c_int>>, UnixStream), String> {
    let mut stream = UnixStream::connect(path).map_err(|e| format!("connect {path}: {e}"))?;
    line("dmabuf", "socket", format!("PASS|{path}"));

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
    line(
        "dmabuf",
        "meta",
        format!(
            "PASS|{}x{} stride={} format={} usage={:#x} count={} frames={}",
            meta.width, meta.height, meta.stride, meta.format, meta.usage, meta.count, meta.frames
        ),
    );

    let mut handles = Vec::with_capacity(meta.count);
    for slot in 0..meta.count {
        let fds = recv_handle_fds(&stream)
            .map_err(|e| format!("receive handle {slot}/{}: {e}", meta.count))?;
        handles.push(fds);
    }
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

fn frame_color(frame: usize) -> ([f32; 4], u32) {
    match frame % 3 {
        0 => ([1.0, 0.0, 0.0, 1.0], 0xff00_00ff),
        1 => ([0.0, 1.0, 0.0, 1.0], 0xff00_ff00),
        _ => ([0.0, 0.0, 1.0, 1.0], 0xffff_0000),
    }
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
    line(
        "ring",
        "BEGIN",
        format!(
            "buffers={} frames={} {}x{}",
            meta.count, meta.frames, meta.width, meta.height
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

    let reader_stream = match stream.try_clone() {
        Ok(v) => v,
        Err(e) => {
            line("ring", "FAIL", format!("clone reader: {e}"));
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
    let mut used = vec![false; meta.count];
    let mut rendered = 0usize;
    let mut releases = 0usize;
    let mut ok = true;
    let mut completion_joins = Vec::with_capacity(meta.frames);

    for frame in 0..meta.frames {
        let slot = frame % meta.count;
        if used[slot] {
            line("ring", "WAIT_RELEASE", format!("frame={frame} slot={slot}"));
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
                        if response == format!("RELEASE {slot}") {
                            releases += 1;
                            line("ring", "RELEASE", format!("frame={frame} slot={slot}"));
                            break;
                        }
                        line("ring", "RX", response);
                    }
                    Err(e) => {
                        line("ring", "FAIL", format!("wait release slot={slot}: {e}"));
                        ok = false;
                        break;
                    }
                }
            }
        }
        if !ok {
            break;
        }

        let (color, expected) = frame_color(frame);
        line(
            "ring",
            "RENDER",
            format!("frame={frame} slot={slot} expected={expected:#010x}",),
        );
        let mut completion = AsyncCompletion::default();
        let rendered_ok = render_linear_image(
            lib,
            physical,
            device,
            queue,
            family_index,
            dmabufs[slot],
            meta,
            color,
            expected,
            frame,
            slot,
            Some(&mut completion),
        );
        let acquire_fd = completion.acquire_fd;
        let ready = rendered_ok
            && acquire_fd >= 0
            && send_ready_with_fd(
                &writer,
                &format!("READY {slot} {frame} {expected:08x}\n"),
                acquire_fd,
            );
        if acquire_fd >= 0 {
            close(acquire_fd);
            completion.acquire_fd = -1;
        }
        if let Some(join) = completion.join.take() {
            completion_joins.push(join);
        }
        line(
            "ring",
            "READY",
            format!("frame={frame} slot={slot} rendered={rendered_ok} acquire_fd={acquire_fd} sent={ready}"),
        );
        if !ready {
            ok = false;
            break;
        }
        used[slot] = true;
        rendered += 1;
    }

    for join in completion_joins {
        if join.join().is_err() {
            line("ring", "FAIL", "GPU completion thread panicked");
            ok = false;
        }
    }
    let done = send_text(&writer, &format!("DONE {rendered}\n"));
    line(
        "ring",
        if ok && rendered == meta.frames && done {
            "PASS"
        } else {
            "FAIL"
        },
        format!(
            "rendered={rendered}/{} releases={releases} done={done}",
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
