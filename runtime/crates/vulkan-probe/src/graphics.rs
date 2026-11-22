//! Minimal offscreen graphics pipeline probe.
//!
//! This deliberately avoids a window system: it renders one fullscreen triangle
//! into a 64x64 color image, copies the image to host-visible memory, waits on a
//! fence, and verifies the GPU wrote red pixels. It exercises the graphics
//! pipeline path (render pass, vertex/fragment shaders, draw, image copy) without
//! depending on SurfaceFlinger or a Wayland/X11 compositor.

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::ptr;

use crate::{
    api_result, line, symbol, VkDevice, VkDeviceSize, VkFlags, VkPhysicalDevice, VkQueue, VkResult,
    VK_SUCCESS,
};

type VkBuffer = *mut c_void;
type VkDeviceMemory = *mut c_void;
type VkImage = *mut c_void;
type VkImageView = *mut c_void;
type VkRenderPass = *mut c_void;
type VkFramebuffer = *mut c_void;
type VkShaderModule = *mut c_void;
type VkPipelineLayout = *mut c_void;
type VkPipeline = *mut c_void;
type VkCommandPool = *mut c_void;
type VkCommandBuffer = *mut c_void;
type VkFence = *mut c_void;

const IMAGE_WIDTH: u32 = 64;
const IMAGE_HEIGHT: u32 = 64;
const IMAGE_BYTES: VkDeviceSize = IMAGE_WIDTH as VkDeviceSize * IMAGE_HEIGHT as VkDeviceSize * 4;

const VK_STRUCTURE_TYPE_SUBMIT_INFO: u32 = 4;
const VK_STRUCTURE_TYPE_FENCE_CREATE_INFO: u32 = 8;
const VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO: u32 = 14;
const VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO: u32 = 15;
const VK_STRUCTURE_TYPE_SHADER_MODULE_CREATE_INFO: u32 = 16;
const VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO: u32 = 18;
const VK_STRUCTURE_TYPE_PIPELINE_VERTEX_INPUT_STATE_CREATE_INFO: u32 = 19;
const VK_STRUCTURE_TYPE_PIPELINE_INPUT_ASSEMBLY_STATE_CREATE_INFO: u32 = 20;
const VK_STRUCTURE_TYPE_PIPELINE_VIEWPORT_STATE_CREATE_INFO: u32 = 22;
const VK_STRUCTURE_TYPE_PIPELINE_RASTERIZATION_STATE_CREATE_INFO: u32 = 23;
const VK_STRUCTURE_TYPE_PIPELINE_MULTISAMPLE_STATE_CREATE_INFO: u32 = 24;
const VK_STRUCTURE_TYPE_PIPELINE_COLOR_BLEND_STATE_CREATE_INFO: u32 = 26;
const VK_STRUCTURE_TYPE_GRAPHICS_PIPELINE_CREATE_INFO: u32 = 28;
const VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO: u32 = 30;
const VK_STRUCTURE_TYPE_FRAMEBUFFER_CREATE_INFO: u32 = 37;
const VK_STRUCTURE_TYPE_RENDER_PASS_CREATE_INFO: u32 = 38;
const VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO: u32 = 39;
const VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO: u32 = 40;
const VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO: u32 = 42;
const VK_STRUCTURE_TYPE_RENDER_PASS_BEGIN_INFO: u32 = 43;
const VK_STRUCTURE_TYPE_MEMORY_BARRIER: u32 = 46;

const VK_IMAGE_TYPE_2D: u32 = 1;
const VK_FORMAT_R8G8B8A8_UNORM: u32 = 37;
const VK_FORMAT_R32G32B32A32_SFLOAT: u32 = 109;
const VK_IMAGE_TILING_OPTIMAL: u32 = 0;
const VK_IMAGE_USAGE_TRANSFER_SRC_BIT: u32 = 1 << 0;
const VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT: u32 = 1 << 4;
const VK_BUFFER_USAGE_TRANSFER_DST_BIT: u32 = 1 << 1;
const VK_BUFFER_USAGE_VERTEX_BUFFER_BIT: u32 = 1 << 7;
const VK_IMAGE_LAYOUT_UNDEFINED: u32 = 0;
const VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL: u32 = 6;
const VK_IMAGE_ASPECT_COLOR_BIT: u32 = 1;
const VK_IMAGE_VIEW_TYPE_2D: u32 = 1;
const VK_SAMPLE_COUNT_1_BIT: u32 = 1;
const VK_ATTACHMENT_LOAD_OP_CLEAR: u32 = 1;
const VK_ATTACHMENT_STORE_OP_STORE: u32 = 0;
const VK_ATTACHMENT_LOAD_OP_DONT_CARE: u32 = 2;
const VK_ATTACHMENT_STORE_OP_DONT_CARE: u32 = 2;
const VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL_VALUE: u32 = 2;
const VK_SUBPASS_CONTENTS_INLINE: u32 = 0;
const VK_PIPELINE_BIND_POINT_GRAPHICS: u32 = 0;
const VK_PRIMITIVE_TOPOLOGY_TRIANGLE_LIST: u32 = 3;
const VK_POLYGON_MODE_FILL: u32 = 0;
const VK_CULL_MODE_NONE: u32 = 0;
const VK_FRONT_FACE_COUNTER_CLOCKWISE: u32 = 1;
const VK_COLOR_COMPONENT_R_BIT: u32 = 1 << 0;
const VK_COLOR_COMPONENT_G_BIT: u32 = 1 << 1;
const VK_COLOR_COMPONENT_B_BIT: u32 = 1 << 2;
const VK_COLOR_COMPONENT_A_BIT: u32 = 1 << 3;
const VK_SHADER_STAGE_VERTEX_BIT: u32 = 1 << 0;
const VK_SHADER_STAGE_FRAGMENT_BIT: u32 = 1 << 4;
const VK_COMMAND_BUFFER_LEVEL_PRIMARY: u32 = 0;
const VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT: u32 = 1;
const VK_SUBPASS_EXTERNAL: u32 = u32::MAX;
const VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT: u32 = 1 << 10;
const VK_PIPELINE_STAGE_TRANSFER_BIT: u32 = 1 << 12;
const VK_PIPELINE_STAGE_HOST_BIT: u32 = 1 << 14;
const VK_ACCESS_TRANSFER_READ_BIT: u32 = 1 << 9;
const VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT: u32 = 1 << 10;
const VK_ACCESS_TRANSFER_WRITE_BIT: u32 = 1 << 11;
const VK_ACCESS_HOST_READ_BIT: u32 = 1 << 9;
const VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT: u32 = 1 << 0;
const VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT: u32 = 1 << 1;
const VK_MEMORY_PROPERTY_HOST_COHERENT_BIT: u32 = 1 << 2;
const VK_TRUE: u32 = 1;

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
#[derive(Clone, Copy, Default)]
struct VkExtent2D {
    width: u32,
    height: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct VkOffset2D {
    x: i32,
    y: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct VkRect2D {
    offset: VkOffset2D,
    extent: VkExtent2D,
}

#[repr(C)]
struct VkImageCreateInfo {
    s_type: u32,
    p_next: *const c_void,
    flags: VkFlags,
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
#[derive(Clone, Copy, Default)]
struct VkComponentMapping {
    r: u32,
    g: u32,
    b: u32,
    a: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct VkImageSubresourceRange {
    aspect_mask: u32,
    base_mip_level: u32,
    level_count: u32,
    base_array_layer: u32,
    layer_count: u32,
}

#[repr(C)]
struct VkImageViewCreateInfo {
    s_type: u32,
    p_next: *const c_void,
    flags: VkFlags,
    image: VkImage,
    view_type: u32,
    format: u32,
    components: VkComponentMapping,
    subresource_range: VkImageSubresourceRange,
}

#[repr(C)]
struct VkShaderModuleCreateInfo {
    s_type: u32,
    p_next: *const c_void,
    flags: VkFlags,
    code_size: usize,
    p_code: *const u32,
}

#[repr(C)]
struct VkPipelineShaderStageCreateInfo {
    s_type: u32,
    p_next: *const c_void,
    flags: VkFlags,
    stage: u32,
    module: VkShaderModule,
    p_name: *const c_char,
    p_specialization_info: *const c_void,
}

#[repr(C)]
struct VkVertexInputBindingDescription {
    binding: u32,
    stride: u32,
    input_rate: u32,
}

#[repr(C)]
struct VkVertexInputAttributeDescription {
    location: u32,
    binding: u32,
    format: u32,
    offset: u32,
}

#[repr(C)]
struct VkPipelineVertexInputStateCreateInfo {
    s_type: u32,
    p_next: *const c_void,
    flags: VkFlags,
    vertex_binding_description_count: u32,
    p_vertex_binding_descriptions: *const VkVertexInputBindingDescription,
    vertex_attribute_description_count: u32,
    p_vertex_attribute_descriptions: *const VkVertexInputAttributeDescription,
}

#[repr(C)]
struct VkPipelineInputAssemblyStateCreateInfo {
    s_type: u32,
    p_next: *const c_void,
    flags: VkFlags,
    topology: u32,
    primitive_restart_enable: u32,
}

#[repr(C)]
struct VkViewport {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    min_depth: f32,
    max_depth: f32,
}

#[repr(C)]
struct VkPipelineViewportStateCreateInfo {
    s_type: u32,
    p_next: *const c_void,
    flags: VkFlags,
    viewport_count: u32,
    p_viewports: *const VkViewport,
    scissor_count: u32,
    p_scissors: *const VkRect2D,
}

#[repr(C)]
struct VkPipelineRasterizationStateCreateInfo {
    s_type: u32,
    p_next: *const c_void,
    flags: VkFlags,
    depth_clamp_enable: u32,
    rasterizer_discard_enable: u32,
    polygon_mode: u32,
    cull_mode: u32,
    front_face: u32,
    depth_bias_enable: u32,
    depth_bias_constant_factor: f32,
    depth_bias_clamp: f32,
    depth_bias_slope_factor: f32,
    line_width: f32,
}

#[repr(C)]
struct VkPipelineMultisampleStateCreateInfo {
    s_type: u32,
    p_next: *const c_void,
    flags: VkFlags,
    rasterization_samples: u32,
    sample_shading_enable: u32,
    min_sample_shading: f32,
    p_sample_mask: *const u32,
    alpha_to_coverage_enable: u32,
    alpha_to_one_enable: u32,
}

#[repr(C)]
struct VkPipelineColorBlendAttachmentState {
    blend_enable: u32,
    src_color_blend_factor: u32,
    dst_color_blend_factor: u32,
    color_blend_op: u32,
    src_alpha_blend_factor: u32,
    dst_alpha_blend_factor: u32,
    alpha_blend_op: u32,
    color_write_mask: u32,
}

#[repr(C)]
struct VkPipelineColorBlendStateCreateInfo {
    s_type: u32,
    p_next: *const c_void,
    flags: VkFlags,
    logic_op_enable: u32,
    logic_op: u32,
    attachment_count: u32,
    p_attachments: *const VkPipelineColorBlendAttachmentState,
    blend_constants: [f32; 4],
}

#[repr(C)]
struct VkPipelineLayoutCreateInfo {
    s_type: u32,
    p_next: *const c_void,
    flags: VkFlags,
    set_layout_count: u32,
    p_set_layouts: *const *mut c_void,
    push_constant_range_count: u32,
    p_push_constant_ranges: *const c_void,
}

#[repr(C)]
struct VkGraphicsPipelineCreateInfo {
    s_type: u32,
    p_next: *const c_void,
    flags: VkFlags,
    stage_count: u32,
    p_stages: *const VkPipelineShaderStageCreateInfo,
    p_vertex_input_state: *const VkPipelineVertexInputStateCreateInfo,
    p_input_assembly_state: *const VkPipelineInputAssemblyStateCreateInfo,
    p_tessellation_state: *const c_void,
    p_viewport_state: *const VkPipelineViewportStateCreateInfo,
    p_rasterization_state: *const VkPipelineRasterizationStateCreateInfo,
    p_multisample_state: *const VkPipelineMultisampleStateCreateInfo,
    p_depth_stencil_state: *const c_void,
    p_color_blend_state: *const VkPipelineColorBlendStateCreateInfo,
    p_dynamic_state: *const c_void,
    layout: VkPipelineLayout,
    render_pass: VkRenderPass,
    subpass: u32,
    base_pipeline_handle: VkPipeline,
    base_pipeline_index: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct VkAttachmentDescription {
    flags: u32,
    format: u32,
    samples: u32,
    load_op: u32,
    store_op: u32,
    stencil_load_op: u32,
    stencil_store_op: u32,
    initial_layout: u32,
    final_layout: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct VkAttachmentReference {
    attachment: u32,
    layout: u32,
}

#[repr(C)]
struct VkSubpassDescription {
    flags: u32,
    pipeline_bind_point: u32,
    input_attachment_count: u32,
    p_input_attachments: *const VkAttachmentReference,
    color_attachment_count: u32,
    p_color_attachments: *const VkAttachmentReference,
    p_resolve_attachments: *const VkAttachmentReference,
    p_depth_stencil_attachment: *const VkAttachmentReference,
    preserve_attachment_count: u32,
    p_preserve_attachments: *const u32,
}

#[repr(C)]
struct VkSubpassDependency {
    src_subpass: u32,
    dst_subpass: u32,
    src_stage_mask: u32,
    dst_stage_mask: u32,
    src_access_mask: u32,
    dst_access_mask: u32,
    dependency_flags: u32,
}

#[repr(C)]
struct VkRenderPassCreateInfo {
    s_type: u32,
    p_next: *const c_void,
    flags: VkFlags,
    attachment_count: u32,
    p_attachments: *const VkAttachmentDescription,
    subpass_count: u32,
    p_subpasses: *const VkSubpassDescription,
    dependency_count: u32,
    p_dependencies: *const VkSubpassDependency,
}

#[repr(C)]
struct VkFramebufferCreateInfo {
    s_type: u32,
    p_next: *const c_void,
    flags: VkFlags,
    render_pass: VkRenderPass,
    attachment_count: u32,
    p_attachments: *const VkImageView,
    width: u32,
    height: u32,
    layers: u32,
}

#[repr(C)]
struct VkClearValue {
    color: [f32; 4],
}

#[repr(C)]
struct VkRenderPassBeginInfo {
    s_type: u32,
    p_next: *const c_void,
    render_pass: VkRenderPass,
    framebuffer: VkFramebuffer,
    render_area: VkRect2D,
    clear_value_count: u32,
    p_clear_values: *const VkClearValue,
}

#[repr(C)]
struct VkImageSubresourceLayers {
    aspect_mask: u32,
    mip_level: u32,
    base_array_layer: u32,
    layer_count: u32,
}

#[repr(C)]
struct VkBufferImageCopy {
    buffer_offset: VkDeviceSize,
    buffer_row_length: u32,
    buffer_image_height: u32,
    image_subresource: VkImageSubresourceLayers,
    image_offset: VkOffset3D,
    image_extent: VkExtent3D,
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
struct VkMemoryRequirements {
    size: VkDeviceSize,
    alignment: VkDeviceSize,
    memory_type_bits: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct VkMemoryType {
    property_flags: u32,
    heap_index: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct VkMemoryHeap {
    size: VkDeviceSize,
    flags: u32,
}

#[repr(C)]
struct VkPhysicalDeviceMemoryProperties {
    memory_type_count: u32,
    memory_types: [VkMemoryType; 32],
    memory_heap_count: u32,
    memory_heaps: [VkMemoryHeap; 16],
}

#[repr(C)]
struct VkMemoryAllocateInfo {
    s_type: u32,
    p_next: *const c_void,
    allocation_size: VkDeviceSize,
    memory_type_index: u32,
}

#[repr(C)]
struct VkBufferCreateInfo {
    s_type: u32,
    p_next: *const c_void,
    flags: VkFlags,
    size: VkDeviceSize,
    usage: u32,
    sharing_mode: u32,
    queue_family_index_count: u32,
    p_queue_family_indices: *const u32,
}

#[repr(C)]
struct VkCommandPoolCreateInfo {
    s_type: u32,
    p_next: *const c_void,
    flags: VkFlags,
    queue_family_index: u32,
}

#[repr(C)]
struct VkCommandBufferAllocateInfo {
    s_type: u32,
    p_next: *const c_void,
    command_pool: VkCommandPool,
    level: u32,
    command_buffer_count: u32,
}

#[repr(C)]
struct VkCommandBufferBeginInfo {
    s_type: u32,
    p_next: *const c_void,
    flags: VkFlags,
    p_inheritance_info: *const c_void,
}

#[repr(C)]
struct VkSubmitInfo {
    s_type: u32,
    p_next: *const c_void,
    wait_semaphore_count: u32,
    p_wait_semaphores: *const c_void,
    p_wait_dst_stage_mask: *const u32,
    command_buffer_count: u32,
    p_command_buffers: *const VkCommandBuffer,
    signal_semaphore_count: u32,
    p_signal_semaphores: *const c_void,
}

#[repr(C)]
struct VkFenceCreateInfo {
    s_type: u32,
    p_next: *const c_void,
    flags: VkFlags,
}

type FnGetMemoryProperties =
    unsafe extern "C" fn(VkPhysicalDevice, *mut VkPhysicalDeviceMemoryProperties);
type FnCreateBuffer = unsafe extern "C" fn(
    VkDevice,
    *const VkBufferCreateInfo,
    *const c_void,
    *mut VkBuffer,
) -> VkResult;
type FnDestroyBuffer = unsafe extern "C" fn(VkDevice, VkBuffer, *const c_void);
type FnGetBufferRequirements = unsafe extern "C" fn(VkDevice, VkBuffer, *mut VkMemoryRequirements);
type FnCreateImage = unsafe extern "C" fn(
    VkDevice,
    *const VkImageCreateInfo,
    *const c_void,
    *mut VkImage,
) -> VkResult;
type FnDestroyImage = unsafe extern "C" fn(VkDevice, VkImage, *const c_void);
type FnGetImageRequirements = unsafe extern "C" fn(VkDevice, VkImage, *mut VkMemoryRequirements);
type FnBindImageMemory =
    unsafe extern "C" fn(VkDevice, VkImage, VkDeviceMemory, VkDeviceSize) -> VkResult;
type FnCreateImageView = unsafe extern "C" fn(
    VkDevice,
    *const VkImageViewCreateInfo,
    *const c_void,
    *mut VkImageView,
) -> VkResult;
type FnDestroyImageView = unsafe extern "C" fn(VkDevice, VkImageView, *const c_void);
type FnAllocateMemory = unsafe extern "C" fn(
    VkDevice,
    *const VkMemoryAllocateInfo,
    *const c_void,
    *mut VkDeviceMemory,
) -> VkResult;
type FnFreeMemory = unsafe extern "C" fn(VkDevice, VkDeviceMemory, *const c_void);
type FnBindBufferMemory =
    unsafe extern "C" fn(VkDevice, VkBuffer, VkDeviceMemory, VkDeviceSize) -> VkResult;
type FnMapMemory = unsafe extern "C" fn(
    VkDevice,
    VkDeviceMemory,
    VkDeviceSize,
    VkDeviceSize,
    u32,
    *mut *mut c_void,
) -> VkResult;
type FnUnmapMemory = unsafe extern "C" fn(VkDevice, VkDeviceMemory);
type FnCreateShaderModule = unsafe extern "C" fn(
    VkDevice,
    *const VkShaderModuleCreateInfo,
    *const c_void,
    *mut VkShaderModule,
) -> VkResult;
type FnDestroyShaderModule = unsafe extern "C" fn(VkDevice, VkShaderModule, *const c_void);
type FnCreatePipelineLayout = unsafe extern "C" fn(
    VkDevice,
    *const VkPipelineLayoutCreateInfo,
    *const c_void,
    *mut VkPipelineLayout,
) -> VkResult;
type FnDestroyPipelineLayout = unsafe extern "C" fn(VkDevice, VkPipelineLayout, *const c_void);
type FnCreateRenderPass = unsafe extern "C" fn(
    VkDevice,
    *const VkRenderPassCreateInfo,
    *const c_void,
    *mut VkRenderPass,
) -> VkResult;
type FnDestroyRenderPass = unsafe extern "C" fn(VkDevice, VkRenderPass, *const c_void);
type FnCreateFramebuffer = unsafe extern "C" fn(
    VkDevice,
    *const VkFramebufferCreateInfo,
    *const c_void,
    *mut VkFramebuffer,
) -> VkResult;
type FnDestroyFramebuffer = unsafe extern "C" fn(VkDevice, VkFramebuffer, *const c_void);
type FnCreateGraphicsPipelines = unsafe extern "C" fn(
    VkDevice,
    *mut c_void,
    u32,
    *const VkGraphicsPipelineCreateInfo,
    *const c_void,
    *mut VkPipeline,
) -> VkResult;
type FnDestroyPipeline = unsafe extern "C" fn(VkDevice, VkPipeline, *const c_void);
type FnCreateCommandPool = unsafe extern "C" fn(
    VkDevice,
    *const VkCommandPoolCreateInfo,
    *const c_void,
    *mut VkCommandPool,
) -> VkResult;
type FnDestroyCommandPool = unsafe extern "C" fn(VkDevice, VkCommandPool, *const c_void);
type FnAllocateCommandBuffers = unsafe extern "C" fn(
    VkDevice,
    *const VkCommandBufferAllocateInfo,
    *mut VkCommandBuffer,
) -> VkResult;
type FnBeginCommandBuffer =
    unsafe extern "C" fn(VkCommandBuffer, *const VkCommandBufferBeginInfo) -> VkResult;
type FnEndCommandBuffer = unsafe extern "C" fn(VkCommandBuffer) -> VkResult;
type FnCmdBeginRenderPass =
    unsafe extern "C" fn(VkCommandBuffer, *const VkRenderPassBeginInfo, u32);
type FnCmdEndRenderPass = unsafe extern "C" fn(VkCommandBuffer);
type FnCmdBindPipeline = unsafe extern "C" fn(VkCommandBuffer, u32, VkPipeline);
type FnCmdBindVertexBuffers =
    unsafe extern "C" fn(VkCommandBuffer, u32, u32, *const VkBuffer, *const VkDeviceSize);
type FnCmdDraw = unsafe extern "C" fn(VkCommandBuffer, u32, u32, u32, u32);
type FnCmdCopyImageToBuffer =
    unsafe extern "C" fn(VkCommandBuffer, VkImage, u32, VkBuffer, u32, *const VkBufferImageCopy);
type FnCmdPipelineBarrier = unsafe extern "C" fn(
    VkCommandBuffer,
    u32,
    u32,
    u32,
    u32,
    *const VkMemoryBarrier,
    u32,
    *const c_void,
    u32,
    *const c_void,
);
type FnCreateFence = unsafe extern "C" fn(
    VkDevice,
    *const VkFenceCreateInfo,
    *const c_void,
    *mut VkFence,
) -> VkResult;
type FnDestroyFence = unsafe extern "C" fn(VkDevice, VkFence, *const c_void);
type FnWaitForFences = unsafe extern "C" fn(VkDevice, u32, *const VkFence, u32, u64) -> VkResult;
type FnQueueSubmit = unsafe extern "C" fn(VkQueue, u32, *const VkSubmitInfo, VkFence) -> VkResult;

const TRI_VERT_SPV: &[u8] = include_bytes!("../shaders/tri.vert.spv");
const TRI_FRAG_SPV: &[u8] = include_bytes!("../shaders/tri.frag.spv");

fn words(bytes: &[u8]) -> Vec<u32> {
    bytes
        .chunks_exact(4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect()
}

fn choose_type(props: &VkPhysicalDeviceMemoryProperties, bits: u32, wanted: u32) -> Option<u32> {
    for i in 0..props.memory_type_count.min(32) {
        let t = props.memory_types[i as usize];
        if (bits & (1 << i)) != 0 && (t.property_flags & wanted) == wanted {
            return Some(i);
        }
    }
    None
}

pub(crate) unsafe fn run_graphics(
    lib: *mut c_void,
    physical: VkPhysicalDevice,
    device: VkDevice,
    queue: VkQueue,
    queue_family_index: u32,
) -> bool {
    let get_memory_props: FnGetMemoryProperties =
        match symbol(lib, "vkGetPhysicalDeviceMemoryProperties") {
            Some(f) => f,
            None => {
                line(
                    "graphics",
                    "FAIL",
                    "vkGetPhysicalDeviceMemoryProperties missing",
                );
                return false;
            }
        };
    let create_buffer: FnCreateBuffer = match symbol(lib, "vkCreateBuffer") {
        Some(f) => f,
        None => {
            line("graphics", "FAIL", "vkCreateBuffer missing");
            return false;
        }
    };
    let destroy_buffer: FnDestroyBuffer = match symbol(lib, "vkDestroyBuffer") {
        Some(f) => f,
        None => {
            line("graphics", "FAIL", "vkDestroyBuffer missing");
            return false;
        }
    };
    let get_buffer_requirements: FnGetBufferRequirements =
        match symbol(lib, "vkGetBufferMemoryRequirements") {
            Some(f) => f,
            None => {
                line("graphics", "FAIL", "vkGetBufferMemoryRequirements missing");
                return false;
            }
        };
    let create_image: FnCreateImage = match symbol(lib, "vkCreateImage") {
        Some(f) => f,
        None => {
            line("graphics", "FAIL", "vkCreateImage missing");
            return false;
        }
    };
    let destroy_image: FnDestroyImage = match symbol(lib, "vkDestroyImage") {
        Some(f) => f,
        None => {
            line("graphics", "FAIL", "vkDestroyImage missing");
            return false;
        }
    };
    let get_image_requirements: FnGetImageRequirements =
        match symbol(lib, "vkGetImageMemoryRequirements") {
            Some(f) => f,
            None => {
                line("graphics", "FAIL", "vkGetImageMemoryRequirements missing");
                return false;
            }
        };
    let bind_image_memory: FnBindImageMemory = match symbol(lib, "vkBindImageMemory") {
        Some(f) => f,
        None => {
            line("graphics", "FAIL", "vkBindImageMemory missing");
            return false;
        }
    };
    let create_image_view: FnCreateImageView = match symbol(lib, "vkCreateImageView") {
        Some(f) => f,
        None => {
            line("graphics", "FAIL", "vkCreateImageView missing");
            return false;
        }
    };
    let destroy_image_view: FnDestroyImageView = match symbol(lib, "vkDestroyImageView") {
        Some(f) => f,
        None => {
            line("graphics", "FAIL", "vkDestroyImageView missing");
            return false;
        }
    };
    let allocate_memory: FnAllocateMemory = match symbol(lib, "vkAllocateMemory") {
        Some(f) => f,
        None => {
            line("graphics", "FAIL", "vkAllocateMemory missing");
            return false;
        }
    };
    let free_memory: FnFreeMemory = match symbol(lib, "vkFreeMemory") {
        Some(f) => f,
        None => {
            line("graphics", "FAIL", "vkFreeMemory missing");
            return false;
        }
    };
    let bind_buffer_memory: FnBindBufferMemory = match symbol(lib, "vkBindBufferMemory") {
        Some(f) => f,
        None => {
            line("graphics", "FAIL", "vkBindBufferMemory missing");
            return false;
        }
    };
    let map_memory: FnMapMemory = match symbol(lib, "vkMapMemory") {
        Some(f) => f,
        None => {
            line("graphics", "FAIL", "vkMapMemory missing");
            return false;
        }
    };
    let unmap_memory: FnUnmapMemory = match symbol(lib, "vkUnmapMemory") {
        Some(f) => f,
        None => {
            line("graphics", "FAIL", "vkUnmapMemory missing");
            return false;
        }
    };
    let create_shader: FnCreateShaderModule = match symbol(lib, "vkCreateShaderModule") {
        Some(f) => f,
        None => {
            line("graphics", "FAIL", "vkCreateShaderModule missing");
            return false;
        }
    };
    let destroy_shader: FnDestroyShaderModule = match symbol(lib, "vkDestroyShaderModule") {
        Some(f) => f,
        None => {
            line("graphics", "FAIL", "vkDestroyShaderModule missing");
            return false;
        }
    };
    let create_layout: FnCreatePipelineLayout = match symbol(lib, "vkCreatePipelineLayout") {
        Some(f) => f,
        None => {
            line("graphics", "FAIL", "vkCreatePipelineLayout missing");
            return false;
        }
    };
    let destroy_layout: FnDestroyPipelineLayout = match symbol(lib, "vkDestroyPipelineLayout") {
        Some(f) => f,
        None => {
            line("graphics", "FAIL", "vkDestroyPipelineLayout missing");
            return false;
        }
    };
    let create_render_pass: FnCreateRenderPass = match symbol(lib, "vkCreateRenderPass") {
        Some(f) => f,
        None => {
            line("graphics", "FAIL", "vkCreateRenderPass missing");
            return false;
        }
    };
    let destroy_render_pass: FnDestroyRenderPass = match symbol(lib, "vkDestroyRenderPass") {
        Some(f) => f,
        None => {
            line("graphics", "FAIL", "vkDestroyRenderPass missing");
            return false;
        }
    };
    let create_framebuffer: FnCreateFramebuffer = match symbol(lib, "vkCreateFramebuffer") {
        Some(f) => f,
        None => {
            line("graphics", "FAIL", "vkCreateFramebuffer missing");
            return false;
        }
    };
    let destroy_framebuffer: FnDestroyFramebuffer = match symbol(lib, "vkDestroyFramebuffer") {
        Some(f) => f,
        None => {
            line("graphics", "FAIL", "vkDestroyFramebuffer missing");
            return false;
        }
    };
    let create_pipelines: FnCreateGraphicsPipelines = match symbol(lib, "vkCreateGraphicsPipelines")
    {
        Some(f) => f,
        None => {
            line("graphics", "FAIL", "vkCreateGraphicsPipelines missing");
            return false;
        }
    };
    let destroy_pipeline: FnDestroyPipeline = match symbol(lib, "vkDestroyPipeline") {
        Some(f) => f,
        None => {
            line("graphics", "FAIL", "vkDestroyPipeline missing");
            return false;
        }
    };
    let create_pool: FnCreateCommandPool = match symbol(lib, "vkCreateCommandPool") {
        Some(f) => f,
        None => {
            line("graphics", "FAIL", "vkCreateCommandPool missing");
            return false;
        }
    };
    let destroy_pool: FnDestroyCommandPool = match symbol(lib, "vkDestroyCommandPool") {
        Some(f) => f,
        None => {
            line("graphics", "FAIL", "vkDestroyCommandPool missing");
            return false;
        }
    };
    let allocate_commands: FnAllocateCommandBuffers = match symbol(lib, "vkAllocateCommandBuffers")
    {
        Some(f) => f,
        None => {
            line("graphics", "FAIL", "vkAllocateCommandBuffers missing");
            return false;
        }
    };
    let begin_command: FnBeginCommandBuffer = match symbol(lib, "vkBeginCommandBuffer") {
        Some(f) => f,
        None => {
            line("graphics", "FAIL", "vkBeginCommandBuffer missing");
            return false;
        }
    };
    let end_command: FnEndCommandBuffer = match symbol(lib, "vkEndCommandBuffer") {
        Some(f) => f,
        None => {
            line("graphics", "FAIL", "vkEndCommandBuffer missing");
            return false;
        }
    };
    let cmd_begin_render_pass: FnCmdBeginRenderPass = match symbol(lib, "vkCmdBeginRenderPass") {
        Some(f) => f,
        None => {
            line("graphics", "FAIL", "vkCmdBeginRenderPass missing");
            return false;
        }
    };
    let cmd_end_render_pass: FnCmdEndRenderPass = match symbol(lib, "vkCmdEndRenderPass") {
        Some(f) => f,
        None => {
            line("graphics", "FAIL", "vkCmdEndRenderPass missing");
            return false;
        }
    };
    let cmd_bind_pipeline: FnCmdBindPipeline = match symbol(lib, "vkCmdBindPipeline") {
        Some(f) => f,
        None => {
            line("graphics", "FAIL", "vkCmdBindPipeline missing");
            return false;
        }
    };
    let cmd_bind_vertex_buffers: FnCmdBindVertexBuffers =
        match symbol(lib, "vkCmdBindVertexBuffers") {
            Some(f) => f,
            None => {
                line("graphics", "FAIL", "vkCmdBindVertexBuffers missing");
                return false;
            }
        };
    let cmd_draw: FnCmdDraw = match symbol(lib, "vkCmdDraw") {
        Some(f) => f,
        None => {
            line("graphics", "FAIL", "vkCmdDraw missing");
            return false;
        }
    };
    let cmd_copy: FnCmdCopyImageToBuffer = match symbol(lib, "vkCmdCopyImageToBuffer") {
        Some(f) => f,
        None => {
            line("graphics", "FAIL", "vkCmdCopyImageToBuffer missing");
            return false;
        }
    };
    let cmd_barrier: FnCmdPipelineBarrier = match symbol(lib, "vkCmdPipelineBarrier") {
        Some(f) => f,
        None => {
            line("graphics", "FAIL", "vkCmdPipelineBarrier missing");
            return false;
        }
    };
    let create_fence: FnCreateFence = match symbol(lib, "vkCreateFence") {
        Some(f) => f,
        None => {
            line("graphics", "FAIL", "vkCreateFence missing");
            return false;
        }
    };
    let destroy_fence: FnDestroyFence = match symbol(lib, "vkDestroyFence") {
        Some(f) => f,
        None => {
            line("graphics", "FAIL", "vkDestroyFence missing");
            return false;
        }
    };
    let wait_fences: FnWaitForFences = match symbol(lib, "vkWaitForFences") {
        Some(f) => f,
        None => {
            line("graphics", "FAIL", "vkWaitForFences missing");
            return false;
        }
    };
    let queue_submit: FnQueueSubmit = match symbol(lib, "vkQueueSubmit") {
        Some(f) => f,
        None => {
            line("graphics", "FAIL", "vkQueueSubmit missing");
            return false;
        }
    };

    let mut mem_props: VkPhysicalDeviceMemoryProperties = std::mem::zeroed();
    get_memory_props(physical, &mut mem_props);

    let buffer_info = VkBufferCreateInfo {
        s_type: 12,
        p_next: ptr::null(),
        flags: 0,
        size: IMAGE_BYTES,
        usage: VK_BUFFER_USAGE_TRANSFER_DST_BIT | VK_BUFFER_USAGE_VERTEX_BUFFER_BIT,
        sharing_mode: 0,
        queue_family_index_count: 0,
        p_queue_family_indices: ptr::null(),
    };
    let mut readback: VkBuffer = ptr::null_mut();
    let r = create_buffer(device, &buffer_info, ptr::null(), &mut readback);
    if r != VK_SUCCESS {
        line(
            "graphics",
            "FAIL",
            format!("readback buffer {}", api_result(r)),
        );
        return false;
    }
    let mut readback_req = VkMemoryRequirements::default();
    get_buffer_requirements(device, readback, &mut readback_req);
    let Some(readback_type) = choose_type(
        &mem_props,
        readback_req.memory_type_bits,
        VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT,
    ) else {
        line("graphics", "FAIL", "no host-visible readback memory type");
        destroy_buffer(device, readback, ptr::null());
        return false;
    };
    let readback_alloc = VkMemoryAllocateInfo {
        s_type: 5,
        p_next: ptr::null(),
        allocation_size: readback_req.size,
        memory_type_index: readback_type,
    };
    let mut readback_memory: VkDeviceMemory = ptr::null_mut();
    let r = allocate_memory(device, &readback_alloc, ptr::null(), &mut readback_memory);
    if r != VK_SUCCESS {
        line(
            "graphics",
            "FAIL",
            format!("readback memory {}", api_result(r)),
        );
        destroy_buffer(device, readback, ptr::null());
        return false;
    }
    let r = bind_buffer_memory(device, readback, readback_memory, 0);
    if r != VK_SUCCESS {
        line(
            "graphics",
            "FAIL",
            format!("readback bind {}", api_result(r)),
        );
        free_memory(device, readback_memory, ptr::null());
        destroy_buffer(device, readback, ptr::null());
        return false;
    }

    let vertices: [[f32; 4]; 3] = [
        [-1.0, -1.0, 0.0, 1.0],
        [3.0, -1.0, 0.0, 1.0],
        [-1.0, 3.0, 0.0, 1.0],
    ];
    let mut vertex_map: *mut c_void = ptr::null_mut();
    let r = map_memory(device, readback_memory, 0, IMAGE_BYTES, 0, &mut vertex_map);
    if r != VK_SUCCESS || vertex_map.is_null() {
        line("graphics", "FAIL", format!("vertex map {}", api_result(r)));
        free_memory(device, readback_memory, ptr::null());
        destroy_buffer(device, readback, ptr::null());
        return false;
    }
    std::slice::from_raw_parts_mut(vertex_map as *mut u32, (IMAGE_BYTES / 4) as usize)
        .fill(0xDEAD_BEEF);
    ptr::copy_nonoverlapping(
        vertices.as_ptr() as *const u8,
        vertex_map as *mut u8,
        core::mem::size_of_val(&vertices),
    );
    unmap_memory(device, readback_memory);

    let image_info = VkImageCreateInfo {
        s_type: VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO,
        p_next: ptr::null(),
        flags: 0,
        image_type: VK_IMAGE_TYPE_2D,
        format: VK_FORMAT_R8G8B8A8_UNORM,
        extent: VkExtent3D {
            width: IMAGE_WIDTH,
            height: IMAGE_HEIGHT,
            depth: 1,
        },
        mip_levels: 1,
        array_layers: 1,
        samples: VK_SAMPLE_COUNT_1_BIT,
        tiling: VK_IMAGE_TILING_OPTIMAL,
        usage: VK_IMAGE_USAGE_TRANSFER_SRC_BIT | VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT,
        sharing_mode: 0,
        queue_family_index_count: 0,
        p_queue_family_indices: ptr::null(),
        initial_layout: VK_IMAGE_LAYOUT_UNDEFINED,
    };
    let mut image: VkImage = ptr::null_mut();
    let r = create_image(device, &image_info, ptr::null(), &mut image);
    if r != VK_SUCCESS {
        line("graphics", "FAIL", format!("image {}", api_result(r)));
        free_memory(device, readback_memory, ptr::null());
        destroy_buffer(device, readback, ptr::null());
        return false;
    }
    let mut image_req = VkMemoryRequirements::default();
    get_image_requirements(device, image, &mut image_req);
    let Some(image_type) = choose_type(
        &mem_props,
        image_req.memory_type_bits,
        VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT,
    ) else {
        line("graphics", "FAIL", "no device-local image memory type");
        destroy_image(device, image, ptr::null());
        free_memory(device, readback_memory, ptr::null());
        destroy_buffer(device, readback, ptr::null());
        return false;
    };
    let image_alloc = VkMemoryAllocateInfo {
        s_type: 5,
        p_next: ptr::null(),
        allocation_size: image_req.size,
        memory_type_index: image_type,
    };
    let mut image_memory: VkDeviceMemory = ptr::null_mut();
    let r = allocate_memory(device, &image_alloc, ptr::null(), &mut image_memory);
    if r != VK_SUCCESS {
        line(
            "graphics",
            "FAIL",
            format!("image memory {}", api_result(r)),
        );
        destroy_image(device, image, ptr::null());
        free_memory(device, readback_memory, ptr::null());
        destroy_buffer(device, readback, ptr::null());
        return false;
    }
    let r = bind_image_memory(device, image, image_memory, 0);
    if r != VK_SUCCESS {
        line("graphics", "FAIL", format!("image bind {}", api_result(r)));
        free_memory(device, image_memory, ptr::null());
        destroy_image(device, image, ptr::null());
        free_memory(device, readback_memory, ptr::null());
        destroy_buffer(device, readback, ptr::null());
        return false;
    }

    let view_info = VkImageViewCreateInfo {
        s_type: VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO,
        p_next: ptr::null(),
        flags: 0,
        image,
        view_type: VK_IMAGE_VIEW_TYPE_2D,
        format: VK_FORMAT_R8G8B8A8_UNORM,
        components: VkComponentMapping::default(),
        subresource_range: VkImageSubresourceRange {
            aspect_mask: VK_IMAGE_ASPECT_COLOR_BIT,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        },
    };
    let mut view: VkImageView = ptr::null_mut();
    let r = create_image_view(device, &view_info, ptr::null(), &mut view);
    if r != VK_SUCCESS {
        line("graphics", "FAIL", format!("image view {}", api_result(r)));
        free_memory(device, image_memory, ptr::null());
        destroy_image(device, image, ptr::null());
        free_memory(device, readback_memory, ptr::null());
        destroy_buffer(device, readback, ptr::null());
        return false;
    }

    let attachment = VkAttachmentDescription {
        flags: 0,
        format: VK_FORMAT_R8G8B8A8_UNORM,
        samples: VK_SAMPLE_COUNT_1_BIT,
        load_op: VK_ATTACHMENT_LOAD_OP_CLEAR,
        store_op: VK_ATTACHMENT_STORE_OP_STORE,
        stencil_load_op: VK_ATTACHMENT_LOAD_OP_DONT_CARE,
        stencil_store_op: VK_ATTACHMENT_STORE_OP_DONT_CARE,
        initial_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        final_layout: VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
    };
    let color_ref = VkAttachmentReference {
        attachment: 0,
        layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL_VALUE,
    };
    let subpass = VkSubpassDescription {
        flags: 0,
        pipeline_bind_point: VK_PIPELINE_BIND_POINT_GRAPHICS,
        input_attachment_count: 0,
        p_input_attachments: ptr::null(),
        color_attachment_count: 1,
        p_color_attachments: &color_ref,
        p_resolve_attachments: ptr::null(),
        p_depth_stencil_attachment: ptr::null(),
        preserve_attachment_count: 0,
        p_preserve_attachments: ptr::null(),
    };
    let dependency = VkSubpassDependency {
        src_subpass: 0,
        dst_subpass: VK_SUBPASS_EXTERNAL,
        src_stage_mask: VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
        dst_stage_mask: VK_PIPELINE_STAGE_TRANSFER_BIT,
        src_access_mask: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        dst_access_mask: VK_ACCESS_TRANSFER_READ_BIT,
        dependency_flags: 0,
    };
    let render_pass_info = VkRenderPassCreateInfo {
        s_type: VK_STRUCTURE_TYPE_RENDER_PASS_CREATE_INFO,
        p_next: ptr::null(),
        flags: 0,
        attachment_count: 1,
        p_attachments: &attachment,
        subpass_count: 1,
        p_subpasses: &subpass,
        dependency_count: 1,
        p_dependencies: &dependency,
    };
    let mut render_pass: VkRenderPass = ptr::null_mut();
    let r = create_render_pass(device, &render_pass_info, ptr::null(), &mut render_pass);
    if r != VK_SUCCESS {
        line("graphics", "FAIL", format!("render pass {}", api_result(r)));
        destroy_image_view(device, view, ptr::null());
        free_memory(device, image_memory, ptr::null());
        destroy_image(device, image, ptr::null());
        free_memory(device, readback_memory, ptr::null());
        destroy_buffer(device, readback, ptr::null());
        return false;
    }
    let framebuffer_info = VkFramebufferCreateInfo {
        s_type: VK_STRUCTURE_TYPE_FRAMEBUFFER_CREATE_INFO,
        p_next: ptr::null(),
        flags: 0,
        render_pass,
        attachment_count: 1,
        p_attachments: &view,
        width: IMAGE_WIDTH,
        height: IMAGE_HEIGHT,
        layers: 1,
    };
    let mut framebuffer: VkFramebuffer = ptr::null_mut();
    let r = create_framebuffer(device, &framebuffer_info, ptr::null(), &mut framebuffer);
    if r != VK_SUCCESS {
        line("graphics", "FAIL", format!("framebuffer {}", api_result(r)));
        destroy_render_pass(device, render_pass, ptr::null());
        destroy_image_view(device, view, ptr::null());
        free_memory(device, image_memory, ptr::null());
        destroy_image(device, image, ptr::null());
        free_memory(device, readback_memory, ptr::null());
        destroy_buffer(device, readback, ptr::null());
        return false;
    }

    let vert_words = words(TRI_VERT_SPV);
    let frag_words = words(TRI_FRAG_SPV);
    let vert_info = VkShaderModuleCreateInfo {
        s_type: VK_STRUCTURE_TYPE_SHADER_MODULE_CREATE_INFO,
        p_next: ptr::null(),
        flags: 0,
        code_size: vert_words.len() * 4,
        p_code: vert_words.as_ptr(),
    };
    let frag_info = VkShaderModuleCreateInfo {
        s_type: VK_STRUCTURE_TYPE_SHADER_MODULE_CREATE_INFO,
        p_next: ptr::null(),
        flags: 0,
        code_size: frag_words.len() * 4,
        p_code: frag_words.as_ptr(),
    };
    let mut vert: VkShaderModule = ptr::null_mut();
    let mut frag: VkShaderModule = ptr::null_mut();
    let r = create_shader(device, &vert_info, ptr::null(), &mut vert);
    if r != VK_SUCCESS {
        line(
            "graphics",
            "FAIL",
            format!("vertex shader {}", api_result(r)),
        );
        return false;
    }
    let r = create_shader(device, &frag_info, ptr::null(), &mut frag);
    if r != VK_SUCCESS {
        line(
            "graphics",
            "FAIL",
            format!("fragment shader {}", api_result(r)),
        );
        destroy_shader(device, vert, ptr::null());
        return false;
    }
    let main_name = CString::new("main").unwrap();
    let stages = [
        VkPipelineShaderStageCreateInfo {
            s_type: VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO,
            p_next: ptr::null(),
            flags: 0,
            stage: VK_SHADER_STAGE_VERTEX_BIT,
            module: vert,
            p_name: main_name.as_ptr(),
            p_specialization_info: ptr::null(),
        },
        VkPipelineShaderStageCreateInfo {
            s_type: VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO,
            p_next: ptr::null(),
            flags: 0,
            stage: VK_SHADER_STAGE_FRAGMENT_BIT,
            module: frag,
            p_name: main_name.as_ptr(),
            p_specialization_info: ptr::null(),
        },
    ];
    let layout_info = VkPipelineLayoutCreateInfo {
        s_type: VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO,
        p_next: ptr::null(),
        flags: 0,
        set_layout_count: 0,
        p_set_layouts: ptr::null(),
        push_constant_range_count: 0,
        p_push_constant_ranges: ptr::null(),
    };
    let mut pipeline_layout: VkPipelineLayout = ptr::null_mut();
    let r = create_layout(device, &layout_info, ptr::null(), &mut pipeline_layout);
    if r != VK_SUCCESS {
        line(
            "graphics",
            "FAIL",
            format!("pipeline layout {}", api_result(r)),
        );
        destroy_shader(device, frag, ptr::null());
        destroy_shader(device, vert, ptr::null());
        return false;
    }
    let vertex_binding = VkVertexInputBindingDescription {
        binding: 0,
        stride: core::mem::size_of::<[f32; 4]>() as u32,
        input_rate: 0,
    };
    let vertex_attribute = VkVertexInputAttributeDescription {
        location: 0,
        binding: 0,
        format: VK_FORMAT_R32G32B32A32_SFLOAT,
        offset: 0,
    };
    let vertex_input = VkPipelineVertexInputStateCreateInfo {
        s_type: VK_STRUCTURE_TYPE_PIPELINE_VERTEX_INPUT_STATE_CREATE_INFO,
        p_next: ptr::null(),
        flags: 0,
        vertex_binding_description_count: 1,
        p_vertex_binding_descriptions: &vertex_binding,
        vertex_attribute_description_count: 1,
        p_vertex_attribute_descriptions: &vertex_attribute,
    };
    let input_assembly = VkPipelineInputAssemblyStateCreateInfo {
        s_type: VK_STRUCTURE_TYPE_PIPELINE_INPUT_ASSEMBLY_STATE_CREATE_INFO,
        p_next: ptr::null(),
        flags: 0,
        topology: VK_PRIMITIVE_TOPOLOGY_TRIANGLE_LIST,
        primitive_restart_enable: 0,
    };
    let viewport = VkViewport {
        x: 0.0,
        y: 0.0,
        width: IMAGE_WIDTH as f32,
        height: IMAGE_HEIGHT as f32,
        min_depth: 0.0,
        max_depth: 1.0,
    };
    let scissor = VkRect2D {
        offset: VkOffset2D::default(),
        extent: VkExtent2D {
            width: IMAGE_WIDTH,
            height: IMAGE_HEIGHT,
        },
    };
    let viewport_state = VkPipelineViewportStateCreateInfo {
        s_type: VK_STRUCTURE_TYPE_PIPELINE_VIEWPORT_STATE_CREATE_INFO,
        p_next: ptr::null(),
        flags: 0,
        viewport_count: 1,
        p_viewports: &viewport,
        scissor_count: 1,
        p_scissors: &scissor,
    };
    let raster = VkPipelineRasterizationStateCreateInfo {
        s_type: VK_STRUCTURE_TYPE_PIPELINE_RASTERIZATION_STATE_CREATE_INFO,
        p_next: ptr::null(),
        flags: 0,
        depth_clamp_enable: 0,
        rasterizer_discard_enable: 0,
        polygon_mode: VK_POLYGON_MODE_FILL,
        cull_mode: VK_CULL_MODE_NONE,
        front_face: VK_FRONT_FACE_COUNTER_CLOCKWISE,
        depth_bias_enable: 0,
        depth_bias_constant_factor: 0.0,
        depth_bias_clamp: 0.0,
        depth_bias_slope_factor: 0.0,
        line_width: 1.0,
    };
    let multisample = VkPipelineMultisampleStateCreateInfo {
        s_type: VK_STRUCTURE_TYPE_PIPELINE_MULTISAMPLE_STATE_CREATE_INFO,
        p_next: ptr::null(),
        flags: 0,
        rasterization_samples: VK_SAMPLE_COUNT_1_BIT,
        sample_shading_enable: 0,
        min_sample_shading: 0.0,
        p_sample_mask: ptr::null(),
        alpha_to_coverage_enable: 0,
        alpha_to_one_enable: 0,
    };
    let blend_attachment = VkPipelineColorBlendAttachmentState {
        blend_enable: 0,
        src_color_blend_factor: 0,
        dst_color_blend_factor: 0,
        color_blend_op: 0,
        src_alpha_blend_factor: 0,
        dst_alpha_blend_factor: 0,
        alpha_blend_op: 0,
        color_write_mask: VK_COLOR_COMPONENT_R_BIT
            | VK_COLOR_COMPONENT_G_BIT
            | VK_COLOR_COMPONENT_B_BIT
            | VK_COLOR_COMPONENT_A_BIT,
    };
    let blend = VkPipelineColorBlendStateCreateInfo {
        s_type: VK_STRUCTURE_TYPE_PIPELINE_COLOR_BLEND_STATE_CREATE_INFO,
        p_next: ptr::null(),
        flags: 0,
        logic_op_enable: 0,
        logic_op: 0,
        attachment_count: 1,
        p_attachments: &blend_attachment,
        blend_constants: [0.0; 4],
    };
    let pipeline_info = VkGraphicsPipelineCreateInfo {
        s_type: VK_STRUCTURE_TYPE_GRAPHICS_PIPELINE_CREATE_INFO,
        p_next: ptr::null(),
        flags: 0,
        stage_count: 2,
        p_stages: stages.as_ptr(),
        p_vertex_input_state: &vertex_input,
        p_input_assembly_state: &input_assembly,
        p_tessellation_state: ptr::null(),
        p_viewport_state: &viewport_state,
        p_rasterization_state: &raster,
        p_multisample_state: &multisample,
        p_depth_stencil_state: ptr::null(),
        p_color_blend_state: &blend,
        p_dynamic_state: ptr::null(),
        layout: pipeline_layout,
        render_pass,
        subpass: 0,
        base_pipeline_handle: ptr::null_mut(),
        base_pipeline_index: -1,
    };
    let mut pipeline: VkPipeline = ptr::null_mut();
    let r = create_pipelines(
        device,
        ptr::null_mut(),
        1,
        &pipeline_info,
        ptr::null(),
        &mut pipeline,
    );
    if r != VK_SUCCESS {
        line("graphics", "FAIL", format!("pipeline {}", api_result(r)));
        return false;
    }
    line("graphics", "PASS", "pipeline");

    let pool_info = VkCommandPoolCreateInfo {
        s_type: VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO,
        p_next: ptr::null(),
        flags: 0,
        queue_family_index,
    };
    let mut command_pool: VkCommandPool = ptr::null_mut();
    let r = create_pool(device, &pool_info, ptr::null(), &mut command_pool);
    if r != VK_SUCCESS {
        line(
            "graphics",
            "FAIL",
            format!("command pool {}", api_result(r)),
        );
        return false;
    }
    let command_alloc = VkCommandBufferAllocateInfo {
        s_type: VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO,
        p_next: ptr::null(),
        command_pool,
        level: VK_COMMAND_BUFFER_LEVEL_PRIMARY,
        command_buffer_count: 1,
    };
    let mut command_buffer: VkCommandBuffer = ptr::null_mut();
    let r = allocate_commands(device, &command_alloc, &mut command_buffer);
    if r != VK_SUCCESS {
        line(
            "graphics",
            "FAIL",
            format!("command buffer {}", api_result(r)),
        );
        return false;
    }
    let begin_info = VkCommandBufferBeginInfo {
        s_type: VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO,
        p_next: ptr::null(),
        flags: VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT,
        p_inheritance_info: ptr::null(),
    };
    let r = begin_command(command_buffer, &begin_info);
    if r != VK_SUCCESS {
        line(
            "graphics",
            "FAIL",
            format!("begin command {}", api_result(r)),
        );
        return false;
    }
    let clear = VkClearValue {
        color: [0.0, 1.0, 0.0, 1.0],
    };
    let render_begin = VkRenderPassBeginInfo {
        s_type: VK_STRUCTURE_TYPE_RENDER_PASS_BEGIN_INFO,
        p_next: ptr::null(),
        render_pass,
        framebuffer,
        render_area: scissor,
        clear_value_count: 1,
        p_clear_values: &clear,
    };
    cmd_begin_render_pass(command_buffer, &render_begin, VK_SUBPASS_CONTENTS_INLINE);
    cmd_bind_pipeline(command_buffer, VK_PIPELINE_BIND_POINT_GRAPHICS, pipeline);
    let vertex_offset: VkDeviceSize = 0;
    cmd_bind_vertex_buffers(command_buffer, 0, 1, &readback, &vertex_offset);
    cmd_draw(command_buffer, 3, 1, 0, 0);
    cmd_end_render_pass(command_buffer);
    let copy_region = VkBufferImageCopy {
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
            width: IMAGE_WIDTH,
            height: IMAGE_HEIGHT,
            depth: 1,
        },
    };
    cmd_copy(
        command_buffer,
        image,
        VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
        readback,
        1,
        &copy_region,
    );
    let barrier = VkMemoryBarrier {
        s_type: VK_STRUCTURE_TYPE_MEMORY_BARRIER,
        p_next: ptr::null(),
        src_access_mask: VK_ACCESS_TRANSFER_WRITE_BIT,
        dst_access_mask: VK_ACCESS_HOST_READ_BIT,
    };
    cmd_barrier(
        command_buffer,
        VK_PIPELINE_STAGE_TRANSFER_BIT,
        VK_PIPELINE_STAGE_HOST_BIT,
        0,
        1,
        &barrier,
        0,
        ptr::null(),
        0,
        ptr::null(),
    );
    let r = end_command(command_buffer);
    if r != VK_SUCCESS {
        line("graphics", "FAIL", format!("end command {}", api_result(r)));
        return false;
    }
    line("graphics", "PASS", "commands");

    let fence_info = VkFenceCreateInfo {
        s_type: VK_STRUCTURE_TYPE_FENCE_CREATE_INFO,
        p_next: ptr::null(),
        flags: 0,
    };
    let mut fence: VkFence = ptr::null_mut();
    let r = create_fence(device, &fence_info, ptr::null(), &mut fence);
    if r != VK_SUCCESS {
        line("graphics", "FAIL", format!("fence {}", api_result(r)));
        return false;
    }
    let submit = VkSubmitInfo {
        s_type: VK_STRUCTURE_TYPE_SUBMIT_INFO,
        p_next: ptr::null(),
        wait_semaphore_count: 0,
        p_wait_semaphores: ptr::null(),
        p_wait_dst_stage_mask: ptr::null(),
        command_buffer_count: 1,
        p_command_buffers: &command_buffer,
        signal_semaphore_count: 0,
        p_signal_semaphores: ptr::null(),
    };
    let r = queue_submit(queue, 1, &submit, fence);
    if r != VK_SUCCESS {
        line(
            "graphics",
            "FAIL",
            format!("queue submit {}", api_result(r)),
        );
        return false;
    }
    let r = wait_fences(device, 1, &fence, VK_TRUE, 5_000_000_000);
    if r != VK_SUCCESS {
        line("graphics", "FAIL", format!("fence wait {}", api_result(r)));
        return false;
    }
    line("graphics", "PASS", "queue_submit+fence");

    let mut mapped: *mut c_void = ptr::null_mut();
    let r = map_memory(device, readback_memory, 0, IMAGE_BYTES, 0, &mut mapped);
    let ok = if r == VK_SUCCESS && !mapped.is_null() {
        let pixel = *(mapped as *const u32);
        let red_ok = pixel == 0xFF00_00FF;
        line(
            "graphics",
            if red_ok { "PASS" } else { "FAIL" },
            format!("pixel={pixel:#010x} expected=0xff0000ff"),
        );
        unmap_memory(device, readback_memory);
        red_ok
    } else {
        line(
            "graphics",
            "FAIL",
            format!("readback map {}", api_result(r)),
        );
        false
    };

    destroy_fence(device, fence, ptr::null());
    destroy_pool(device, command_pool, ptr::null());
    destroy_pipeline(device, pipeline, ptr::null());
    destroy_layout(device, pipeline_layout, ptr::null());
    destroy_shader(device, frag, ptr::null());
    destroy_shader(device, vert, ptr::null());
    destroy_framebuffer(device, framebuffer, ptr::null());
    destroy_render_pass(device, render_pass, ptr::null());
    destroy_image_view(device, view, ptr::null());
    free_memory(device, image_memory, ptr::null());
    destroy_image(device, image, ptr::null());
    free_memory(device, readback_memory, ptr::null());
    destroy_buffer(device, readback, ptr::null());
    line("graphics", "PASS", "teardown");
    ok
}
