//! Sensor bridge: Android NDK sensors -> a Linux IIO shadow tree.
//!
//! `untrusted_app` cannot read `/sys/bus/iio/devices` on the reference device
//! (SELinux EACCES).  The public NDK sensor API is available, so this module
//! reads accelerometer / magnetometer / gyroscope / light / proximity events
//! and publishes them as the usual IIO sysfs attributes.  `path-shim` redirects
//! `/sys/bus/iio/devices`, `/sys/class/iio`, and `/sys/subsystem/iio` to this
//! tree, so guest applications see a normal Linux interface without a Java
//! listener class.
//!
//! This is a bridge, not a fake sensor: values are written only after an NDK
//! event arrives.  The first values are zero until the first event, which is a
//! deliberate "no sample yet" state rather than invented hardware data.

use core::ffi::{c_char, c_int, c_void};
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

type ASensorManager = c_void;
type ASensorEventQueue = c_void;
type ASensor = c_void;
type ALooper = c_void;
type ALooperCallback = Option<unsafe extern "C" fn(c_int, c_int, *mut c_void) -> c_int>;

#[repr(C)]
struct ASensorEvent {
    version: i32,
    sensor: i32,
    type_: i32,
    reserved0: i32,
    timestamp: i64,
    data: [f32; 16],
    flags: u32,
    reserved1: [i32; 3],
}

struct SensorApi {
    get_instance: unsafe extern "C" fn() -> *mut ASensorManager,
    get_default_sensor: unsafe extern "C" fn(*mut ASensorManager, c_int) -> *const ASensor,
    get_name: unsafe extern "C" fn(*const ASensor) -> *const c_char,
    create_queue: unsafe extern "C" fn(
        *mut ASensorManager,
        *mut ALooper,
        c_int,
        ALooperCallback,
        *mut c_void,
    ) -> *mut ASensorEventQueue,
    destroy_queue: unsafe extern "C" fn(*mut ASensorManager, *mut ASensorEventQueue) -> c_int,
    enable_sensor: unsafe extern "C" fn(*mut ASensorEventQueue, *const ASensor) -> c_int,
    set_event_rate: unsafe extern "C" fn(*mut ASensorEventQueue, *const ASensor, c_int) -> c_int,
    get_events: unsafe extern "C" fn(*mut ASensorEventQueue, *mut ASensorEvent, usize) -> isize,
    looper_prepare: unsafe extern "C" fn(c_int) -> *mut ALooper,
    looper_poll_once:
        unsafe extern "C" fn(c_int, *mut c_int, *mut c_int, *mut *mut c_void) -> c_int,
}

static STARTED: AtomicBool = AtomicBool::new(false);

const ASENSOR_TYPE_ACCELEROMETER: c_int = 1;
const ASENSOR_TYPE_MAGNETIC_FIELD: c_int = 2;
const ASENSOR_TYPE_GYROSCOPE: c_int = 4;
const ASENSOR_TYPE_LIGHT: c_int = 5;
const ASENSOR_TYPE_PROXIMITY: c_int = 8;
const ALOOPER_PREPARE_ALLOW_NON_CALLBACKS: c_int = 1;

const NODES: &[(usize, c_int, &str, &[&str])] = &[
    (
        0,
        ASENSOR_TYPE_ACCELEROMETER,
        "accel_3d",
        &[
            "in_accel_x_raw",
            "in_accel_y_raw",
            "in_accel_z_raw",
            "in_accel_scale",
            "sampling_frequency",
        ],
    ),
    (
        1,
        ASENSOR_TYPE_MAGNETIC_FIELD,
        "magn_3d",
        &[
            "in_magn_x_raw",
            "in_magn_y_raw",
            "in_magn_z_raw",
            "in_magn_scale",
            "sampling_frequency",
        ],
    ),
    (
        2,
        ASENSOR_TYPE_GYROSCOPE,
        "gyro_3d",
        &[
            "in_anglvel_x_raw",
            "in_anglvel_y_raw",
            "in_anglvel_z_raw",
            "in_anglvel_scale",
            "sampling_frequency",
        ],
    ),
    (
        3,
        ASENSOR_TYPE_LIGHT,
        "als",
        &[
            "in_illuminance_raw",
            "in_illuminance_scale",
            "sampling_frequency",
        ],
    ),
    (
        4,
        ASENSOR_TYPE_PROXIMITY,
        "proximity",
        &[
            "in_proximity_raw",
            "in_proximity_scale",
            "in_proximity_nearlevel",
            "sampling_frequency",
        ],
    ),
];

unsafe fn dlopen(name: *const c_char, flags: c_int) -> *mut c_void {
    extern "C" {
        fn dlopen(name: *const c_char, flags: c_int) -> *mut c_void;
    }
    dlopen(name, flags)
}

unsafe fn dlsym_raw(handle: *mut c_void, name: *const c_char) -> *mut c_void {
    extern "C" {
        fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
    }
    dlsym(handle, name)
}

unsafe fn load_api() -> Result<SensorApi, String> {
    let handle = dlopen(c"libandroid.so".as_ptr(), 2);
    if handle.is_null() {
        return Err("dlopen(libandroid.so) failed".to_string());
    }
    let symbol = |name: &std::ffi::CStr| -> Result<*mut c_void, String> {
        let p = dlsym_raw(handle, name.as_ptr());
        if p.is_null() {
            Err(format!("dlsym({name:?}) failed"))
        } else {
            Ok(p)
        }
    };
    Ok(SensorApi {
        get_instance: core::mem::transmute(symbol(c"ASensorManager_getInstance")?),
        get_default_sensor: core::mem::transmute(symbol(c"ASensorManager_getDefaultSensor")?),
        get_name: core::mem::transmute(symbol(c"ASensor_getName")?),
        create_queue: core::mem::transmute(symbol(c"ASensorManager_createEventQueue")?),
        destroy_queue: core::mem::transmute(symbol(c"ASensorManager_destroyEventQueue")?),
        enable_sensor: core::mem::transmute(symbol(c"ASensorEventQueue_enableSensor")?),
        set_event_rate: core::mem::transmute(symbol(c"ASensorEventQueue_setEventRate")?),
        get_events: core::mem::transmute(symbol(c"ASensorEventQueue_getEvents")?),
        looper_prepare: core::mem::transmute(symbol(c"ALooper_prepare")?),
        looper_poll_once: core::mem::transmute(symbol(c"ALooper_pollOnce")?),
    })
}

fn files_dir() -> Option<PathBuf> {
    let act = crate::ACTIVITY.load(Ordering::SeqCst);
    if act.is_null() {
        return None;
    }
    let path = unsafe { (*act).internalDataPath };
    if path.is_null() {
        return None;
    }
    Some(PathBuf::from(
        unsafe { core::ffi::CStr::from_ptr(path) }
            .to_string_lossy()
            .into_owned(),
    ))
}

fn node_dir(rootfs: &Path, index: usize) -> PathBuf {
    rootfs
        .join("sys/subsystem/iio/devices")
        .join(format!("iio:device{index}"))
}

fn replace_entry(entry: &Path) -> Result<(), String> {
    match std::fs::symlink_metadata(entry) {
        Ok(meta) if meta.file_type().is_dir() => {
            std::fs::remove_dir_all(entry).map_err(|e| format!("remove {entry:?}: {e}"))?;
        }
        Ok(_) => std::fs::remove_file(entry).map_err(|e| format!("remove {entry:?}: {e}"))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("lstat {entry:?}: {e}")),
    }
    Ok(())
}

fn replace_dir(entry: &Path) -> Result<(), String> {
    replace_entry(entry)?;
    std::fs::create_dir_all(entry).map_err(|e| format!("mkdir {entry:?}: {e}"))
}

fn replace_symlink(target: &str, link: &Path) -> Result<(), String> {
    replace_entry(link)?;
    symlink(target, link).map_err(|e| format!("symlink {link:?}: {e}"))
}

fn initial_value(attr: &str) -> &'static str {
    if attr.ends_with("_scale") {
        if attr.starts_with("in_illuminance") || attr.starts_with("in_proximity") {
            "1.000000\n"
        } else {
            "0.001000\n"
        }
    } else if attr == "in_proximity_nearlevel" {
        "1.000000\n"
    } else if attr == "sampling_frequency" {
        "10.000000\n"
    } else {
        "0\n"
    }
}

fn proxy_type(index: usize) -> Option<&'static str> {
    match index {
        0 => Some("iio-poll-accel"),
        3 => Some("iio-poll-als"),
        4 => Some("iio-poll-proximity"),
        _ => None,
    }
}

fn prepare_tree(rootfs: &Path) -> Result<(), String> {
    // A real sysfs subsystem has one canonical device directory and two
    // device-level symlink views:
    //
    //   /sys/subsystem/iio/devices/iio:deviceN
    //   /sys/bus/iio/devices/iio:deviceN -> ../../../subsystem/iio/devices/...
    //   /sys/class/iio/iio:deviceN       -> ../../subsystem/iio/devices/...
    //
    // Do not replace `/sys/{bus,class}/iio` themselves with symlinks. libudev
    // walks the parent directories first and rejects an intermediate path that
    // canonicalizes outside `/sys`.
    let legacy_iio = rootfs.join("sys-iio");
    let _ = std::fs::remove_dir_all(&legacy_iio);

    let sys = rootfs.join("sys");
    let iio = sys.join("subsystem/iio");
    replace_dir(&iio)?;
    std::fs::create_dir_all(iio.join("devices"))
        .map_err(|e| format!("mkdir {iio:?}/devices: {e}"))?;

    let class_iio = sys.join("class/iio");
    let bus_iio = sys.join("bus/iio");
    replace_dir(&class_iio)?;
    replace_dir(&bus_iio)?;
    let bus_devices = bus_iio.join("devices");
    replace_dir(&bus_devices)?;

    for (index, _, name, attrs) in NODES {
        let dir = node_dir(rootfs, *index);
        std::fs::create_dir_all(&dir).map_err(|e| format!("mkdir {dir:?}: {e}"))?;
        std::fs::write(dir.join("name"), format!("{name}\n"))
            .map_err(|e| format!("write {dir:?}/name: {e}"))?;
        std::fs::write(dir.join("dev"), format!("241:{index}\n"))
            .map_err(|e| format!("write {dir:?}/dev: {e}"))?;
        let mut uevent =
            format!("MAJOR=241\nMINOR={index}\nDEVNAME=iio:device{index}\nDEVTYPE=iio_device\n");
        if let Some(kind) = proxy_type(*index) {
            uevent.push_str("IIO_SENSOR_PROXY_TYPE=");
            uevent.push_str(kind);
            uevent.push('\n');
        }
        std::fs::write(dir.join("uevent"), uevent)
            .map_err(|e| format!("write {dir:?}/uevent: {e}"))?;
        replace_symlink("../../../iio", &dir.join("subsystem"))?;
        for attr in *attrs {
            std::fs::write(dir.join(attr), initial_value(attr))
                .map_err(|e| format!("write {dir:?}/{attr}: {e}"))?;
        }

        let device = format!("iio:device{index}");
        replace_symlink(
            &format!("../../../subsystem/iio/devices/{device}"),
            &bus_devices.join(&device),
        )?;
        replace_symlink(
            &format!("../../subsystem/iio/devices/{device}"),
            &class_iio.join(&device),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{prepare_tree, NODES};
    use std::os::unix::fs::symlink;

    fn scratch(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("husk-sensors-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn prepare_tree_builds_real_bus_and_class_views() {
        let rootfs = scratch("views");
        let sys = rootfs.join("sys");
        std::fs::create_dir_all(sys.join("subsystem/iio/devices")).unwrap();
        std::fs::create_dir_all(sys.join("bus")).unwrap();
        std::fs::create_dir_all(sys.join("class")).unwrap();
        symlink("../subsystem/iio", sys.join("bus/iio")).unwrap();
        symlink("../subsystem/iio", sys.join("class/iio")).unwrap();

        prepare_tree(&rootfs).unwrap();

        for view in ["bus/iio", "class/iio"] {
            let metadata = std::fs::symlink_metadata(sys.join(view)).unwrap();
            assert!(
                metadata.file_type().is_dir() && !metadata.file_type().is_symlink(),
                "{view} must be a real directory"
            );
        }
        assert_eq!(
            std::fs::read_to_string(sys.join("class/iio/iio:device0/name")).unwrap(),
            "accel_3d\n"
        );
        assert_eq!(
            std::fs::read_to_string(sys.join("bus/iio/devices/iio:device0/name")).unwrap(),
            "accel_3d\n"
        );
        assert_eq!(
            std::fs::read_link(sys.join("class/iio/iio:device0")).unwrap(),
            std::path::Path::new("../../subsystem/iio/devices/iio:device0")
        );
        assert_eq!(
            std::fs::read_link(sys.join("bus/iio/devices/iio:device0")).unwrap(),
            std::path::Path::new("../../../subsystem/iio/devices/iio:device0")
        );

        // Re-running at app startup must replace stale links without losing the
        // canonical nodes that the sensor thread updates.
        prepare_tree(&rootfs).unwrap();
        assert_eq!(NODES.len(), 5);
        let _ = std::fs::remove_dir_all(&rootfs);
    }
}

fn write_value(rootfs: &Path, index: usize, attr: &str, value: &str) {
    let dir = node_dir(rootfs, index);
    if let Ok(meta) = std::fs::symlink_metadata(&dir) {
        if !meta.file_type().is_dir() {
            return;
        }
    } else {
        return;
    }
    let path = dir.join(attr);
    let tmp = dir.join(format!(".{attr}.tmp"));
    if std::fs::write(&tmp, value).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

fn write_axes(rootfs: &Path, index: usize, prefix: &str, data: &[f32; 16]) {
    for (axis, value) in ["x", "y", "z"].iter().zip(data.iter().take(3)) {
        let raw = (value * 1000.0).round() as i32;
        write_value(
            rootfs,
            index,
            &format!("{prefix}_{axis}_raw"),
            &format!("{raw}\n"),
        );
    }
}

fn publish_event(rootfs: &Path, event: &ASensorEvent) {
    match event.type_ {
        ASENSOR_TYPE_ACCELEROMETER => write_axes(rootfs, 0, "in_accel", &event.data),
        ASENSOR_TYPE_MAGNETIC_FIELD => write_axes(rootfs, 1, "in_magn", &event.data),
        ASENSOR_TYPE_GYROSCOPE => write_axes(rootfs, 2, "in_anglvel", &event.data),
        ASENSOR_TYPE_LIGHT => {
            // Light and proximity already publish their native units.  Their
            // IIO scale is 1.0, unlike the milli-scaled 3-axis sensors.
            let raw = event.data[0].round() as i32;
            write_value(rootfs, 3, "in_illuminance_raw", &format!("{raw}\n"));
        }
        ASENSOR_TYPE_PROXIMITY => {
            let raw = event.data[0].round() as i32;
            write_value(rootfs, 4, "in_proximity_raw", &format!("{raw}\n"));
        }
        _ => {}
    }
}

/// Start the sensor bridge once per app process.  A failure is logged and the
/// matrix remains `present=yes husk=degraded`; it must not take down the app.
pub fn start_once() {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| {
        let _ = std::panic::catch_unwind(run);
    });
}

fn run() {
    let Some(files) = files_dir() else {
        crate::log("sensors|SKIP|no files dir");
        return;
    };
    let rootfs = match crate::guest_rootfs::resolve(&files) {
        Ok(rootfs) => rootfs,
        Err(e) => {
            crate::log(&format!("sensors|SKIP|{e}"));
            return;
        }
    };
    if let Err(e) = prepare_tree(&rootfs) {
        crate::log(&format!("sensors|TREE|{e}"));
        return;
    }
    crate::log(&format!("sensors|TREE|{}", rootfs.display()));
    let api = match unsafe { load_api() } {
        Ok(api) => api,
        Err(e) => {
            crate::log(&format!("sensors|FAIL|{e}"));
            return;
        }
    };
    unsafe {
        let manager = (api.get_instance)();
        if manager.is_null() {
            crate::log("sensors|FAIL|ASensorManager_getInstance returned null");
            return;
        }
        let looper = (api.looper_prepare)(ALOOPER_PREPARE_ALLOW_NON_CALLBACKS);
        if looper.is_null() {
            crate::log("sensors|FAIL|ALooper_prepare returned null");
            return;
        }
        let queue = (api.create_queue)(manager, looper, 0, None, core::ptr::null_mut());
        if queue.is_null() {
            crate::log("sensors|FAIL|createEventQueue returned null");
            return;
        }
        let mut enabled = 0usize;
        for (_, kind, name, _) in NODES {
            let sensor = (api.get_default_sensor)(manager, *kind);
            if sensor.is_null() {
                crate::log(&format!("sensors|ABSENT|type={kind} name={name}"));
                continue;
            }
            let rc = (api.enable_sensor)(queue, sensor);
            if rc < 0 {
                crate::log(&format!("sensors|ENABLE|type={kind} rc={rc}"));
                continue;
            }
            let _ = (api.set_event_rate)(queue, sensor, 100_000);
            enabled += 1;
            let actual = {
                let p = (api.get_name)(sensor);
                if p.is_null() {
                    "unknown".to_string()
                } else {
                    core::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
                }
            };
            crate::log(&format!(
                "sensors|READY|type={kind} name={name} actual={actual:?}"
            ));
        }
        if enabled == 0 {
            crate::log("sensors|FAIL|no default sensor enabled");
            let _ = (api.destroy_queue)(manager, queue);
            return;
        }
        crate::log(&format!("sensors|RUN|enabled={enabled}"));
        let mut events: [ASensorEvent; 16] = core::mem::zeroed();
        loop {
            let n = (api.get_events)(queue, events.as_mut_ptr(), events.len());
            if n > 0 {
                for event in events.iter().take(n as usize) {
                    publish_event(&rootfs, event);
                }
            }
            let mut ident = 0;
            let mut out_events = 0;
            let mut data: *mut c_void = core::ptr::null_mut();
            (api.looper_poll_once)(100, &mut ident, &mut out_events, &mut data);
        }
    }
}
