//! Platform SurfaceView bridge, implemented without shipping Java bytecode.
//!
//! NativeActivity does not expose its Java Activity object, and its window's
//! Surface has no SurfaceControl handle under BLAST.  A SurfaceView does create
//! a real BLAST SurfaceControl, so we ask the platform (through JNI) to attach
//! one to the current Activity and later read the native pointer from
//! `SurfaceView.mSurfaceControl.mNativeObject`.
//!
//! This is bridge code only: no application Java class, no dex, and no Java
//! code in the APK.  The platform objects belong to Android itself.  The
//! resulting pointer is the public ABI consumed by `ASurfaceControl_create`.

use core::ffi::c_void;
use jni::objects::{GlobalRef, JIntArray, JObject, JObjectArray, JString, JValue};
use jni::{JNIEnv, JavaVM};
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::sys::ANativeActivity;

static VM: AtomicPtr<c_void> = AtomicPtr::new(core::ptr::null_mut());
static SURFACE_VIEW: Mutex<Option<GlobalRef>> = Mutex::new(None);
static NATIVE_CONTROL: AtomicUsize = AtomicUsize::new(0);
static LAST_CONTROL_REFRESH_MS: AtomicU64 = AtomicU64::new(0);
static READ_ERROR_LOGGED: AtomicBool = AtomicBool::new(false);

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or(0)
}

fn clear_java_exception(env: &JNIEnv, stage: &str, err: impl core::fmt::Display) {
    crate::log(&format!("surface: {stage}|FAIL|{err}"));
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_describe();
        let _ = env.exception_clear();
    }
}

fn current_activity<'local>(env: &mut JNIEnv<'local>) -> Result<JObject<'local>, String> {
    let thread_class = env
        .find_class("android/app/ActivityThread")
        .map_err(|e| format!("find ActivityThread: {e}"))?;
    let thread = env
        .call_static_method(
            thread_class,
            "currentActivityThread",
            "()Landroid/app/ActivityThread;",
            &[],
        )
        .and_then(|v| v.l())
        .map_err(|e| format!("currentActivityThread: {e}"))?;
    if thread.is_null() {
        return Err("currentActivityThread returned null".into());
    }

    let activities = env
        .get_field(&thread, "mActivities", "Landroid/util/ArrayMap;")
        .and_then(|v| v.l())
        .map_err(|e| format!("ActivityThread.mActivities: {e}"))?;
    if activities.is_null() {
        return Err("mActivities is null".into());
    }

    let size = env
        .call_method(&activities, "size", "()I", &[])
        .and_then(|v| v.i())
        .map_err(|e| format!("ArrayMap.size: {e}"))?;
    for i in (0..size).rev() {
        let record = env
            .call_method(
                &activities,
                "valueAt",
                "(I)Ljava/lang/Object;",
                &[JValue::Int(i)],
            )
            .and_then(|v| v.l())
            .map_err(|e| format!("ArrayMap.valueAt({i}): {e}"))?;
        if record.is_null() {
            continue;
        }
        let activity = env
            .get_field(&record, "activity", "Landroid/app/Activity;")
            .and_then(|v| v.l())
            .map_err(|e| format!("ActivityClientRecord.activity: {e}"))?;
        if !activity.is_null() {
            return Ok(activity);
        }
    }
    Err("no live ActivityClientRecord".into())
}

/// Install a transparent SurfaceView over the NativeActivity and retain it in
/// a JVM global reference.  Must run on the Android UI thread, from
/// `ANativeActivity_onCreate` before NativeActivity returns.
pub unsafe fn install(activity: *mut ANativeActivity) -> bool {
    if activity.is_null() || (*activity).vm.is_null() || (*activity).env.is_null() {
        crate::log("surface: install|SKIP|null activity/vm/env");
        return false;
    }
    let mut env = match JNIEnv::from_raw((*activity).env as *mut jni::sys::JNIEnv) {
        Ok(env) => env,
        Err(e) => {
            crate::log(&format!("surface: install|FAIL|JNIEnv: {e}"));
            return false;
        }
    };

    let activity_obj = match current_activity(&mut env) {
        Ok(obj) => obj,
        Err(e) => {
            clear_java_exception(&env, "activity", e);
            return false;
        }
    };
    crate::log("surface: activity|PASS|ActivityThread.mActivities");

    let view = match env
        .find_class("android/view/SurfaceView")
        .and_then(|class| {
            env.new_object(
                class,
                "(Landroid/content/Context;)V",
                &[JValue::Object(&activity_obj)],
            )
        }) {
        Ok(view) => view,
        Err(e) => {
            clear_java_exception(&env, "new SurfaceView", e);
            return false;
        }
    };

    // Keep the SurfaceView layer from painting an opaque black rectangle over
    // the terminal.  The child ASurfaceControl is what carries guest frames.
    let _ = env.call_method(&view, "setBackgroundColor", "(I)V", &[JValue::Int(0)]);
    let _ = env.call_method(&view, "setZOrderOnTop", "(Z)V", &[JValue::Bool(1)]);
    if let Err(e) = env.call_method(
        &activity_obj,
        "setContentView",
        "(Landroid/view/View;)V",
        &[JValue::Object(&view)],
    ) {
        clear_java_exception(&env, "setContentView", e);
        return false;
    }

    let global = match env.new_global_ref(&view) {
        Ok(global) => global,
        Err(e) => {
            clear_java_exception(&env, "new_global_ref", e);
            return false;
        }
    };
    let thread = match env.get_java_vm() {
        Ok(vm) => vm.get_java_vm_pointer(),
        Err(e) => {
            crate::log(&format!("surface: get_java_vm|FAIL|{e}"));
            return false;
        }
    };

    VM.store(thread as *mut c_void, Ordering::SeqCst);
    NATIVE_CONTROL.store(0, Ordering::SeqCst);
    LAST_CONTROL_REFRESH_MS.store(0, Ordering::SeqCst);
    READ_ERROR_LOGGED.store(false, Ordering::SeqCst);
    match SURFACE_VIEW.lock() {
        Ok(mut slot) => *slot = Some(global),
        Err(_) => {
            crate::log("surface: store|FAIL|mutex poisoned");
            return false;
        }
    }
    crate::log(&format!(
        "surface: install|PASS|SurfaceView on Activity {:#x}",
        activity_obj.as_raw() as usize
    ));
    true
}

fn read_control_once() -> Option<usize> {
    let vm_ptr = VM.load(Ordering::SeqCst) as *mut jni::sys::JavaVM;
    if vm_ptr.is_null() {
        return None;
    }
    let vm = unsafe { JavaVM::from_raw(vm_ptr) }.ok()?;
    let mut env = vm.attach_current_thread_as_daemon().ok()?;
    let guard = SURFACE_VIEW.lock().ok()?;
    let view = guard.as_ref()?;

    // SurfaceView creates a parent layer (`mSurfaceControl`) and a BLAST
    // child (`mBlastSurfaceControl`) that carries its own buffer.  A child of
    // the parent is rendered below that BLAST child and is therefore invisible
    // on this Android 13 stack.  Use the BLAST layer as our parent so the
    // guest buffer is above the SurfaceView's own (empty) buffer queue.
    let mut control = env
        .get_field(
            view.as_obj(),
            "mBlastSurfaceControl",
            "Landroid/view/SurfaceControl;",
        )
        .and_then(|v| v.l())
        .unwrap_or_else(|e| {
            if !READ_ERROR_LOGGED.swap(true, Ordering::SeqCst) {
                clear_java_exception(&env, "SurfaceView.mSurfaceControl", e);
            }
            JObject::null()
        });
    if control.is_null() {
        control = env
            .get_field(
                view.as_obj(),
                "mSurfaceControl",
                "Landroid/view/SurfaceControl;",
            )
            .and_then(|v| v.l())
            .unwrap_or_else(|_| JObject::null());
    }
    if control.is_null() {
        return None;
    }
    let native = env
        .get_field(&control, "mNativeObject", "J")
        .and_then(|v| v.j())
        .map_err(|e| {
            if !READ_ERROR_LOGGED.swap(true, Ordering::SeqCst) {
                clear_java_exception(&env, "SurfaceControl.mNativeObject", e);
            }
        })
        .ok()?;
    if native == 0 {
        return None;
    }
    Some(native as usize)
}

/// Wait until SurfaceView has created its BLAST SurfaceControl.
pub fn wait_parent(timeout: Duration) -> Option<usize> {
    let cached = NATIVE_CONTROL.load(Ordering::SeqCst);
    if cached != 0 {
        let now = now_ms();
        let last = LAST_CONTROL_REFRESH_MS.load(Ordering::Relaxed);
        if now.saturating_sub(last) >= 1_000 {
            LAST_CONTROL_REFRESH_MS.store(now, Ordering::Relaxed);
            if let Some(ptr) = read_control_once() {
                if ptr != cached {
                    NATIVE_CONTROL.store(ptr, Ordering::SeqCst);
                    crate::log(&format!(
                        "surface: parent|CHANGED|old={cached:#x}|new={ptr:#x}"
                    ));
                    return Some(ptr);
                }
            }
        }
        return Some(cached);
    }
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(ptr) = read_control_once() {
            NATIVE_CONTROL.store(ptr, Ordering::SeqCst);
            LAST_CONTROL_REFRESH_MS.store(now_ms(), Ordering::Relaxed);
            crate::log(&format!("surface: parent|PASS|native={ptr:#x}"));
            return Some(ptr);
        }
        if Instant::now() >= deadline {
            crate::log("surface: parent|FAIL|SurfaceView did not publish SurfaceControl");
            return None;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

pub fn drop_view() {
    NATIVE_CONTROL.store(0, Ordering::SeqCst);
    LAST_CONTROL_REFRESH_MS.store(0, Ordering::SeqCst);
    if let Ok(mut slot) = SURFACE_VIEW.lock() {
        *slot = None;
    }
}

// --------------------------------------------------------------- audio JNI ---

#[derive(Clone, Debug)]
pub struct AudioDeviceInfo {
    pub id: i32,
    pub ty: i32,
    pub is_sink: bool,
    pub name: String,
    pub sample_rates: String,
    pub channel_counts: String,
}

fn jni_fail(env: &JNIEnv, stage: &str, err: impl core::fmt::Display) -> String {
    clear_java_exception(env, stage, &err);
    format!("{stage}: {err}")
}

fn audio_manager<'local>(
    env: &mut JNIEnv<'local>,
    activity: &JObject<'local>,
) -> Result<JObject<'local>, String> {
    let service = env
        .new_string("audio")
        .map_err(|e| jni_fail(env, "audio service string", e))?;
    env.call_method(
        activity,
        "getSystemService",
        "(Ljava/lang/String;)Ljava/lang/Object;",
        &[JValue::Object(&service)],
    )
    .and_then(|v| v.l())
    .map_err(|e| jni_fail(env, "getSystemService(audio)", e))
}

fn int_array_values(env: &mut JNIEnv, object: JObject, stage: &str) -> Result<String, String> {
    let array = unsafe { JIntArray::from_raw(object.into_raw()) };
    let len = env
        .get_array_length(&array)
        .map_err(|e| jni_fail(env, stage, e))? as usize;
    if len == 0 {
        return Ok("-".into());
    }
    let mut values = vec![0i32; len.min(16)];
    env.get_int_array_region(&array, 0, &mut values)
        .map_err(|e| jni_fail(env, stage, e))?;
    Ok(values
        .iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join(","))
}

pub fn audio_device_type_name(ty: i32) -> &'static str {
    match ty {
        1 => "earpiece",
        2 => "speaker",
        3 => "wired_headset",
        4 => "wired_headphones",
        5 => "line_analog",
        6 => "line_digital",
        7 => "bluetooth_sco",
        8 => "bluetooth_a2dp",
        9 => "hdmi",
        10 => "hdmi_arc",
        11 => "usb_device",
        12 => "usb_accessory",
        13 => "dock",
        14 => "fm",
        15 => "builtin_mic",
        16 => "fm_tuner",
        17 => "tv_tuner",
        18 => "builtin_speaker_safe",
        19 => "remote_submix",
        20 => "telephony",
        21 => "aux_line",
        22 => "ip",
        23 => "bus",
        24 => "usb_headset",
        25 => "hearing_aid",
        _ => "unknown",
    }
}

/// Enumerate Android output devices through the platform AudioManager. This
/// is the bionic side of the bridge; the returned records are sent to the
/// guest probe as text so no JSON or Java code is shipped in the APK.
pub fn audio_devices() -> Result<Vec<AudioDeviceInfo>, String> {
    let vm_ptr = VM.load(Ordering::SeqCst) as *mut jni::sys::JavaVM;
    if vm_ptr.is_null() {
        return Err("JavaVM not installed".into());
    }
    let vm = unsafe { JavaVM::from_raw(vm_ptr) }.map_err(|e| format!("JavaVM: {e}"))?;
    let mut env = vm
        .attach_current_thread_as_daemon()
        .map_err(|e| format!("attach audio thread: {e}"))?;
    let activity = current_activity(&mut env)?;
    let manager = audio_manager(&mut env, &activity)?;
    let devices_obj = env
        .call_method(
            &manager,
            "getDevices",
            "(I)[Landroid/media/AudioDeviceInfo;",
            &[JValue::Int(2)],
        )
        .and_then(|v| v.l())
        .map_err(|e| jni_fail(&env, "AudioManager.getDevices", e))?;
    if devices_obj.is_null() {
        return Err("AudioManager.getDevices returned null".into());
    }
    let devices = unsafe { JObjectArray::from_raw(devices_obj.into_raw()) };
    let len = env
        .get_array_length(&devices)
        .map_err(|e| jni_fail(&env, "devices.length", e))?;
    let mut out = Vec::new();
    for i in 0..len {
        let device = env
            .get_object_array_element(&devices, i)
            .map_err(|e| jni_fail(&env, "devices.element", e))?;
        if device.is_null() {
            continue;
        }
        let id = env
            .call_method(&device, "getId", "()I", &[])
            .and_then(|v| v.i())
            .map_err(|e| jni_fail(&env, "AudioDeviceInfo.getId", e))?;
        let ty = env
            .call_method(&device, "getType", "()I", &[])
            .and_then(|v| v.i())
            .map_err(|e| jni_fail(&env, "AudioDeviceInfo.getType", e))?;
        let is_sink = env
            .call_method(&device, "isSink", "()Z", &[])
            .and_then(|v| v.z())
            .map_err(|e| jni_fail(&env, "AudioDeviceInfo.isSink", e))?;
        let product = env
            .call_method(&device, "getProductName", "()Ljava/lang/CharSequence;", &[])
            .and_then(|v| v.l())
            .map_err(|e| jni_fail(&env, "AudioDeviceInfo.getProductName", e))?;
        let name = if product.is_null() {
            audio_device_type_name(ty).to_string()
        } else {
            let string = env
                .call_method(&product, "toString", "()Ljava/lang/String;", &[])
                .and_then(|v| v.l())
                .map_err(|e| jni_fail(&env, "productName.toString", e))?;
            if string.is_null() {
                audio_device_type_name(ty).to_string()
            } else {
                let string = unsafe { JString::from_raw(string.into_raw()) };
                env.get_string(&string)
                    .map(|v| v.to_string_lossy().into_owned())
                    .map_err(|e| jni_fail(&env, "productName string", e))?
            }
        };
        let rates_obj = env
            .call_method(&device, "getSampleRates", "()[I", &[])
            .and_then(|v| v.l())
            .map_err(|e| jni_fail(&env, "AudioDeviceInfo.getSampleRates", e))?;
        let sample_rates = if rates_obj.is_null() {
            "-".into()
        } else {
            int_array_values(&mut env, rates_obj, "sampleRates")?
        };
        let channels_obj = env
            .call_method(&device, "getChannelCounts", "()[I", &[])
            .and_then(|v| v.l())
            .map_err(|e| jni_fail(&env, "AudioDeviceInfo.getChannelCounts", e))?;
        let channel_counts = if channels_obj.is_null() {
            "-".into()
        } else {
            int_array_values(&mut env, channels_obj, "channelCounts")?
        };
        out.push(AudioDeviceInfo {
            id,
            ty,
            is_sink,
            name,
            sample_rates,
            channel_counts,
        });
    }
    Ok(out)
}

fn audio_volume_parts(env: &mut JNIEnv, manager: &JObject) -> Result<(i32, i32), String> {
    let index = env
        .call_method(manager, "getStreamVolume", "(I)I", &[JValue::Int(3)])
        .and_then(|v| v.i())
        .map_err(|e| jni_fail(env, "AudioManager.getStreamVolume", e))?;
    let max = env
        .call_method(manager, "getStreamMaxVolume", "(I)I", &[JValue::Int(3)])
        .and_then(|v| v.i())
        .map_err(|e| jni_fail(env, "AudioManager.getStreamMaxVolume", e))?;
    Ok((index, max))
}

pub fn audio_music_volume() -> Result<(i32, i32), String> {
    let vm_ptr = VM.load(Ordering::SeqCst) as *mut jni::sys::JavaVM;
    if vm_ptr.is_null() {
        return Err("JavaVM not installed".into());
    }
    let vm = unsafe { JavaVM::from_raw(vm_ptr) }.map_err(|e| format!("JavaVM: {e}"))?;
    let mut env = vm
        .attach_current_thread_as_daemon()
        .map_err(|e| format!("attach audio thread: {e}"))?;
    let activity = current_activity(&mut env)?;
    let manager = audio_manager(&mut env, &activity)?;
    audio_volume_parts(&mut env, &manager)
}

pub fn audio_set_music_volume_percent(percent: u8) -> Result<(i32, i32), String> {
    let (_, max) = audio_music_volume()?;
    let index = ((percent as f32 / 100.0) * max as f32).round() as i32;
    audio_set_music_volume_index(index.clamp(0, max))
}

pub fn audio_set_music_volume_index(index: i32) -> Result<(i32, i32), String> {
    let vm_ptr = VM.load(Ordering::SeqCst) as *mut jni::sys::JavaVM;
    if vm_ptr.is_null() {
        return Err("JavaVM not installed".into());
    }
    let vm = unsafe { JavaVM::from_raw(vm_ptr) }.map_err(|e| format!("JavaVM: {e}"))?;
    let mut env = vm
        .attach_current_thread_as_daemon()
        .map_err(|e| format!("attach audio thread: {e}"))?;
    let activity = current_activity(&mut env)?;
    let manager = audio_manager(&mut env, &activity)?;
    let (_, max) = audio_volume_parts(&mut env, &manager)?;
    let requested = index.clamp(0, max);
    env.call_method(
        &manager,
        "setStreamVolume",
        "(III)V",
        &[JValue::Int(3), JValue::Int(requested), JValue::Int(0)],
    )
    .map_err(|e| jni_fail(&env, "AudioManager.setStreamVolume", e))?;
    audio_volume_parts(&mut env, &manager)
}
