//! Build the guest's V4L2 sysfs facade.
//!
//! The Android kernel has no V4L2 video nodes. `libhusk_v4l2` redirects
//! `/sys/class/video4linux` into a shadow tree while libudev still checks that
//! the real subsystem roots exist. Both halves are required: missing shadow
//! entries hide the device, while missing real roots leave WirePlumber with an
//! accessible `/dev/video*` that never becomes a `Video/Device`.

use std::path::Path;

/// Write one V4L2 facade for `labels`, where index N maps to `/dev/videoN`.
///
/// Labels are the guest-visible camera names, normally `BACK` and `FRONT`.
pub fn prepare(rootfs: &Path, labels: &[&str]) -> Result<(), String> {
    let class = rootfs.join("sys-v4l2/class");
    let devices = rootfs.join("sys-v4l2/subsystem/devices");
    let subsystem = rootfs.join("sys-v4l2/subsystem/video4linux");
    let bus = rootfs.join("sys-v4l2/bus/video4linux/devices");
    let udev = rootfs.join("run/udev/data");

    // libudev checks the real subsystem directories before it follows the
    // loader's shadow-tree redirect.
    let real_class = rootfs.join("sys/class/video4linux");
    let real_bus = rootfs.join("sys/bus/video4linux");
    let real_subsystem = rootfs.join("sys/subsystem/video4linux");

    // Camera permission can be revoked between launches. Rebuild only the
    // shadow tree; never remove the real sysfs roots.
    for stale in [&class, &devices, &subsystem, &bus] {
        let _ = std::fs::remove_dir_all(stale);
    }
    for dir in [
        &class,
        &devices,
        &subsystem.join("devices"),
        &bus,
        &udev,
        &real_class,
        &real_bus,
        &real_subsystem,
    ] {
        std::fs::create_dir_all(dir).map_err(|e| format!("mkdir {dir:?}: {e}"))?;
    }

    if let Ok(entries) = std::fs::read_dir(&udev) {
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().starts_with("c81:") {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }

    for (k, label) in labels.iter().enumerate() {
        write_device(&class, &devices, &subsystem, &bus, &udev, k, label)?;
    }
    Ok(())
}

fn write_device(
    class: &Path,
    devices: &Path,
    subsystem: &Path,
    bus: &Path,
    udev: &Path,
    k: usize,
    label: &str,
) -> Result<(), String> {
    let uevent = format!("MAJOR=81\nMINOR={k}\nDEVNAME=video{k}\nDEVTYPE=video4linux\n");
    let dev = format!("81:{k}\n");
    let name = format!("{label}\n");
    let index = format!("{k}\n");
    let fields = [
        ("uevent", uevent.as_str()),
        ("dev", dev.as_str()),
        ("name", name.as_str()),
        ("index", index.as_str()),
    ];

    for dir in [
        class.join(format!("video{k}")),
        devices.join(format!("video{k}")),
    ] {
        std::fs::create_dir_all(&dir).map_err(|e| format!("mkdir {dir:?}: {e}"))?;
        for (field, value) in fields {
            std::fs::write(dir.join(field), value)
                .map_err(|e| format!("write {dir:?}/{field}: {e}"))?;
        }
    }

    let canonical = subsystem.join("devices").join(format!("video{k}"));
    std::fs::create_dir_all(&canonical).map_err(|e| format!("mkdir {canonical:?}: {e}"))?;
    for (field, value) in fields {
        std::fs::write(canonical.join(field), value)
            .map_err(|e| format!("write {canonical:?}/{field}: {e}"))?;
    }

    make_symlink(Path::new("../.."), &canonical.join("subsystem"))?;
    // sd-device derives SUBSYSTEM from the basename of this link. It must point
    // at `/sys/subsystem/video4linux`, not at the device directory.
    make_symlink(
        Path::new("../../subsystem/video4linux"),
        &class.join(format!("video{k}")).join("subsystem"),
    )?;
    make_symlink(
        Path::new(&format!("../../../subsystem/video4linux/devices/video{k}")),
        &bus.join(format!("video{k}")),
    )?;

    let db = format!(
        "I:1\nE:ID_V4L_CAPABILITIES=:capture:\nE:ID_V4L_PRODUCT={label}\nE:DEVNAME=/dev/video{k}\nE:DEVTYPE=video4linux\n"
    );
    let db_path = udev.join(format!("c81:{k}"));
    std::fs::write(&db_path, db).map_err(|e| format!("write {db_path:?}: {e}"))
}

fn make_symlink(target: &Path, link: &Path) -> Result<(), String> {
    let _ = std::fs::remove_file(link);
    std::os::unix::fs::symlink(target, link)
        .map_err(|e| format!("symlink {:?} -> {:?}: {e}", link, target))
}

#[cfg(test)]
mod tests {
    use super::prepare;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "husk-camera-tree-{name}-{}-{nonce}",
            std::process::id()
        ))
    }

    use std::path::PathBuf;

    #[test]
    fn creates_real_roots_shadow_devices_and_udev_rows() {
        let root = temp_root("full");
        prepare(&root, &["BACK", "FRONT"]).unwrap();

        for rel in [
            "sys/class/video4linux",
            "sys/bus/video4linux",
            "sys/subsystem/video4linux",
            "sys-v4l2/class/video0",
            "sys-v4l2/class/video1",
            "sys-v4l2/subsystem/video4linux/devices/video0",
            "sys-v4l2/subsystem/video4linux/devices/video1",
        ] {
            assert!(root.join(rel).is_dir(), "missing {rel}");
        }
        assert_eq!(
            std::fs::read_to_string(root.join("sys-v4l2/class/video0/name")).unwrap(),
            "BACK\n"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("sys-v4l2/class/video1/name")).unwrap(),
            "FRONT\n"
        );
        assert!(root.join("run/udev/data/c81:0").is_file());
        assert!(root.join("run/udev/data/c81:1").is_file());

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rebuild_removes_stale_shadow_nodes_but_keeps_real_roots() {
        let root = temp_root("empty");
        prepare(&root, &["BACK", "FRONT"]).unwrap();
        prepare(&root, &[]).unwrap();

        assert!(root.join("sys/class/video4linux").is_dir());
        assert!(root.join("sys/bus/video4linux").is_dir());
        assert!(root.join("sys/subsystem/video4linux").is_dir());
        assert!(!root.join("sys-v4l2/class/video0").exists());
        assert!(!root.join("sys-v4l2/class/video1").exists());
        assert!(!root.join("run/udev/data/c81:0").exists());
        assert!(!root.join("run/udev/data/c81:1").exists());

        std::fs::remove_dir_all(root).unwrap();
    }
}
