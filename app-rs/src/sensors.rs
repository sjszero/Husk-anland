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

fn active_rootfs(files: &Path) -> PathBuf {
    let full = files.join("rootfs-full");
    if full.is_dir() {
        full
    } else {
        files.join("rootfs")
    }
}

fn node_dirs(rootfs: &Path, index: usize) -> [PathBuf; 3] {
    let name = format!("iio:device{index}");
    [
        rootfs.join("sys-iio/bus/iio/devices").join(&name),
        rootfs.join("sys-iio/class/iio").join(&name),
        rootfs.join("sys-iio/subsystem/iio").join(&name),
    ]
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

fn prepare_tree(rootfs: &Path) -> Result<(), String> {
    let iio = rootfs.join("sys-iio");
    let _ = std::fs::remove_dir_all(&iio);
    for (index, _, name, attrs) in NODES {
        for dir in node_dirs(rootfs, *index) {
            std::fs::create_dir_all(&dir).map_err(|e| format!("mkdir {dir:?}: {e}"))?;
            std::fs::write(dir.join("name"), format!("{name}\n"))
                .map_err(|e| format!("write {dir:?}/name: {e}"))?;
            std::fs::write(dir.join("uevent"), b"IIO_DEVICE=1\n")
                .map_err(|e| format!("write {dir:?}/uevent: {e}"))?;
            for attr in *attrs {
                std::fs::write(dir.join(attr), initial_value(attr))
                    .map_err(|e| format!("write {dir:?}/{attr}: {e}"))?;
            }
        }
    }
    Ok(())
}

fn write_value(rootfs: &Path, index: usize, attr: &str, value: &str) {
    for dir in node_dirs(rootfs, index) {
        let path = dir.join(attr);
        let tmp = dir.join(format!(".{attr}.tmp"));
        if std::fs::write(&tmp, value).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
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
    let rootfs = active_rootfs(&files);
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
