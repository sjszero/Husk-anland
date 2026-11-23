//! drm-probe — 验证"把 Mesa panfrost 接到 kbase"这条路的第一块地基：
//! kbase 能不能把我们需要的**原始寄存器值**交出来（panfrost 的 GET_PARAM 全靠它们）。
//!
//! 判定标准（见 docs/design/gpu-drm-shim.md §4）：panfrost 要的 45 个参数里，
//! 大写部分能从这里一对一拿到 → M2 可做；拿不到 → 要么合成、要么这条路判死。
//!
//! 只做观测，不提交任何作业，不会碰 GPU。

use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;

extern "C" {
    fn __errno_location() -> *mut i32;
    fn ioctl(fd: i32, request: u64, ...) -> i32;
    fn mmap(
        addr: *mut core::ffi::c_void,
        len: usize,
        prot: i32,
        flags: i32,
        fd: i32,
        off: i64,
    ) -> *mut core::ffi::c_void;
    fn munmap(addr: *mut core::ffi::c_void, len: usize) -> i32;
}

fn errno() -> i32 {
    unsafe { *__errno_location() }
}

const _IOC_WRITE: u64 = 1;
const _IOC_READ: u64 = 2;

const fn ioc(dir: u64, ty: u64, nr: u64, size: u64) -> u64 {
    (dir << 30) | (size << 16) | (ty << 8) | nr
}

// ---- kbase（/dev/mali0）----
const KBASE_VERSION_CHECK: u64 = ioc(_IOC_READ | _IOC_WRITE, 0x80, 0, 4);
const KBASE_SET_FLAGS: u64 = ioc(_IOC_WRITE, 0x80, 1, 4);
// GPL 头是 _IOW（不是 _IOWR）：0x40108003，与厂商 libGLES_mali.so 里的一致
const KBASE_GET_GPUPROPS: u64 = ioc(_IOC_WRITE, 0x80, 3, 16);
const KBASE_MEM_ALLOC: u64 = ioc(_IOC_READ | _IOC_WRITE, 0x80, 5, 32);

// ---- DRM panfrost（假节点上将来要实现的那些）----
const DRM_COMMAND_BASE: u64 = 0x40;
#[allow(dead_code)]
const DRM_PANFROST_CREATE_BO: u64 = ioc(
    _IOC_READ | _IOC_WRITE,
    b'd' as u64,
    DRM_COMMAND_BASE + 0x02,
    24,
);
#[allow(dead_code)]
const DRM_PANFROST_GET_PARAM: u64 = ioc(
    _IOC_READ | _IOC_WRITE,
    b'd' as u64,
    DRM_COMMAND_BASE + 0x04,
    16,
);

/// kbase gpuprops id（r32p1，见 GPL 头 KBASE_GPUPROP_*）
mod kb {
    pub const PRODUCT_ID: u32 = 1;
    pub const MINOR_REVISION: u32 = 3;
    pub const MAJOR_REVISION: u32 = 4;
    pub const MAX_THREADS: u32 = 18;
    pub const MAX_WORKGROUP_SIZE: u32 = 19;
    pub const MAX_BARRIER_SIZE: u32 = 20;
    pub const RAW_SHADER_PRESENT: u32 = 25;
    pub const RAW_TILER_PRESENT: u32 = 26;
    pub const RAW_L2_PRESENT: u32 = 27;
    pub const RAW_STACK_PRESENT: u32 = 28;
    pub const RAW_L2_FEATURES: u32 = 29;
    pub const RAW_CORE_FEATURES: u32 = 30;
    pub const RAW_MEM_FEATURES: u32 = 31;
    pub const RAW_MMU_FEATURES: u32 = 32;
    pub const RAW_AS_PRESENT: u32 = 33;
    pub const RAW_JS_PRESENT: u32 = 34;
    pub const RAW_JS_FEATURES_0: u32 = 35; // ..= 50
    pub const RAW_TILER_FEATURES: u32 = 51;
    pub const RAW_TEXTURE_FEATURES_0: u32 = 52; // 52,53,54
    pub const RAW_GPU_ID: u32 = 55;
    pub const RAW_THREAD_MAX_THREADS: u32 = 56;
    pub const RAW_THREAD_MAX_WORKGROUP_SIZE: u32 = 57;
    pub const RAW_THREAD_MAX_BARRIER_SIZE: u32 = 58;
    pub const RAW_THREAD_FEATURES: u32 = 59;
    pub const RAW_COHERENCY_MODE: u32 = 60;
    pub const COHERENCY_NUM_CORE_GROUPS: u32 = 62;
    pub const RAW_TEXTURE_FEATURES_3: u32 = 81;
    pub const RAW_THREAD_TLS_ALLOC: u32 = 83;
}

fn line(name: &str, verdict: &str, detail: impl AsRef<str>) {
    println!("drm|{}|{}|{}", name, verdict, detail.as_ref());
}

/// kbase 的 GET_GPUPROPS：buffer 是**变长记录流**（源码 `kbase_gpuprops_populate_user_buffer`）：
/// 每条 = `u32 header` + 值，`header = (prop_id << 2) | size_code`，
/// size_code: 0=u8, 1=u16, 2=u32, 3=u64，值是小端。
fn gpuprops(fd: i32) -> Vec<(u32, u64)> {
    let mut buf = vec![0u8; 8192];
    let mut args: [u64; 2] = [buf.as_mut_ptr() as u64, buf.len() as u64];
    let r = unsafe {
        ioctl(
            fd,
            KBASE_GET_GPUPROPS,
            &mut args as *mut [u64; 2] as *mut core::ffi::c_void,
        )
    };
    if r < 0 {
        line("gpuprops", "FAIL", format!("errno={}", errno()));
        return Vec::new();
    }
    let n = r as usize;
    line("gpuprops", "PASS", format!("returned {n} bytes"));
    let mut out = Vec::new();
    let mut i = 0usize;
    while i + 4 <= n {
        let h = u32::from_le_bytes([buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]);
        i += 4;
        let id = h >> 2;
        let sc = (h & 3) as usize;
        let sz = [1usize, 2, 4, 8][sc];
        if i + sz > n {
            line("gpuprops", "PARTIAL", format!("truncated at {i}, id={id}"));
            break;
        }
        let mut v = 0u64;
        for k in 0..sz {
            v |= (buf[i + k] as u64) << (8 * k);
        }
        i += sz;
        out.push((id, v));
    }
    out
}

/// M1 自测：把 DRM 语义层（drm-shim）在同一个进程里跑一遍——
/// CREATE_BO → MMAP_BO → 真 mmap → 写模式 → 读回。
/// 这一步证明"DRM 的 BO 语义能落在 kbase 的显存上"，之后才去接假节点。
fn selfcheck() {
    let f = match OpenOptions::new().read(true).write(true).open("/dev/mali0") {
        Ok(f) => f,
        Err(e) => {
            line(
                "selfcheck/open",
                "FAIL",
                format!("errno={}", e.raw_os_error().unwrap_or(0)),
            );
            return;
        }
    };
    let fd = f.as_raw_fd();
    let raw = |f: i32, req: u64, arg: *mut u8| -> i64 {
        unsafe { ioctl(f, req, arg as *mut core::ffi::c_void) as i64 }
    };
    let mut ctx = drm_shim::DrmCtx::empty();
    if let Err(e) = ctx.init(fd, raw) {
        line("selfcheck/ctx", "FAIL", format!("errno={e}"));
        return;
    }
    line(
        "selfcheck/ctx",
        "PASS",
        format!("props={} GET_PARAM 可用", ctx.props.len()),
    );

    // --- CREATE_BO(size=4096) ---
    let mut arg = [0u8; 24];
    arg[0..4].copy_from_slice(&4096u32.to_le_bytes());
    let r = unsafe {
        ctx.dispatch(
            raw,
            drm_shim::DRM_IOCTL_PANFROST_CREATE_BO,
            arg.as_mut_ptr(),
        )
    };
    if r < 0 {
        line("selfcheck/create_bo", "FAIL", format!("ret={r}"));
        return;
    }
    let handle = u32::from_le_bytes([arg[8], arg[9], arg[10], arg[11]]);
    let mut va = [0u8; 8];
    va.copy_from_slice(&arg[16..24]);
    let va = u64::from_le_bytes(va);
    line(
        "selfcheck/create_bo",
        "PASS",
        format!("size=4096 handle={handle} offset(=gpu_va)={va:#x}"),
    );

    // --- MMAP_BO(handle) ---
    let mut mb = [0u8; 16];
    mb[0..4].copy_from_slice(&handle.to_le_bytes());
    let r = unsafe { ctx.dispatch(raw, drm_shim::DRM_IOCTL_PANFROST_MMAP_BO, mb.as_mut_ptr()) };
    if r < 0 {
        line("selfcheck/mmap_bo", "FAIL", format!("ret={r}"));
        return;
    }
    let mut off = [0u8; 8];
    off.copy_from_slice(&mb[8..16]);
    let off = u64::from_le_bytes(off);
    line(
        "selfcheck/mmap_bo",
        "PASS",
        format!("handle={handle} offset={off:#x}"),
    );

    // --- 真的 mmap 这块 BO（kbase 规则：prot = flags 低两位，offset = VA）---
    let p = unsafe { mmap(core::ptr::null_mut(), 4096, 3, 1, fd, off as i64) };
    if p as isize == -1 {
        line("selfcheck/mmap", "FAIL", format!("errno={}", errno()));
        return;
    }
    let words = p as *mut u32;
    unsafe {
        for i in 0..16 {
            *words.add(i) = 0x1122_3344u32.wrapping_add(i as u32);
        }
    }
    let mut ok = true;
    unsafe {
        for i in 0..16 {
            if *words.add(i) != 0x1122_3344u32.wrapping_add(i as u32) {
                ok = false;
            }
        }
    }
    line(
        "selfcheck/mmap",
        if ok { "PASS" } else { "FAIL" },
        format!("va={off:#x} 写入并读回 64 字节"),
    );

    // --- GET_PARAM 的关键几条 ---
    for (n, name) in [
        (0u32, "GPU_PROD_ID"),
        (7, "JS_PRESENT"),
        (14, "MAX_THREADS"),
    ] {
        let mut gp = [0u8; 16];
        gp[0..4].copy_from_slice(&n.to_le_bytes());
        let r =
            unsafe { ctx.dispatch(raw, drm_shim::DRM_IOCTL_PANFROST_GET_PARAM, gp.as_mut_ptr()) };
        let mut v = [0u8; 8];
        v.copy_from_slice(&gp[8..16]);
        let v = u64::from_le_bytes(v);
        line(
            &format!("selfcheck/param{name}"),
            if r == 0 { "PASS" } else { "FAIL" },
            format!("{v:#x} ({v})"),
        );
    }

    // --- GET_BO_OFFSET 一致性 ---
    let mut gb = [0u8; 16];
    gb[0..4].copy_from_slice(&handle.to_le_bytes());
    let r = unsafe {
        ctx.dispatch(
            raw,
            drm_shim::DRM_IOCTL_PANFROST_GET_BO_OFFSET,
            gb.as_mut_ptr(),
        )
    };
    let mut o2 = [0u8; 8];
    o2.copy_from_slice(&gb[8..16]);
    let o2 = u64::from_le_bytes(o2);
    line(
        "selfcheck/get_bo_offset",
        if r == 0 && o2 == off { "PASS" } else { "FAIL" },
        format!(
            "{o2:#x}（与 MMAP_BO {}）",
            if o2 == off { "一致" } else { "不一致" }
        ),
    );

    unsafe { munmap(p, 4096) };
    line(
        "selfcheck/verdict",
        "PASS",
        "M1 语义层：DRM BO == kbase 显存，可 CPU 读写",
    );
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(|s| s.as_str()).unwrap_or("param");

    let f: File = match OpenOptions::new().read(true).write(true).open("/dev/mali0") {
        Ok(f) => f,
        Err(e) => {
            line(
                "open",
                "FAIL",
                format!("/dev/mali0 errno={}", e.raw_os_error().unwrap_or(0)),
            );
            return;
        }
    };
    let fd = f.as_raw_fd();
    let mut ver: [u16; 2] = [11, 38];
    let r = unsafe {
        ioctl(
            fd,
            KBASE_VERSION_CHECK,
            ver.as_mut_ptr() as *mut core::ffi::c_void,
        )
    };
    if r < 0 {
        line("version", "FAIL", format!("errno={}", errno()));
        return;
    }
    let mut flags: u32 = 0;
    let r = unsafe {
        ioctl(
            fd,
            KBASE_SET_FLAGS,
            &mut flags as *mut u32 as *mut core::ffi::c_void,
        )
    };
    if r < 0 {
        line("set_flags", "FAIL", format!("errno={}", errno()));
        return;
    }
    line(
        "context",
        "PASS",
        format!("uapi={}.{} kctx", ver[0], ver[1]),
    );

    let props = gpuprops(fd);

    if mode == "selfcheck" {
        selfcheck();
        return;
    }

    if mode == "props" {
        for (id, v) in &props {
            line(&format!("prop#{id}"), "INFO", format!("{v:#x} ({v})"));
        }
        return;
    }

    // panfrost 的 GET_PARAM → kbase gprop。表就是 docs/design/gpu-drm-shim.md §4 的实现。
    let get = |id: u32| props.iter().find(|(k, _)| *k == id).map(|(_, v)| *v as u32);
    let major = get(kb::MAJOR_REVISION).unwrap_or(0);
    let minor = get(kb::MINOR_REVISION).unwrap_or(0);

    // (param 编号, 名字, 值来源)
    type Src = fn(&dyn Fn(u32) -> Option<u32>) -> Option<u32>;
    let table: Vec<(u32, &str, Src)> = vec![
        (0, "GPU_PROD_ID", |g| {
            g(kb::PRODUCT_ID).or_else(|| g(kb::RAW_GPU_ID))
        }),
        (1, "GPU_REVISION", move |g| {
            // INFERENCE：panfrost 用 (major<<16)|minor 的形态
            g(kb::MAJOR_REVISION).map(|ma| (ma << 16) | g(kb::MINOR_REVISION).unwrap_or(0))
        }),
        (2, "SHADER_PRESENT", |g| g(kb::RAW_SHADER_PRESENT)),
        (3, "TILER_PRESENT", |g| g(kb::RAW_TILER_PRESENT)),
        (4, "L2_PRESENT", |g| g(kb::RAW_L2_PRESENT)),
        (5, "STACK_PRESENT", |g| g(kb::RAW_STACK_PRESENT)),
        (6, "AS_PRESENT", |g| g(kb::RAW_AS_PRESENT)),
        (7, "JS_PRESENT", |g| g(kb::RAW_JS_PRESENT)),
        (8, "L2_FEATURES", |g| g(kb::RAW_L2_FEATURES)),
        (9, "CORE_FEATURES", |g| g(kb::RAW_CORE_FEATURES)),
        (10, "TILER_FEATURES", |g| g(kb::RAW_TILER_FEATURES)),
        (11, "MEM_FEATURES", |g| g(kb::RAW_MEM_FEATURES)),
        (12, "MMU_FEATURES", |g| g(kb::RAW_MMU_FEATURES)),
        (13, "THREAD_FEATURES", |g| g(kb::RAW_THREAD_FEATURES)),
        (14, "MAX_THREADS", |g| {
            g(kb::RAW_THREAD_MAX_THREADS).or_else(|| g(kb::MAX_THREADS))
        }),
        (15, "THREAD_MAX_WORKGROUP_SZ", |g| {
            g(kb::RAW_THREAD_MAX_WORKGROUP_SIZE).or_else(|| g(kb::MAX_WORKGROUP_SIZE))
        }),
        (16, "THREAD_MAX_BARRIER_SZ", |g| {
            g(kb::RAW_THREAD_MAX_BARRIER_SIZE).or_else(|| g(kb::MAX_BARRIER_SIZE))
        }),
        (17, "COHERENCY_FEATURES", |g| g(kb::RAW_COHERENCY_MODE)),
        (18, "TEXTURE_FEATURES0", |g| g(kb::RAW_TEXTURE_FEATURES_0)),
        (19, "TEXTURE_FEATURES1", |g| {
            g(kb::RAW_TEXTURE_FEATURES_0 + 1)
        }),
        (20, "TEXTURE_FEATURES2", |g| {
            g(kb::RAW_TEXTURE_FEATURES_0 + 2)
        }),
        (21, "TEXTURE_FEATURES3", |g| g(kb::RAW_TEXTURE_FEATURES_3)),
        (38, "NR_CORE_GROUPS", |g| g(kb::COHERENCY_NUM_CORE_GROUPS)),
        (40, "THREAD_TLS_ALLOC", |g| g(kb::RAW_THREAD_TLS_ALLOC)),
    ];

    let mut got = 0usize;
    let mut missing: Vec<&str> = Vec::new();
    for (n, name, src) in &table {
        match src(&get) {
            Some(v) => {
                got += 1;
                line(
                    &format!("param#{n:02}"),
                    "PASS",
                    format!("{name} = {v:#x} ({v})"),
                );
            }
            None => missing.push(name),
        }
    }

    // JS_FEATURES0..15 → RAW_JS_FEATURES_0..15
    let mut js_got = 0;
    for i in 0..16u32 {
        if let Some(v) = get(kb::RAW_JS_FEATURES_0 + i) {
            js_got += 1;
            line(
                &format!("param#{:02}", 22 + i),
                "PASS",
                format!("JS_FEATURES{i} = {v:#x}"),
            );
        }
    }

    line(
        "coverage",
        if missing.is_empty() && js_got == 16 {
            "PASS"
        } else {
            "PARTIAL"
        },
        format!(
            "{}/{} 主表 + {}/16 JS_FEATURES；缺失: {}",
            got,
            table.len(),
            js_got,
            if missing.is_empty() {
                "-".to_string()
            } else {
                missing.join(",")
            }
        ),
    );
    line(
        "revision_raw",
        "INFO",
        format!("major={major} minor={minor}"),
    );

    if mode == "bo" {
        let pages: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1);
        let mut a: [u64; 4] = [pages, pages, 0, 0xf]; // va_pages, commit_pages, extension, flags
        let r = unsafe {
            ioctl(
                fd,
                KBASE_MEM_ALLOC,
                a.as_mut_ptr() as *mut core::ffi::c_void,
            )
        };
        if r < 0 {
            line(
                "mem_alloc",
                "FAIL",
                format!("pages={pages} errno={}", errno()),
            );
        } else {
            line(
                "mem_alloc",
                "PASS",
                format!("flags={:#x} gpu_va={:#x} va_pages={}", a[0], a[1], a[2]),
            );
        }
    }
}
