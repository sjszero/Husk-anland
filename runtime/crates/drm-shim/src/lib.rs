//! drm-shim — 把 DRM（panfrost）语义翻译成 kbase。**no_std**：要能编进 path-shim 的 loader。
//!
//! 最终形态：guest 打开 `/dev/dri/renderD128` 时 path-shim 给它一个假 fd，
//! 这个 fd 上的 `ioctl` / `mmap` 都进到这里。
//!
//! 事实依据（evidence 41/42/43）：
//! - `/dev/mali0` 只有 kbase ioctl；UAPI 11.38；`GET_GPUPROPS = 0x40108003`（_IOW）。
//! - kbase 没有 export，只有 MEM_IMPORT → 中间缓冲只能 CPU 拷回。
//! - 完成信号只能看 KINSTR（`0xc0088033`）的 START / COMPLETE。

#![no_std]

pub const MAX_PROPS: usize = 192;
pub const MAX_BOS: usize = 256;

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
pub const DRM_IOCTL_PANFROST_CREATE_BO: u64 =
    ioc(_IOC_READ | _IOC_WRITE, b'd' as u64, DRM_CMD + 0x02, 24);
pub const DRM_IOCTL_PANFROST_MMAP_BO: u64 =
    ioc(_IOC_READ | _IOC_WRITE, b'd' as u64, DRM_CMD + 0x03, 16);
pub const DRM_IOCTL_PANFROST_GET_PARAM: u64 =
    ioc(_IOC_READ | _IOC_WRITE, b'd' as u64, DRM_CMD + 0x04, 16);
pub const DRM_IOCTL_PANFROST_GET_BO_OFFSET: u64 =
    ioc(_IOC_READ | _IOC_WRITE, b'd' as u64, DRM_CMD + 0x05, 16);
pub const DRM_IOCTL_PANFROST_PERFCNT_ENABLE: u64 = ioc(_IOC_WRITE, b'd' as u64, DRM_CMD + 0x06, 8);
pub const DRM_IOCTL_PANFROST_PERFCNT_DUMP: u64 =
    ioc(_IOC_READ | _IOC_WRITE, b'd' as u64, DRM_CMD + 0x07, 16);
pub const DRM_IOCTL_PANFROST_MADVISE: u64 =
    ioc(_IOC_READ | _IOC_WRITE, b'd' as u64, DRM_CMD + 0x08, 16);
pub const DRM_IOCTL_PANFROST_SET_LABEL_BO: u64 =
    ioc(_IOC_READ | _IOC_WRITE, b'd' as u64, DRM_CMD + 0x09, 24);
pub const DRM_IOCTL_PANFROST_JM_CTX_CREATE: u64 =
    ioc(_IOC_READ | _IOC_WRITE, b'd' as u64, DRM_CMD + 0x0a, 16);
pub const DRM_IOCTL_PANFROST_JM_CTX_DESTROY: u64 =
    ioc(_IOC_READ | _IOC_WRITE, b'd' as u64, DRM_CMD + 0x0b, 8);
pub const DRM_IOCTL_PANFROST_SYNC_BO: u64 =
    ioc(_IOC_READ | _IOC_WRITE, b'd' as u64, DRM_CMD + 0x0c, 24);
pub const DRM_IOCTL_PANFROST_QUERY_BO_INFO: u64 =
    ioc(_IOC_READ | _IOC_WRITE, b'd' as u64, DRM_CMD + 0x0d, 24);

// ---------- DRM 核心（libdrm 的 drmGetVersion / drmGetCap 会问这些）----------
pub const DRM_IOCTL_VERSION: u64 = ioc(_IOC_READ | _IOC_WRITE, b'd' as u64, 0x00, 64);
pub const DRM_IOCTL_GET_CAP: u64 = ioc(_IOC_READ | _IOC_WRITE, b'd' as u64, 0x0c, 16);
/// DRM 核心里释放 GEM handle 的那个 ioctl（`struct drm_gem_close { u32 handle; u32 pad; }`）。
/// panfrost 自己没有 close_bo，Mesa 关缓冲走的就是它。原来这里没实现：每建一个 BO 就
/// 永久占掉一次 kbase 分配，攒到 64 次之后 `CREATE_BO` 一律 `ENOMEM`（evidence 229）。
pub const DRM_IOCTL_GEM_CLOSE: u64 = ioc(0, b'd' as u64, 0x09, 8);
pub const DRM_IOCTL_SET_MASTER: u64 = ioc(0, b'd' as u64, 0x1e, 0);
pub const DRM_IOCTL_DROP_MASTER: u64 = ioc(0, b'd' as u64, 0x1f, 0);

/// libdrm 的 `drm_panfrost` 期望的版本串。名字必须是 panfrost，否则 loader 不认。
const DRV_NAME: &[u8] = b"panfrost";
const DRV_DATE: &[u8] = b"20260101";
const DRV_DESC: &[u8] = b"Husk DRM semantic layer over kbase";

pub const PANFROST_BO_NOEXEC: u32 = 1;
pub const PANFROST_BO_HEAP: u32 = 2;
pub const PANFROST_BO_COHERENT: u32 = 4;

// kbase gpuprops id
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

#[derive(Clone, Copy)]
pub struct Bo {
    pub handle: u32,
    pub va: u64,
    pub bytes: u64,
    pub kbase_flags: u64,
}

/// 硬件参数表（固定容量，无分配）
pub struct Props {
    ids: [u32; MAX_PROPS],
    vals: [u64; MAX_PROPS],
    n: usize,
}

impl Props {
    pub const fn new() -> Self {
        Props {
            ids: [0; MAX_PROPS],
            vals: [0; MAX_PROPS],
            n: 0,
        }
    }
    pub fn len(&self) -> usize {
        self.n
    }
    pub fn is_empty(&self) -> bool {
        self.n == 0
    }
    pub fn get(&self, id: u32) -> Option<u64> {
        let mut i = 0;
        while i < self.n {
            if self.ids[i] == id {
                return Some(self.vals[i]);
            }
            i += 1;
        }
        None
    }
    fn set(&mut self, id: u32, v: u64) {
        let mut i = 0;
        while i < self.n {
            if self.ids[i] == id {
                self.vals[i] = v;
                return;
            }
            i += 1;
        }
        if self.n < MAX_PROPS {
            self.ids[self.n] = id;
            self.vals[self.n] = v;
            self.n += 1;
        }
    }
}

/// DRM 侧的上下文：一个 kbase context + 参数表 + BO 表
pub struct DrmCtx {
    pub kfd: i32,
    pub props: Props,
    bos: [Bo; MAX_BOS],
    n_bos: usize,
    next_handle: u32,
    pub ready: bool,
}

impl DrmCtx {
    pub const fn empty() -> Self {
        DrmCtx {
            kfd: -1,
            props: Props::new(),
            bos: [Bo {
                handle: 0,
                va: 0,
                bytes: 0,
                kbase_flags: 0,
            }; MAX_BOS],
            n_bos: 0,
            next_handle: 1,
            ready: false,
        }
    }

    pub fn bo_count(&self) -> usize {
        self.n_bos
    }

    /// 建 kbase context 并抓一次硬件参数。`ioctl_fn` 是调用方的 ioctl 原语
    /// （path-shim 里是裸 syscall，测试里是 libc::ioctl），这样本 crate 不需要链接任何东西。
    pub fn init<F>(&mut self, kfd: i32, mut ioctl_fn: F) -> Result<(), i32>
    where
        F: FnMut(i32, u64, *mut u8) -> i64,
    {
        let mut ver = [11u8, 0, 38u8, 0];
        let r = ioctl_fn(kfd, KBASE_VERSION_CHECK, ver.as_mut_ptr());
        if r < 0 {
            return Err(r as i32);
        }
        let mut flags = [0u8; 4];
        let r = ioctl_fn(kfd, KBASE_SET_FLAGS, flags.as_mut_ptr());
        if r < 0 {
            return Err(r as i32);
        }
        self.kfd = kfd;
        self.load_props(&mut ioctl_fn);
        self.ready = true;
        Ok(())
    }

    fn load_props<F: FnMut(i32, u64, *mut u8) -> i64>(&mut self, ioctl_fn: &mut F) {
        let mut buf = [0u8; 4096];
        let mut args: [u64; 2] = [buf.as_mut_ptr() as u64, buf.len() as u64];
        let r = ioctl_fn(self.kfd, KBASE_GET_GPUPROPS, args.as_mut_ptr() as *mut u8);
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
            let mut k = 0;
            while k < sz {
                v |= (buf[i + k] as u64) << (8 * k);
                k += 1;
            }
            i += sz;
            self.props.set(id, v);
        }
    }

    pub fn prop(&self, id: u32) -> Option<u64> {
        self.props.get(id)
    }

    pub fn mem_alloc<F: FnMut(i32, u64, *mut u8) -> i64>(
        &self,
        mut ioctl_fn: F,
        pages: u64,
        flags: u64,
    ) -> Result<u64, i32> {
        let mut a: [u64; 5] = [pages, pages, 0, flags, 0];
        let r = ioctl_fn(self.kfd, KBASE_MEM_ALLOC, a.as_mut_ptr() as *mut u8);
        if r < 0 {
            return Err(r as i32);
        }
        Ok(a[1]) // gpu_va
    }

    pub fn bo(&self, handle: u32) -> Option<&Bo> {
        self.bo_index(handle).map(|i| &self.bos[i])
    }

    fn bo_index(&self, handle: u32) -> Option<usize> {
        let mut i = 0;
        while i < self.n_bos {
            if self.bos[i].handle == handle {
                return Some(i);
            }
            i += 1;
        }
        None
    }

    pub fn get_param(&self, param: u32) -> Option<u64> {
        match param {
            0 => self.prop(KB_PRODUCT_ID).or_else(|| self.prop(55)),
            1 => self
                .prop(KB_MAJOR_REVISION)
                .map(|ma| (ma << 16) | self.prop(KB_MINOR_REVISION).unwrap_or(0)),
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
            39 => Some(0),
            40 => self.prop(83),
            // Mesa 25.2 新加的两个：时间戳与频率。频率给 1 GHz（纳秒时钟）
            41 => Some(0),
            42 => Some(1_000_000_000),
            _ => Some(0),
        }
    }

    /// DRM ioctl 分发。返回 0 或 `-errno`。
    ///
    /// # Safety
    /// `arg` 指向至少与对应结构体等长的可读写内存。
    pub unsafe fn dispatch<F: FnMut(i32, u64, *mut u8) -> i64>(
        &mut self,
        mut ioctl_fn: F,
        request: u64,
        arg: *mut u8,
    ) -> i64 {
        match request {
            DRM_IOCTL_PANFROST_CREATE_BO => {
                let size = u32::from_le_bytes([*arg, *arg.add(1), *arg.add(2), *arg.add(3)]) as u64;
                let dflags =
                    u32::from_le_bytes([*arg.add(4), *arg.add(5), *arg.add(6), *arg.add(7)]);
                let pages = if size == 0 { 1 } else { (size + 4095) / 4096 };
                let mut kb = 0xf; // CPU 可映射、非缓存
                if dflags & PANFROST_BO_COHERENT != 0 {
                    kb |= 0x40; // BASE_MEM_COHERENT_SYSTEM
                }
                let va = match self.mem_alloc(&mut ioctl_fn, pages, kb) {
                    Ok(v) => v,
                    Err(e) => return e as i64,
                };
                if self.n_bos >= MAX_BOS {
                    return -12; // -ENOMEM
                }
                let handle = self.next_handle;
                self.next_handle += 1;
                self.bos[self.n_bos] = Bo {
                    handle,
                    va,
                    bytes: pages * 4096,
                    kbase_flags: kb,
                };
                self.n_bos += 1;
                core::ptr::copy_nonoverlapping(handle.to_le_bytes().as_ptr(), arg.add(8), 4);
                core::ptr::copy_nonoverlapping(0u32.to_le_bytes().as_ptr(), arg.add(12), 4);
                core::ptr::copy_nonoverlapping(va.to_le_bytes().as_ptr(), arg.add(16), 8);
                0
            }
            DRM_IOCTL_GEM_CLOSE => {
                // kbase 的 MEM_FREE 只认「当初分配它的那个 context」，所以必须用
                // 我们自己的 kfd；换一个 fd 发过去是 EPERM（evidence 229 §2）。
                let handle = u32::from_le_bytes([*arg, *arg.add(1), *arg.add(2), *arg.add(3)]);
                let i = match self.bo_index(handle) {
                    Some(i) => i,
                    None => return -22, // 未知 handle：和真内核一样 EINVAL
                };
                // struct kbase_ioctl_mem_free { u64 gpu_addr; u32 flags; u32 pad; }
                let mut f = [0u8; 16];
                f[..8].copy_from_slice(&self.bos[i].va.to_le_bytes());
                let _ = ioctl_fn(self.kfd, KBASE_MEM_FREE, f.as_mut_ptr());
                // 不管 kbase 认不认，这个 handle 在 DRM 语义上都关掉了：留着它只会让
                // 下一次 CREATE_BO 更早撞 ENOMEM。真失败时泄漏一次 VA，不影响正确性。
                let last = self.n_bos - 1;
                self.bos[i] = self.bos[last];
                self.bos[last] = Bo {
                    handle: 0,
                    va: 0,
                    bytes: 0,
                    kbase_flags: 0,
                };
                self.n_bos -= 1;
                0
            }
            DRM_IOCTL_PANFROST_MMAP_BO | DRM_IOCTL_PANFROST_GET_BO_OFFSET => {
                let handle = u32::from_le_bytes([*arg, *arg.add(1), *arg.add(2), *arg.add(3)]);
                match self.bo(handle) {
                    Some(bo) => {
                        let va = bo.va;
                        core::ptr::copy_nonoverlapping(va.to_le_bytes().as_ptr(), arg.add(8), 8);
                        0
                    }
                    None => -22, // -EINVAL
                }
            }
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
            DRM_IOCTL_PANFROST_SUBMIT => {
                let mut jc = [0u8; 8];
                jc.copy_from_slice(core::slice::from_raw_parts(arg, 8));
                let jc = u64::from_le_bytes(jc);
                let req =
                    u32::from_le_bytes([*arg.add(36), *arg.add(37), *arg.add(38), *arg.add(39)]);
                let core_req: u32 = if req & 1 != 0 { 0x1 } else { 0x2 };
                let atom = build_atom(jc, core_req);
                let mut js: [u64; 3] = [atom.as_ptr() as u64, 1, 72];
                let r = ioctl_fn(self.kfd, KBASE_JOB_SUBMIT, js.as_mut_ptr() as *mut u8);
                if r < 0 {
                    return r;
                }
                0
            }
            // DRM_IOCTL_VERSION: {int major,minor,patch; sz name_len; *name; sz date_len; *date; sz desc_len; *desc}
            DRM_IOCTL_VERSION => {
                let rd_u64 = |off: usize| -> u64 {
                    let mut b = [0u8; 8];
                    b.copy_from_slice(core::slice::from_raw_parts(arg.add(off), 8));
                    u64::from_le_bytes(b)
                };
                // 版本号：说 1.0.0（libdrm 不校验具体值）
                let ver: [u32; 3] = [1, 0, 0];
                for (i, v) in ver.iter().enumerate() {
                    core::ptr::copy_nonoverlapping(v.to_le_bytes().as_ptr(), arg.add(i * 4), 4);
                }
                let mut put = |len_off: usize, ptr_off: usize, data: &[u8]| {
                    let user_len = rd_u64(len_off) as usize;
                    let user_ptr = rd_u64(ptr_off) as *mut u8;
                    if !user_ptr.is_null() && user_len > 0 {
                        let n = if user_len < data.len() {
                            user_len
                        } else {
                            data.len()
                        };
                        core::ptr::copy_nonoverlapping(data.as_ptr(), user_ptr, n);
                    }
                    core::ptr::copy_nonoverlapping(
                        (data.len() as u64).to_le_bytes().as_ptr(),
                        arg.add(len_off),
                        8,
                    );
                };
                put(16, 24, DRV_NAME);
                put(32, 40, DRV_DATE);
                put(48, 56, DRV_DESC);
                0
            }
            // DRM_IOCTL_GET_CAP: 全部答 0（不声称任何可选能力），libdrm 会走保守路径
            DRM_IOCTL_GET_CAP => {
                core::ptr::copy_nonoverlapping(0u64.to_le_bytes().as_ptr(), arg.add(8), 8);
                0
            }
            DRM_IOCTL_SET_MASTER | DRM_IOCTL_DROP_MASTER => 0,
            DRM_IOCTL_PANFROST_MADVISE
            | DRM_IOCTL_PANFROST_SET_LABEL_BO
            | DRM_IOCTL_PANFROST_SYNC_BO
            | DRM_IOCTL_PANFROST_JM_CTX_CREATE
            | DRM_IOCTL_PANFROST_JM_CTX_DESTROY
            | DRM_IOCTL_PANFROST_WAIT_BO => 0,
            DRM_IOCTL_PANFROST_QUERY_BO_INFO => {
                let handle = u32::from_le_bytes([*arg, *arg.add(1), *arg.add(2), *arg.add(3)]);
                if self.bo(handle).is_some() {
                    0
                } else {
                    -22
                }
            }
            // The emulation is present and explicitly does not implement this
            // request. EOPNOTSUPP is the same answer path-shim gives for a
            // valid fake DRM fd whose context cannot be recovered; ENOSYS lets
            // some clients interpret the ioctl as "the kernel has no DRM at
            // all" and take a different fallback path.
            _ => -95, // -EOPNOTSUPP
        }
    }
}

/// r32p1 的 `base_jd_atom_v2`（72 字节，evidence 41）
pub fn build_atom(jc: u64, core_req: u32) -> [u8; 72] {
    let mut a = [0u8; 72];
    a[0..8].copy_from_slice(&jc.to_le_bytes());
    a[0x2c..0x2e].copy_from_slice(&1u16.to_le_bytes()); // atom_number
    a[0x34..0x38].copy_from_slice(&core_req.to_le_bytes());
    a
}
