//! Persistent YV12 -> RGBA8 converter through imported AHB dma-bufs.
//!
//! Each fixed video slot owns one `Converter`. The input YV12 and output RGBA
//! dma-bufs are imported once, along with the shader, pipeline, descriptors,
//! command buffer and fence. A frame only resets the command pool/fence,
//! records one dispatch and waits for GPU completion before CPU verification.

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
    output_offset_words: u32,
    output_stride_words: u32,
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
type FnResetCommandPool = unsafe extern "C" fn(VkDevice, VkCommandPool, u32) -> VkResult;
type FnResetFences = unsafe extern "C" fn(VkDevice, u32, *const VkFence) -> VkResult;

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
    fn close(fd: c_int) -> c_int;
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
    let mapped = mmap(ptr::null_mut(), len, PROT_READ, MAP_SHARED, fd, 0);
    if mapped as isize == -1 {
        return Err(format!(
            "mmap YV12 dma-buf: {}",
            std::io::Error::last_os_error()
        ));
    }
    let base = mapped as *const u8;
    let mut expected = [0u32; 9];
    for (index, &(x, y)) in sample_points(width, height).iter().enumerate() {
        let y_pos = offsets[0] as usize + y as usize * strides[0] as usize + x as usize;
        let chroma_x = x as usize / 2;
        let chroma_y = y as usize / 2;
        let v_pos = offsets[1] as usize + chroma_y * strides[1] as usize + chroma_x;
        let u_pos = offsets[2] as usize + chroma_y * strides[2] as usize + chroma_x;
        expected[index] = yuv_to_rgba(*base.add(y_pos), *base.add(v_pos), *base.add(u_pos));
    }
    if munmap(mapped, len) != 0 {
        return Err(format!(
            "munmap YV12 dma-buf: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(expected)
}

unsafe fn barrier(
    command: VkCommandBuffer,
    cmd_pipeline_barrier: FnCmdPipelineBarrier,
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

struct FrameFns {
    begin_command: FnBeginCommandBuffer,
    end_command: FnEndCommandBuffer,
    bind_pipeline: FnCmdBindPipeline,
    bind_sets: FnCmdBindDescriptorSets,
    push_constants: FnCmdPushConstants,
    dispatch: FnCmdDispatch,
    pipeline_barrier: FnCmdPipelineBarrier,
    reset_pool: FnResetCommandPool,
    reset_fences: FnResetFences,
    queue_submit: FnQueueSubmit,
    wait_fences: FnWaitForFences,
}

pub(crate) struct Converter {
    device: VkDevice,
    queue: VkQueue,
    cleanup: Cleanup,
    descriptor_set: VkDescriptorSet,
    command: VkCommandBuffer,
    fns: FrameFns,
    input_fd: c_int,
    output_fd: c_int,
    width: u32,
    height: u32,
    offsets: [u64; 3],
    strides: [u32; 3],
    output_offset: u64,
    output_stride: u32,
    output_bytes: u64,
    slot: usize,
    frames: u64,
    passes: u64,
    verified: u64,
    failures: u64,
}

impl Drop for Converter {
    fn drop(&mut self) {
        unsafe {
            if self.output_fd >= 0 {
                close(self.output_fd);
            }
            if self.input_fd >= 0 {
                close(self.input_fd);
            }
        }
    }
}

unsafe fn require_symbol<T: Copy>(lib: *mut c_void, name: &str) -> Result<T, String> {
    symbol(lib, name).ok_or_else(|| format!("{name} missing"))
}

impl Converter {
    #[allow(clippy::too_many_arguments)]
    pub(crate) unsafe fn new(
        lib: *mut c_void,
        physical: VkPhysicalDevice,
        device: VkDevice,
        queue: VkQueue,
        family_index: u32,
        input_fd: c_int,
        output_fd: c_int,
        width: u32,
        height: u32,
        offsets: [u64; 3],
        strides: [u32; 3],
        output_offset: u64,
        output_stride: u32,
        slot: usize,
    ) -> Result<Self, String> {
        if width == 0 || height == 0 {
            return Err("zero-sized frame".into());
        }
        if offsets.iter().any(|&offset| offset > u32::MAX as u64)
            || strides.iter().any(|&stride| stride == 0)
        {
            return Err("plane geometry exceeds shader limits".into());
        }
        if strides[1] != strides[2] {
            return Err(format!(
                "chroma strides differ: {} vs {}",
                strides[1], strides[2]
            ));
        }
        if output_offset > u32::MAX as u64
            || output_stride < width.saturating_mul(4)
            || output_stride % 4 != 0
            || output_offset % 4 != 0
        {
            return Err(format!(
                "invalid RGBA layout: stride={output_stride} offset={output_offset}"
            ));
        }
        let input_bytes = input_size(width, height, offsets, strides)
            .ok_or_else(|| "invalid input size".to_string())?;
        let output_bytes = output_offset
            .checked_add(output_stride as u64 * height as u64)
            .ok_or_else(|| "output size overflow".to_string())?;
        let input_fd_size = lseek(input_fd, 0, 2);
        let output_fd_size = lseek(output_fd, 0, 2);
        if input_fd_size < 0 || (input_fd_size as u64) < input_bytes {
            return Err(format!(
                "input dma-buf too small: fd_size={input_fd_size} required={input_bytes}"
            ));
        }
        if output_fd_size < 0 || (output_fd_size as u64) < output_bytes {
            return Err(format!(
                "output dma-buf too small: fd_size={output_fd_size} required={output_bytes}"
            ));
        }

        let create_buffer: FnCreateBuffer = require_symbol(lib, "vkCreateBuffer")?;
        let destroy_buffer: FnDestroyBuffer = require_symbol(lib, "vkDestroyBuffer")?;
        let get_buffer_requirements: FnGetBufferMemoryRequirements =
            require_symbol(lib, "vkGetBufferMemoryRequirements")?;
        let allocate_memory: FnAllocateMemory = require_symbol(lib, "vkAllocateMemory")?;
        let free_memory: FnFreeMemory = require_symbol(lib, "vkFreeMemory")?;
        let bind_buffer: FnBindBufferMemory = require_symbol(lib, "vkBindBufferMemory")?;
        let get_external_props: FnGetExternalBufferProperties =
            require_symbol(lib, "vkGetPhysicalDeviceExternalBufferProperties")?;
        let get_memory_props: FnGetPhysicalDeviceMemoryProperties =
            require_symbol(lib, "vkGetPhysicalDeviceMemoryProperties")?;
        let create_shader: FnCreateShaderModule = require_symbol(lib, "vkCreateShaderModule")?;
        let destroy_shader: FnDestroyShaderModule = require_symbol(lib, "vkDestroyShaderModule")?;
        let create_set_layout: FnCreateDescriptorSetLayout =
            require_symbol(lib, "vkCreateDescriptorSetLayout")?;
        let destroy_set_layout: FnDestroyDescriptorSetLayout =
            require_symbol(lib, "vkDestroyDescriptorSetLayout")?;
        let create_pipeline_layout: FnCreatePipelineLayout =
            require_symbol(lib, "vkCreatePipelineLayout")?;
        let destroy_pipeline_layout: FnDestroyPipelineLayout =
            require_symbol(lib, "vkDestroyPipelineLayout")?;
        let create_pipeline: FnCreateComputePipelines =
            require_symbol(lib, "vkCreateComputePipelines")?;
        let destroy_pipeline: FnDestroyPipeline = require_symbol(lib, "vkDestroyPipeline")?;
        let create_descriptor_pool: FnCreateDescriptorPool =
            require_symbol(lib, "vkCreateDescriptorPool")?;
        let destroy_descriptor_pool: FnDestroyDescriptorPool =
            require_symbol(lib, "vkDestroyDescriptorPool")?;
        let allocate_sets: FnAllocateDescriptorSets =
            require_symbol(lib, "vkAllocateDescriptorSets")?;
        let update_sets: FnUpdateDescriptorSets = require_symbol(lib, "vkUpdateDescriptorSets")?;
        let create_command_pool: FnCreateCommandPool = require_symbol(lib, "vkCreateCommandPool")?;
        let destroy_command_pool: FnDestroyCommandPool =
            require_symbol(lib, "vkDestroyCommandPool")?;
        let allocate_commands: FnAllocateCommandBuffers =
            require_symbol(lib, "vkAllocateCommandBuffers")?;
        let create_fence: FnCreateFence = require_symbol(lib, "vkCreateFence")?;
        let destroy_fence: FnDestroyFence = require_symbol(lib, "vkDestroyFence")?;

        let fns = FrameFns {
            begin_command: require_symbol(lib, "vkBeginCommandBuffer")?,
            end_command: require_symbol(lib, "vkEndCommandBuffer")?,
            bind_pipeline: require_symbol(lib, "vkCmdBindPipeline")?,
            bind_sets: require_symbol(lib, "vkCmdBindDescriptorSets")?,
            push_constants: require_symbol(lib, "vkCmdPushConstants")?,
            dispatch: require_symbol(lib, "vkCmdDispatch")?,
            pipeline_barrier: require_symbol(lib, "vkCmdPipelineBarrier")?,
            reset_pool: require_symbol(lib, "vkResetCommandPool")?,
            reset_fences: require_symbol(lib, "vkResetFences")?,
            queue_submit: require_symbol(lib, "vkQueueSubmit")?,
            wait_fences: require_symbol(lib, "vkWaitForFences")?,
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
            "yuv_conv",
            "external_props",
            format!(
                "slot={slot}|features={:#x}|export={:#x}|compatible={:#x}",
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
            return Err("DMA_BUF storage-buffer import not advertised".into());
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
        if create_buffer(device, &input_info, ptr::null(), &mut cleanup.input_buffer) != VK_SUCCESS
        {
            return Err("create external YV12 buffer".into());
        }
        let output_info = VkBufferCreateInfo {
            s_type: VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO,
            p_next: &external as *const _ as *const c_void,
            flags: 0,
            size: output_bytes,
            usage,
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
            return Err("create external RGBA buffer".into());
        }

        let mut memory_properties: VkPhysicalDeviceMemoryProperties = std::mem::zeroed();
        get_memory_props(physical, &mut memory_properties);

        for (label, buffer, size, source_fd) in [
            ("input", cleanup.input_buffer, input_bytes, input_fd),
            ("output", cleanup.output_buffer, output_bytes, output_fd),
        ] {
            let mut req = VkMemoryRequirements::default();
            get_buffer_requirements(device, buffer, &mut req);
            let Some(memory_type) = choose_memory_type(&memory_properties, req.memory_type_bits)
            else {
                return Err(format!("no memory type for {label} buffer"));
            };
            let import_fd = dup(source_fd);
            if import_fd < 0 {
                return Err(format!(
                    "dup {label} dma-buf: {}",
                    std::io::Error::last_os_error()
                ));
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
                memory_type_index: memory_type,
            };
            let memory = if label == "input" {
                &mut cleanup.input_memory
            } else {
                &mut cleanup.output_memory
            };
            let result = allocate_memory(device, &alloc, ptr::null(), memory);
            close(import_fd);
            line(
                "yuv_conv",
                label,
                format!(
                    "slot={slot}|import={}|required={}|buffer={size}|type={memory_type}|fd_size={}",
                    api_result(result),
                    req.size,
                    lseek(source_fd, 0, 2)
                ),
            );
            if result != VK_SUCCESS {
                return Err(format!("{label} dma-buf import failed"));
            }
            if bind_buffer(device, buffer, *memory, 0) != VK_SUCCESS {
                return Err(format!("bind {label} buffer"));
            }
        }

        let shader_words = words(YUV_SPV);
        if shader_words.is_empty() || shader_words.len() * 4 != YUV_SPV.len() {
            return Err("invalid embedded shader".into());
        }
        let shader_info = VkShaderModuleCreateInfo {
            s_type: VK_STRUCTURE_TYPE_SHADER_MODULE_CREATE_INFO,
            p_next: ptr::null(),
            flags: 0,
            code_size: shader_words.len() * 4,
            p_code: shader_words.as_ptr(),
        };
        if create_shader(device, &shader_info, ptr::null(), &mut cleanup.shader) != VK_SUCCESS {
            return Err("create shader module".into());
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
            return Err("create descriptor set layout".into());
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
            return Err("create pipeline layout".into());
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
            return Err("create compute pipeline".into());
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
            return Err("create descriptor pool".into());
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
            return Err("allocate descriptor set".into());
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
            return Err("create command pool".into());
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
            return Err("allocate command buffer".into());
        }
        let fence_info = VkFenceCreateInfo {
            s_type: VK_STRUCTURE_TYPE_FENCE_CREATE_INFO,
            p_next: ptr::null(),
            flags: 0,
        };
        if create_fence(device, &fence_info, ptr::null(), &mut cleanup.fence) != VK_SUCCESS {
            return Err("create fence".into());
        }

        let owned_input = dup(input_fd);
        let owned_output = dup(output_fd);
        if owned_input < 0 || owned_output < 0 {
            if owned_input >= 0 {
                close(owned_input);
            }
            if owned_output >= 0 {
                close(owned_output);
            }
            return Err("dup converter fds".into());
        }

        line(
            "yuv_conv",
            "INIT",
            format!(
                "PASS|slot={slot}|frame={width}x{height}|input={input_bytes}|output={output_bytes}|stride={output_stride}|offset={output_offset}"
            ),
        );
        Ok(Self {
            device,
            queue,
            cleanup,
            descriptor_set,
            command,
            fns,
            input_fd: owned_input,
            output_fd: owned_output,
            width,
            height,
            offsets,
            strides,
            output_offset,
            output_stride,
            output_bytes,
            slot,
            frames: 0,
            passes: 0,
            verified: 0,
            failures: 0,
        })
    }

    pub(crate) unsafe fn convert(&mut self, verify: bool) -> bool {
        let expected = if verify {
            match cpu_expected(
                self.input_fd,
                self.width,
                self.height,
                self.offsets,
                self.strides,
            ) {
                Ok(value) => Some(value),
                Err(error) => {
                    self.failures = self.failures.saturating_add(1);
                    line(
                        "yuv_conv",
                        "FAIL",
                        format!("slot={}|cpu_reference|{error}", self.slot),
                    );
                    return false;
                }
            }
        } else {
            None
        };

        if (self.fns.reset_pool)(self.device, self.cleanup.command_pool, 0) != VK_SUCCESS {
            return self.fail("reset command pool");
        }
        let begin_info = VkCommandBufferBeginInfo {
            s_type: VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO,
            p_next: ptr::null(),
            flags: VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT,
            p_inheritance_info: ptr::null(),
        };
        if (self.fns.begin_command)(self.command, &begin_info) != VK_SUCCESS {
            return self.fail("begin command buffer");
        }
        barrier(
            self.command,
            self.fns.pipeline_barrier,
            VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT | VK_PIPELINE_STAGE_HOST_BIT,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_HOST_WRITE_BIT,
            VK_ACCESS_SHADER_READ_BIT,
        );
        (self.fns.bind_pipeline)(
            self.command,
            VK_PIPELINE_BIND_POINT_COMPUTE,
            self.cleanup.pipeline,
        );
        (self.fns.bind_sets)(
            self.command,
            VK_PIPELINE_BIND_POINT_COMPUTE,
            self.cleanup.pipeline_layout,
            0,
            1,
            &self.descriptor_set,
            0,
            ptr::null(),
        );
        let params = PushParams {
            width: self.width,
            height: self.height,
            y_offset: self.offsets[0] as u32,
            v_offset: self.offsets[1] as u32,
            u_offset: self.offsets[2] as u32,
            y_stride: self.strides[0],
            chroma_stride: self.strides[1],
            output_offset_words: (self.output_offset / 4) as u32,
            output_stride_words: self.output_stride / 4,
        };
        (self.fns.push_constants)(
            self.command,
            self.cleanup.pipeline_layout,
            VK_SHADER_STAGE_COMPUTE_BIT,
            0,
            core::mem::size_of::<PushParams>() as u32,
            &params as *const _ as *const c_void,
        );
        (self.fns.dispatch)(
            self.command,
            self.width.div_ceil(64),
            self.height.div_ceil(8),
            1,
        );
        barrier(
            self.command,
            self.fns.pipeline_barrier,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_PIPELINE_STAGE_HOST_BIT,
            VK_ACCESS_SHADER_WRITE_BIT,
            VK_ACCESS_HOST_READ_BIT,
        );
        if (self.fns.end_command)(self.command) != VK_SUCCESS {
            return self.fail("end command buffer");
        }
        if (self.fns.reset_fences)(self.device, 1, &self.cleanup.fence) != VK_SUCCESS {
            return self.fail("reset fence");
        }
        let submit = VkSubmitInfo {
            s_type: VK_STRUCTURE_TYPE_SUBMIT_INFO,
            p_next: ptr::null(),
            wait_semaphore_count: 0,
            p_wait_semaphores: ptr::null(),
            p_wait_dst_stage_mask: ptr::null(),
            command_buffer_count: 1,
            p_command_buffers: &self.command,
            signal_semaphore_count: 0,
            p_signal_semaphores: ptr::null(),
        };
        if (self.fns.queue_submit)(self.queue, 1, &submit, self.cleanup.fence) != VK_SUCCESS {
            return self.fail("queue submit");
        }
        if (self.fns.wait_fences)(self.device, 1, &self.cleanup.fence, VK_TRUE, 5_000_000_000)
            != VK_SUCCESS
        {
            return self.fail("fence timeout");
        }

        self.frames = self.frames.saturating_add(1);
        let Some(expected) = expected else {
            self.passes = self.passes.saturating_add(1);
            return true;
        };

        let len = match usize::try_from(self.output_bytes) {
            Ok(value) => value,
            Err(_) => return self.fail("output mapping too large"),
        };
        let mapped = mmap(
            ptr::null_mut(),
            len,
            PROT_READ,
            MAP_SHARED,
            self.output_fd,
            0,
        );
        if mapped as isize == -1 {
            return self.fail(&format!(
                "mmap output dma-buf: {}",
                std::io::Error::last_os_error()
            ));
        }
        let mut matched = 0usize;
        let mut samples = [0u32; 9];
        for (index, &(x, y)) in sample_points(self.width, self.height).iter().enumerate() {
            let offset = self.output_offset as usize
                + y as usize * self.output_stride as usize
                + x as usize * 4;
            let value = (mapped as *const u8)
                .add(offset)
                .cast::<u32>()
                .read_unaligned();
            samples[index] = value;
            if value == expected[index] {
                matched += 1;
            }
        }
        munmap(mapped, len);

        if matched == expected.len() {
            self.passes = self.passes.saturating_add(1);
            self.verified = self.verified.saturating_add(1);
        } else {
            self.failures = self.failures.saturating_add(1);
        }
        if self.frames == 1 || matched != expected.len() || self.frames % 30 == 0 {
            line(
                "yuv_conv",
                "frame",
                format!(
                    "slot={}|frame={}|pixels={matched}/9|expected={expected:08x?}|gpu={samples:08x?}|stride={}|offset={}",
                    self.slot, self.frames, self.output_stride, self.output_offset
                ),
            );
        }
        matched == expected.len()
    }

    fn fail(&mut self, reason: &str) -> bool {
        self.frames = self.frames.saturating_add(1);
        self.failures = self.failures.saturating_add(1);
        line(
            "yuv_conv",
            "FAIL",
            format!("slot={}|frame={}|{reason}", self.slot, self.frames),
        );
        false
    }

    pub(crate) fn summary(&self) -> String {
        format!(
            "slot={}|frames={}|passes={}|verified={}|failures={}",
            self.slot, self.frames, self.passes, self.verified, self.failures
        )
    }
}
