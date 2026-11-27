//! Resolve the one Linux rootfs that belongs to this app install.
//!
//! `rootfs-full` is the current install tree. `rootfs` is kept as the legacy
//! development fallback; callers must not guess a distribution directory
//! because the runtime is intentionally distribution-neutral.

use std::path::{Path, PathBuf};

const CURRENT: &str = "rootfs-full";
const LEGACY: &str = "rootfs";

pub(crate) fn resolve(files: &Path) -> Result<PathBuf, String> {
    for name in [CURRENT, LEGACY] {
        let candidate = files.join(name);
        if candidate.is_dir() {
            return Ok(std::fs::canonicalize(&candidate).unwrap_or(candidate));
        }
    }
    Err(format!(
        "no Linux rootfs under {} (looked for {CURRENT}/ and {LEGACY}/)",
        files.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    fn temp_files() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "husk-rootfs-select-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn prefers_current_tree() {
        let files = temp_files();
        std::fs::create_dir_all(files.join(CURRENT)).unwrap();
        std::fs::create_dir_all(files.join(LEGACY)).unwrap();
        assert_eq!(
            resolve(&files).unwrap(),
            std::fs::canonicalize(files.join(CURRENT)).unwrap()
        );
        std::fs::remove_dir_all(files).unwrap();
    }

    #[test]
    fn accepts_legacy_fallback() {
        let files = temp_files();
        std::fs::create_dir_all(files.join(LEGACY)).unwrap();
        assert_eq!(
            resolve(&files).unwrap(),
            std::fs::canonicalize(files.join(LEGACY)).unwrap()
        );
        std::fs::remove_dir_all(files).unwrap();
    }

    #[test]
    fn missing_tree_is_an_error_not_a_distro_guess() {
        let files = temp_files();
        let err = resolve(&files).unwrap_err();
        assert!(err.contains("rootfs-full/"), "{err}");
        assert!(err.contains("rootfs/"), "{err}");
        std::fs::remove_dir_all(files).unwrap();
    }
}
