//! Runtime permission bootstrap without Java bytecode.
//!
//! `NativeActivity` is a platform Java class, so the process already owns a
//! real Activity object. We use that object through JNI to call the standard
//! `checkSelfPermission` and `requestPermissions` methods. There is no custom
//! Java class and the APK remains `android:hasCode="false"`.
//!
//! Husk asks once per installation for the hardware permissions that need a
//! user decision. A successful request is recorded in the app-private files
//! directory so denying the dialog does not turn every launch into another
//! prompt. Revoking a permission later is respected by the next query.

use jni::objects::{JObject, JValue};
use jni::sys::JNIEnv as RawJNIEnv;
use jni::JNIEnv;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::sys::ANativeActivity;

const CAMERA: &str = "android.permission.CAMERA";
const RECORD_AUDIO: &str = "android.permission.RECORD_AUDIO";
const PERMISSION_GRANTED: i32 = 0;
const REQUEST_MARKER: &str = ".husk-runtime-permissions-v1";

static ATTEMPTED: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Granted,
    Denied,
    Unknown,
}

impl State {
    fn as_str(self) -> &'static str {
        match self {
            State::Granted => "granted",
            State::Denied => "denied",
            State::Unknown => "unknown",
        }
    }
}

/// The Activity object, taken from the one field the NDK already gives us.
///
/// `ANativeActivity::clazz` is the instance's global reference (the NDK named
/// it badly); it stays valid for the lifetime of the activity. The alternative
/// used elsewhere in this app is to reflect `ActivityThread`'s private
/// `mActivities` map, which depends on a hidden API whose allow/deny list
/// changes between Android releases — a path that fails silently here would
/// mean "camera never asks for permission and never says why".
unsafe fn activity_env(
    a: *mut ANativeActivity,
) -> Result<(JNIEnv<'static>, JObject<'static>), String> {
    if a.is_null() {
        return Err("null ANativeActivity".into());
    }
    let env_ptr = (*a).env as *mut RawJNIEnv;
    if env_ptr.is_null() {
        return Err("null JNIEnv".into());
    }
    let clazz = (*a).clazz as jni::sys::jobject;
    if clazz.is_null() {
        return Err("null ANativeActivity.clazz".into());
    }
    let env = JNIEnv::from_raw(env_ptr).map_err(|e| format!("JNIEnv: {e}"))?;
    Ok((env, JObject::from_raw(clazz)))
}

fn marker_path(a: *mut ANativeActivity) -> Option<PathBuf> {
    if a.is_null() {
        return None;
    }
    let path = unsafe { (*a).internalDataPath };
    if path.is_null() {
        return None;
    }
    let files = unsafe { core::ffi::CStr::from_ptr(path) }
        .to_string_lossy()
        .into_owned();
    Some(PathBuf::from(files).join(REQUEST_MARKER))
}

fn was_requested(a: *mut ANativeActivity, permission: &str) -> bool {
    let Some(path) = marker_path(a) else {
        return false;
    };
    std::fs::read_to_string(path)
        .map(|text| text.lines().any(|line| line == permission))
        .unwrap_or(false)
}

fn record_requested(a: *mut ANativeActivity, requested: &[&str]) {
    let Some(path) = marker_path(a) else {
        return;
    };
    let mut text = std::fs::read_to_string(&path).unwrap_or_default();
    for permission in requested {
        if !text.lines().any(|line| line == *permission) {
            if !text.is_empty() && !text.ends_with('\n') {
                text.push('\n');
            }
            text.push_str(permission);
            text.push('\n');
        }
    }
    if let Err(e) = std::fs::write(&path, text) {
        crate::log(&format!("permissions|WARN|marker write failed: {e}"));
    }
}

unsafe fn query(env: &mut JNIEnv, activity: &JObject, permission: &str) -> State {
    let name = match env.new_string(permission) {
        Ok(name) => name,
        Err(e) => {
            clear_exception(env, "new_string", e);
            return State::Unknown;
        }
    };
    match env
        .call_method(
            activity,
            "checkSelfPermission",
            "(Ljava/lang/String;)I",
            &[JValue::Object(&name)],
        )
        .and_then(|value| value.i())
    {
        Ok(PERMISSION_GRANTED) => State::Granted,
        Ok(_) => State::Denied,
        Err(e) => {
            clear_exception(env, "Activity.checkSelfPermission", e);
            State::Unknown
        }
    }
}

fn clear_exception(env: &JNIEnv, stage: &str, err: impl core::fmt::Display) {
    crate::log(&format!("permissions|FAIL|{stage}: {err}"));
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_describe();
        let _ = env.exception_clear();
    }
}

unsafe fn request(env: &mut JNIEnv, activity: &JObject, requested: &[&str]) -> Result<(), String> {
    let array = env
        .new_object_array(requested.len() as i32, "java/lang/String;", JObject::null())
        .map_err(|e| format!("new String[]: {e}"))?;
    for (index, permission) in requested.iter().enumerate() {
        let value = env
            .new_string(permission)
            .map_err(|e| format!("new_string({permission}): {e}"))?;
        env.set_object_array_element(&array, index as i32, &value)
            .map_err(|e| format!("set String[{index}]: {e}"))?;
    }
    env.call_method(
        activity,
        "requestPermissions",
        "([Ljava/lang/String;I)V",
        &[JValue::Object(&array), JValue::Int(0)],
    )
    .map_err(|e| format!("Activity.requestPermissions: {e}"))?;
    Ok(())
}

/// Called from `onResume`, after the Activity is visible. It logs the current
/// state on every resume; the actual OS dialog is requested only once per
/// process and only for permissions not already recorded in this installation.
pub unsafe fn on_resume(a: *mut ANativeActivity) {
    if a.is_null() {
        return;
    }
    let (mut env, activity) = match activity_env(a) {
        Ok(pair) => pair,
        Err(e) => {
            crate::log(&format!("permissions|SKIP|{e}"));
            return;
        }
    };

    let camera = query(&mut env, &activity, CAMERA);
    let microphone = query(&mut env, &activity, RECORD_AUDIO);
    crate::log(&format!(
        "permissions|STATE|camera={} microphone={}",
        camera.as_str(),
        microphone.as_str()
    ));

    if ATTEMPTED.load(Ordering::SeqCst) {
        return;
    }

    let mut requested = Vec::new();
    for (permission, state) in [(CAMERA, camera), (RECORD_AUDIO, microphone)] {
        if state != State::Granted && !was_requested(a, permission) {
            requested.push(permission);
        }
    }
    if requested.is_empty() {
        ATTEMPTED.store(true, Ordering::SeqCst);
        if camera != State::Granted || microphone != State::Granted {
            crate::log("permissions|WAIT|previously requested; enable in Android settings");
        }
        return;
    }

    match request(&mut env, &activity, &requested) {
        Ok(()) => {
            ATTEMPTED.store(true, Ordering::SeqCst);
            record_requested(a, &requested);
            crate::log(&format!("permissions|REQUEST|{}", requested.join(",")));
        }
        Err(e) => {
            // Do not record a failed JNI call as if the user had seen a dialog.
            clear_exception(&env, "request", e);
        }
    }
}
