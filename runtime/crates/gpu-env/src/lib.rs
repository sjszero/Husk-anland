//! GPU defaults shared by the Android App and the loader.
//!
//! The App calls this before starting the initial guest shell. A successful
//! hardware build also writes `/etc/husk/gpu-env`, which the loader reads for
//! nested exec and `env -i`. Keeping both paths in one crate prevents the App
//! and loader from disagreeing about the selected GPU family or driver.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

const PANVK_CANONICAL_ICD: &str = "/usr/share/vulkan/icd.d/husk_panvk_guest.json";
const PANVK_DRIVERS: &[&str] = &[
    "usr/lib/aarch64-linux-gnu/libvulkan_panfrost_frist370_execva.so",
    "usr/lib/aarch64-linux-gnu/libvulkan_panfrost_frist370.so",
    "usr/lib/aarch64-linux-gnu/libvulkan_panfrost_x11.so",
    "usr/lib/aarch64-linux-gnu/libvulkan_panfrost_g77.so",
    "usr/lib/aarch64-linux-gnu/libvulkan_panfrost.so",
];
const PANVK_SOURCE_ICDS: &[&str] = &[
    "usr/share/vulkan/icd.d/husk_panvk_frist370_execva.json",
    "usr/share/vulkan/icd.d/husk_panvk_frist370.json",
    "usr/share/vulkan/icd.d/husk_panvk_x11.json",
    "usr/share/vulkan/icd.d/husk_panvk_g77.json",
    "usr/share/vulkan/icd.d/husk_panvk.json",
];

const FREEDRENO_CANONICAL_ICD: &str = "/usr/share/vulkan/icd.d/husk_freedreno_guest.json";
const FREEDRENO_DRIVERS: &[&str] = &[
    "usr/lib/aarch64-linux-gnu/libvulkan_freedreno.so",
    "usr/lib/aarch64-linux-gnu/libvulkan_freedreno.so.1",
    "usr/lib/libvulkan_freedreno.so",
    "usr/lib/libvulkan_freedreno.so.1",
];
const FREEDRENO_SOURCE_ICDS: &[&str] = &[
    "usr/share/vulkan/icd.d/freedreno_icd.aarch64.json",
    "usr/share/vulkan/icd.d/freedreno_icd.json",
    "usr/share/vulkan/icd.d/turnip_icd.aarch64.json",
    "usr/share/vulkan/icd.d/turnip_icd.json",
];
const FREEDRENO_NATIVE_DRI: &[&str] = &[
    "usr/lib/aarch64-linux-gnu/dri/kgsl_dri.so",
    "usr/lib/aarch64-linux-gnu/dri/freedreno_dri.so",
    "usr/lib/dri/kgsl_dri.so",
    "usr/lib/dri/freedreno_dri.so",
];

static NEXT_FILE_TMP: AtomicU64 = AtomicU64::new(1);

fn json_string_field<'a>(text: &'a str, field: &str) -> Option<&'a str> {
    let key = format!("\"{field}\"");
    let key_at = text.find(&key)?;
    let after_key = &text[key_at + key.len()..];
    let colon = after_key.find(':')?;
    let after_colon = &after_key[colon + 1..];
    let start = after_colon.find('"')? + 1;
    let value = &after_colon[start..];
    let end = value.find('"')?;
    Some(&value[..end])
}

fn icd_api_version(rootfs: &Path, sources: &[&str]) -> String {
    for rel in sources {
        let Ok(text) = std::fs::read_to_string(rootfs.join(rel)) else {
            continue;
        };
        if let Some(version) = json_string_field(&text, "api_version") {
            if !version.is_empty() && version.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
                return version.to_string();
            }
        }
    }
    "1.0.0".to_string()
}

fn atomic_write(path: &Path, contents: &str) -> bool {
    let Some(parent) = path.parent() else {
        return false;
    };
    if std::fs::create_dir_all(parent).is_err() {
        return false;
    }
    let nonce = NEXT_FILE_TMP.fetch_add(1, Ordering::Relaxed);
    let tmp = path.with_extension(format!("tmp.{}.{nonce}", std::process::id()));
    if std::fs::write(&tmp, contents).is_err() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o644));
    }
    if std::fs::rename(&tmp, path).is_err() {
        let _ = std::fs::remove_file(&tmp);
        return false;
    }
    true
}

fn ensure_icd(
    rootfs: &Path,
    canonical: &'static str,
    drivers: &[&str],
    sources: &[&str],
) -> Option<&'static str> {
    let driver = drivers.iter().find(|rel| rootfs.join(rel).is_file())?;
    let api_version = icd_api_version(rootfs, sources);
    let manifest = format!(
        "{{\n  \"ICD\": {{\n    \"api_version\": \"{api_version}\",\n    \"library_arch\": \"64\",\n    \"library_path\": \"/{driver}\"\n  }},\n  \"file_format_version\": \"1.0.1\"\n}}\n"
    );
    atomic_write(&rootfs.join(canonical.trim_start_matches('/')), &manifest).then_some(canonical)
}

fn write_guest_gpu_defaults(rootfs: &Path, family: &str, icd: &str, body: &str) {
    let path = rootfs.join("etc/husk/gpu-env");
    let family_line = format!("@GPU_FAMILY={family}");
    if let Ok(existing) = std::fs::read_to_string(&path) {
        if existing.contains(&family_line) && existing.contains(icd) {
            return;
        }
    }
    let defaults = format!(
        "{family_line}\n\
__EGL_VENDOR_LIBRARY_FILENAMES=/usr/share/glvnd/egl_vendor.d/50_mesa.json\n\
{body}"
    );
    let _ = atomic_write(&path, &defaults);
}

fn ensure_mali_defaults(rootfs: &Path, icd: &str) {
    write_guest_gpu_defaults(
        rootfs,
        "mali-kbase",
        icd,
        &format!(
            "\
VK_ICD_FILENAMES={icd}\n\
VK_DRIVER_FILES={icd}\n\
!MESA_LOADER_DRIVER_OVERRIDE=zink\n\
!GALLIUM_DRIVER=zink\n\
-LIBGL_ALWAYS_SOFTWARE\n\
MESA_GLES_VERSION_OVERRIDE=3.2\n\
GSK_RENDERER=cairo\n"
        ),
    );
}

fn ensure_adreno_defaults(rootfs: &Path, icd: &str, native_gl: bool) {
    let gl_lines = if native_gl {
        "!MESA_LOADER_DRIVER_OVERRIDE=kgsl\n!GALLIUM_DRIVER=freedreno\n"
    } else {
        "!MESA_LOADER_DRIVER_OVERRIDE=zink\n!GALLIUM_DRIVER=zink\n"
    };
    write_guest_gpu_defaults(
        rootfs,
        "adreno-kgsl",
        icd,
        &format!(
            "\
VK_ICD_FILENAMES={icd}\n\
VK_DRIVER_FILES={icd}\n\
TURNIP_KMD=kgsl\n\
FD_FORCE_KGSL=1\n\
MESA_VK_WSI_PRESENT_MODE=mailbox\n\
vblank_mode=0\n\
-LIBGL_ALWAYS_SOFTWARE\n\
{gl_lines}"
        ),
    );
}

fn freedreno_native_gl(rootfs: &Path) -> bool {
    FREEDRENO_NATIVE_DRI
        .iter()
        .any(|rel| rootfs.join(rel).is_file())
}

/// Build the environment inherited by the initial guest shell and, for a
/// hardware path, persist the loader defaults used by later execs.
///
/// `software=true` is an explicit escape hatch. It emits `HUSK_GPU=llvmpipe`
/// as well as the Mesa variables so the loader does not re-apply a hardware
/// default from `/etc/husk/gpu-env` in a nested process.
pub fn build(
    rootfs: &Path,
    mali_available: bool,
    adreno_available: bool,
    software: bool,
) -> Vec<String> {
    let mut env = Vec::new();
    if software {
        env.push("HUSK_GPU=llvmpipe".to_string());
        env.push("LIBGL_ALWAYS_SOFTWARE=1".to_string());
        env.push("GALLIUM_DRIVER=llvmpipe".to_string());
        env.push("MESA_LOADER_DRIVER_OVERRIDE=swrast".to_string());
        let lvp = rootfs.join("usr/share/vulkan/icd.d/lvp_icd.json");
        if lvp.exists() {
            let lvp = lvp.display().to_string();
            env.push(format!("VK_ICD_FILENAMES={lvp}"));
            env.push(format!("VK_DRIVER_FILES={lvp}"));
        }
        return env;
    }

    let egl_vendor = rootfs.join("usr/share/glvnd/egl_vendor.d/50_mesa.json");
    if egl_vendor.exists() {
        env.push(format!(
            "__EGL_VENDOR_LIBRARY_FILENAMES={}",
            egl_vendor.display()
        ));
    }

    if mali_available {
        if let Some(icd) = ensure_icd(
            rootfs,
            PANVK_CANONICAL_ICD,
            PANVK_DRIVERS,
            PANVK_SOURCE_ICDS,
        ) {
            ensure_mali_defaults(rootfs, icd);
            env.push(format!("VK_ICD_FILENAMES={icd}"));
            env.push(format!("VK_DRIVER_FILES={icd}"));
            env.push("MESA_LOADER_DRIVER_OVERRIDE=zink".to_string());
            env.push("GALLIUM_DRIVER=zink".to_string());
            env.push("MESA_GLES_VERSION_OVERRIDE=3.2".to_string());
        }
    } else if adreno_available {
        if let Some(icd) = ensure_icd(
            rootfs,
            FREEDRENO_CANONICAL_ICD,
            FREEDRENO_DRIVERS,
            FREEDRENO_SOURCE_ICDS,
        ) {
            let native_gl = freedreno_native_gl(rootfs);
            ensure_adreno_defaults(rootfs, icd, native_gl);
            env.push(format!("VK_ICD_FILENAMES={icd}"));
            env.push(format!("VK_DRIVER_FILES={icd}"));
            env.push("TURNIP_KMD=kgsl".to_string());
            env.push("FD_FORCE_KGSL=1".to_string());
            env.push("MESA_VK_WSI_PRESENT_MODE=mailbox".to_string());
            env.push("vblank_mode=0".to_string());
            if native_gl {
                env.push("MESA_LOADER_DRIVER_OVERRIDE=kgsl".to_string());
                env.push("GALLIUM_DRIVER=freedreno".to_string());
            } else {
                env.push("MESA_LOADER_DRIVER_OVERRIDE=zink".to_string());
                env.push("GALLIUM_DRIVER=zink".to_string());
            }
        }
    }

    env
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_dir(name: &str) -> std::path::PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::current_dir()
            .unwrap()
            .join(format!("husk-{name}-{}-{nonce}", std::process::id()))
    }

    fn write(path: &Path, contents: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    #[test]
    fn mali_generates_guest_relative_icd_and_loader_defaults() {
        let root = test_dir("gpu-env");
        let egl = root.join("usr/share/glvnd/egl_vendor.d/50_mesa.json");
        let source_icd = root.join(PANVK_SOURCE_ICDS[0]);
        let driver = root.join(PANVK_DRIVERS[0]);
        write(&egl, b"{}");
        write(
            &source_icd,
            format!(
                "{{\"ICD\":{{\"api_version\":\"1.4.352\",\"library_path\":\"{}/usr/lib/aarch64-linux-gnu/libvulkan_panfrost_frist370_execva.so\"}}}}",
                root.display()
            )
            .as_bytes(),
        );
        write(&driver, b"driver");

        let env = build(&root, true, false, false);
        assert!(env
            .iter()
            .any(|v| v == &format!("VK_ICD_FILENAMES={PANVK_CANONICAL_ICD}")));
        assert!(env.iter().any(|v| v == "MESA_LOADER_DRIVER_OVERRIDE=zink"));
        assert!(env.iter().any(|v| v == "GALLIUM_DRIVER=zink"));
        assert!(env.iter().any(|v| v == "MESA_GLES_VERSION_OVERRIDE=3.2"));

        let canonical =
            std::fs::read_to_string(root.join(PANVK_CANONICAL_ICD.trim_start_matches('/')))
                .unwrap();
        assert!(canonical.contains(
            "\"library_path\": \"/usr/lib/aarch64-linux-gnu/libvulkan_panfrost_frist370_execva.so\""
        ));
        assert!(canonical.contains("\"api_version\": \"1.4.352\""));
        assert!(!canonical.contains(&root.display().to_string()));
        assert!(!canonical.contains("/data/"));

        let defaults = std::fs::read_to_string(root.join("etc/husk/gpu-env")).unwrap();
        assert!(defaults.contains("@GPU_FAMILY=mali-kbase"));
        assert!(defaults.contains(&format!("VK_ICD_FILENAMES={PANVK_CANONICAL_ICD}")));
        assert!(defaults.contains("!MESA_LOADER_DRIVER_OVERRIDE=zink"));
        assert!(defaults.contains("-LIBGL_ALWAYS_SOFTWARE"));
        assert!(!defaults.contains("/data/"));

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn adreno_prefers_native_freedreno_then_falls_back_to_zink() {
        let root = test_dir("gpu-adreno");
        let egl = root.join("usr/share/glvnd/egl_vendor.d/50_mesa.json");
        let source_icd = root.join(FREEDRENO_SOURCE_ICDS[0]);
        let driver = root.join(FREEDRENO_DRIVERS[0]);
        let native_dri = root.join(FREEDRENO_NATIVE_DRI[0]);
        write(&egl, b"{}");
        write(
            &source_icd,
            b"{\"ICD\":{\"api_version\":\"1.4.352\",\"library_path\":\"/usr/lib/aarch64-linux-gnu/libvulkan_freedreno.so\"}}",
        );
        write(&driver, b"driver");
        write(&native_dri, b"dri");

        let env = build(&root, false, true, false);
        assert!(env
            .iter()
            .any(|v| v == &format!("VK_ICD_FILENAMES={FREEDRENO_CANONICAL_ICD}")));
        assert!(env.iter().any(|v| v == "TURNIP_KMD=kgsl"));
        assert!(env.iter().any(|v| v == "FD_FORCE_KGSL=1"));
        assert!(env.iter().any(|v| v == "MESA_LOADER_DRIVER_OVERRIDE=kgsl"));
        assert!(env.iter().any(|v| v == "GALLIUM_DRIVER=freedreno"));

        let canonical =
            std::fs::read_to_string(root.join(FREEDRENO_CANONICAL_ICD.trim_start_matches('/')))
                .unwrap();
        assert!(canonical
            .contains("\"library_path\": \"/usr/lib/aarch64-linux-gnu/libvulkan_freedreno.so\""));
        assert!(!canonical.contains(&root.display().to_string()));

        std::fs::remove_file(&native_dri).unwrap();
        std::fs::remove_file(root.join("etc/husk/gpu-env")).unwrap();
        let env = build(&root, false, true, false);
        assert!(env.iter().any(|v| v == "MESA_LOADER_DRIVER_OVERRIDE=zink"));
        assert!(env.iter().any(|v| v == "GALLIUM_DRIVER=zink"));

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn missing_driver_does_not_fake_hardware_or_write_defaults() {
        let root = test_dir("gpu-empty");
        std::fs::create_dir_all(&root).unwrap();
        assert!(build(&root, false, false, false).is_empty());
        assert!(build(&root, true, false, false).is_empty());
        assert!(build(&root, false, true, false).is_empty());
        assert!(!root.join("etc/husk/gpu-env").exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn software_override_is_forwarded_to_loader() {
        let root = test_dir("gpu-soft");
        std::fs::create_dir_all(&root).unwrap();
        let env = build(&root, true, false, true);
        assert!(env.iter().any(|v| v == "HUSK_GPU=llvmpipe"));
        assert!(env.iter().any(|v| v == "GALLIUM_DRIVER=llvmpipe"));
        assert!(env
            .iter()
            .any(|v| v == "MESA_LOADER_DRIVER_OVERRIDE=swrast"));
        assert!(!root.join("etc/husk/gpu-env").exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn preserves_existing_matching_loader_defaults() {
        let root = test_dir("gpu-custom");
        write(
            &root.join("usr/share/vulkan/icd.d/husk_panvk_frist370_execva.json"),
            b"{\"ICD\":{\"api_version\":\"1.4.352\"}}",
        );
        write(&root.join(PANVK_DRIVERS[0]), b"driver");
        write(
            &root.join("etc/husk/gpu-env"),
            format!("# custom\n@GPU_FAMILY=mali-kbase\nVK_ICD_FILENAMES={PANVK_CANONICAL_ICD}\n")
                .as_bytes(),
        );
        let _ = build(&root, true, false, false);
        let preserved = std::fs::read_to_string(root.join("etc/husk/gpu-env")).unwrap();
        assert!(preserved.starts_with("# custom\n"));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
