//! Minimal Vulkan capability probe for a Husk guest.
//!
//! It proves the layer above the kbase probe: the guest can create a Vulkan
//! instance, enumerate a real Mali physical device, create a logical device and
//! one queue, allocate/bind/map a small host-visible buffer, execute a compute
//! shader, render an offscreen triangle, and tear everything down. All Vulkan
//! entry points are resolved dynamically so the binary has no build-time
//! dependency on a particular ICD manifest or loader soname.

use core::ffi::{c_char, c_int, c_void};
use std::ffi::CString;
use std::ptr;

mod graphics;

type VkResult = i32;
type VkFlags = u32;
type VkDeviceSize = u64;
type VkInstance = *mut c_void;
type VkPhysicalDevice = *mut c_void;
type VkDevice = *mut c_void;
type VkQueue = *mut c_void;
type VkBuffer = *mut c_void;
type VkDeviceMemory = *mut c_void;
type VkShaderModule = *mut c_void;
type VkPipelineLayout = *mut c_void;
type VkPipeline = *mut c_void;
type VkDescriptorSetLayout = *mut c_void;
type VkDescriptorPool = *mut c_void;
type VkDescriptorSet = *mut c_void;
type VkCommandPool = *mut c_void;
type VkCommandBuffer = *mut c_void;
type VkFence = *mut c_void;

const VK_SUCCESS: VkResult = 0;
const VK_STRUCTURE_TYPE_APPLICATION_INFO: u32 = 0;
const VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO: u32 = 1;
const VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO: u32 = 3;
const VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO: u32 = 4;
const VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO: u32 = 5;
const VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO: u32 = 12;
const VK_STRUCTURE_TYPE_SHADER_MODULE_CREATE_INFO: u32 = 16;
const VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO: u32 = 18;
const VK_STRUCTURE_TYPE_COMPUTE_PIPELINE_CREATE_INFO: u32 = 29;
const VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO: u32 = 30;
const VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_CREATE_INFO: u32 = 32;
const VK_STRUCTURE_TYPE_DESCRIPTOR_POOL_CREATE_INFO: u32 = 33;
const VK_STRUCTURE_TYPE_DESCRIPTOR_SET_ALLOCATE_INFO: u32 = 34;
const VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET: u32 = 35;
const VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO: u32 = 39;
const VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO: u32 = 40;
const VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO: u32 = 42;
const VK_STRUCTURE_TYPE_SUBMIT_INFO: u32 = 4;
const VK_STRUCTURE_TYPE_FENCE_CREATE_INFO: u32 = 8;
const VK_QUEUE_GRAPHICS_BIT: u32 = 1 << 0;
const VK_QUEUE_COMPUTE_BIT: u32 = 1 << 1;
const VK_QUEUE_TRANSFER_BIT: u32 = 1 << 2;
const VK_BUFFER_USAGE_TRANSFER_SRC_BIT: u32 = 1 << 0;
const VK_BUFFER_USAGE_TRANSFER_DST_BIT: u32 = 1 << 1;
const VK_BUFFER_USAGE_STORAGE_BUFFER_BIT: u32 = 1 << 5;
const VK_SHADER_STAGE_COMPUTE_BIT: u32 = 1 << 5;
const VK_DESCRIPTOR_TYPE_STORAGE_BUFFER: u32 = 7;
const VK_PIPELINE_BIND_POINT_COMPUTE: u32 = 1;
const VK_COMMAND_BUFFER_LEVEL_PRIMARY: u32 = 0;
const VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT: u32 = 1;
const VK_TRUE: u32 = 1;
const VK_SHARING_MODE_EXCLUSIVE: u32 = 0;
const VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT: u32 = 1 << 1;
const VK_MEMORY_PROPERTY_HOST_COHERENT_BIT: u32 = 1 << 2;
const VK_API_VERSION_1_3: u32 = (1 << 22) | (3 << 12);

#[repr(C)]
struct VkApplicationInfo {
    s_type: u32,
    p_next: *const c_void,
    p_application_name: *const c_char,
    application_version: u32,
    p_engine_name: *const c_char,
    engine_version: u32,
    api_version: u32,
}

#[repr(C)]
struct VkInstanceCreateInfo {
    s_type: u32,
    p_next: *const c_void,
    flags: VkFlags,
    p_application_info: *const VkApplicationInfo,
    enabled_layer_count: u32,
    pp_enabled_layer_names: *const *const c_char,
    enabled_extension_count: u32,
    pp_enabled_extension_names: *const *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct VkQueueFamilyProperties {
    queue_flags: u32,
    queue_count: u32,
    timestamp_valid_bits: u32,
    min_image_transfer_granularity: [u32; 3],
}

#[repr(C)]
struct VkDeviceQueueCreateInfo {
    s_type: u32,
    p_next: *const c_void,
    flags: VkFlags,
    queue_family_index: u32,
    queue_count: u32,
    p_queue_priorities: *const f32,
}

#[repr(C)]
struct VkDeviceCreateInfo {
    s_type: u32,
    p_next: *const c_void,
    flags: VkFlags,
    queue_create_info_count: u32,
    p_queue_create_infos: *const VkDeviceQueueCreateInfo,
    enabled_layer_count: u32,
    pp_enabled_layer_names: *const *const c_char,
    enabled_extension_count: u32,
    pp_enabled_extension_names: *const *const c_char,
    p_enabled_features: *const c_void,
}

#[repr(C)]
struct VkBufferCreateInfo {
    s_type: u32,
    p_next: *const c_void,
    flags: VkFlags,
    size: VkDeviceSize,
    usage: VkFlags,
    sharing_mode: u32,
    queue_family_index_count: u32,
    p_queue_family_indices: *const u32,
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
struct VkComputePipelineCreateInfo {
    s_type: u32,
    p_next: *const c_void,
    flags: VkFlags,
    stage: VkPipelineShaderStageCreateInfo,
    layout: VkPipelineLayout,
    base_pipeline_handle: VkPipeline,
    base_pipeline_index: i32,
}

#[repr(C)]
struct VkPipelineLayoutCreateInfo {
    s_type: u32,
    p_next: *const c_void,
    flags: VkFlags,
    set_layout_count: u32,
    p_set_layouts: *const VkDescriptorSetLayout,
    push_constant_range_count: u32,
    p_push_constant_ranges: *const c_void,
}

#[repr(C)]
struct VkDescriptorSetLayoutBinding {
    binding: u32,
    descriptor_type: u32,
    descriptor_count: u32,
    stage_flags: u32,
    p_immutable_samplers: *const c_void,
}

#[repr(C)]
struct VkDescriptorSetLayoutCreateInfo {
    s_type: u32,
    p_next: *const c_void,
    flags: VkFlags,
    binding_count: u32,
    p_bindings: *const VkDescriptorSetLayoutBinding,
}

#[repr(C)]
struct VkDescriptorPoolSize {
    ty: u32,
    descriptor_count: u32,
}

#[repr(C)]
struct VkDescriptorPoolCreateInfo {
    s_type: u32,
    p_next: *const c_void,
    flags: VkFlags,
    max_sets: u32,
    pool_size_count: u32,
    p_pool_sizes: *const VkDescriptorPoolSize,
}

#[repr(C)]
struct VkDescriptorSetAllocateInfo {
    s_type: u32,
    p_next: *const c_void,
    descriptor_pool: VkDescriptorPool,
    descriptor_set_count: u32,
    p_set_layouts: *const VkDescriptorSetLayout,
}

#[repr(C)]
struct VkDescriptorBufferInfo {
    buffer: VkBuffer,
    offset: VkDeviceSize,
    range: VkDeviceSize,
}

#[repr(C)]
struct VkWriteDescriptorSet {
    s_type: u32,
    p_next: *const c_void,
    dst_set: VkDescriptorSet,
    dst_binding: u32,
    dst_array_element: u32,
    descriptor_count: u32,
    descriptor_type: u32,
    p_image_info: *const c_void,
    p_buffer_info: *const VkDescriptorBufferInfo,
    p_texel_buffer_view: *const c_void,
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

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct VkMemoryRequirements {
    size: VkDeviceSize,
    alignment: VkDeviceSize,
    memory_type_bits: u32,
}

#[repr(C)]
struct VkMemoryAllocateInfo {
    s_type: u32,
    p_next: *const c_void,
    allocation_size: VkDeviceSize,
    memory_type_index: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct VkMemoryType {
    property_flags: VkFlags,
    heap_index: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct VkMemoryHeap {
    size: VkDeviceSize,
    flags: VkFlags,
}

#[repr(C)]
struct VkPhysicalDeviceMemoryProperties {
    memory_type_count: u32,
    memory_types: [VkMemoryType; 32],
    memory_heap_count: u32,
    memory_heaps: [VkMemoryHeap; 16],
}

#[repr(C, align(16))]
struct AlignedProperties([u8; 1024]);

#[repr(C)]
#[derive(Clone, Copy)]
struct VkExtensionProperties {
    extension_name: [c_char; 256],
    spec_version: u32,
}

type FnCreateInstance =
    unsafe extern "C" fn(*const VkInstanceCreateInfo, *const c_void, *mut VkInstance) -> VkResult;
type FnDestroyInstance = unsafe extern "C" fn(VkInstance, *const c_void);
type FnEnumeratePhysicalDevices =
    unsafe extern "C" fn(VkInstance, *mut u32, *mut VkPhysicalDevice) -> VkResult;
type FnGetPhysicalDeviceProperties = unsafe extern "C" fn(VkPhysicalDevice, *mut c_void);
type FnGetPhysicalDeviceQueueFamilyProperties =
    unsafe extern "C" fn(VkPhysicalDevice, *mut u32, *mut VkQueueFamilyProperties);
type FnGetPhysicalDeviceMemoryProperties =
    unsafe extern "C" fn(VkPhysicalDevice, *mut VkPhysicalDeviceMemoryProperties);
type FnEnumerateInstanceExtensionProperties =
    unsafe extern "C" fn(*const c_char, *mut u32, *mut VkExtensionProperties) -> VkResult;
type FnEnumerateDeviceExtensionProperties = unsafe extern "C" fn(
    VkPhysicalDevice,
    *const c_char,
    *mut u32,
    *mut VkExtensionProperties,
) -> VkResult;
type FnCreateDevice = unsafe extern "C" fn(
    VkPhysicalDevice,
    *const VkDeviceCreateInfo,
    *const c_void,
    *mut VkDevice,
) -> VkResult;
type FnDestroyDevice = unsafe extern "C" fn(VkDevice, *const c_void);
type FnGetDeviceQueue = unsafe extern "C" fn(VkDevice, u32, u32, *mut VkQueue);
type FnCreateBuffer = unsafe extern "C" fn(
    VkDevice,
    *const VkBufferCreateInfo,
    *const c_void,
    *mut VkBuffer,
) -> VkResult;
type FnDestroyBuffer = unsafe extern "C" fn(VkDevice, VkBuffer, *const c_void);
type FnGetBufferMemoryRequirements =
    unsafe extern "C" fn(VkDevice, VkBuffer, *mut VkMemoryRequirements);
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
    VkFlags,
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
type FnCreateDescriptorSetLayout = unsafe extern "C" fn(
    VkDevice,
    *const VkDescriptorSetLayoutCreateInfo,
    *const c_void,
    *mut VkDescriptorSetLayout,
) -> VkResult;
type FnDestroyDescriptorSetLayout =
    unsafe extern "C" fn(VkDevice, VkDescriptorSetLayout, *const c_void);
type FnCreatePipelineLayout = unsafe extern "C" fn(
    VkDevice,
    *const VkPipelineLayoutCreateInfo,
    *const c_void,
    *mut VkPipelineLayout,
) -> VkResult;
type FnDestroyPipelineLayout = unsafe extern "C" fn(VkDevice, VkPipelineLayout, *const c_void);
type FnCreateComputePipelines = unsafe extern "C" fn(
    VkDevice,
    *mut c_void,
    u32,
    *const VkComputePipelineCreateInfo,
    *const c_void,
    *mut VkPipeline,
) -> VkResult;
type FnDestroyPipeline = unsafe extern "C" fn(VkDevice, VkPipeline, *const c_void);
type FnCreateDescriptorPool = unsafe extern "C" fn(
    VkDevice,
    *const VkDescriptorPoolCreateInfo,
    *const c_void,
    *mut VkDescriptorPool,
) -> VkResult;
type FnDestroyDescriptorPool = unsafe extern "C" fn(VkDevice, VkDescriptorPool, *const c_void);
type FnAllocateDescriptorSets = unsafe extern "C" fn(
    VkDevice,
    *const VkDescriptorSetAllocateInfo,
    *mut VkDescriptorSet,
) -> VkResult;
type FnUpdateDescriptorSets =
    unsafe extern "C" fn(VkDevice, u32, *const VkWriteDescriptorSet, u32, *const c_void);
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
type FnCmdBindPipeline = unsafe extern "C" fn(VkCommandBuffer, u32, VkPipeline);
type FnCmdBindDescriptorSets = unsafe extern "C" fn(
    VkCommandBuffer,
    u32,
    VkPipelineLayout,
    u32,
    u32,
    *const VkDescriptorSet,
    u32,
    *const u32,
);
type FnCmdDispatch = unsafe extern "C" fn(VkCommandBuffer, u32, u32, u32);
type FnEndCommandBuffer = unsafe extern "C" fn(VkCommandBuffer) -> VkResult;
type FnCreateFence = unsafe extern "C" fn(
    VkDevice,
    *const VkFenceCreateInfo,
    *const c_void,
    *mut VkFence,
) -> VkResult;
type FnDestroyFence = unsafe extern "C" fn(VkDevice, VkFence, *const c_void);
type FnWaitForFences = unsafe extern "C" fn(VkDevice, u32, *const VkFence, u32, u64) -> VkResult;
type FnQueueSubmit = unsafe extern "C" fn(VkQueue, u32, *const VkSubmitInfo, VkFence) -> VkResult;

extern "C" {
    fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlclose(handle: *mut c_void) -> c_int;
}

const RTLD_NOW: c_int = 2;

fn line(name: &str, verdict: &str, detail: impl AsRef<str>) {
    println!("vk|{}|{}|{}", name, verdict, detail.as_ref());
}

unsafe fn symbol<T: Copy>(lib: *mut c_void, name: &str) -> Option<T> {
    let cname = CString::new(name).ok()?;
    let p = dlsym(lib, cname.as_ptr());
    if p.is_null() {
        None
    } else {
        Some(std::mem::transmute_copy(&p))
    }
}

fn api_result(r: VkResult) -> String {
    format!("VkResult={r}")
}

fn choose_memory_type(props: &VkPhysicalDeviceMemoryProperties, bits: u32) -> Option<u32> {
    let wanted = VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT;
    for i in 0..props.memory_type_count.min(32) {
        let t = props.memory_types[i as usize];
        if (bits & (1 << i)) != 0 && (t.property_flags & wanted) == wanted {
            return Some(i);
        }
    }
    for i in 0..props.memory_type_count.min(32) {
        let t = props.memory_types[i as usize];
        if (bits & (1 << i)) != 0 && (t.property_flags & VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT) != 0 {
            return Some(i);
        }
    }
    for i in 0..props.memory_type_count.min(32) {
        if (bits & (1 << i)) != 0 {
            return Some(i);
        }
    }
    None
}

fn extension_name(props: &VkExtensionProperties) -> String {
    let bytes: Vec<u8> = props
        .extension_name
        .iter()
        .map(|&c| c as u8)
        .take_while(|&c| c != 0)
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

unsafe fn enumerate_list(
    count_fn: impl Fn(*mut u32, *mut VkExtensionProperties) -> VkResult,
) -> Result<Vec<VkExtensionProperties>, VkResult> {
    let mut count = 0u32;
    let r = count_fn(&mut count, ptr::null_mut());
    if r != VK_SUCCESS && r != 5 {
        return Err(r);
    }
    if count == 0 {
        return Ok(Vec::new());
    }
    let mut props = vec![
        VkExtensionProperties {
            extension_name: [0; 256],
            spec_version: 0,
        };
        count as usize
    ];
    let r = count_fn(&mut count, props.as_mut_ptr());
    if r != VK_SUCCESS && r != 5 {
        return Err(r);
    }
    props.truncate(count as usize);
    Ok(props)
}

unsafe fn report_extensions(lib: *mut c_void, instance: VkInstance, physical: VkPhysicalDevice) {
    let instance_fn: FnEnumerateInstanceExtensionProperties =
        match symbol(lib, "vkEnumerateInstanceExtensionProperties") {
            Some(f) => f,
            None => {
                line(
                    "ext",
                    "FAIL",
                    "vkEnumerateInstanceExtensionProperties missing",
                );
                return;
            }
        };
    let device_fn: FnEnumerateDeviceExtensionProperties =
        match symbol(lib, "vkEnumerateDeviceExtensionProperties") {
            Some(f) => f,
            None => {
                line(
                    "ext",
                    "FAIL",
                    "vkEnumerateDeviceExtensionProperties missing",
                );
                return;
            }
        };
    let _ = instance;
    let instance_list = enumerate_list(|count, props| instance_fn(ptr::null(), count, props));
    let device_list = enumerate_list(|count, props| device_fn(physical, ptr::null(), count, props));
    let (instance_list, device_list) = match (instance_list, device_list) {
        (Ok(i), Ok(d)) => (i, d),
        (Err(i), _) => {
            line(
                "ext",
                "FAIL",
                format!("instance enumerate {}", api_result(i)),
            );
            return;
        }
        (_, Err(d)) => {
            line("ext", "FAIL", format!("device enumerate {}", api_result(d)));
            return;
        }
    };
    let instance_names: Vec<String> = instance_list.iter().map(extension_name).collect();
    let device_names: Vec<String> = device_list.iter().map(extension_name).collect();
    line(
        "ext",
        "INFO",
        format!(
            "instance={} device={}",
            instance_names.len(),
            device_names.len()
        ),
    );

    const WANTED: &[(&str, bool)] = &[
        ("VK_KHR_surface", true),
        ("VK_KHR_android_surface", true),
        ("VK_KHR_get_physical_device_properties2", true),
        ("VK_KHR_external_memory_capabilities", true),
        ("VK_KHR_external_fence_capabilities", true),
        ("VK_KHR_external_semaphore_capabilities", true),
        ("VK_KHR_get_surface_capabilities2", true),
        ("VK_EXT_surface_maintenance1", true),
        ("VK_KHR_swapchain", false),
        ("VK_ANDROID_external_memory_android_hardware_buffer", false),
        ("VK_KHR_external_memory", false),
        ("VK_KHR_external_memory_fd", false),
        ("VK_EXT_external_memory_dma_buf", false),
        ("VK_EXT_external_memory_host", false),
        ("VK_EXT_image_drm_format_modifier", false),
        ("VK_KHR_external_fence", false),
        ("VK_KHR_external_fence_fd", false),
        ("VK_KHR_external_semaphore", false),
        ("VK_KHR_external_semaphore_fd", false),
        ("VK_EXT_queue_family_foreign", false),
        ("VK_KHR_timeline_semaphore", false),
        ("VK_KHR_synchronization2", false),
        ("VK_EXT_swapchain_maintenance1", false),
        ("VK_KHR_present_id", false),
        ("VK_KHR_present_wait", false),
    ];
    for &(name, instance_ext) in WANTED {
        let names = if instance_ext {
            &instance_names
        } else {
            &device_names
        };
        let state = if names.iter().any(|n| n == name) {
            "support"
        } else {
            "missing"
        };
        line("ext", state, name);
    }
}

const COMPUTE_SPV: &[u8] = include_bytes!("../shaders/compute.spv");

unsafe fn submit_compute(
    lib: *mut c_void,
    device: VkDevice,
    queue: VkQueue,
    queue_family_index: u32,
    buffer: VkBuffer,
    memory: VkDeviceMemory,
) -> bool {
    let create_shader: FnCreateShaderModule = match symbol(lib, "vkCreateShaderModule") {
        Some(f) => f,
        None => {
            line("compute", "FAIL", "vkCreateShaderModule missing");
            return false;
        }
    };
    let destroy_shader: FnDestroyShaderModule = match symbol(lib, "vkDestroyShaderModule") {
        Some(f) => f,
        None => {
            line("compute", "FAIL", "vkDestroyShaderModule missing");
            return false;
        }
    };
    let create_set_layout: FnCreateDescriptorSetLayout =
        match symbol(lib, "vkCreateDescriptorSetLayout") {
            Some(f) => f,
            None => {
                line("compute", "FAIL", "vkCreateDescriptorSetLayout missing");
                return false;
            }
        };
    let destroy_set_layout: FnDestroyDescriptorSetLayout =
        match symbol(lib, "vkDestroyDescriptorSetLayout") {
            Some(f) => f,
            None => {
                line("compute", "FAIL", "vkDestroyDescriptorSetLayout missing");
                return false;
            }
        };
    let create_pipeline_layout: FnCreatePipelineLayout = match symbol(lib, "vkCreatePipelineLayout")
    {
        Some(f) => f,
        None => {
            line("compute", "FAIL", "vkCreatePipelineLayout missing");
            return false;
        }
    };
    let destroy_pipeline_layout: FnDestroyPipelineLayout =
        match symbol(lib, "vkDestroyPipelineLayout") {
            Some(f) => f,
            None => {
                line("compute", "FAIL", "vkDestroyPipelineLayout missing");
                return false;
            }
        };
    let create_pipelines: FnCreateComputePipelines = match symbol(lib, "vkCreateComputePipelines") {
        Some(f) => f,
        None => {
            line("compute", "FAIL", "vkCreateComputePipelines missing");
            return false;
        }
    };
    let destroy_pipeline: FnDestroyPipeline = match symbol(lib, "vkDestroyPipeline") {
        Some(f) => f,
        None => {
            line("compute", "FAIL", "vkDestroyPipeline missing");
            return false;
        }
    };
    let create_pool: FnCreateDescriptorPool = match symbol(lib, "vkCreateDescriptorPool") {
        Some(f) => f,
        None => {
            line("compute", "FAIL", "vkCreateDescriptorPool missing");
            return false;
        }
    };
    let destroy_pool: FnDestroyDescriptorPool = match symbol(lib, "vkDestroyDescriptorPool") {
        Some(f) => f,
        None => {
            line("compute", "FAIL", "vkDestroyDescriptorPool missing");
            return false;
        }
    };
    let allocate_sets: FnAllocateDescriptorSets = match symbol(lib, "vkAllocateDescriptorSets") {
        Some(f) => f,
        None => {
            line("compute", "FAIL", "vkAllocateDescriptorSets missing");
            return false;
        }
    };
    let update_sets: FnUpdateDescriptorSets = match symbol(lib, "vkUpdateDescriptorSets") {
        Some(f) => f,
        None => {
            line("compute", "FAIL", "vkUpdateDescriptorSets missing");
            return false;
        }
    };
    let create_command_pool: FnCreateCommandPool = match symbol(lib, "vkCreateCommandPool") {
        Some(f) => f,
        None => {
            line("compute", "FAIL", "vkCreateCommandPool missing");
            return false;
        }
    };
    let destroy_command_pool: FnDestroyCommandPool = match symbol(lib, "vkDestroyCommandPool") {
        Some(f) => f,
        None => {
            line("compute", "FAIL", "vkDestroyCommandPool missing");
            return false;
        }
    };
    let allocate_commands: FnAllocateCommandBuffers = match symbol(lib, "vkAllocateCommandBuffers")
    {
        Some(f) => f,
        None => {
            line("compute", "FAIL", "vkAllocateCommandBuffers missing");
            return false;
        }
    };
    let begin_command: FnBeginCommandBuffer = match symbol(lib, "vkBeginCommandBuffer") {
        Some(f) => f,
        None => {
            line("compute", "FAIL", "vkBeginCommandBuffer missing");
            return false;
        }
    };
    let bind_pipeline: FnCmdBindPipeline = match symbol(lib, "vkCmdBindPipeline") {
        Some(f) => f,
        None => {
            line("compute", "FAIL", "vkCmdBindPipeline missing");
            return false;
        }
    };
    let bind_sets: FnCmdBindDescriptorSets = match symbol(lib, "vkCmdBindDescriptorSets") {
        Some(f) => f,
        None => {
            line("compute", "FAIL", "vkCmdBindDescriptorSets missing");
            return false;
        }
    };
    let dispatch: FnCmdDispatch = match symbol(lib, "vkCmdDispatch") {
        Some(f) => f,
        None => {
            line("compute", "FAIL", "vkCmdDispatch missing");
            return false;
        }
    };
    let end_command: FnEndCommandBuffer = match symbol(lib, "vkEndCommandBuffer") {
        Some(f) => f,
        None => {
            line("compute", "FAIL", "vkEndCommandBuffer missing");
            return false;
        }
    };
    let create_fence: FnCreateFence = match symbol(lib, "vkCreateFence") {
        Some(f) => f,
        None => {
            line("compute", "FAIL", "vkCreateFence missing");
            return false;
        }
    };
    let destroy_fence: FnDestroyFence = match symbol(lib, "vkDestroyFence") {
        Some(f) => f,
        None => {
            line("compute", "FAIL", "vkDestroyFence missing");
            return false;
        }
    };
    let wait_fences: FnWaitForFences = match symbol(lib, "vkWaitForFences") {
        Some(f) => f,
        None => {
            line("compute", "FAIL", "vkWaitForFences missing");
            return false;
        }
    };
    let queue_submit: FnQueueSubmit = match symbol(lib, "vkQueueSubmit") {
        Some(f) => f,
        None => {
            line("compute", "FAIL", "vkQueueSubmit missing");
            return false;
        }
    };
    let map_memory: FnMapMemory = match symbol(lib, "vkMapMemory") {
        Some(f) => f,
        None => {
            line("compute", "FAIL", "vkMapMemory missing");
            return false;
        }
    };
    let unmap_memory: FnUnmapMemory = match symbol(lib, "vkUnmapMemory") {
        Some(f) => f,
        None => {
            line("compute", "FAIL", "vkUnmapMemory missing");
            return false;
        }
    };

    let words: Vec<u32> = COMPUTE_SPV
        .chunks_exact(4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect();
    let shader_info = VkShaderModuleCreateInfo {
        s_type: VK_STRUCTURE_TYPE_SHADER_MODULE_CREATE_INFO,
        p_next: ptr::null(),
        flags: 0,
        code_size: words.len() * 4,
        p_code: words.as_ptr(),
    };
    let mut shader: VkShaderModule = ptr::null_mut();
    let r = create_shader(device, &shader_info, ptr::null(), &mut shader);
    if r != VK_SUCCESS {
        line("compute", "FAIL", format!("shader {}", api_result(r)));
        return false;
    }
    line("compute", "PASS", format!("shader words={}", words.len()));

    let binding = VkDescriptorSetLayoutBinding {
        binding: 0,
        descriptor_type: VK_DESCRIPTOR_TYPE_STORAGE_BUFFER,
        descriptor_count: 1,
        stage_flags: VK_SHADER_STAGE_COMPUTE_BIT,
        p_immutable_samplers: ptr::null(),
    };
    let set_layout_info = VkDescriptorSetLayoutCreateInfo {
        s_type: VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_CREATE_INFO,
        p_next: ptr::null(),
        flags: 0,
        binding_count: 1,
        p_bindings: &binding,
    };
    let mut set_layout: VkDescriptorSetLayout = ptr::null_mut();
    let r = create_set_layout(device, &set_layout_info, ptr::null(), &mut set_layout);
    if r != VK_SUCCESS {
        line("compute", "FAIL", format!("set_layout {}", api_result(r)));
        destroy_shader(device, shader, ptr::null());
        return false;
    }

    let pipeline_layout_info = VkPipelineLayoutCreateInfo {
        s_type: VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO,
        p_next: ptr::null(),
        flags: 0,
        set_layout_count: 1,
        p_set_layouts: &set_layout,
        push_constant_range_count: 0,
        p_push_constant_ranges: ptr::null(),
    };
    let mut pipeline_layout: VkPipelineLayout = ptr::null_mut();
    let r = create_pipeline_layout(
        device,
        &pipeline_layout_info,
        ptr::null(),
        &mut pipeline_layout,
    );
    if r != VK_SUCCESS {
        line(
            "compute",
            "FAIL",
            format!("pipeline_layout {}", api_result(r)),
        );
        destroy_set_layout(device, set_layout, ptr::null());
        destroy_shader(device, shader, ptr::null());
        return false;
    }

    let main_name = CString::new("main").unwrap();
    let stage = VkPipelineShaderStageCreateInfo {
        s_type: VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO,
        p_next: ptr::null(),
        flags: 0,
        stage: VK_SHADER_STAGE_COMPUTE_BIT,
        module: shader,
        p_name: main_name.as_ptr(),
        p_specialization_info: ptr::null(),
    };
    let pipeline_info = VkComputePipelineCreateInfo {
        s_type: VK_STRUCTURE_TYPE_COMPUTE_PIPELINE_CREATE_INFO,
        p_next: ptr::null(),
        flags: 0,
        stage,
        layout: pipeline_layout,
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
        line("compute", "FAIL", format!("pipeline {}", api_result(r)));
        destroy_pipeline_layout(device, pipeline_layout, ptr::null());
        destroy_set_layout(device, set_layout, ptr::null());
        destroy_shader(device, shader, ptr::null());
        return false;
    }
    line("compute", "PASS", "pipeline");

    let pool_size = VkDescriptorPoolSize {
        ty: VK_DESCRIPTOR_TYPE_STORAGE_BUFFER,
        descriptor_count: 1,
    };
    let pool_info = VkDescriptorPoolCreateInfo {
        s_type: VK_STRUCTURE_TYPE_DESCRIPTOR_POOL_CREATE_INFO,
        p_next: ptr::null(),
        flags: 0,
        max_sets: 1,
        pool_size_count: 1,
        p_pool_sizes: &pool_size,
    };
    let mut descriptor_pool: VkDescriptorPool = ptr::null_mut();
    let r = create_pool(device, &pool_info, ptr::null(), &mut descriptor_pool);
    if r != VK_SUCCESS {
        line(
            "compute",
            "FAIL",
            format!("descriptor_pool {}", api_result(r)),
        );
        destroy_pipeline(device, pipeline, ptr::null());
        destroy_pipeline_layout(device, pipeline_layout, ptr::null());
        destroy_set_layout(device, set_layout, ptr::null());
        destroy_shader(device, shader, ptr::null());
        return false;
    }
    let alloc_info = VkDescriptorSetAllocateInfo {
        s_type: VK_STRUCTURE_TYPE_DESCRIPTOR_SET_ALLOCATE_INFO,
        p_next: ptr::null(),
        descriptor_pool,
        descriptor_set_count: 1,
        p_set_layouts: &set_layout,
    };
    let mut descriptor_set: VkDescriptorSet = ptr::null_mut();
    let r = allocate_sets(device, &alloc_info, &mut descriptor_set);
    if r != VK_SUCCESS {
        line(
            "compute",
            "FAIL",
            format!("descriptor_set {}", api_result(r)),
        );
        destroy_pool(device, descriptor_pool, ptr::null());
        destroy_pipeline(device, pipeline, ptr::null());
        destroy_pipeline_layout(device, pipeline_layout, ptr::null());
        destroy_set_layout(device, set_layout, ptr::null());
        destroy_shader(device, shader, ptr::null());
        return false;
    }
    let buffer_info = VkDescriptorBufferInfo {
        buffer,
        offset: 0,
        range: 4096,
    };
    let write = VkWriteDescriptorSet {
        s_type: VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET,
        p_next: ptr::null(),
        dst_set: descriptor_set,
        dst_binding: 0,
        dst_array_element: 0,
        descriptor_count: 1,
        descriptor_type: VK_DESCRIPTOR_TYPE_STORAGE_BUFFER,
        p_image_info: ptr::null(),
        p_buffer_info: &buffer_info,
        p_texel_buffer_view: ptr::null(),
    };
    update_sets(device, 1, &write, 0, ptr::null());
    line("compute", "PASS", "descriptor");

    let mut mapped: *mut c_void = ptr::null_mut();
    let r = map_memory(device, memory, 0, 4096, 0, &mut mapped);
    if r != VK_SUCCESS || mapped.is_null() {
        line(
            "compute",
            "FAIL",
            format!("map before submit {}", api_result(r)),
        );
        destroy_pool(device, descriptor_pool, ptr::null());
        destroy_pipeline(device, pipeline, ptr::null());
        destroy_pipeline_layout(device, pipeline_layout, ptr::null());
        destroy_set_layout(device, set_layout, ptr::null());
        destroy_shader(device, shader, ptr::null());
        return false;
    }
    *(mapped as *mut u32) = 0;
    unmap_memory(device, memory);

    let command_pool_info = VkCommandPoolCreateInfo {
        s_type: VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO,
        p_next: ptr::null(),
        flags: 0,
        queue_family_index,
    };
    let mut command_pool: VkCommandPool = ptr::null_mut();
    let r = create_command_pool(device, &command_pool_info, ptr::null(), &mut command_pool);
    if r != VK_SUCCESS {
        line("compute", "FAIL", format!("command_pool {}", api_result(r)));
        destroy_pool(device, descriptor_pool, ptr::null());
        destroy_pipeline(device, pipeline, ptr::null());
        destroy_pipeline_layout(device, pipeline_layout, ptr::null());
        destroy_set_layout(device, set_layout, ptr::null());
        destroy_shader(device, shader, ptr::null());
        return false;
    }
    let command_alloc_info = VkCommandBufferAllocateInfo {
        s_type: VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO,
        p_next: ptr::null(),
        command_pool,
        level: VK_COMMAND_BUFFER_LEVEL_PRIMARY,
        command_buffer_count: 1,
    };
    let mut command_buffer: VkCommandBuffer = ptr::null_mut();
    let r = allocate_commands(device, &command_alloc_info, &mut command_buffer);
    if r != VK_SUCCESS {
        line(
            "compute",
            "FAIL",
            format!("command_buffer {}", api_result(r)),
        );
        destroy_command_pool(device, command_pool, ptr::null());
        destroy_pool(device, descriptor_pool, ptr::null());
        destroy_pipeline(device, pipeline, ptr::null());
        destroy_pipeline_layout(device, pipeline_layout, ptr::null());
        destroy_set_layout(device, set_layout, ptr::null());
        destroy_shader(device, shader, ptr::null());
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
        line("compute", "FAIL", format!("begin {}", api_result(r)));
        destroy_command_pool(device, command_pool, ptr::null());
        destroy_pool(device, descriptor_pool, ptr::null());
        destroy_pipeline(device, pipeline, ptr::null());
        destroy_pipeline_layout(device, pipeline_layout, ptr::null());
        destroy_set_layout(device, set_layout, ptr::null());
        destroy_shader(device, shader, ptr::null());
        return false;
    }
    bind_pipeline(command_buffer, VK_PIPELINE_BIND_POINT_COMPUTE, pipeline);
    bind_sets(
        command_buffer,
        VK_PIPELINE_BIND_POINT_COMPUTE,
        pipeline_layout,
        0,
        1,
        &descriptor_set,
        0,
        ptr::null(),
    );
    dispatch(command_buffer, 1, 1, 1);
    let r = end_command(command_buffer);
    if r != VK_SUCCESS {
        line("compute", "FAIL", format!("end {}", api_result(r)));
        destroy_command_pool(device, command_pool, ptr::null());
        destroy_pool(device, descriptor_pool, ptr::null());
        destroy_pipeline(device, pipeline, ptr::null());
        destroy_pipeline_layout(device, pipeline_layout, ptr::null());
        destroy_set_layout(device, set_layout, ptr::null());
        destroy_shader(device, shader, ptr::null());
        return false;
    }
    line("compute", "PASS", "commands");

    let fence_info = VkFenceCreateInfo {
        s_type: VK_STRUCTURE_TYPE_FENCE_CREATE_INFO,
        p_next: ptr::null(),
        flags: 0,
    };
    let mut fence: VkFence = ptr::null_mut();
    let r = create_fence(device, &fence_info, ptr::null(), &mut fence);
    if r != VK_SUCCESS {
        line("compute", "FAIL", format!("fence {}", api_result(r)));
        destroy_command_pool(device, command_pool, ptr::null());
        destroy_pool(device, descriptor_pool, ptr::null());
        destroy_pipeline(device, pipeline, ptr::null());
        destroy_pipeline_layout(device, pipeline_layout, ptr::null());
        destroy_set_layout(device, set_layout, ptr::null());
        destroy_shader(device, shader, ptr::null());
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
        line("compute", "FAIL", format!("queue_submit {}", api_result(r)));
        destroy_fence(device, fence, ptr::null());
        destroy_command_pool(device, command_pool, ptr::null());
        destroy_pool(device, descriptor_pool, ptr::null());
        destroy_pipeline(device, pipeline, ptr::null());
        destroy_pipeline_layout(device, pipeline_layout, ptr::null());
        destroy_set_layout(device, set_layout, ptr::null());
        destroy_shader(device, shader, ptr::null());
        return false;
    }
    let r = wait_fences(device, 1, &fence, VK_TRUE, 5_000_000_000);
    if r != VK_SUCCESS {
        line("compute", "FAIL", format!("wait_fence {}", api_result(r)));
        destroy_fence(device, fence, ptr::null());
        destroy_command_pool(device, command_pool, ptr::null());
        destroy_pool(device, descriptor_pool, ptr::null());
        destroy_pipeline(device, pipeline, ptr::null());
        destroy_pipeline_layout(device, pipeline_layout, ptr::null());
        destroy_set_layout(device, set_layout, ptr::null());
        destroy_shader(device, shader, ptr::null());
        return false;
    }
    line("compute", "PASS", "queue_submit+fence");

    let mut mapped: *mut c_void = ptr::null_mut();
    let r = map_memory(device, memory, 0, 4096, 0, &mut mapped);
    if r == VK_SUCCESS && !mapped.is_null() {
        let readback = *(mapped as *const u32);
        unmap_memory(device, memory);
        line(
            "compute",
            if readback == 0x4855_534b {
                "PASS"
            } else {
                "FAIL"
            },
            format!("readback={readback:#x}"),
        );
        let ok = readback == 0x4855_534b;
        destroy_fence(device, fence, ptr::null());
        destroy_command_pool(device, command_pool, ptr::null());
        destroy_pool(device, descriptor_pool, ptr::null());
        destroy_pipeline(device, pipeline, ptr::null());
        destroy_pipeline_layout(device, pipeline_layout, ptr::null());
        destroy_set_layout(device, set_layout, ptr::null());
        destroy_shader(device, shader, ptr::null());
        return ok;
    }
    line(
        "compute",
        "FAIL",
        format!("map after submit {}", api_result(r)),
    );
    destroy_fence(device, fence, ptr::null());
    destroy_command_pool(device, command_pool, ptr::null());
    destroy_pool(device, descriptor_pool, ptr::null());
    destroy_pipeline(device, pipeline, ptr::null());
    destroy_pipeline_layout(device, pipeline_layout, ptr::null());
    destroy_set_layout(device, set_layout, ptr::null());
    destroy_shader(device, shader, ptr::null());
    false
}

unsafe fn run() -> i32 {
    let lib_name = CString::new("libvulkan.so.1").unwrap();
    let lib = dlopen(lib_name.as_ptr(), RTLD_NOW);
    if lib.is_null() {
        line("loader", "FAIL", "dlopen(libvulkan.so.1) returned null");
        return 1;
    }
    line("loader", "PASS", "libvulkan.so.1");

    let create_instance: FnCreateInstance = match symbol(lib, "vkCreateInstance") {
        Some(f) => f,
        None => {
            line("symbols", "FAIL", "vkCreateInstance missing");
            dlclose(lib);
            return 1;
        }
    };
    let destroy_instance: FnDestroyInstance = match symbol(lib, "vkDestroyInstance") {
        Some(f) => f,
        None => {
            line("symbols", "FAIL", "vkDestroyInstance missing");
            dlclose(lib);
            return 1;
        }
    };
    let enumerate_physical: FnEnumeratePhysicalDevices =
        match symbol(lib, "vkEnumeratePhysicalDevices") {
            Some(f) => f,
            None => {
                line("symbols", "FAIL", "vkEnumeratePhysicalDevices missing");
                dlclose(lib);
                return 1;
            }
        };
    let get_props: FnGetPhysicalDeviceProperties =
        match symbol(lib, "vkGetPhysicalDeviceProperties") {
            Some(f) => f,
            None => {
                line("symbols", "FAIL", "vkGetPhysicalDeviceProperties missing");
                dlclose(lib);
                return 1;
            }
        };
    let get_queue_families: FnGetPhysicalDeviceQueueFamilyProperties =
        match symbol(lib, "vkGetPhysicalDeviceQueueFamilyProperties") {
            Some(f) => f,
            None => {
                line(
                    "symbols",
                    "FAIL",
                    "vkGetPhysicalDeviceQueueFamilyProperties missing",
                );
                dlclose(lib);
                return 1;
            }
        };
    let get_memory_props: FnGetPhysicalDeviceMemoryProperties =
        match symbol(lib, "vkGetPhysicalDeviceMemoryProperties") {
            Some(f) => f,
            None => {
                line(
                    "symbols",
                    "FAIL",
                    "vkGetPhysicalDeviceMemoryProperties missing",
                );
                dlclose(lib);
                return 1;
            }
        };
    let create_device: FnCreateDevice = match symbol(lib, "vkCreateDevice") {
        Some(f) => f,
        None => {
            line("symbols", "FAIL", "vkCreateDevice missing");
            dlclose(lib);
            return 1;
        }
    };
    let destroy_device: FnDestroyDevice = match symbol(lib, "vkDestroyDevice") {
        Some(f) => f,
        None => {
            line("symbols", "FAIL", "vkDestroyDevice missing");
            dlclose(lib);
            return 1;
        }
    };
    let get_device_queue: FnGetDeviceQueue = match symbol(lib, "vkGetDeviceQueue") {
        Some(f) => f,
        None => {
            line("symbols", "FAIL", "vkGetDeviceQueue missing");
            dlclose(lib);
            return 1;
        }
    };
    let create_buffer: FnCreateBuffer = match symbol(lib, "vkCreateBuffer") {
        Some(f) => f,
        None => {
            line("symbols", "FAIL", "vkCreateBuffer missing");
            dlclose(lib);
            return 1;
        }
    };
    let destroy_buffer: FnDestroyBuffer = match symbol(lib, "vkDestroyBuffer") {
        Some(f) => f,
        None => {
            line("symbols", "FAIL", "vkDestroyBuffer missing");
            dlclose(lib);
            return 1;
        }
    };
    let get_buffer_requirements: FnGetBufferMemoryRequirements =
        match symbol(lib, "vkGetBufferMemoryRequirements") {
            Some(f) => f,
            None => {
                line("symbols", "FAIL", "vkGetBufferMemoryRequirements missing");
                dlclose(lib);
                return 1;
            }
        };
    let allocate_memory: FnAllocateMemory = match symbol(lib, "vkAllocateMemory") {
        Some(f) => f,
        None => {
            line("symbols", "FAIL", "vkAllocateMemory missing");
            dlclose(lib);
            return 1;
        }
    };
    let free_memory: FnFreeMemory = match symbol(lib, "vkFreeMemory") {
        Some(f) => f,
        None => {
            line("symbols", "FAIL", "vkFreeMemory missing");
            dlclose(lib);
            return 1;
        }
    };
    let bind_buffer: FnBindBufferMemory = match symbol(lib, "vkBindBufferMemory") {
        Some(f) => f,
        None => {
            line("symbols", "FAIL", "vkBindBufferMemory missing");
            dlclose(lib);
            return 1;
        }
    };
    let map_memory: FnMapMemory = match symbol(lib, "vkMapMemory") {
        Some(f) => f,
        None => {
            line("symbols", "FAIL", "vkMapMemory missing");
            dlclose(lib);
            return 1;
        }
    };
    let unmap_memory: FnUnmapMemory = match symbol(lib, "vkUnmapMemory") {
        Some(f) => f,
        None => {
            line("symbols", "FAIL", "vkUnmapMemory missing");
            dlclose(lib);
            return 1;
        }
    };
    line("symbols", "PASS", "core entrypoints resolved");

    let app_name = CString::new("husk-vulkan-probe").unwrap();
    let engine_name = CString::new("husk").unwrap();
    let app = VkApplicationInfo {
        s_type: VK_STRUCTURE_TYPE_APPLICATION_INFO,
        p_next: ptr::null(),
        p_application_name: app_name.as_ptr(),
        application_version: 1,
        p_engine_name: engine_name.as_ptr(),
        engine_version: 1,
        api_version: VK_API_VERSION_1_3,
    };
    let instance_info = VkInstanceCreateInfo {
        s_type: VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO,
        p_next: ptr::null(),
        flags: 0,
        p_application_info: &app,
        enabled_layer_count: 0,
        pp_enabled_layer_names: ptr::null(),
        enabled_extension_count: 0,
        pp_enabled_extension_names: ptr::null(),
    };
    let mut instance: VkInstance = ptr::null_mut();
    let r = create_instance(&instance_info, ptr::null(), &mut instance);
    if r != VK_SUCCESS {
        line("instance", "FAIL", api_result(r));
        dlclose(lib);
        return 1;
    }
    line("instance", "PASS", api_result(r));

    let mut physical_count: u32 = 0;
    let r = enumerate_physical(instance, &mut physical_count, ptr::null_mut());
    if r != VK_SUCCESS || physical_count == 0 {
        line(
            "physical",
            "FAIL",
            format!("count={physical_count} {}", api_result(r)),
        );
        destroy_instance(instance, ptr::null());
        dlclose(lib);
        return 1;
    }
    let mut physical = vec![ptr::null_mut(); physical_count as usize];
    let r = enumerate_physical(instance, &mut physical_count, physical.as_mut_ptr());
    if r != VK_SUCCESS {
        line("physical", "FAIL", format!("second {}", api_result(r)));
        destroy_instance(instance, ptr::null());
        dlclose(lib);
        return 1;
    }
    let dev = physical[0];
    let mut props = AlignedProperties([0u8; 1024]);
    get_props(dev, props.0.as_mut_ptr() as *mut c_void);
    let name_bytes = &props.0[20..20 + 256];
    let end = name_bytes.iter().position(|&b| b == 0).unwrap_or(256);
    let name = String::from_utf8_lossy(&name_bytes[..end]).into_owned();
    line(
        "physical",
        "PASS",
        format!("count={physical_count} name={name}"),
    );
    report_extensions(lib, instance, dev);

    let mut family_count: u32 = 0;
    get_queue_families(dev, &mut family_count, ptr::null_mut());
    if family_count == 0 {
        line("queue_family", "FAIL", "count=0");
        destroy_instance(instance, ptr::null());
        dlclose(lib);
        return 1;
    }
    let mut families = vec![
        VkQueueFamilyProperties {
            queue_flags: 0,
            queue_count: 0,
            timestamp_valid_bits: 0,
            min_image_transfer_granularity: [0; 3],
        };
        family_count as usize
    ];
    get_queue_families(dev, &mut family_count, families.as_mut_ptr());
    let family_index = match families.iter().position(|f| {
        f.queue_count > 0
            && (f.queue_flags
                & (VK_QUEUE_GRAPHICS_BIT | VK_QUEUE_COMPUTE_BIT | VK_QUEUE_TRANSFER_BIT))
                != 0
    }) {
        Some(i) => i as u32,
        None => {
            line("queue_family", "FAIL", "no usable queue family");
            destroy_instance(instance, ptr::null());
            dlclose(lib);
            return 1;
        }
    };
    line(
        "queue_family",
        "PASS",
        format!(
            "index={family_index} flags={:#x} queues={}",
            families[family_index as usize].queue_flags,
            families[family_index as usize].queue_count
        ),
    );

    let priority: f32 = 1.0;
    let queue_info = VkDeviceQueueCreateInfo {
        s_type: VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO,
        p_next: ptr::null(),
        flags: 0,
        queue_family_index: family_index,
        queue_count: 1,
        p_queue_priorities: &priority,
    };
    let device_info = VkDeviceCreateInfo {
        s_type: VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO,
        p_next: ptr::null(),
        flags: 0,
        queue_create_info_count: 1,
        p_queue_create_infos: &queue_info,
        enabled_layer_count: 0,
        pp_enabled_layer_names: ptr::null(),
        enabled_extension_count: 0,
        pp_enabled_extension_names: ptr::null(),
        p_enabled_features: ptr::null(),
    };
    let mut device: VkDevice = ptr::null_mut();
    let r = create_device(dev, &device_info, ptr::null(), &mut device);
    if r != VK_SUCCESS {
        line("device", "FAIL", api_result(r));
        destroy_instance(instance, ptr::null());
        dlclose(lib);
        return 1;
    }
    let mut queue: VkQueue = ptr::null_mut();
    get_device_queue(device, family_index, 0, &mut queue);
    line(
        "device",
        "PASS",
        format!("queue={queue:p} {}", api_result(r)),
    );

    let mut mem_props: VkPhysicalDeviceMemoryProperties = std::mem::zeroed();
    get_memory_props(dev, &mut mem_props);
    line(
        "memory_types",
        "PASS",
        format!("count={}", mem_props.memory_type_count),
    );

    let buffer_info = VkBufferCreateInfo {
        s_type: VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO,
        p_next: ptr::null(),
        flags: 0,
        size: 4096,
        usage: VK_BUFFER_USAGE_TRANSFER_SRC_BIT
            | VK_BUFFER_USAGE_TRANSFER_DST_BIT
            | VK_BUFFER_USAGE_STORAGE_BUFFER_BIT,
        sharing_mode: VK_SHARING_MODE_EXCLUSIVE,
        queue_family_index_count: 0,
        p_queue_family_indices: ptr::null(),
    };
    let mut buffer: VkBuffer = ptr::null_mut();
    let r = create_buffer(device, &buffer_info, ptr::null(), &mut buffer);
    if r != VK_SUCCESS {
        line("buffer", "FAIL", api_result(r));
        destroy_device(device, ptr::null());
        destroy_instance(instance, ptr::null());
        dlclose(lib);
        return 1;
    }
    let mut req = VkMemoryRequirements::default();
    get_buffer_requirements(device, buffer, &mut req);
    let Some(memory_type_index) = choose_memory_type(&mem_props, req.memory_type_bits) else {
        line(
            "buffer",
            "FAIL",
            format!(
                "size={} bits={:#x} no memory type",
                req.size, req.memory_type_bits
            ),
        );
        destroy_buffer(device, buffer, ptr::null());
        destroy_device(device, ptr::null());
        destroy_instance(instance, ptr::null());
        dlclose(lib);
        return 1;
    };
    let alloc_info = VkMemoryAllocateInfo {
        s_type: VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO,
        p_next: ptr::null(),
        allocation_size: req.size,
        memory_type_index,
    };
    let mut memory: VkDeviceMemory = ptr::null_mut();
    let r = allocate_memory(device, &alloc_info, ptr::null(), &mut memory);
    if r != VK_SUCCESS {
        line(
            "buffer",
            "FAIL",
            format!(
                "alloc size={} type={} {}",
                req.size,
                memory_type_index,
                api_result(r)
            ),
        );
        destroy_buffer(device, buffer, ptr::null());
        destroy_device(device, ptr::null());
        destroy_instance(instance, ptr::null());
        dlclose(lib);
        return 1;
    }
    let r = bind_buffer(device, buffer, memory, 0);
    if r != VK_SUCCESS {
        line("buffer", "FAIL", format!("bind {}", api_result(r)));
        free_memory(device, memory, ptr::null());
        destroy_buffer(device, buffer, ptr::null());
        destroy_device(device, ptr::null());
        destroy_instance(instance, ptr::null());
        dlclose(lib);
        return 1;
    }
    let mut mapped: *mut c_void = ptr::null_mut();
    let r = map_memory(device, memory, 0, 4096, 0, &mut mapped);
    let map_ok = r == VK_SUCCESS && !mapped.is_null();
    if !map_ok {
        line(
            "buffer",
            "FAIL",
            format!(
                "created/bound size={} type={} map={}",
                req.size,
                memory_type_index,
                api_result(r)
            ),
        );
    } else {
        unsafe { *(mapped as *mut u32) = 0x4855_534b };
        let read = unsafe { *(mapped as *const u32) };
        unmap_memory(device, memory);
        line(
            "buffer",
            if read == 0x4855_534b { "PASS" } else { "FAIL" },
            format!(
                "created/bound/mapped size={} type={} readback={:#x}",
                req.size, memory_type_index, read
            ),
        );
    }

    let compute_ok = map_ok && submit_compute(lib, device, queue, family_index, buffer, memory);
    let graphics_ok = compute_ok && graphics::run_graphics(lib, dev, device, queue, family_index);

    free_memory(device, memory, ptr::null());
    destroy_buffer(device, buffer, ptr::null());
    destroy_device(device, ptr::null());
    destroy_instance(instance, ptr::null());
    dlclose(lib);
    line("teardown", "PASS", "device/instance/loader released");
    if compute_ok && graphics_ok {
        0
    } else {
        1
    }
}

fn main() {
    let rc = unsafe { run() };
    std::process::exit(rc);
}
