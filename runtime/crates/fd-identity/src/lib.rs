//! Husk 假 fd 的身份判定（第 3 轮 · fd 与路径语义）。
//!
//! 所有假设备 fd 的身份都来自**内核**（`/proc/self/fd/N` 的 readlink 目标），
//! 而不是某张按 fd 号索引的私有表：`dup2` 会用普通文件盖掉同一个号，glibc 内部
//! 也可能绕过本进程的 `close` 直接关掉，表按号索引迟早会认错 fd。
//!
//! 三个子系统各有自己的名字，规则在这里只写一遍，避免"各自的字符串匹配慢慢漂":
//!
//! * DRM：`/dev/mali0`（真 kbase 节点）与 `/memfd:husk-drm`（加载器发的假节点）
//! * V4L2：`/memfd:husk-v4l2-<N>`，N 是 0..MAX_V4L2_DEVS 的十进制下标
//! * netlink：不在这张表里 —— 它的假 fd 是 socketpair，身份是 **socket inode**，
//!   比名字更硬（见 path-shim 的 `nl_slot()`）
//!
//! 规则是**精确匹配**，不是子串/前缀：
//!
//! * `/tmp/malicious-data`、`/home/user/mali-cache` 都不是 GPU（旧代码里
//!   `s.contains("mali")` 会把它们当成 kbase 节点送进 DRM 模拟）；
//! * `/memfd:husk-v4l2-` 前缀同时吞掉了 DRM 的 `/memfd:husk-drm`（旧代码里
//!   `s.starts_with("/memfd:husk-")` 造成过这个串台）；
//! * `/memfd:husk-v4l2-12abc`、`/memfd:husk-drm2` 这类"前缀对、尾巴不对"的名字
//!   一律不认 —— 认了就等于给任何人一个伪造设备 fd 的入口。
//!
//! `readlink` 在名字被 unlink 之后会补 `" (deleted)"`；身份要能穿过它，所以比较
//! 之前剥掉这个**只出现在结尾**的后缀。
//!
//! 纯函数、无分配、`no_std`：loader 内嵌的 shim、两个 preload 层和单测用的是同一份。

#![no_std]

/// 真 kbase 节点。
pub const DRM_NODE: &[u8] = b"/dev/mali0";
/// 加载器发给 guest 的假 DRM 节点。
pub const DRM_MEMFD: &[u8] = b"/memfd:husk-drm";
/// 假 V4L2 节点前缀，后面只能接十进制下标。
pub const V4L2_MEMFD_PREFIX: &[u8] = b"/memfd:husk-v4l2-";
/// 与 `v4l2-shim::MAX_DEVS` 对齐。
pub const MAX_V4L2_DEVS: u32 = 4;
/// 下标最长三位（MAX_V4L2_DEVS 远小于 1000，写死三位是为了不给长串开口子）。
const V4L2_INDEX_MAX_DIGITS: usize = 3;
/// unlink 之后 readlink 会补这个后缀。
const DELETED_SUFFIX: &[u8] = b" (deleted)";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FakeFd {
    /// `/dev/mali0` 或 `/memfd:husk-drm`
    Drm,
    /// `/memfd:husk-v4l2-<N>`
    V4l2(u32),
}

/// 剥掉 `readlink` 的 `" (deleted)"` 后缀（只在结尾，且只剥一次）。
pub fn strip_deleted(target: &[u8]) -> &[u8] {
    match target.strip_suffix(DELETED_SUFFIX) {
        Some(rest) => rest,
        None => target,
    }
}

/// 判定一个 `/proc/self/fd/N` 的 readlink 目标是不是我们的假 fd。
///
/// 不是就返回 `None` —— 调用方必须把这个 fd 当普通 fd 交给内核，不能"猜"。
pub fn classify(target: &[u8]) -> Option<FakeFd> {
    let t = strip_deleted(target);
    if t == DRM_NODE || t == DRM_MEMFD {
        return Some(FakeFd::Drm);
    }
    let rest = t.strip_prefix(V4L2_MEMFD_PREFIX)?;
    if rest.is_empty() || rest.len() > V4L2_INDEX_MAX_DIGITS {
        return None;
    }
    let mut index: u32 = 0;
    for &c in rest {
        if !c.is_ascii_digit() {
            return None;
        }
        index = index * 10 + (c - b'0') as u32;
    }
    if index >= MAX_V4L2_DEVS {
        return None;
    }
    Some(FakeFd::V4l2(index))
}

/// 是不是我们的假 fd（DRM 或 V4L2）。
pub fn is_fake(target: &[u8]) -> bool {
    classify(target).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drm_names_are_exact() {
        assert_eq!(classify(b"/dev/mali0"), Some(FakeFd::Drm));
        assert_eq!(classify(b"/memfd:husk-drm"), Some(FakeFd::Drm));
        // 前缀对、名字不对：不认
        assert_eq!(classify(b"/dev/mali0.bak"), None);
        assert_eq!(classify(b"/memfd:husk-drm2"), None);
        assert_eq!(classify(b"/memfd:husk-drm "), None);
    }

    #[test]
    fn ordinary_paths_that_look_like_devices_are_not_devices() {
        // 旧实现用 s.contains("mali")，这两条会被当成 kbase 节点。
        assert_eq!(classify(b"/tmp/malicious-data"), None);
        assert_eq!(classify(b"/home/user/mali-cache"), None);
        assert_eq!(
            classify(b"/data/user/0/com.linuxhub.myth.husk/files/mali.log"),
            None
        );
        // 也不是 v4l2：只有完整前缀+纯十进制下标才算。
        assert_eq!(classify(b"/memfd:husk-v4l2-"), None);
        assert_eq!(classify(b"/memfd:husk-v4l2-12abc"), None);
        assert_eq!(classify(b"/memfd:husk-v4l2--1"), None);
        assert_eq!(classify(b"/memfd:husk-v4l2x-1"), None);
        assert_eq!(classify(b"/memfd:husk-nl-1"), None);
    }

    #[test]
    fn v4l2_indices_are_bounded() {
        for i in 0..MAX_V4L2_DEVS {
            let name = match i {
                0 => b"/memfd:husk-v4l2-0".to_vec(),
                1 => b"/memfd:husk-v4l2-1".to_vec(),
                2 => b"/memfd:husk-v4l2-2".to_vec(),
                _ => b"/memfd:husk-v4l2-3".to_vec(),
            };
            assert_eq!(classify(&name), Some(FakeFd::V4l2(i)), "index {i}");
        }
        // 越界与过长
        assert_eq!(classify(b"/memfd:husk-v4l2-4"), None);
        assert_eq!(classify(b"/memfd:husk-v4l2-99"), None);
        assert_eq!(classify(b"/memfd:husk-v4l2-000"), Some(FakeFd::V4l2(0)));
        assert_eq!(classify(b"/memfd:husk-v4l2-0000"), None);
    }

    #[test]
    fn deleted_suffix_is_stripped_only_at_the_end() {
        assert_eq!(classify(b"/memfd:husk-drm (deleted)"), Some(FakeFd::Drm));
        assert_eq!(classify(b"/dev/mali0 (deleted)"), Some(FakeFd::Drm));
        assert_eq!(
            classify(b"/memfd:husk-v4l2-2 (deleted)"),
            Some(FakeFd::V4l2(2))
        );
        // 后缀不在结尾就不是后缀
        assert_eq!(classify(b"/memfd:husk-drm (deleted)/x"), None);
        assert_eq!(classify(b" (deleted)"), None);
        assert_eq!(classify(b""), None);
    }

    #[test]
    fn the_three_subsystems_do_not_cross_claim() {
        // DRM 的假 fd 不能被 v4l2 认领，反之亦然。
        assert_eq!(classify(b"/memfd:husk-drm"), Some(FakeFd::Drm));
        assert!(matches!(
            classify(b"/memfd:husk-v4l2-1"),
            Some(FakeFd::V4l2(1))
        ));
        assert!(!matches!(
            classify(b"/memfd:husk-v4l2-1"),
            Some(FakeFd::Drm)
        ));
        // netlink 的 socketpair 从来不会是这两种名字
        assert_eq!(classify(b"socket:[12345]"), None);
    }
}
