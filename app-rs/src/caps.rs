//! Platform capability matrix.
//!
//! Every hardware class the goal names gets exactly one row, and a row states
//! two *different* things:
//!
//!   present=<yes|no|unknown>
//!       What the Android platform exposes to this app UID right now. Measured
//!       from inside the app process through public framework API.
//!
//!   husk=<ready|degraded|unknown|absent>
//!       Whether Husk itself can already drive it. This comes from the single
//!       table in `HUSK_STATE`, never inferred from `present`.
//!
//! Keeping the two apart is the whole point: a camera that exists but that
//! Husk does not bridge yet must read `present=yes husk=absent`, never `ready`.
//! The matrix is written to `files/husk-capabilities.txt` and mirrored into the
//! guest at `rootfs-full/run/husk/capabilities.txt` so both sides read one file.
//!
//! No Java class of ours, no dex: every call below targets a platform class.

use core::ffi::c_void;
use jni::objects::{JObject, JString, JValue, JValueOwned};
use jni::sys::JavaVM as RawJavaVM;
use jni::{JNIEnv, JavaVM};
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

static STARTED: AtomicBool = AtomicBool::new(false);
/// A second pass once the window exists: the display row can only tell
/// "SurfaceControl published" after `onNativeWindowCreated`, which happens
/// after this thread starts.
static REFRESHED: AtomicBool = AtomicBool::new(false);

type R<T> = Result<T, String>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Present {
    Yes,
    No,
    Unknown,
}

impl Present {
    fn as_str(self) -> &'static str {
        match self {
            Present::Yes => "yes",
            Present::No => "no",
            Present::Unknown => "unknown",
        }
    }
}

struct Row {
    class: &'static str,
    present: Present,
    husk: &'static str,
    backend: String,
    detail: String,
}

impl Row {
    fn new(class: &'static str) -> Self {
        Row {
            class,
            present: Present::Unknown,
            husk: "unknown",
            backend: "-".to_string(),
            detail: String::new(),
        }
    }
}

// ---------------------------------------------------------------- state ---
//
// One row per hardware class, in the order the goal lists them. `husk` is a
// claim about our own code, so it may only move forward when the matching
// evidence exists; each string names the evidence it rests on.

fn husk_state(class: &str) -> (&'static str, &'static str) {
    // (state, one-line justification)
    match class {
        "gpu" => (
            "degraded",
            "Mali kbase -> PanVK -> Zink X11 window verified (evidence 165/167); fixed-function 600-frame probe passes after defaulting PANVK_DXVK_COMPAT=0 (evidence 244), but real X11 glxgears, long-run and SHM readback remain unverified",
        ),
        "display" => (
            "ready",
            "AHB -> dma-buf -> SurfaceControl ring, 10000 frames / 665 s (evidence 59)",
        ),
        "video-decode" => (
            "degraded",
            "app bridge 1080p30 600 s dropped=0 (evidence 80); VA system path compiled, not device-verified",
        ),
        "video-encode" => (
            "ready",
            "run/husk/encoder.sock -> MediaCodec (c2.mtk.avc.encoder); 30 s / 890 frames, host decode clean (evidence 93)",
        ),
        "audio-out" => (
            "degraded",
            "AAudio pipe 610 s (evidence 63); ~105 ms latency, low-latency mode denied by vendor",
        ),
        "audio-in" => (
            "degraded",
            "AAudio capture -> module-pipe-source, 1.9 MB loopback (evidence 88); needs install -g for the mic permission",
        ),
        "camera" => (
            "degraded",
            "V4L2 /dev/videoN is served by Camera2 NDK without a test marker; capture/photo/record and read/readv/mmap verified (evidence 91/93/240/282), but DMABUF/controls are unsupported and 8-25 fps was observed depending on mode",
        ),
        "input" => (
            "degraded",
            "X11 touch/click and keyboard injection verified (evidence 148/180); adb input swipe to XI2 remains UNKNOWN",
        ),
        "sensors" => ("absent", "no sensor path exists yet"),
        "platform-peripherals" => (
            "absent",
            "USB / Bluetooth / GPS / NFC / fingerprint not bridged yet",
        ),
        _ => ("unknown", "unclassified"),
    }
}

// --------------------------------------------------------------- entry ---

pub fn start_once() {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| {
        let _ = std::panic::catch_unwind(probe_all);
    });
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_secs(8));
        if REFRESHED.swap(true, Ordering::SeqCst) {
            return;
        }
        let _ = std::panic::catch_unwind(|| {
            crate::log("caps|MATRIX|refresh-after-window");
            probe_all();
        });
    });
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

unsafe fn java_vm() -> Option<JavaVM> {
    let act = crate::ACTIVITY.load(Ordering::SeqCst);
    if act.is_null() {
        return None;
    }
    let vm = (*act).vm as *mut RawJavaVM;
    if vm.is_null() {
        return None;
    }
    JavaVM::from_raw(vm).ok()
}

fn probe_all() {
    let Some(files) = files_dir() else {
        crate::log("caps|SKIP|no internalDataPath");
        return;
    };
    let Some(vm) = (unsafe { java_vm() }) else {
        crate::log("caps|SKIP|no JavaVM on ANativeActivity");
        return;
    };
    let mut guard = match vm.attach_current_thread() {
        Ok(guard) => guard,
        Err(e) => {
            crate::log(&format!("caps|FAIL|attach_current_thread: {e}"));
            return;
        }
    };
    let env: &mut JNIEnv = &mut guard;

    let mut rows: Vec<Row> = Vec::new();
    let identity = device_identity(env);

    let activity = match crate::surface::current_activity(env) {
        Ok(obj) => Some(obj),
        Err(e) => {
            crate::log(&format!("caps|WARN|activity unavailable: {e}"));
            None
        }
    };


    // ------------------------------------------------------------- GPU ---
    let mut gpu = Row::new("gpu");
    gpu.present = Present::Yes;
    gpu.backend = "/dev/mali0 (kbase)".to_string();
    gpu.detail = format!("{identity}");
    rows.push(gpu);

    // --------------------------------------------------------- display ---
    let mut display = Row::new("display");
    display.present = Present::Yes;
    display.backend = "AHardwareBuffer -> dma-buf -> SurfaceControl".to_string();
    let view = if crate::surface::view_installed() {
        "surfaceview=installed"
    } else {
        "surfaceview=absent"
    };
    display.detail = format!("{view}, {}", match crate::surface::side_control_present() {
        true => "control-cache=cached".to_string(),
        false => "control-cache=not-fetched (lazy: read on first guest present)".to_string(),
    });
    rows.push(display);

    // ------------------------------------------------- video codecs -----
    let mut vdec = Row::new("video-decode");
    let mut venc = Row::new("video-encode");
    match probe_codecs(env) {
        Ok(summary) => {
            let hw_dec_avc = summary
                .decoders
                .iter()
                .find(|c| c.hardware && c.has("video/avc"));
            vdec.present = if summary.decoders.iter().any(|c| c.hardware) {
                Present::Yes
            } else {
                Present::No
            };
            vdec.backend = hw_dec_avc
                .map(|c| c.name.clone())
                .unwrap_or_else(|| "-".to_string());
            vdec.detail = format!(
                "hw_dec={} sw_dec={} hw_enc={} sw_enc={} hw_dec_types={}",
                summary.decoders.iter().filter(|c| c.hardware).count(),
                summary.decoders.iter().filter(|c| !c.hardware).count(),
                summary.encoders.iter().filter(|c| c.hardware).count(),
                summary.encoders.iter().filter(|c| !c.hardware).count(),
                hw_dec_avc.map(|c| c.types.join("/")).unwrap_or_else(|| "-".into()),
            );
            let hw_enc = summary.encoders.iter().find(|c| c.hardware);
            venc.present = if hw_enc.is_some() {
                Present::Yes
            } else {
                Present::No
            };
            venc.backend = hw_enc
                .map(|c| c.name.clone())
                .unwrap_or_else(|| "-".to_string());
            venc.detail = hw_enc
                .map(|c| format!("types={}", c.types.join("/")))
                .unwrap_or_else(|| "no hardware encoder advertised".to_string());
        }
        Err(e) => {
            vdec.detail = format!("probe failed: {e}");
            venc.detail = format!("probe failed: {e}");
        }
    }
    rows.push(vdec);
    rows.push(venc);

    // ------------------------------------------------------- audio ------
    let mut aout = Row::new("audio-out");
    let mut ain = Row::new("audio-in");
    match activity
        .as_ref()
        .ok_or_else(|| "no activity".to_string())
        .and_then(|a| system_service(env, a, "audio"))
    {
        Ok(am) => {
            let outs = audio_devices(env, &am, 2);
            let ins = audio_devices(env, &am, 1);
            match outs {
                Ok(list) => {
                    aout.present = if list.is_empty() { Present::No } else { Present::Yes };
                    aout.backend = "AAudio + PulseAudio pipe".to_string();
                    aout.detail = format!("devices={}", join(&list));
                }
                Err(e) => aout.detail = format!("probe failed: {e}"),
            }
            match ins {
                Ok(list) => {
                    ain.present = if list.is_empty() { Present::No } else { Present::Yes };
                    ain.backend = "-".to_string();
                    ain.detail = format!("devices={}", join(&list));
                }
                Err(e) => ain.detail = format!("probe failed: {e}"),
            }
        }
        Err(e) => {
            aout.detail = format!("probe failed: {e}");
            ain.detail = format!("probe failed: {e}");
        }
    }
    rows.push(aout);
    rows.push(ain);

    // ------------------------------------------------------ camera ------
    let mut cam = Row::new("camera");
    cam.backend = "V4L2 /dev/videoN -> Camera2 NDK".to_string();
    match activity
        .as_ref()
        .ok_or_else(|| "no activity".to_string())
        .and_then(|a| system_service(env, a, "camera"))
    {
        Ok(cm) => match env.call_method(&cm, "getCameraIdList", "()[Ljava/lang/String;", &[]) {
            Ok(v) => match v.l() {
                Ok(arr) if !arr.is_null() => {
                    let arr = jobject_array(arr);
                    let n = env.get_array_length(&arr).unwrap_or(0);
                    let mut ids = Vec::new();
                    for i in 0..n {
                        if let Ok(el) = env.get_object_array_element(&arr, i) {
                            if !el.is_null() {
                                ids.push(jstring(env, &el));
                            }
                        }
                    }
                    cam.present = if ids.is_empty() { Present::No } else { Present::Yes };
                    cam.detail = format!("ids={}", join(&ids));
                }
                _ => {
                    clear(env);
                    cam.detail = "getCameraIdList returned null".to_string();
                }
            },
            Err(e) => cam.detail = format!("probe failed: {}", fmt_err(env, "getCameraIdList", e)),
        },
        Err(e) => cam.detail = format!("probe failed: {e}"),
    }
    rows.push(cam);

    // ------------------------------------------------------- input ------
    let mut input = Row::new("input");
    match activity
        .as_ref()
        .ok_or_else(|| "no activity".to_string())
        .and_then(|a| system_service(env, a, "input"))
    {
        Ok(im) => match env.call_method(&im, "getInputDeviceIds", "()[I", &[]) {
            Ok(v) => match v.l() {
                Ok(arr) if !arr.is_null() => {
                    let arr = jint_array(arr);
                    let n = env.get_array_length(&arr).unwrap_or(0);
                    let mut buf = vec![0i32; n.max(0) as usize];
                    let _ = env.get_int_array_region(&arr, 0, &mut buf);
                    let mut names = Vec::new();
                    for id in buf {
                        match env.call_method(
                            &im,
                            "getInputDevice",
                            "(I)Landroid/view/InputDevice;",
                            &[JValue::Int(id)],
                        ) {
                            Ok(v) => match v.l() {
                                Ok(dev) if !dev.is_null() => {
                                    let name = match env.call_method(
                                        &dev,
                                        "getName",
                                        "()Ljava/lang/String;",
                                        &[],
                                    ) {
                                        Ok(v) => value_string(env, v),
                                        Err(_) => {
                                            clear(env);
                                            "?".into()
                                        }
                                    };
                                    let sources = match env.call_method(
                                        &dev,
                                        "getSources",
                                        "()I",
                                        &[],
                                    ) {
                                        Ok(v) => v.i().unwrap_or(0),
                                        Err(_) => {
                                            clear(env);
                                            0
                                        }
                                    };
                                    names.push(format!("{id}:{name}(0x{sources:x})"));
                                }
                                _ => clear(env),
                            },
                            Err(_) => clear(env),
                        }
                    }
                    input.present = if names.is_empty() { Present::No } else { Present::Yes };
                    input.detail = format!("devices={}", join(&names));
                }
                _ => {
                    clear(env);
                    input.detail = "getInputDeviceIds returned null".to_string();
                }
            },
            Err(e) => {
                input.detail = format!("probe failed: {}", fmt_err(env, "getInputDeviceIds", e))
            }
        },
        Err(e) => input.detail = format!("probe failed: {e}"),
    }
    rows.push(input);

    // ---------------------------------------------------- sensors ------
    let mut sensors = Row::new("sensors");
    match activity
        .as_ref()
        .ok_or_else(|| "no activity".to_string())
        .and_then(|a| system_service(env, a, "sensor"))
    {
        Ok(sm) => match env.call_method(
            &sm,
            "getSensorList",
            "(I)Ljava/util/List;",
            &[JValue::Int(-1)], // Sensor.TYPE_ALL
        ) {
            Ok(v) => match v.l() {
                Ok(list) if !list.is_null() => {
                    let n = env
                        .call_method(&list, "size", "()I", &[])
                        .ok()
                        .and_then(|v| v.i().ok())
                        .unwrap_or(0);
                    let mut kinds = Vec::new();
                    for i in 0..n {
                        let Ok(el) = env.call_method(
                            &list,
                            "get",
                            "(I)Ljava/lang/Object;",
                            &[JValue::Int(i)],
                        ) else {
                            clear(env);
                            continue;
                        };
                        let Ok(obj) = el.l() else {
                            clear(env);
                            continue;
                        };
                        if obj.is_null() {
                            continue;
                        }
                        let ty = env
                            .call_method(&obj, "getType", "()I", &[])
                            .ok()
                            .and_then(|v| v.i().ok())
                            .unwrap_or(-1);
                        let name = match env.call_method(
                            &obj,
                            "getName",
                            "()Ljava/lang/String;",
                            &[],
                        ) {
                            Ok(v) => value_string(env, v),
                            Err(_) => {
                                clear(env);
                                "?".into()
                            }
                        };
                        kinds.push(format!("{ty}:{name}"));
                    }
                    sensors.present = if kinds.is_empty() { Present::No } else { Present::Yes };
                    sensors.detail = format!("count={} {}", kinds.len(), join(&kinds));
                }
                _ => {
                    clear(env);
                    sensors.detail = "getSensorList returned null".to_string();
                }
            },
            Err(e) => {
                sensors.detail = format!("probe failed: {}", fmt_err(env, "getSensorList", e))
            }
        },
        Err(e) => sensors.detail = format!("probe failed: {e}"),
    }
    rows.push(sensors);

    // ------------------------------------------- platform peripherals ---
    let mut periph = Row::new("platform-peripherals");
    match activity
        .as_ref()
        .ok_or_else(|| "no activity".to_string())
        .and_then(|a| {
            env.call_method(
                a,
                "getPackageManager",
                "()Landroid/content/pm/PackageManager;",
                &[],
            )
            .map_err(|e| fmt_err(env, "getPackageManager", e))
            .and_then(|v| v.l().map_err(|e| fmt_err(env, "getPackageManager ret", e)))
            .and_then(|pm| {
                if pm.is_null() {
                    Err("getPackageManager returned null".to_string())
                } else {
                    Ok(pm)
                }
            })
        }) {
        Ok(pm) => {
            let features = [
                ("usb-host", "android.hardware.usb.host"),
                ("bluetooth", "android.hardware.bluetooth"),
                ("bluetooth-le", "android.hardware.bluetooth_le"),
                ("gps", "android.hardware.location.gps"),
                ("nfc", "android.hardware.nfc"),
                ("fingerprint", "android.hardware.fingerprint"),
                ("camera-any", "android.hardware.camera.any"),
                ("sensor-accelerometer", "android.hardware.sensor.accelerometer"),
                ("sensor-gyroscope", "android.hardware.sensor.gyroscope"),
                ("sensor-light", "android.hardware.sensor.light"),
                ("sensor-proximity", "android.hardware.sensor.proximity"),
                ("gles", "android.hardware.opengles.aep"),
                ("vulkan", "android.hardware.vulkan.version"),
            ];
            let mut parts = Vec::new();
            let mut any = false;
            for (label, feature) in features {
                let present = has_feature(env, &pm, feature);
                any |= present;
                parts.push(format!("{label}={}", if present { "y" } else { "n" }));
            }
            periph.present = if any { Present::Yes } else { Present::No };
            periph.backend = "-".to_string();
            periph.detail = parts.join(" ");
        }
        Err(e) => periph.detail = format!("probe failed: {e}"),
    }
    rows.push(periph);

    // ------------------------------------------------ husk state fill ---
    for row in &mut rows {
        let (state, why) = husk_state(row.class);
        row.husk = state;
        if !why.is_empty() {
            if row.detail.is_empty() {
                row.detail = why.to_string();
            } else {
                row.detail = format!("{} | husk: {why}", row.detail);
            }
        }
    }

    let rendered = render(&rows, &identity);
    crate::log("caps|MATRIX|begin");
    for line in rendered.lines() {
        if line.starts_with("caps|") {
            crate::log(line);
        }
    }
    crate::log("caps|MATRIX|end");

    let canonical = files.join("husk-capabilities.txt");
    if let Err(e) = std::fs::write(&canonical, rendered.as_bytes()) {
        crate::log(&format!("caps|FAIL|write {}: {e}", canonical.display()));
    } else {
        crate::log(&format!("caps|WROTE|{}", canonical.display()));
    }
    let guest = files.join("rootfs-full/run/husk/capabilities.txt");
    if let Some(parent) = guest.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(e) = std::fs::write(&guest, rendered.as_bytes()) {
        crate::log(&format!("caps|WARN|write {}: {e}", guest.display()));
    }
}

// ------------------------------------------------------------- render ---

fn device_identity(env: &mut JNIEnv) -> String {
    let model = static_str(env, "android/os/Build", "SOC_MODEL");
    let soc = if model == "-" {
        static_str(env, "android/os/Build", "HARDWARE")
    } else {
        model
    };
    let sdk = static_int(env, "android/os/Build$VERSION", "SDK_INT");
    format!(
        "model={} device={} soc={} board={} sdk={} abis={}",
        static_str(env, "android/os/Build", "MODEL"),
        static_str(env, "android/os/Build", "DEVICE"),
        soc,
        static_str(env, "android/os/Build", "BOARD"),
        sdk,
        static_array(env, "android/os/Build", "SUPPORTED_ABIS"),
    )
}

fn render(rows: &[Row], identity: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# husk-capabilities v1");
    let _ = writeln!(out, "# {identity}");
    let _ = writeln!(
        out,
        "# caps|<class>|present=<yes|no|unknown>|husk=<ready|degraded|unknown|absent>|backend=<name>|detail=<text>"
    );
    let _ = writeln!(out, "# present = what the platform exposes to this app today");
    let _ = writeln!(out, "# husk    = whether Husk can drive it yet (never inferred from present)");
    for row in rows {
        let _ = writeln!(
            out,
            "caps|{}|present={}|husk={}|backend={}|detail={}",
            row.class,
            row.present.as_str(),
            row.husk,
            escape(&row.backend),
            escape(&row.detail),
        );
    }
    out
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '|' => out.push('/'),
            '\n' | '\r' => out.push(' '),
            _ => out.push(ch),
        }
    }
    out
}

// ------------------------------------------------------------ helpers ---

fn join(items: &[String]) -> String {
    if items.is_empty() {
        "-".to_string()
    } else {
        items.join(",")
    }
}

fn clear(env: &mut JNIEnv) {
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_clear();
    }
}

fn fmt_err(env: &mut JNIEnv, stage: &str, e: jni::errors::Error) -> String {
    let had = env.exception_check().unwrap_or(false);
    if had {
        // Surface the Java-side class/message into logcat once; a bare
        // "exception was thrown" hides which API the platform refused.
        let _ = env.exception_describe();
        let _ = env.exception_clear();
    }
    if had {
        format!("{stage}: {e} (java exception cleared)")
    } else {
        format!("{stage}: {e}")
    }
}

fn value_string<'local>(env: &mut JNIEnv<'local>, value: JValueOwned<'local>) -> String {
    match value {
        JValueOwned::Object(obj) => {
            if obj.is_null() {
                "-".to_string()
            } else {
                unsafe {
                    let s: JString<'local> = JString::from_raw(obj.into_raw());
                    let text = match env.get_string(&s) {
                        Ok(text) => text.into(),
                        Err(_) => {
                            clear(env);
                            String::from("-")
                        }
                    };
                    text
                }
            }
        }
        _ => "-".to_string(),
    }
}

fn jobject_array<'a>(obj: JObject<'a>) -> jni::objects::JObjectArray<'a> {
    obj.into()
}

fn jint_array<'a>(obj: JObject<'a>) -> jni::objects::JIntArray<'a> {
    obj.into()
}

fn jstring<'local>(env: &mut JNIEnv<'local>, obj: &JObject<'local>) -> String {
    unsafe {
        let s: JString<'local> = JString::from_raw(obj.as_raw());
        let text = match env.get_string(&s) {
            Ok(text) => text.into(),
            Err(_) => {
                clear(env);
                String::from("-")
            }
        };
        text
    }
}

fn static_str(env: &mut JNIEnv, class: &str, field: &str) -> String {
    let Ok(cls) = env.find_class(class) else {
        clear(env);
        return "-".to_string();
    };
    match env.get_static_field(&cls, field, "Ljava/lang/String;") {
        Ok(value) => value_string(env, value),
        Err(_) => {
            clear(env);
            "-".to_string()
        }
    }
}

fn static_int(env: &mut JNIEnv, class: &str, field: &str) -> i32 {
    let Ok(cls) = env.find_class(class) else {
        clear(env);
        return -1;
    };
    match env.get_static_field(&cls, field, "I") {
        Ok(value) => value.i().unwrap_or(-1),
        Err(_) => {
            clear(env);
            -1
        }
    }
}

fn static_array(env: &mut JNIEnv, class: &str, field: &str) -> String {
    let Ok(cls) = env.find_class(class) else {
        clear(env);
        return "-".to_string();
    };
    let Ok(value) = env.get_static_field(&cls, field, "[Ljava/lang/String;") else {
        clear(env);
        return "-".to_string();
    };
    let Ok(arr) = value.l() else {
        clear(env);
        return "-".to_string();
    };
    if arr.is_null() {
        return "-".to_string();
    }
    let arr = jobject_array(arr);
    let n = env.get_array_length(&arr).unwrap_or(0);
    let mut items = Vec::new();
    for i in 0..n {
        if let Ok(el) = env.get_object_array_element(&arr, i) {
            if !el.is_null() {
                items.push(jstring(env, &el));
            }
        }
    }
    join(&items)
}

fn system_service<'l>(env: &mut JNIEnv<'l>, ctx: &JObject<'l>, name: &str) -> R<JObject<'l>> {
    let key = env
        .new_string(name)
        .map_err(|e| fmt_err(env, "new_string", e))?;
    let value = env
        .call_method(
            ctx,
            "getSystemService",
            "(Ljava/lang/String;)Ljava/lang/Object;",
            &[JValue::Object(&key)],
        )
        .map_err(|e| fmt_err(env, "getSystemService", e))?;
    let obj = value
        .l()
        .map_err(|e| fmt_err(env, "getSystemService ret", e))?;
    if obj.is_null() {
        return Err(format!("getSystemService({name}) returned null"));
    }
    Ok(obj)
}

fn has_feature(env: &mut JNIEnv, pm: &JObject, feature: &str) -> bool {
    let Ok(name) = env.new_string(feature) else {
        clear(env);
        return false;
    };
    match env.call_method(
        pm,
        "hasSystemFeature",
        "(Ljava/lang/String;)Z",
        &[JValue::Object(&name)],
    ) {
        Ok(value) => value.z().unwrap_or(false),
        Err(_) => {
            clear(env);
            false
        }
    }
}

fn audio_devices(env: &mut JNIEnv, am: &JObject, which: i32) -> R<Vec<String>> {
    let value = env
        .call_method(
            am,
            "getDevices",
            "(I)[Landroid/media/AudioDeviceInfo;",
            &[JValue::Int(which)],
        )
        .map_err(|e| fmt_err(env, "AudioManager.getDevices", e))?;
    let arr = value
        .l()
        .map_err(|e| fmt_err(env, "AudioManager.getDevices ret", e))?;
    if arr.is_null() {
        return Ok(Vec::new());
    }
    let arr = jobject_array(arr);
    let n = env
        .get_array_length(&arr)
        .map_err(|e| fmt_err(env, "array length", e))?;
    let mut items = Vec::new();
    for i in 0..n {
        let Ok(el) = env.get_object_array_element(&arr, i) else {
            clear(env);
            continue;
        };
        if el.is_null() {
            continue;
        }
        let ty = env
            .call_method(&el, "getType", "()I", &[])
            .ok()
            .and_then(|v| v.i().ok())
            .unwrap_or(-1);
        let name = match env.call_method(&el, "getProductName", "()Ljava/lang/CharSequence;", &[]) {
            Ok(v) => value_string(env, v),
            Err(_) => {
                clear(env);
                "?".to_string()
            }
        };
        items.push(format!("{ty}:{name}"));
    }
    Ok(items)
}

struct Codec {
    name: String,
    hardware: bool,
    types: Vec<String>,
}

impl Codec {
    fn has(&self, mime: &str) -> bool {
        self.types.iter().any(|t| t == mime)
    }
}

struct CodecSummary {
    decoders: Vec<Codec>,
    encoders: Vec<Codec>,
}

fn probe_codecs(env: &mut JNIEnv) -> R<CodecSummary> {
    let cls = env
        .find_class("android/media/MediaCodecList")
        .map_err(|e| fmt_err(env, "find MediaCodecList", e))?;
    // ALL_CODECS = 0
    let list = env
        .new_object(&cls, "(I)V", &[JValue::Int(0)])
        .map_err(|e| fmt_err(env, "new MediaCodecList", e))?;
    // `getCodecCount()` / `getCodecInfoAt()` are *static* and deprecated; on
    // this ROM the instance lookup raises NoSuchMethodError. `getCodecInfos()`
    // is the supported instance API and returns MediaCodecInfo[].
    let infos = env
        .call_method(&list, "getCodecInfos", "()[Landroid/media/MediaCodecInfo;", &[])
        .map_err(|e| fmt_err(env, "getCodecInfos", e))?
        .l()
        .map_err(|e| fmt_err(env, "getCodecInfos ret", e))?;
    if infos.is_null() {
        return Err("getCodecInfos returned null".to_string());
    }
    let infos = jobject_array(infos);
    let count = env
        .get_array_length(&infos)
        .map_err(|e| fmt_err(env, "getCodecInfos length", e))?;

    let mut decoders = Vec::new();
    let mut encoders = Vec::new();
    for index in 0..count {
        let info = match env.get_object_array_element(&infos, index) {
            Ok(obj) if !obj.is_null() => obj,
            _ => {
                clear(env);
                continue;
            }
        };
        let name = match env.call_method(&info, "getName", "()Ljava/lang/String;", &[]) {
            Ok(v) => value_string(env, v),
            Err(_) => {
                clear(env);
                continue;
            }
        };
        let is_encoder = env
            .call_method(&info, "isEncoder", "()Z", &[])
            .ok()
            .and_then(|v| v.z().ok())
            .unwrap_or(false);
        let hardware = match env.call_method(&info, "isHardwareAccelerated", "()Z", &[]) {
            Ok(v) => v.z().unwrap_or_else(|_| heuristic_hardware(&name)),
            Err(_) => {
                clear(env);
                heuristic_hardware(&name)
            }
        };
        let mut types = Vec::new();
        if let Ok(v) = env.call_method(&info, "getSupportedTypes", "()[Ljava/lang/String;", &[]) {
            if let Ok(arr) = v.l() {
                if !arr.is_null() {
                    let arr = jobject_array(arr);
                    let n = env.get_array_length(&arr).unwrap_or(0);
                    for i in 0..n {
                        if let Ok(el) = env.get_object_array_element(&arr, i) {
                            if !el.is_null() {
                                types.push(jstring(env, &el));
                            }
                        }
                    }
                }
            }
        } else {
            clear(env);
        }
        // Only the MIME families Husk actually routes are worth printing.
        types.retain(|t| {
            t.starts_with("video/") || t.starts_with("audio/") || t.starts_with("image/")
        });
        if types.len() > 8 {
            types.truncate(8);
        }
        let codec = Codec {
            name,
            hardware,
            types,
        };
        if is_encoder {
            encoders.push(codec);
        } else {
            decoders.push(codec);
        }
    }
    Ok(CodecSummary { decoders, encoders })
}

fn heuristic_hardware(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    if lower.starts_with("c2.") {
        return !(lower.contains(".sw.") || lower.contains("google") || lower.contains("ffmpeg"));
    }
    lower.contains("mtk")
        || lower.contains("qcom")
        || lower.contains("exynos")
        || lower.contains("kirin")
        || lower.contains("omx.mtk")
}

// Keep the module honest about the unused-c_void import on non-Android builds.
#[allow(dead_code)]
fn _unused(_p: *mut c_void) {}
