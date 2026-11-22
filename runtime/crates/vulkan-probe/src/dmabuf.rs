//! Receive an AHardwareBuffer native handle from the Husk app and import the
//! contained dma-buf into PanVK as external buffer memory.
//!
//! The host side sends through AHardwareBuffer_sendHandleToUnixSocket(), so the
//! guest receives a normal SCM_RIGHTS message. We intentionally do not try to
//! reconstruct the Android native_handle_t: the only object PanVK needs is the
//! dma-buf descriptor, which we locate by inspecting /proc/self/fd.

use super::*;
use core::ffi::{c_int, c_void};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;

const SOL_SOCKET: c_int = 1;
const SCM_RIGHTS: c_int = 1;
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

extern "C" {
    fn recvmsg(fd: c_int, msg: *mut MsgHdr, flags: c_int) -> isize;
    fn close(fd: c_int) -> c_int;
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

unsafe fn receive_fds(path: &str) -> Result<Vec<c_int>, String> {
    let stream = UnixStream::connect(path).map_err(|e| format!("connect {path}: {e}"))?;
    line("dmabuf", "socket", format!("PASS|{path}"));

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
    line("dmabuf", "fd_count", format!("{}", fds.len()));
    Ok(fds)
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

pub unsafe fn run(
    lib: *mut c_void,
    physical: VkPhysicalDevice,
    device: VkDevice,
    queue: VkQueue,
    family_index: u32,
    path: &str,
) -> bool {
    let fds = match receive_fds(path) {
        Ok(v) => v,
        Err(e) => {
            line("dmabuf", "FAIL", e);
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

    let mut import = VkImportMemoryFdInfo {
        s_type: VK_STRUCTURE_TYPE_IMPORT_MEMORY_FD_INFO_KHR,
        p_next: ptr::null(),
        handle_type: VK_EXTERNAL_MEMORY_HANDLE_TYPE_DMA_BUF_BIT_EXT,
        fd,
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
        destroy_buffer(device, buffer, ptr::null());
        for &fd in &fds {
            close(fd);
        }
        return false;
    }

    let r = bind_buffer(device, buffer, memory, 0);
    line("dmabuf", "bind", api_result(r));
    let gpu_ok =
        r == VK_SUCCESS && submit_compute(lib, device, queue, family_index, buffer, memory);
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
    for &fd in &fds {
        close(fd);
    }
    line("dmabuf", "teardown", "memory/buffer/fds released");
    r == VK_SUCCESS && gpu_ok
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
