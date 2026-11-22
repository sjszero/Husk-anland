//! drm-shim — 把 DRM（panfrost）语义翻译成 kbase。
//!
//! 这一层最终挂在 path-shim 里：guest 打开 `/dev/dri/renderD128` 时给它一个假 fd，
//! 这个 fd 上的 `ioctl` / `mmap` 都进到这里。现在先用同进程自测把语义跑通，
//! 再接进 interpose（见 docs/design/gpu-drm-shim.md）。
//!
//! 关键事实（evidence 41/42）：
//! - `/dev/mali0` 只有 kbase ioctl；UAPI 11.38；GET_GPUPROPS = `0x40108003`（_IOW）。
//! - kbase 没有 export，只有 MEM_IMPORT，所以渲染中间缓冲只能 CPU 拷回。
//! - 完成信号只能看 KINSTR（`0xc0088033`）的 START/COMPLETE。

use std::collections::HashMap;

extern "C" {
    fn ioctl(fd: i32, request: u64, ...) -> i32;
    fn __errno_location() -> *mut i32;
}

fn errno() -> i32 {
    unsafe { *__errno_location() }
}

const _IOC_WRITE: u64 = 1;
const _IOC_READ: u64 = 2;

pub const fn ioc(dir: u64, ty: u64, nr: u64, size: u64) -> u64 {
    (dir << 30) | (size << 16) | (ty << 8) | nr
}

// ---------- kbase ----------
pub const KBASE_VERSION_CHECK: u64 = ioc(_IOC_READ | _IOC_WRITE, 0x80, 0, 4);
pub const KBASE_SET_FLAGS: u64 = ioc(_IOC_WRITE, 0x80, 1, 4);
pub const KBASE_GET_GPUPROPS: u64 = ioc(_IOC_WRITE, 0x80, 3, 16);
pub const KBASE_JOB_SUBMIT: u64 = ioc(_IOC_WRITE, 0x80, 2, 16);
pub const KBASE_MEM_ALLOC: u64 = ioc(_IOC_READ | _IOC_WRITE, 0x80, 5, 32);
pub const KBASE_MEM_FREE: u64 = ioc(_IOC_WRITE, 0x80, 7, 8);

// ---------- DRM panfrost ----------
const DRM_CMD: u64 = 0x40;
pub const DRM_IOCTL_PANFROST_SUBMIT: u64 = ioc(_IOC_WRITE, b'd' as u64, DRM_CMD + 0x00, 48);
pub const DRM_IOCTL_PANFROST_WAIT_BO: u64 = ioc(_IOC_WRITE, b'd' as u64, DRM_CMD + 0x01, 16);
pub const DRM_IOCTL_PANFROST_CREATE_BO: u64 = ioc(_IOC_READ | _IOC_WRITE, b'd' as u64, DRM_CMD + 0x02, 24);
pub const DRM_IOCTL_PANFROST_MMAP_BO: u64 = ioc(_IOC_READ | _IOC_WRITE, b'd' as u64, DRM_CMD + 0x03, 16);
pub const DRM_IOCTL_PANFROST_GET_PARAM: u64 = ioc(_IOC_READ | _IOC_WRITE, b'd' as u64, DRM_CMD + 0x04, 16);
pub const DRM_IOCTL_PANFROST_GET_BO_OFFSET: u64 = ioc(_IOC_READ | _IOC_WRITE, b'd' as u64, DRM_CMD + 0x05, 16);
pub const DRM_IOCTL_PANFROST_PERFCNT_ENABLE: u64 = ioc(_IOC_WRITE, b'd' as u64, DRM_CMD + 0x06, 8);
pub const DRM_IOCTL_PANFROST_PERFCNT_DUMP: u64 = ioc(_IOC_READ | _IOC_WRITE, b'd' as u64, DRM_CMD + 0x07, 16);
pub const DRM_IOCTL_PANFROST_MADVISE: u64 = ioc(_IOC_READ | _IOC_WRITE, b'd' as u64, DRM_CMD + 0x08, 16);
pub const DRM_IOCTL_PANFROST_SET_LABEL_BO: u64 = ioc(_IOC_READ | _IOC_WRITE, b'd' as u64, DRM_CMD + 0x09, 24);
pub const DRM_IOCTL_PANFROST_JM_CTX_CREATE: u64 = ioc(_IOC_READ | _IOC_WRITE, b'd' as u64, DRM_CMD + 0x0a, 16);
pub const DRM_IOCTL_PANFROST_JM_CTX_DESTROY: u64 = ioc(_IOC_READ | _IOC_WRITE, b'd' as u64, DRM_CMD + 0x0b, 8);
pub const DRM_IOCTL_PANFROST_SYNC_BO: u64 = ioc(_IOC_READ | _IOC_WRITE, b'd' as u64, DRM_CMD + 0x0c, 24);
pub const DRM_IOCTL_PANFROST_QUERY_BO_INFO: u64 = ioc(_IOC_READ | _IOC_WRITE, b'd' as u64, DRM_CMD + 0x0d, 24);

pub const PANFROST_BO_NOEXEC: u32 = 1;
pub const PANFROST_BO_HEAP: u32 = 2;
pub const PANFROST_BO_COHERENT: u32 = 4;

// kbase gpuprops id（用到的几个）
pub const KB_PRODUCT_ID: u32 = 1;
pub const KB_MINOR_REVISION: u32 = 3;
pub const KB_MAJOR_REVISION: u32 = 4;
pub const KB_RAW_SHADER_PRESENT: u32 = 25;
pub const KB_RAW_TILER_PRESENT: u32 = 26;
pub const KB_RAW_L2_PRESENT: u32 = 27;
pub const KB_RAW_STACK_PRESENT: u32 = 28;
pub const KB_RAW_L2_FEATURES: u32 = 29;
pub const KB_RAW_CORE_FEATURES: u32 = 30;
pub const KB_RAW_MEM_FEATURES: u32 = 31;
pub const KB_RAW_MMU_FEATURES: u32 = 32;
pub const KB_RAW_AS_PRESENT: u32 = 33;
pub const KB_RAW_JS_PRESENT: u32 = 34;
pub const KB_RAW_JS_FEATURES_0: u32 = 35;
pub const KB_RAW_TILER_FEATURES: u32 = 51;
pub const KB_RAW_TEXTURE_FEATURES_0: u32 = 52;
pub const KB_RAW_THREAD_FEATURES: u32 = 59;
pub const KB_RAW_COHERENCY_MODE: u32 = 60;

/// 一块 kbase 显存（对我们来说就是一块 BO）
#[derive(Debug, Clone, Copy)]
pub struct Bo {
    pub va: u64,
    pub bytes: u64,
    pub kbase_flags: u64,
}

pub struct DrmCtx {
    pub kfd: i32,
    pub props: HashMap<u32, u64>,
    pub bos: HashMap<u32, Bo>,
    next_handle: u32,
}

impl DrmCtx {
    /// 建 kbase context（VERSION_CHECK + SET_FLAGS）并抓一遍硬件参数。
    pub fn new(kfd: i32) -> Result<Self, i32> {
        let mut ver: [u16; 2] = [11, 38];
        let r = unsafe { ioctl(kfd, KBASE_VERSION_CHECK, ver.as_mut_ptr() as *mut core::ffi::c_void) };
        if r < 0 {
            return Err(errno());
        }
        let mut flags: u32 = 0;
        let r = unsafe { ioctl(kfd, KBASE_SET_FLAGS, &mut flags as *mut u32 as *mut core::ffi::c_void) };
        if r < 0 {
            return Err(errno());
        }
        let mut ctx = DrmCtx { kfd, props: HashMap::new(), bos: HashMap::new(), next_handle: 1 };
        ctx.load_props();
        Ok(ctx)
    }

    fn load_props(&mut self) {
        let mut buf = vec![0u8; 8192];
        let mut args: [u64; 2] = [buf.as_mut_ptr() as u64, buf.len() as u64];
        let r = unsafe { ioctl(self.kfd, KBASE_GET_GPUPROPS, &mut args as *mut [u64; 2] as *mut core::ffi::c_void) };
        if r <= 0 {
            return;
        }
        let n = r as usize;
        let mut i = 0usize;
        while i + 4 <= n {
            let h = u32::from_le_bytes([buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]);
            i += 4;
            let id = h >> 2;
            let sz = [1usize, 2, 4, 8][(h & 3) as usize];
            if i + sz > n {
                break;
            }
            let mut v = 0u64;
            for k in 0..sz {
                v |= (buf[i + k] as u64) << (8 * k);
            }
            i += sz;
            self.props.insert(id, v);
        }
    }

    pub fn prop(&self, id: u32) -> Option<u64> {
        self.props.get(&id).copied()
    }

    /// kbase mem_alloc。返回 (gpu_va, va_pages)。
    pub fn mem_alloc(&self, pages: u64, flags: u64) -> Result<(u64, u64), i32> {
        let mut a: [u64; 5] = [pages, pages, 0, flags, 0];
        let r = unsafe { ioctl(self.kfd, KBASE_MEM_ALLOC, a.as_mut_ptr() as *mut core::ffi::c_void) };
        if r < 0 {
            return Err(errno());
        }
        Ok((a[1], a[2])) // out: flags, gpu_va, va_pages
    }

    /// panfrost `DRM_PANFROST_PARAM_*` → 值
    pub fn get_param(&self, param: u32) -> Option<u64> {
        match param {
            0 => self.prop(KB_PRODUCT_ID).or_else(|| self.prop(55)),
            1 => self.prop(KB_MAJOR_REVISION).map(|ma| {
                (ma << 16) | self.prop(KB_MINOR_REVISION).unwrap_or(0)
            }),
            2 => self.prop(KB_RAW_SHADER_PRESENT),
            3 => self.prop(KB_RAW_TILER_PRESENT),
            4 => self.prop(KB_RAW_L2_PRESENT),
            5 => self.prop(KB_RAW_STACK_PRESENT),
            6 => self.prop(KB_RAW_AS_PRESENT),
            7 => self.prop(KB_RAW_JS_PRESENT),
            8 => self.prop(KB_RAW_L2_FEATURES),
            9 => self.prop(KB_RAW_CORE_FEATURES),
            10 => self.prop(KB_RAW_TILER_FEATURES),
            11 => self.prop(KB_RAW_MEM_FEATURES),
            12 => self.prop(KB_RAW_MMU_FEATURES),
            13 => self.prop(KB_RAW_THREAD_FEATURES),
            14 => self.prop(56).or_else(|| self.prop(18)),
            15 => self.prop(57).or_else(|| self.prop(19)),
            16 => self.prop(58).or_else(|| self.prop(20)),
            17 => self.prop(KB_RAW_COHERENCY_MODE),
            18 => self.prop(KB_RAW_TEXTURE_FEATURES_0),
            19 => self.prop(KB_RAW_TEXTURE_FEATURES_0 + 1),
            20 => self.prop(KB_RAW_TEXTURE_FEATURES_0 + 2),
            21 => self.prop(81),
            22..=37 => self.prop(KB_RAW_JS_FEATURES_0 + (param - 22)),
            38 => self.prop(62),
            40 => self.prop(83),
            41 => None, // AFBC_FEATURES: 待定
            43 => None, // SYSTEM_TIMESTAMP: 走 clock_gettime
            _ => None,
        }
    }

    /// DRM ioctl 分发。`arg` 指向 guest 传进来的结构体；成功返回 0，失败返回 `-errno`。
    ///
    /// # Safety
    /// `arg` 必须指向至少与对应 ioctl 结构体等长的可读写内存。
    pub unsafe fn dispatch(&mut self, request: u64, arg: *mut u8) -> i64 {
        match request {
            // CREATE_BO: {u32 size; u32 flags; u32 handle; u32 pad; u64 offset}
            DRM_IOCTL_PANFROST_CREATE_BO => {
                let size = u32::from_le_bytes([*arg, *arg.add(1), *arg.add(2), *arg.add(3)]) as u64;
                let dflags = u32::from_le_bytes([*arg.add(4), *arg.add(5), *arg.add(6), *arg.add(7)]);
                let pages = (size.max(4096) + 4095) / 4096;
                // DRM 语义 → kbase 语义：默认给可 CPU 映射的 uncached 内存（0xf）
                let mut kb = 0xf;
                if dflags & PANFROST_BO_COHERENT != 0 {
                    kb |= 0x40; // BASE_MEM_COHERENT_SYSTEM
                }
                let (va, _pages) = match self.mem_alloc(pages, kb) {
                    Ok(v) => v,
                    Err(e) => return -(e as i64),
                };
                let handle = self.next_handle;
                self.next_handle += 1;
                self.bos.insert(handle, Bo { va, bytes: pages * 4096, kbase_flags: kb });
                core::ptr::copy_nonoverlapping(handle.to_le_bytes().as_ptr(), arg.add(8), 4); // out.handle
                core::ptr::copy_nonoverlapping(0u32.to_le_bytes().as_ptr(), arg.add(12), 4); // pad
                core::ptr::copy_nonoverlapping(va.to_le_bytes().as_ptr(), arg.add(16), 8); // out.offset == GPU VA
                0
            }
            // MMAP_BO: {u32 handle; u32 flags; u64 offset} → 只回 offset，真 mmap 由 mmap 钩子做
            DRM_IOCTL_PANFROST_MMAP_BO => {
                let handle = u32::from_le_bytes([*arg, *arg.add(1), *arg.add(2), *arg.add(3)]);
                match self.bos.get(&handle) {
                    Some(bo) => {
                        core::ptr::copy_nonoverlapping(bo.va.to_le_bytes().as_ptr(), arg.add(8), 8);
                        0
                    }
                    None => -22,
                }
            }
            // GET_BO_OFFSET: {u32 handle; u32 pad; u64 offset}
            DRM_IOCTL_PANFROST_GET_BO_OFFSET => {
                let handle = u32::from_le_bytes([*arg, *arg.add(1), *arg.add(2), *arg.add(3)]);
                match self.bos.get(&handle) {
                    Some(bo) => {
                        core::ptr::copy_nonoverlapping(bo.va.to_le_bytes().as_ptr(), arg.add(8), 8);
                        0
                    }
                    None => -22,
                }
            }
            // GET_PARAM: {u32 param; u32 pad; u64 value}
            DRM_IOCTL_PANFROST_GET_PARAM => {
                let param = u32::from_le_bytes([*arg, *arg.add(1), *arg.add(2), *arg.add(3)]);
                match self.get_param(param) {
                    Some(v) => {
                        core::ptr::copy_nonoverlapping(v.to_le_bytes().as_ptr(), arg.add(8), 8);
                        0
                    }
                    None => -22,
                }
            }
            // SUBMIT: {u64 jc; u64 in_syncs; u32 in_sync_count; u32 out_sync; u64 bo_handles; …}
            DRM_IOCTL_PANFROST_SUBMIT => {
                let mut jc = [0u8; 8];
                jc.copy_from_slice(core::slice::from_raw_parts(arg, 8));
                let jc = u64::from_le_bytes(jc);
                let req = u32::from_le_bytes([*arg.add(36), *arg.add(37), *arg.add(38), *arg.add(39)]);
                // requirements: PANFROST_JD_REQ_FS(1)=3D, 否则 compute→CS(2)
                let core_req: u32 = if req & 1 != 0 { 0x1 } else { 0x2 };
                let atom = build_atom(jc, core_req);
                let mut js = [atom.as_ptr() as u64, 1u64, 72u64, 0u64];
                let r = ioctl(self.kfd, KBASE_JOB_SUBMIT, js.as_mut_ptr() as *mut core::ffi::c_void);
                if r < 0 {
                    return -(errno() as i64);
                }
                0
            }
            // 簿记类：MADVISE / SET_LABEL_BO / SYNC_BO / QUERY_BO_INFO / JM_CTX_*
            DRM_IOCTL_PANFROST_MADVISE
            | DRM_IOCTL_PANFROST_SET_LABEL_BO
            | DRM_IOCTL_PANFROST_SYNC_BO
            | DRM_IOCTL_PANFROST_JM_CTX_CREATE
            | DRM_IOCTL_PANFROST_JM_CTX_DESTROY => 0,
            // WAIT_BO：完成信号要靠 KINSTR 记录，这里先直接放行（M3 再补真实等待）
            DRM_IOCTL_PANFROST_WAIT_BO => 0,
            // QUERY_BO_INFO: {u32 handle; u32 pad; ...} 先把 handle 原样回写
            DRM_IOCTL_PANFROST_QUERY_BO_INFO => {
                let handle = u32::from_le_bytes([*arg, *arg.add(1), *arg.add(2), *arg.add(3)]);
                if self.bos.contains_key(&handle) { 0 } else { -22 }
            }
            _ => -38, // -ENOSYS
        }
    }
}

/// r32p1 的 `base_jd_atom_v2`（72 字节，evidence 41）
fn build_atom(jc: u64, core_req: u32) -> [u8; 72] {
    let mut a = [0u8; 72];
    a[0..8].copy_from_slice(&jc.to_le_bytes()); // jc
    a[0x2c..0x2e].copy_from_slice(&1u16.to_le_bytes()); // atom_number
    a[0x34..0x38].copy_from_slice(&core_req.to_le_bytes()); // core_req
    a
}
