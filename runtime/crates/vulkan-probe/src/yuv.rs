//! GPU YV12 -> RGBA8 conversion through an imported AHB dma-buf.
//!
//! This is deliberately narrower than a production video renderer: the probe
//! imports the whole YV12 dma-buf as an external storage buffer, runs one
//! compute pass, and compares nine GPU-produced pixels with a CPU reference
//! computed from the same dma-buf.  It proves the data path and colour maths
//! before we add a persistent converter, output image, and WSI.

use super::*;
use core::ffi::{c_int, c_void};
use core::ptr;
use std::ffi::CString;

const VK_EXTERNAL_MEMORY_HANDLE_TYPE_DMA_BUF_BIT_EXT: u32 = 0x200;
const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_EXTERNAL_BUFFER_INFO: u32 = 1_000_071_002;
const VK_STRUCTURE_TYPE_EXTERNAL_BUFFER_PROPERTIES: u32 = 1_000_071_003;
const VK_STRUCTURE_TYPE_EXTERNAL_MEMORY_BUFFER_CREATE_INFO: u32 = 1_000_072_000;
const VK_STRUCTURE_TYPE_IMPORT_MEMORY_FD_INFO_KHR: u32 = 1_000_074_000;
const VK_STRUCTURE_TYPE_MEMORY_BARRIER: u32 = 46;
const VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT: u32 = 1 << 0;
const VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT: u32 = 1 << 5;
const VK_PIPELINE_STAGE_HOST_BIT: u32 = 1 << 14;
const VK_ACCESS_SHADER_READ_BIT: u32 = 1 << 5;
const VK_ACCESS_SHADER_WRITE_BIT: u32 = 1 << 6;
const VK_ACCESS_HOST_WRITE_BIT: u32 = 1 << 8;
const VK_ACCESS_HOST_READ_BIT: u32 = 1 << 9;
const PROT_READ: c_int = 1;
const MAP_SHARED: c_int = 1;
const YUV_SPV: &[u8] = include_bytes!("../shaders/yuv_to_rgba.spv");

#[repr(C)]
struct VkMemoryBarrier {
    s_type: u32,
    p_next: *const c_void,
    src_access_mask: u32,
    dst_access_mask: u32,
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
#[derive(Clone, Copy, Default)]
struct VkExternalMemoryProperties {
    external_memory_features: u32,
    export_from_imported_handle_types: u32,
    compatible_handle_types: u32,
}

#[repr(C)]
struct VkExternalBufferProperties {
    s_type: u32,
    p_next: *mut c_void,
    external_memory_properties: VkExternalMemoryProperties,
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
struct VkPushConstantRange {
    stage_flags: u32,
    offset: u32,
    size: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct PushParams {
    width: u32,
    height: u32,
    y_offset: u32,
    v_offset: u32,
    u_offset: u32,
    y_stride: u32,
    chroma_stride: u32,
    reserved: u32,
}

type FnCmdPushConstants =
    unsafe extern "C" fn(VkCommandBuffer, VkPipelineLayout, u32, u32, u32, *const c_void);
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
    *const c_void,
);
type FnGetExternalBufferProperties = unsafe extern "C" fn(
    VkPhysicalDevice,
    *const VkPhysicalDeviceExternalBufferInfo,
    *mut VkExternalBufferProperties,
);

extern "C" {
    fn mmap(
        addr: *mut c_void,
        len: usize,
        prot: c_int,
        flags: c_int,
        fd: c_int,
        offset: i64,
    ) -> *mut c_void;
    fn munmap(addr: *mut c_void, len: usize) -> c_int;
    fn dup(fd: c_int) -> c_int;
    fn lseek(fd: c_int, offset: i64, whence: c_int) -> i64;
}

fn words(bytes: &[u8]) -> Vec<u32> {
    bytes
        .chunks_exact(4)
        .map(|chunk| u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

fn sample_points(width: u32, height: u32) -> [(u32, u32); 9] {
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

fn yuv_to_rgba(y: u8, v: u8, u: u8) -> u32 {
    let c = i32::from(y).saturating_sub(16).max(0);
    let d = i32::from(u) - 128;
    let e = i32::from(v) - 128;

    let rv = 298 * c + 409 * e;
    let gv = 298 * c - 100 * d - 208 * e;
    let bv = 298 * c + 516 * d;
    let scale = |value: i32| -> u32 {
        if value <= 0 {
            0
        } else {
            ((value + 128) >> 8).clamp(0, 255) as u32
        }
    };
    scale(rv) | (scale(gv) << 8) | (scale(bv) << 16) | 0xff00_0000
}

fn input_size(width: u32, height: u32, offsets: [u64; 3], strides: [u32; 3]) -> Option<u64> {
    if width == 0 || height == 0 || strides[0] < width || strides[1] < width.div_ceil(2) {
        return None;
    }
    let chroma_height = height.div_ceil(2) as u64;
    let y = offsets[0].checked_add(strides[0] as u64 * height as u64)?;
    let v = offsets[1].checked_add(strides[1] as u64 * chroma_height)?;
    let u = offsets[2].checked_add(strides[2] as u64 * chroma_height)?;
    Some(y.max(v).max(u))
}

unsafe fn cpu_expected(
    fd: c_int,
    width: u32,
    height: u32,
    offsets: [u64; 3],
    strides: [u32; 3],
) -> Result<[u32; 9], String> {
    let bytes = input_size(width, height, offsets, strides)
        .ok_or_else(|| "invalid YUV plane geometry".to_string())?;
    let len = usize::try_from(bytes).map_err(|_| "YUV mapping is too large".to_string())?;
    let map = mmap(ptr::null_mut(), len, PROT_READ, MAP_SHARED, fd, 0);
    if map as isize == -1 {
        return Err(format!(
            "mmap YV12 dma-buf: {}",
            std::io::Error::last_os_error()
        ));
    }
    let base = map as *const u8;
    let byte = |plane: usize, x: u32, y: u32| -> u8 {
        let offset = offsets[plane] as usize + y as usize * strides[plane] as usize + x as usize;
        *base.add(offset)
    };
    let mut expected = [0u32; 9];
    for (index, &(x, y)) in sample_points(width, height).iter().enumerate() {
        let chroma_x = x / 2;
        let chroma_y = y / 2;
        expected[index] = yuv_to_rgba(
            byte(0, x, y),
            byte(1, chroma_x, chroma_y),
            byte(2, chroma_x, chroma_y),
        );
    }
    let rc = munmap(map, len);
    if rc != 0 {
        return Err(format!(
            "munmap YV12 dma-buf: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(expected)
}

unsafe fn barrier(
    command: VkCommandBuffer,
    cmd_pipeline_barrier: unsafe extern "C" fn(
        VkCommandBuffer,
        u32,
        u32,
        u32,
        u32,
        *const c_void,
        u32,
        *const c_void,
        u32,
        *const c_void,
    ),
    src_stage: u32,
    dst_stage: u32,
    src_access: u32,
    dst_access: u32,
) {
    let memory_barrier = VkMemoryBarrier {
        s_type: VK_STRUCTURE_TYPE_MEMORY_BARRIER,
        p_next: ptr::null(),
        src_access_mask: src_access,
        dst_access_mask: dst_access,
    };
    cmd_pipeline_barrier(
        command,
        src_stage,
        dst_stage,
        0,
        1,
        &memory_barrier as *const _ as *const c_void,
        0,
        ptr::null(),
        0,
        ptr::null(),
    );
}

struct Cleanup {
    device: VkDevice,
    destroy_buffer: Option<FnDestroyBuffer>,
    free_memory: Option<FnFreeMemory>,
    destroy_shader: Option<FnDestroyShaderModule>,
    destroy_set_layout: Option<FnDestroyDescriptorSetLayout>,
    destroy_pipeline_layout: Option<FnDestroyPipelineLayout>,
    destroy_pipeline: Option<FnDestroyPipeline>,
    destroy_pool: Option<FnDestroyDescriptorPool>,
    destroy_command_pool: Option<FnDestroyCommandPool>,
    destroy_fence: Option<FnDestroyFence>,
    input_buffer: VkBuffer,
    input_memory: VkDeviceMemory,
    output_buffer: VkBuffer,
    output_memory: VkDeviceMemory,
    shader: VkShaderModule,
    set_layout: VkDescriptorSetLayout,
    pipeline_layout: VkPipelineLayout,
    pipeline: VkPipeline,
    descriptor_pool: VkDescriptorPool,
    command_pool: VkCommandPool,
    fence: VkFence,
}

impl Cleanup {
    fn new(device: VkDevice) -> Self {
        Self {
            device,
            destroy_buffer: None,
            free_memory: None,
            destroy_shader: None,
            destroy_set_layout: None,
            destroy_pipeline_layout: None,
            destroy_pipeline: None,
            destroy_pool: None,
            destroy_command_pool: None,
            destroy_fence: None,
            input_buffer: ptr::null_mut(),
            input_memory: ptr::null_mut(),
            output_buffer: ptr::null_mut(),
            output_memory: ptr::null_mut(),
            shader: ptr::null_mut(),
            set_layout: ptr::null_mut(),
            pipeline_layout: ptr::null_mut(),
            pipeline: ptr::null_mut(),
            descriptor_pool: ptr::null_mut(),
            command_pool: ptr::null_mut(),
            fence: ptr::null_mut(),
        }
    }
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        unsafe {
            if let Some(destroy) = self.destroy_fence {
                if !self.fence.is_null() {
                    destroy(self.device, self.fence, ptr::null());
                }
            }
            if let Some(destroy) = self.destroy_command_pool {
                if !self.command_pool.is_null() {
                    destroy(self.device, self.command_pool, ptr::null());
                }
            }
            if let Some(destroy) = self.destroy_pool {
                if !self.descriptor_pool.is_null() {
                    destroy(self.device, self.descriptor_pool, ptr::null());
                }
            }
            if let Some(destroy) = self.destroy_pipeline {
                if !self.pipeline.is_null() {
                    destroy(self.device, self.pipeline, ptr::null());
                }
            }
            if let Some(destroy) = self.destroy_pipeline_layout {
                if !self.pipeline_layout.is_null() {
                    destroy(self.device, self.pipeline_layout, ptr::null());
                }
            }
            if let Some(destroy) = self.destroy_set_layout {
                if !self.set_layout.is_null() {
                    destroy(self.device, self.set_layout, ptr::null());
                }
            }
            if let Some(destroy) = self.destroy_shader {
                if !self.shader.is_null() {
                    destroy(self.device, self.shader, ptr::null());
                }
            }
            if let Some(free) = self.free_memory {
                if !self.output_memory.is_null() {
                    free(self.device, self.output_memory, ptr::null());
                }
                if !self.input_memory.is_null() {
                    free(self.device, self.input_memory, ptr::null());
                }
            }
            if let Some(destroy) = self.destroy_buffer {
                if !self.output_buffer.is_null() {
                    destroy(self.device, self.output_buffer, ptr::null());
                }
                if !self.input_buffer.is_null() {
                    destroy(self.device, self.input_buffer, ptr::null());
                }
            }
        }
    }
}

/// Convert one imported YV12 dma-buf to RGBA8 and compare nine pixels with the
/// CPU reference.  `fd` remains owned by the caller.
pub(crate) unsafe fn run(
    lib: *mut c_void,
    physical: VkPhysicalDevice,
    device: VkDevice,
    queue: VkQueue,
    family_index: u32,
    fd: c_int,
    width: u32,
    height: u32,
    offsets: [u64; 3],
    strides: [u32; 3],
) -> bool {
    if width == 0 || height == 0 {
        line("yuv_rgb", "SKIP", "zero-sized frame");
        return false;
    }
    if offsets.iter().any(|&offset| offset > u32::MAX as u64)
        || strides.iter().any(|&stride| stride == 0)
    {
        line("yuv_rgb", "SKIP", "plane geometry exceeds shader limits");
        return false;
    }
    if strides[1] != strides[2] {
        line(
            "yuv_rgb",
            "SKIP",
            format!("chroma strides differ: {} vs {}", strides[1], strides[2]),
        );
        return false;
    }

    let expected = match cpu_expected(fd, width, height, offsets, strides) {
        Ok(value) => value,
        Err(error) => {
            line("yuv_rgb", "FAIL", error);
            return false;
        }
    };
    let Some(input_bytes) = input_size(width, height, offsets, strides) else {
        line("yuv_rgb", "FAIL", "invalid input size");
        return false;
    };
    let fd_size = lseek(fd, 0, 2);
    if fd_size < 0 || (fd_size as u64) < input_bytes {
        line(
            "yuv_rgb",
            "FAIL",
            format!("dma-buf too small: fd_size={fd_size} required={input_bytes}"),
        );
        return false;
    }
    let Some(output_bytes) = (width as u64)
        .checked_mul(height as u64)
        .and_then(|pixels| pixels.checked_mul(4))
    else {
        line("yuv_rgb", "FAIL", "output size overflow");
        return false;
    };

    let create_buffer: FnCreateBuffer = match symbol(lib, "vkCreateBuffer") {
        Some(function) => function,
        None => {
            line("yuv_rgb", "FAIL", "vkCreateBuffer missing");
            return false;
        }
    };
    let destroy_buffer: FnDestroyBuffer = match symbol(lib, "vkDestroyBuffer") {
        Some(function) => function,
        None => {
            line("yuv_rgb", "FAIL", "vkDestroyBuffer missing");
            return false;
        }
    };
    let get_buffer_requirements: FnGetBufferMemoryRequirements =
        match symbol(lib, "vkGetBufferMemoryRequirements") {
            Some(function) => function,
            None => {
                line("yuv_rgb", "FAIL", "vkGetBufferMemoryRequirements missing");
                return false;
            }
        };
    let allocate_memory: FnAllocateMemory = match symbol(lib, "vkAllocateMemory") {
        Some(function) => function,
        None => {
            line("yuv_rgb", "FAIL", "vkAllocateMemory missing");
            return false;
        }
    };
    let free_memory: FnFreeMemory = match symbol(lib, "vkFreeMemory") {
        Some(function) => function,
        None => {
            line("yuv_rgb", "FAIL", "vkFreeMemory missing");
            return false;
        }
    };
    let bind_buffer: FnBindBufferMemory = match symbol(lib, "vkBindBufferMemory") {
        Some(function) => function,
        None => {
            line("yuv_rgb", "FAIL", "vkBindBufferMemory missing");
            return false;
        }
    };
    let get_external_props: FnGetExternalBufferProperties =
        match symbol(lib, "vkGetPhysicalDeviceExternalBufferProperties") {
            Some(function) => function,
            None => {
                line(
                    "yuv_rgb",
                    "FAIL",
                    "vkGetPhysicalDeviceExternalBufferProperties missing",
                );
                return false;
            }
        };
    let get_memory_props: FnGetPhysicalDeviceMemoryProperties =
        match symbol(lib, "vkGetPhysicalDeviceMemoryProperties") {
            Some(function) => function,
            None => {
                line(
                    "yuv_rgb",
                    "FAIL",
                    "vkGetPhysicalDeviceMemoryProperties missing",
                );
                return false;
            }
        };
    let create_shader: FnCreateShaderModule = match symbol(lib, "vkCreateShaderModule") {
        Some(function) => function,
        None => {
            line("yuv_rgb", "FAIL", "vkCreateShaderModule missing");
            return false;
        }
    };
    let destroy_shader: FnDestroyShaderModule = match symbol(lib, "vkDestroyShaderModule") {
        Some(function) => function,
        None => {
            line("yuv_rgb", "FAIL", "vkDestroyShaderModule missing");
            return false;
        }
    };
    let create_set_layout: FnCreateDescriptorSetLayout =
        match symbol(lib, "vkCreateDescriptorSetLayout") {
            Some(function) => function,
            None => {
                line("yuv_rgb", "FAIL", "vkCreateDescriptorSetLayout missing");
                return false;
            }
        };
    let destroy_set_layout: FnDestroyDescriptorSetLayout =
        match symbol(lib, "vkDestroyDescriptorSetLayout") {
            Some(function) => function,
            None => {
                line("yuv_rgb", "FAIL", "vkDestroyDescriptorSetLayout missing");
                return false;
            }
        };
    let create_pipeline_layout: FnCreatePipelineLayout = match symbol(lib, "vkCreatePipelineLayout")
    {
        Some(function) => function,
        None => {
            line("yuv_rgb", "FAIL", "vkCreatePipelineLayout missing");
            return false;
        }
    };
    let destroy_pipeline_layout: FnDestroyPipelineLayout =
        match symbol(lib, "vkDestroyPipelineLayout") {
            Some(function) => function,
            None => {
                line("yuv_rgb", "FAIL", "vkDestroyPipelineLayout missing");
                return false;
            }
        };
    let create_pipeline: FnCreateComputePipelines = match symbol(lib, "vkCreateComputePipelines") {
        Some(function) => function,
        None => {
            line("yuv_rgb", "FAIL", "vkCreateComputePipelines missing");
            return false;
        }
    };
    let destroy_pipeline: FnDestroyPipeline = match symbol(lib, "vkDestroyPipeline") {
        Some(function) => function,
        None => {
            line("yuv_rgb", "FAIL", "vkDestroyPipeline missing");
            return false;
        }
    };
    let create_descriptor_pool: FnCreateDescriptorPool = match symbol(lib, "vkCreateDescriptorPool")
    {
        Some(function) => function,
        None => {
            line("yuv_rgb", "FAIL", "vkCreateDescriptorPool missing");
            return false;
        }
    };
    let destroy_descriptor_pool: FnDestroyDescriptorPool =
        match symbol(lib, "vkDestroyDescriptorPool") {
            Some(function) => function,
            None => {
                line("yuv_rgb", "FAIL", "vkDestroyDescriptorPool missing");
                return false;
            }
        };
    let allocate_sets: FnAllocateDescriptorSets = match symbol(lib, "vkAllocateDescriptorSets") {
        Some(function) => function,
        None => {
            line("yuv_rgb", "FAIL", "vkAllocateDescriptorSets missing");
            return false;
        }
    };
    let update_sets: FnUpdateDescriptorSets = match symbol(lib, "vkUpdateDescriptorSets") {
        Some(function) => function,
        None => {
            line("yuv_rgb", "FAIL", "vkUpdateDescriptorSets missing");
            return false;
        }
    };
    let create_command_pool: FnCreateCommandPool = match symbol(lib, "vkCreateCommandPool") {
        Some(function) => function,
        None => {
            line("yuv_rgb", "FAIL", "vkCreateCommandPool missing");
            return false;
        }
    };
    let destroy_command_pool: FnDestroyCommandPool = match symbol(lib, "vkDestroyCommandPool") {
        Some(function) => function,
        None => {
            line("yuv_rgb", "FAIL", "vkDestroyCommandPool missing");
            return false;
        }
    };
    let allocate_commands: FnAllocateCommandBuffers = match symbol(lib, "vkAllocateCommandBuffers")
    {
        Some(function) => function,
        None => {
            line("yuv_rgb", "FAIL", "vkAllocateCommandBuffers missing");
            return false;
        }
    };
    let begin_command: FnBeginCommandBuffer = match symbol(lib, "vkBeginCommandBuffer") {
        Some(function) => function,
        None => {
            line("yuv_rgb", "FAIL", "vkBeginCommandBuffer missing");
            return false;
        }
    };
    let end_command: FnEndCommandBuffer = match symbol(lib, "vkEndCommandBuffer") {
        Some(function) => function,
        None => {
            line("yuv_rgb", "FAIL", "vkEndCommandBuffer missing");
            return false;
        }
    };
    let bind_pipeline: FnCmdBindPipeline = match symbol(lib, "vkCmdBindPipeline") {
        Some(function) => function,
        None => {
            line("yuv_rgb", "FAIL", "vkCmdBindPipeline missing");
            return false;
        }
    };
    let bind_sets: FnCmdBindDescriptorSets = match symbol(lib, "vkCmdBindDescriptorSets") {
        Some(function) => function,
        None => {
            line("yuv_rgb", "FAIL", "vkCmdBindDescriptorSets missing");
            return false;
        }
    };
    let push_constants: FnCmdPushConstants = match symbol(lib, "vkCmdPushConstants") {
        Some(function) => function,
        None => {
            line("yuv_rgb", "FAIL", "vkCmdPushConstants missing");
            return false;
        }
    };
    let dispatch: FnCmdDispatch = match symbol(lib, "vkCmdDispatch") {
        Some(function) => function,
        None => {
            line("yuv_rgb", "FAIL", "vkCmdDispatch missing");
            return false;
        }
    };
    let pipeline_barrier: FnCmdPipelineBarrier = match symbol(lib, "vkCmdPipelineBarrier") {
        Some(function) => function,
        None => {
            line("yuv_rgb", "FAIL", "vkCmdPipelineBarrier missing");
            return false;
        }
    };
    let create_fence: FnCreateFence = match symbol(lib, "vkCreateFence") {
        Some(function) => function,
        None => {
            line("yuv_rgb", "FAIL", "vkCreateFence missing");
            return false;
        }
    };
    let destroy_fence: FnDestroyFence = match symbol(lib, "vkDestroyFence") {
        Some(function) => function,
        None => {
            line("yuv_rgb", "FAIL", "vkDestroyFence missing");
            return false;
        }
    };
    let wait_fences: FnWaitForFences = match symbol(lib, "vkWaitForFences") {
        Some(function) => function,
        None => {
            line("yuv_rgb", "FAIL", "vkWaitForFences missing");
            return false;
        }
    };
    let queue_submit: FnQueueSubmit = match symbol(lib, "vkQueueSubmit") {
        Some(function) => function,
        None => {
            line("yuv_rgb", "FAIL", "vkQueueSubmit missing");
            return false;
        }
    };
    let map_memory: FnMapMemory = match symbol(lib, "vkMapMemory") {
        Some(function) => function,
        None => {
            line("yuv_rgb", "FAIL", "vkMapMemory missing");
            return false;
        }
    };
    let unmap_memory: FnUnmapMemory = match symbol(lib, "vkUnmapMemory") {
        Some(function) => function,
        None => {
            line("yuv_rgb", "FAIL", "vkUnmapMemory missing");
            return false;
        }
    };

    let mut cleanup = Cleanup::new(device);
    cleanup.destroy_buffer = Some(destroy_buffer);
    cleanup.free_memory = Some(free_memory);
    cleanup.destroy_shader = Some(destroy_shader);
    cleanup.destroy_set_layout = Some(destroy_set_layout);
    cleanup.destroy_pipeline_layout = Some(destroy_pipeline_layout);
    cleanup.destroy_pipeline = Some(destroy_pipeline);
    cleanup.destroy_pool = Some(destroy_descriptor_pool);
    cleanup.destroy_command_pool = Some(destroy_command_pool);
    cleanup.destroy_fence = Some(destroy_fence);

    let usage = VK_BUFFER_USAGE_STORAGE_BUFFER_BIT;
    let external_info = VkPhysicalDeviceExternalBufferInfo {
        s_type: VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_EXTERNAL_BUFFER_INFO,
        p_next: ptr::null(),
        flags: 0,
        usage,
        handle_type: VK_EXTERNAL_MEMORY_HANDLE_TYPE_DMA_BUF_BIT_EXT,
    };
    let mut external_props = VkExternalBufferProperties {
        s_type: VK_STRUCTURE_TYPE_EXTERNAL_BUFFER_PROPERTIES,
        p_next: ptr::null_mut(),
        external_memory_properties: VkExternalMemoryProperties::default(),
    };
    get_external_props(physical, &external_info, &mut external_props);
    line(
        "yuv_rgb",
        "external_props",
        format!(
            "features={:#x} export={:#x} compatible={:#x}",
            external_props
                .external_memory_properties
                .external_memory_features,
            external_props
                .external_memory_properties
                .export_from_imported_handle_types,
            external_props
                .external_memory_properties
                .compatible_handle_types
        ),
    );
    if external_props
        .external_memory_properties
        .compatible_handle_types
        & VK_EXTERNAL_MEMORY_HANDLE_TYPE_DMA_BUF_BIT_EXT
        == 0
    {
        line(
            "yuv_rgb",
            "FAIL",
            "DMA_BUF storage-buffer import not advertised",
        );
        return false;
    }

    let external = VkExternalMemoryBufferCreateInfo {
        s_type: VK_STRUCTURE_TYPE_EXTERNAL_MEMORY_BUFFER_CREATE_INFO,
        p_next: ptr::null(),
        handle_types: VK_EXTERNAL_MEMORY_HANDLE_TYPE_DMA_BUF_BIT_EXT,
    };
    let input_info = VkBufferCreateInfo {
        s_type: VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO,
        p_next: &external as *const _ as *const c_void,
        flags: 0,
        size: input_bytes,
        usage,
        sharing_mode: VK_SHARING_MODE_EXCLUSIVE,
        queue_family_index_count: 0,
        p_queue_family_indices: ptr::null(),
    };
    if create_buffer(device, &input_info, ptr::null(), &mut cleanup.input_buffer) != VK_SUCCESS {
        line("yuv_rgb", "FAIL", "create external input buffer");
        return false;
    }
    let mut input_req = VkMemoryRequirements::default();
    get_buffer_requirements(device, cleanup.input_buffer, &mut input_req);
    let mut memory_properties: VkPhysicalDeviceMemoryProperties = std::mem::zeroed();
    get_memory_props(physical, &mut memory_properties);
    let Some(input_memory_type) =
        choose_memory_type(&memory_properties, input_req.memory_type_bits)
    else {
        line(
            "yuv_rgb",
            "FAIL",
            "no memory type for external input buffer",
        );
        return false;
    };
    let import_fd = dup(fd);
    if import_fd < 0 {
        line(
            "yuv_rgb",
            "FAIL",
            format!("dup dma-buf: {}", std::io::Error::last_os_error()),
        );
        return false;
    }
    let mut import = VkImportMemoryFdInfo {
        s_type: VK_STRUCTURE_TYPE_IMPORT_MEMORY_FD_INFO_KHR,
        p_next: ptr::null(),
        handle_type: VK_EXTERNAL_MEMORY_HANDLE_TYPE_DMA_BUF_BIT_EXT,
        fd: import_fd,
    };
    let input_alloc = VkMemoryAllocateInfo {
        s_type: VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO,
        p_next: &mut import as *mut _ as *const c_void,
        allocation_size: input_req.size,
        memory_type_index: input_memory_type,
    };
    let import_result =
        allocate_memory(device, &input_alloc, ptr::null(), &mut cleanup.input_memory);
    close(import_fd);
    line(
        "yuv_rgb",
        "input_import",
        format!(
            "{}|size={}|required={}|fd_size={fd_size}|type={input_memory_type}",
            api_result(import_result),
            input_req.size,
            input_bytes
        ),
    );
    if import_result != VK_SUCCESS {
        return false;
    }
    if bind_buffer(device, cleanup.input_buffer, cleanup.input_memory, 0) != VK_SUCCESS {
        line("yuv_rgb", "FAIL", "bind external input buffer");
        return false;
    }

    let output_info = VkBufferCreateInfo {
        s_type: VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO,
        p_next: ptr::null(),
        flags: 0,
        size: output_bytes,
        usage: VK_BUFFER_USAGE_STORAGE_BUFFER_BIT,
        sharing_mode: VK_SHARING_MODE_EXCLUSIVE,
        queue_family_index_count: 0,
        p_queue_family_indices: ptr::null(),
    };
    if create_buffer(
        device,
        &output_info,
        ptr::null(),
        &mut cleanup.output_buffer,
    ) != VK_SUCCESS
    {
        line("yuv_rgb", "FAIL", "create output buffer");
        return false;
    }
    let mut output_req = VkMemoryRequirements::default();
    get_buffer_requirements(device, cleanup.output_buffer, &mut output_req);
    let Some(output_memory_type) =
        choose_memory_type(&memory_properties, output_req.memory_type_bits)
    else {
        line("yuv_rgb", "FAIL", "no host-visible output memory");
        return false;
    };
    let output_alloc = VkMemoryAllocateInfo {
        s_type: VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO,
        p_next: ptr::null(),
        allocation_size: output_req.size,
        memory_type_index: output_memory_type,
    };
    if allocate_memory(
        device,
        &output_alloc,
        ptr::null(),
        &mut cleanup.output_memory,
    ) != VK_SUCCESS
    {
        line("yuv_rgb", "FAIL", "allocate output memory");
        return false;
    }
    if bind_buffer(device, cleanup.output_buffer, cleanup.output_memory, 0) != VK_SUCCESS {
        line("yuv_rgb", "FAIL", "bind output buffer");
        return false;
    }

    let shader_words = words(YUV_SPV);
    if shader_words.is_empty() || shader_words.len() * 4 != YUV_SPV.len() {
        line("yuv_rgb", "FAIL", "invalid embedded shader");
        return false;
    }
    let shader_info = VkShaderModuleCreateInfo {
        s_type: VK_STRUCTURE_TYPE_SHADER_MODULE_CREATE_INFO,
        p_next: ptr::null(),
        flags: 0,
        code_size: shader_words.len() * 4,
        p_code: shader_words.as_ptr(),
    };
    if create_shader(device, &shader_info, ptr::null(), &mut cleanup.shader) != VK_SUCCESS {
        line("yuv_rgb", "FAIL", "create shader module");
        return false;
    }

    let bindings = [
        VkDescriptorSetLayoutBinding {
            binding: 0,
            descriptor_type: VK_DESCRIPTOR_TYPE_STORAGE_BUFFER,
            descriptor_count: 1,
            stage_flags: VK_SHADER_STAGE_COMPUTE_BIT,
            p_immutable_samplers: ptr::null(),
        },
        VkDescriptorSetLayoutBinding {
            binding: 1,
            descriptor_type: VK_DESCRIPTOR_TYPE_STORAGE_BUFFER,
            descriptor_count: 1,
            stage_flags: VK_SHADER_STAGE_COMPUTE_BIT,
            p_immutable_samplers: ptr::null(),
        },
    ];
    let set_layout_info = VkDescriptorSetLayoutCreateInfo {
        s_type: VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_CREATE_INFO,
        p_next: ptr::null(),
        flags: 0,
        binding_count: bindings.len() as u32,
        p_bindings: bindings.as_ptr(),
    };
    if create_set_layout(
        device,
        &set_layout_info,
        ptr::null(),
        &mut cleanup.set_layout,
    ) != VK_SUCCESS
    {
        line("yuv_rgb", "FAIL", "create descriptor set layout");
        return false;
    }
    let push_range = VkPushConstantRange {
        stage_flags: VK_SHADER_STAGE_COMPUTE_BIT,
        offset: 0,
        size: core::mem::size_of::<PushParams>() as u32,
    };
    let pipeline_layout_info = VkPipelineLayoutCreateInfo {
        s_type: VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO,
        p_next: ptr::null(),
        flags: 0,
        set_layout_count: 1,
        p_set_layouts: &cleanup.set_layout,
        push_constant_range_count: 1,
        p_push_constant_ranges: &push_range as *const _ as *const c_void,
    };
    if create_pipeline_layout(
        device,
        &pipeline_layout_info,
        ptr::null(),
        &mut cleanup.pipeline_layout,
    ) != VK_SUCCESS
    {
        line("yuv_rgb", "FAIL", "create pipeline layout");
        return false;
    }

    let main_name = CString::new("main").expect("static");
    let stage = VkPipelineShaderStageCreateInfo {
        s_type: VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO,
        p_next: ptr::null(),
        flags: 0,
        stage: VK_SHADER_STAGE_COMPUTE_BIT,
        module: cleanup.shader,
        p_name: main_name.as_ptr(),
        p_specialization_info: ptr::null(),
    };
    let pipeline_info = VkComputePipelineCreateInfo {
        s_type: VK_STRUCTURE_TYPE_COMPUTE_PIPELINE_CREATE_INFO,
        p_next: ptr::null(),
        flags: 0,
        stage,
        layout: cleanup.pipeline_layout,
        base_pipeline_handle: ptr::null_mut(),
        base_pipeline_index: -1,
    };
    if create_pipeline(
        device,
        ptr::null_mut(),
        1,
        &pipeline_info,
        ptr::null(),
        &mut cleanup.pipeline,
    ) != VK_SUCCESS
    {
        line("yuv_rgb", "FAIL", "create compute pipeline");
        return false;
    }

    let pool_sizes = [VkDescriptorPoolSize {
        ty: VK_DESCRIPTOR_TYPE_STORAGE_BUFFER,
        descriptor_count: 2,
    }];
    let pool_info = VkDescriptorPoolCreateInfo {
        s_type: VK_STRUCTURE_TYPE_DESCRIPTOR_POOL_CREATE_INFO,
        p_next: ptr::null(),
        flags: 0,
        max_sets: 1,
        pool_size_count: 1,
        p_pool_sizes: pool_sizes.as_ptr(),
    };
    if create_descriptor_pool(
        device,
        &pool_info,
        ptr::null(),
        &mut cleanup.descriptor_pool,
    ) != VK_SUCCESS
    {
        line("yuv_rgb", "FAIL", "create descriptor pool");
        return false;
    }
    let set_info = VkDescriptorSetAllocateInfo {
        s_type: VK_STRUCTURE_TYPE_DESCRIPTOR_SET_ALLOCATE_INFO,
        p_next: ptr::null(),
        descriptor_pool: cleanup.descriptor_pool,
        descriptor_set_count: 1,
        p_set_layouts: &cleanup.set_layout,
    };
    let mut descriptor_set: VkDescriptorSet = ptr::null_mut();
    if allocate_sets(device, &set_info, &mut descriptor_set) != VK_SUCCESS {
        line("yuv_rgb", "FAIL", "allocate descriptor set");
        return false;
    }
    let input_binding = VkDescriptorBufferInfo {
        buffer: cleanup.input_buffer,
        offset: 0,
        range: input_bytes,
    };
    let output_binding = VkDescriptorBufferInfo {
        buffer: cleanup.output_buffer,
        offset: 0,
        range: output_bytes,
    };
    let writes = [
        VkWriteDescriptorSet {
            s_type: VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET,
            p_next: ptr::null(),
            dst_set: descriptor_set,
            dst_binding: 0,
            dst_array_element: 0,
            descriptor_count: 1,
            descriptor_type: VK_DESCRIPTOR_TYPE_STORAGE_BUFFER,
            p_image_info: ptr::null(),
            p_buffer_info: &input_binding,
            p_texel_buffer_view: ptr::null(),
        },
        VkWriteDescriptorSet {
            s_type: VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET,
            p_next: ptr::null(),
            dst_set: descriptor_set,
            dst_binding: 1,
            dst_array_element: 0,
            descriptor_count: 1,
            descriptor_type: VK_DESCRIPTOR_TYPE_STORAGE_BUFFER,
            p_image_info: ptr::null(),
            p_buffer_info: &output_binding,
            p_texel_buffer_view: ptr::null(),
        },
    ];
    update_sets(device, writes.len() as u32, writes.as_ptr(), 0, ptr::null());

    let command_pool_info = VkCommandPoolCreateInfo {
        s_type: VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO,
        p_next: ptr::null(),
        flags: 0,
        queue_family_index: family_index,
    };
    if create_command_pool(
        device,
        &command_pool_info,
        ptr::null(),
        &mut cleanup.command_pool,
    ) != VK_SUCCESS
    {
        line("yuv_rgb", "FAIL", "create command pool");
        return false;
    }
    let command_info = VkCommandBufferAllocateInfo {
        s_type: VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO,
        p_next: ptr::null(),
        command_pool: cleanup.command_pool,
        level: VK_COMMAND_BUFFER_LEVEL_PRIMARY,
        command_buffer_count: 1,
    };
    let mut command: VkCommandBuffer = ptr::null_mut();
    if allocate_commands(device, &command_info, &mut command) != VK_SUCCESS {
        line("yuv_rgb", "FAIL", "allocate command buffer");
        return false;
    }
    let begin_info = VkCommandBufferBeginInfo {
        s_type: VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO,
        p_next: ptr::null(),
        flags: VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT,
        p_inheritance_info: ptr::null(),
    };
    if begin_command(command, &begin_info) != VK_SUCCESS {
        line("yuv_rgb", "FAIL", "begin command buffer");
        return false;
    }
    barrier(
        command,
        pipeline_barrier,
        VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT | VK_PIPELINE_STAGE_HOST_BIT,
        VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        VK_ACCESS_HOST_WRITE_BIT,
        VK_ACCESS_SHADER_READ_BIT,
    );
    bind_pipeline(command, VK_PIPELINE_BIND_POINT_COMPUTE, cleanup.pipeline);
    bind_sets(
        command,
        VK_PIPELINE_BIND_POINT_COMPUTE,
        cleanup.pipeline_layout,
        0,
        1,
        &descriptor_set,
        0,
        ptr::null(),
    );
    let params = PushParams {
        width,
        height,
        y_offset: offsets[0] as u32,
        v_offset: offsets[1] as u32,
        u_offset: offsets[2] as u32,
        y_stride: strides[0],
        chroma_stride: strides[1],
        reserved: 0,
    };
    push_constants(
        command,
        cleanup.pipeline_layout,
        VK_SHADER_STAGE_COMPUTE_BIT,
        0,
        core::mem::size_of::<PushParams>() as u32,
        &params as *const _ as *const c_void,
    );
    dispatch(command, width.div_ceil(8), height.div_ceil(8), 1);
    barrier(
        command,
        pipeline_barrier,
        VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        VK_PIPELINE_STAGE_HOST_BIT,
        VK_ACCESS_SHADER_WRITE_BIT,
        VK_ACCESS_HOST_READ_BIT,
    );
    if end_command(command) != VK_SUCCESS {
        line("yuv_rgb", "FAIL", "end command buffer");
        return false;
    }

    let fence_info = VkFenceCreateInfo {
        s_type: VK_STRUCTURE_TYPE_FENCE_CREATE_INFO,
        p_next: ptr::null(),
        flags: 0,
    };
    if create_fence(device, &fence_info, ptr::null(), &mut cleanup.fence) != VK_SUCCESS {
        line("yuv_rgb", "FAIL", "create fence");
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
    if queue_submit(queue, 1, &submit, cleanup.fence) != VK_SUCCESS {
        line("yuv_rgb", "FAIL", "queue submit");
        return false;
    }
    if wait_fences(device, 1, &cleanup.fence, VK_TRUE, 5_000_000_000) != VK_SUCCESS {
        line("yuv_rgb", "FAIL", "fence timeout");
        return false;
    }

    let mut mapped: *mut c_void = ptr::null_mut();
    if map_memory(
        device,
        cleanup.output_memory,
        0,
        output_bytes,
        0,
        &mut mapped,
    ) != VK_SUCCESS
        || mapped.is_null()
    {
        line("yuv_rgb", "FAIL", "map output buffer");
        return false;
    }
    let mut matched = 0usize;
    let mut samples = [0u32; 9];
    for (index, &(x, y)) in sample_points(width, height).iter().enumerate() {
        let offset = (y as usize * width as usize + x as usize) * 4;
        let value = (mapped as *const u8)
            .add(offset)
            .cast::<u32>()
            .read_unaligned();
        samples[index] = value;
        if value == expected[index] {
            matched += 1;
        }
    }
    unmap_memory(device, cleanup.output_memory);

    line(
        "yuv_rgb",
        "samples",
        format!("expected={expected:08x?}|gpu={samples:08x?}"),
    );
    let ok = matched == expected.len();
    line(
        "yuv_rgb",
        if ok { "PASS" } else { "FAIL" },
        format!(
            "pixels={matched}/9|frame={width}x{height}|offsets={offsets:?}|strides={strides:?}"
        ),
    );
    ok
}
