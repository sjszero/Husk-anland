//! Device-family plan: which GPU user-space path this device can take, decided
//! once from nodes (never from a model name), with the reason kept so a degrade
//! can be explained in the log instead of showing up as a silent ENOENT.
//!
//! Rules this module exists to enforce (docs/design/device-adapters.md §4):
//!   * a family that has no adapter yet must say so and fall back to software;
//!   * a node that really works must not be shadowed by our fake one;
//!   * no probe may crash the process, whatever the device answers.

use super::{env_lookup, putnum, rc, AT_FDCWD, O_CLOEXEC, O_RDWR};

pub const GPU_MALI: u8 = 1;
pub const GPU_ADRENO: u8 = 2;
pub const GPU_DRM: u8 = 3;
pub const GPU_SOFTWARE: u8 = 4;

// 0 = not computed yet, 1 = in progress, 2 = done (same discipline as rootfd())
static PLAN_STATE: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);
static mut GPU_FAMILY: u8 = 0;
static mut REASON: [u8; 160] = [0; 160];
static mut REASON_LEN: usize = 0;
static LOGGED: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

unsafe fn push_reason(r: &[u8]) {
    let mut i = 0;
    while i < r.len() && REASON_LEN < REASON.len() {
        REASON[REASON_LEN] = r[i];
        REASON_LEN += 1;
        i += 1;
    }
}

unsafe fn push_num(v: i64) {
    let mut tmp = [0u8; 24];
    let mut k = 0usize;
    putnum(&mut tmp, &mut k, v);
    push_reason(&tmp[..k]);
}

/// `Ok(true)` when the node opens for writing, `Ok(false)` when it does not
/// (the errno goes into `reason`); never fails.
unsafe fn node_rw(path: &[u8], label: &[u8]) -> bool {
    let e = match rc::openat(AT_FDCWD, path, O_RDWR | O_CLOEXEC, 0) {
        Ok(fd) => {
            rc::close(fd);
            return true;
        }
        Err(e) => e,
    };
    push_reason(label);
    push_reason(b"=");
    push_num(e as i64);
    push_reason(b" ");
    false
}

unsafe fn compute() -> u8 {
    if env_lookup(b"NR_FORCE_SOFTWARE_GPU").is_some() {
        push_reason(b"NR_FORCE_SOFTWARE_GPU");
        return GPU_SOFTWARE;
    }
    if node_rw(b"/dev/mali0", b"mali0") {
        push_reason(b"mali0 rw ok");
        return GPU_MALI;
    }
    if node_rw(b"/dev/kgsl-3d0", b"kgsl") {
        push_reason(b"kgsl-3d0 rw ok, adapter not implemented");
        return GPU_ADRENO;
    }
    if node_rw(b"/dev/dri/renderD128", b"dri") {
        push_reason(b"renderD128 rw ok");
        return GPU_DRM;
    }
    push_reason(b"no gpu node openable");
    GPU_SOFTWARE
}

pub unsafe fn gpu_family() -> u8 {
    use core::sync::atomic::Ordering;
    if PLAN_STATE.load(Ordering::Acquire) == 2 {
        return GPU_FAMILY;
    }
    if PLAN_STATE
        .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        while PLAN_STATE.load(Ordering::Acquire) != 2 {
            core::hint::spin_loop();
        }
        return GPU_FAMILY;
    }
    GPU_FAMILY = compute();
    PLAN_STATE.store(2, Ordering::Release);
    GPU_FAMILY
}

pub unsafe fn gpu_reason() -> &'static [u8] {
    core::slice::from_raw_parts(REASON.as_ptr(), REASON_LEN)
}

fn family_name(f: u8) -> &'static [u8] {
    match f {
        GPU_MALI => b"mali-kbase",
        GPU_ADRENO => b"adreno-kgsl",
        GPU_DRM => b"drm-node",
        _ => b"software",
    }
}

/// Log the decision once per process when it is not what the guest asked for.
/// stderr is what the launcher captures; this is the "why" that used to be a
/// silent ENOENT.
pub unsafe fn log_plan_once(kind: &[u8]) {
    use core::sync::atomic::Ordering;
    if LOGGED
        .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    let fam = gpu_family();
    let mut buf = [0u8; 256];
    let mut k = 0usize;
    let mut put = |parts: &[&[u8]]| {
        for p in parts {
            for &c in *p {
                if k < buf.len() {
                    buf[k] = c;
                    k += 1;
                }
            }
        }
    };
    put(&[
        b"nr-plan: ",
        kind,
        b" -> gpu=",
        family_name(fam),
        b" reason=",
        gpu_reason(),
        b"\n",
    ]);
    syscall4(SYS_WRITE, 2, buf.as_ptr() as usize, k, 0);
}

const SYS_WRITE: usize = 64;

pub unsafe fn syscall4(n: usize, a: usize, b: usize, c: usize, d: usize) -> isize {
    let ret: isize;
    core::arch::asm!("svc 0", in("x8") n, in("x0") a, in("x1") b, in("x2") c, in("x3") d,
        lateout("x0") ret, options(nostack));
    ret
}
