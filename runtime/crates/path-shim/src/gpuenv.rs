// 图形 / 硬件运行时的默认环境变量：由加载器在"程序自己没设置"时提供。
//
// 为什么不放在 /etc/profile.d：那里只对登录 shell 生效。cron、systemd user unit、
// 桌面 fork 出来的子进程、脚本里的 `env -i`、以及 `ssh host -- prog` 这类非登录执行
// 都拿不到，于是同一个 rootfs 里会出现"终端里的程序吃得到 GPU、别处的掉回 llvmpipe"。
// 放进加载器之后，只要进程是经 Husk 加载器起来的，看到的就是同一份默认值。
//
// 数据来源是 guest 里的 `/etc/husk/gpu-env`（安装器写，一行一个 `NAME=VALUE`）。
// 不在 Rust 里硬编码 ICD 文件名和设备名：那些随 rootfs、Mesa 构建和设备变。
//
// 三条规矩：
//   1. 程序自己设置过的变量永远优先——先查 environ，查不到才落到这里；
//   2. `HUSK_GPU=llvmpipe`，或 `NR_GPU_DEFAULT_OFF` 取真值（1/true/yes/on）时一条都不注入（bisect 用）；
//      `NR_GPU_DEFAULT_OFF=0`、`false`、`no`、`off`、空串都不关。
//   3. 行前缀决定语义：`NAME=VALUE` 只在程序没设时生效（填空）；
//      `!NAME=VALUE` 无条件生效（覆盖）；`-NAME` 让这一个变量对程序不可见（屏蔽）。
//      覆盖/屏蔽只用于我们自己控制的、明确知道后果的变量，见 install-husk.sh 的注释；
//   4. 文件不存在 = 一条都不注入，行为与没有这个特性时完全一致。

use core::sync::atomic::{AtomicI32, AtomicU8, Ordering};

const ENV_FILE: &[u8] = b"/etc/husk/gpu-env\0";
const MAX_ENTRIES: usize = 12;
const NAME_MAX: usize = 64;
const VALUE_MAX: usize = 320;
const READ_MAX: usize = 4096;
const SYS_READ: usize = 63;
const SYS_GETTID: usize = 178;

// 0 = 填空（程序自己设过就让位），1 = 覆盖（无条件生效），2 = 屏蔽（让 getenv 返回 NULL）
const MODE_FILL: u8 = 0;
const MODE_FORCE: u8 = 1;
const MODE_HIDE: u8 = 2;

#[derive(Clone, Copy)]
struct Slot {
    name: [u8; NAME_MAX],
    nlen: u16,
    value: [u8; VALUE_MAX],
    vlen: u16,
    mode: u8,
}

static mut SLOTS: [Slot; MAX_ENTRIES] = [Slot {
    name: [0; NAME_MAX],
    nlen: 0,
    value: [0; VALUE_MAX],
    vlen: 0,
    mode: MODE_FILL,
}; MAX_ENTRIES];
static mut SLOT_N: usize = 0;

// 0 = 还没读, 1 = 有线程正在读, 2 = 读完了。读文件的那条线程记在 LOAD_OWNER 里，
// 这样它自己再进来（读路径里可能碰到会查环境的 libc 内部代码）不会被自己卡死。
static LOAD_STATE: AtomicU8 = AtomicU8::new(0);
static LOAD_OWNER: AtomicI32 = AtomicI32::new(0);

unsafe fn tid() -> i32 {
    crate::syscall4(SYS_GETTID, 0, 0, 0, 0) as i32
}

unsafe fn open_guest(path: &[u8]) -> i32 {
    let mut gp = [0u8; crate::PATH_MAX];
    let n = match crate::abs_guest_raw(path.as_ptr() as *const crate::CChar, &mut gp) {
        Some(n) => n,
        None => return -1,
    };
    let mut real = [0u8; crate::PATH_MAX];
    let rn = match crate::mnt_real_path(&gp[..n], &mut real) {
        Some(rn) => rn,
        None => return -1,
    };
    real[rn] = 0;
    crate::ret_or_errno(crate::syscall4(
        crate::SYS_OPENAT,
        crate::AT_FDCWD as usize,
        real.as_ptr() as usize,
        0, // O_RDONLY
        0,
    ))
}

unsafe fn parse(buf: &[u8]) {
    let mut i = 0usize;
    SLOT_N = 0;
    while i < buf.len() && SLOT_N < MAX_ENTRIES {
        let mut e = i;
        while e < buf.len() && buf[e] != b'\n' {
            e += 1;
        }
        let mut line = &buf[i..e];
        i = e + 1;
        if line.last() == Some(&b'\r') {
            line = &line[..line.len() - 1];
        }
        if line.is_empty() || line[0] == b'#' {
            continue;
        }
        let mut mode = MODE_FILL;
        if line[0] == b'!' {
            mode = MODE_FORCE;
            line = &line[1..];
        }
        if let Some(rest) = line.strip_prefix(b"-") {
            // `-NAME`：这个变量对程序不可见（例如宿主/上层脚本硬塞的 LIBGL_ALWAYS_SOFTWARE）
            if rest.is_empty() || rest.len() >= NAME_MAX || rest.contains(&b' ') {
                continue;
            }
            let s = &mut SLOTS[SLOT_N];
            s.name[..rest.len()].copy_from_slice(rest);
            s.nlen = rest.len() as u16;
            s.value[0] = 0;
            s.vlen = 0;
            s.mode = MODE_HIDE;
            SLOT_N += 1;
            continue;
        }
        let Some(eq) = line.iter().position(|c| *c == b'=') else {
            continue;
        };
        let (name, value) = (&line[..eq], &line[eq + 1..]);
        if name.is_empty() || name.len() >= NAME_MAX || value.len() >= VALUE_MAX {
            continue;
        }
        let s = &mut SLOTS[SLOT_N];
        s.name[..name.len()].copy_from_slice(name);
        s.nlen = name.len() as u16;
        s.value[..value.len()].copy_from_slice(value);
        s.value[value.len()] = 0;
        s.vlen = value.len() as u16;
        s.mode = mode;
        SLOT_N += 1;
    }
}

// 空串、`0`、`false`、`no`、`off` 都算「不关」；其余非空值算「关」。
fn truthy(v: &[u8]) -> bool {
    let mut i = 0;
    let mut j = v.len();
    while i < j && (v[i] == b' ' || v[i] == b'\t' || v[i] == b'\r' || v[i] == b'\n') {
        i += 1;
    }
    while j > i && (v[j - 1] == b' ' || v[j - 1] == b'\t' || v[j - 1] == b'\r' || v[j - 1] == b'\n')
    {
        j -= 1;
    }
    let t = &v[i..j];
    if t.is_empty() || t == b"0" {
        return false;
    }
    !matches!(
        t,
        b"false" | b"FALSE" | b"False" | b"no" | b"NO" | b"off" | b"OFF"
    )
}

unsafe fn load() {
    // 逃逸开关先看，看命中就不必碰文件系统。
    if let Some(v) = crate::env_lookup(b"HUSK_GPU") {
        if v == b"llvmpipe" {
            return;
        }
    }
    // 值要按布尔看：只要变量存在就关，会让 `NR_GPU_DEFAULT_OFF=0` 也关掉注入
    // （实证过：0/false 写进去，登录 shell 里 `gl-probe` 报 llvmpipe）。
    if let Some(v) = crate::env_lookup(b"NR_GPU_DEFAULT_OFF") {
        if truthy(v) {
            return;
        }
    }
    let fd = open_guest(ENV_FILE);
    if fd < 0 {
        return;
    }
    static mut BUF: [u8; READ_MAX] = [0; READ_MAX];
    let n = crate::syscall4(
        SYS_READ,
        fd as usize,
        core::ptr::addr_of_mut!(BUF) as usize,
        READ_MAX,
        0,
    );
    crate::syscall4(crate::SYS_CLOSE, fd as usize, 0, 0, 0);
    if n <= 0 {
        return;
    }
    let buf = core::slice::from_raw_parts(core::ptr::addr_of!(BUF) as *const u8, n as usize);
    parse(buf);
}

unsafe fn ensure() {
    if LOAD_STATE.load(Ordering::Acquire) == 2 {
        return;
    }
    if LOAD_STATE
        .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
    {
        LOAD_OWNER.store(tid(), Ordering::Release);
        load();
        LOAD_OWNER.store(0, Ordering::Release);
        LOAD_STATE.store(2, Ordering::Release);
        return;
    }
    // 已经在读：同一个线程就别等自己，别的线程等它读完。
    if LOAD_OWNER.load(Ordering::Acquire) == tid() {
        return;
    }
    while LOAD_STATE.load(Ordering::Acquire) == 1 {
        core::hint::spin_loop();
    }
}

/// 这个变量有默认值就返回它（NUL 结尾的静态串），没有返回 NULL。
pub unsafe fn default_value(name: &[u8]) -> *mut crate::CChar {
    ensure();
    let mut i = 0usize;
    while i < SLOT_N {
        let s = &SLOTS[i];
        let nl = s.nlen as usize;
        if s.mode == MODE_FILL && nl == name.len() && &s.name[..nl] == name {
            return s.value.as_ptr() as *mut crate::CChar;
        }
        i += 1;
    }
    core::ptr::null_mut()
}

/// 覆盖/屏蔽查询的结果。
pub enum Forced {
    /// 无条件用这个值（`!NAME=VALUE`）
    Value(*mut crate::CChar),
    /// 这个变量对程序不可见（`-NAME`）
    Hide,
    /// 没有覆盖/屏蔽规则
    None,
}

/// 查覆盖与屏蔽规则。与 `default_value` 分开：一个管"没设时用什么"，一个管"设了也照这个"。
pub unsafe fn forced(name: &[u8]) -> Forced {
    ensure();
    let mut i = 0usize;
    while i < SLOT_N {
        let s = &SLOTS[i];
        let nl = s.nlen as usize;
        if s.mode != MODE_FILL && nl == name.len() && &s.name[..nl] == name {
            return if s.mode == MODE_FORCE {
                Forced::Value(s.value.as_ptr() as *mut crate::CChar)
            } else {
                Forced::Hide
            };
        }
        i += 1;
    }
    Forced::None
}

/// 有没有可用的默认值（只用于诊断输出，不做判断依据）。
pub unsafe fn count() -> usize {
    ensure();
    SLOT_N
}
