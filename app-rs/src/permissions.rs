//! Runtime permission bootstrap without Java bytecode.
//!
//! `NativeActivity` is a platform Java class, so the process already owns a
//! real Activity object. We use that object through JNI to call the standard
//! `checkSelfPermission` / `requestPermissions` / `getPackageName` methods and
//! the platform `Toast` and `Intent` classes. There is no custom Java class and
//! the APK stays `android:hasCode="false"`.
//!
//! ## Why there is no `onRequestPermissionsResult` here
//!
//! The NDK hands native code a fixed `ANativeActivityCallbacks` table, and it has
//! no entry for permission results - that callback only exists for a Java
//! `Activity` subclass, which `hasCode="false"` rules out. The result therefore
//! has to be *observed*: `requestPermissions` is recorded as pending, and the
//! next resume reads the answer back with `checkSelfPermission`. That is a
//! strictly weaker signal (it cannot see "denied" versus "denied, don't ask
//! again"), so everything that depends on the difference - opening the app's
//! settings page - is driven by how many times a pending request came back
//! denied, not by guessing the dialog's button.
//!
//! ## What the user is told
//!
//! A denial is not silent: the app raises a `Toast` that names the missing
//! permission, and on the second denial it also opens this app's settings page,
//! which is the only place the permission can be given back once the system
//! stops showing the dialog. `logcat` is not the user-facing channel.

use jni::objects::{JObject, JValue};
use jni::sys::JNIEnv as RawJNIEnv;
use jni::JNIEnv;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::sys::ANativeActivity;

const CAMERA: &str = "android.permission.CAMERA";
const RECORD_AUDIO: &str = "android.permission.RECORD_AUDIO";
const ACCESS_COARSE_LOCATION: &str = "android.permission.ACCESS_COARSE_LOCATION";
const ALL_PERMISSIONS: [&str; 3] = [CAMERA, RECORD_AUDIO, ACCESS_COARSE_LOCATION];
const PERMISSION_GRANTED: i32 = 0;
const LENGTH_LONG: i32 = 1;

/// How long after `requestPermissions` a still-denied answer is believed to be
/// the user's answer rather than the state the dialog had not yet changed.
///
/// This exists because `onResume` fires again immediately when the system dialog
/// takes focus: measured 2026-11-26, the resume that "read back" the answer came
/// **48 ms** after the request, reported `denied` (the pre-dialog value), and
/// burned both the denial counter and the second attempt in the same pass - two
/// toasts and two settings launches in 300 ms, with nobody having touched the
/// dialog. 800 ms is well under any human tap and well over that bounce.
const SETTLE_MS: u64 = 800;

static LAST_REQUEST_MS: AtomicU64 = AtomicU64::new(0);

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Old marker from 0.0.35: one requested permission per line, nothing else.
/// Kept readable so an upgrade does not re-prompt for something the user has
/// already answered.
const LEGACY_MARKER: &str = ".husk-runtime-permissions-v1";
const STATE_FILE: &str = ".husk-permission-state-v1";

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
    fn parse(text: &str) -> State {
        match text {
            "granted" => State::Granted,
            "denied" => State::Denied,
            _ => State::Unknown,
        }
    }
}

/// Per-permission record. `pending` means "a dialog was shown and the answer has
/// not been read back yet"; that flag is what makes the denial counter count
/// dialogs instead of resumes.
#[derive(Clone, Copy, Default)]
struct Rec {
    /// How many dialogs have been shown. Two, then stop: the system stops
    /// showing the dialog after a couple of refusals anyway, and the settings
    /// page is the honest way back after that.
    asks: u32,
    denials: u32,
    pending: bool,
    last: Option<State>,
    /// Settings page already opened for this denial streak; do not yank the user
    /// out of the app again on every following resume.
    nudged: bool,
}

impl Rec {
    /// A granted state is authoritative. Clear the refusal history even when
    /// the grant came from the system settings page instead of the dialog:
    /// that path has `pending=false`, so only clearing counters inside the
    /// pending branch leaves `asks=2` behind and prevents every future ask.
    fn mark_granted(&mut self) -> bool {
        let changed = self.asks != 0
            || self.denials != 0
            || self.pending
            || self.nudged
            || self.last != Some(State::Granted);
        self.asks = 0;
        self.denials = 0;
        self.pending = false;
        self.nudged = false;
        self.last = Some(State::Granted);
        changed
    }
}

fn short(permission: &str) -> &'static str {
    if permission == CAMERA {
        "相机"
    } else if permission == RECORD_AUDIO {
        "麦克风"
    } else if permission == ACCESS_COARSE_LOCATION {
        "定位"
    } else {
        "硬件"
    }
}

/// The Activity object, taken from the one field the NDK already gives us.
///
/// `ANativeActivity::clazz` is the instance's global reference (the NDK named
/// it badly); it stays valid for the lifetime of the activity. The alternative
/// is to reflect `ActivityThread`'s private `mActivities` map, which depends on
/// a hidden API whose allow/deny list changes between Android releases - a path
/// that fails silently here would mean "camera never asks for permission and
/// never says why".
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

fn files_dir(a: *mut ANativeActivity) -> Option<PathBuf> {
    if a.is_null() {
        return None;
    }
    let path = unsafe { (*a).internalDataPath };
    if path.is_null() {
        return None;
    }
    Some(PathBuf::from(
        unsafe { core::ffi::CStr::from_ptr(path) }.to_string_lossy().into_owned(),
    ))
}

fn load(a: *mut ANativeActivity) -> BTreeMap<&'static str, Rec> {
    let mut map: BTreeMap<&'static str, Rec> = BTreeMap::new();
    for permission in ALL_PERMISSIONS {
        map.insert(permission, Rec::default());
    }
    let Some(dir) = files_dir(a) else {
        return map;
    };
    // Migration: the 0.0.35 marker recorded only "we asked about this". Treating
    // it as asked (denials unknown) keeps an upgrade from re-prompting.
    if let Ok(text) = std::fs::read_to_string(dir.join(LEGACY_MARKER)) {
        for line in text.lines() {
            if let Some(rec) = map.get_mut(line.trim()) {
                rec.asks = 2;
            }
        }
    }
    if let Ok(text) = std::fs::read_to_string(dir.join(STATE_FILE)) {
        for line in text.lines() {
            let mut fields = line.split('\t');
            let Some(name) = fields.next() else { continue };
            let Some(rec) = map.get_mut(name.trim()) else { continue };
            for field in fields {
                let (key, value) = field.split_once('=').unwrap_or((field, ""));
                match key {
                    "asks" => rec.asks = value.parse().unwrap_or(0),
                    "denials" => rec.denials = value.parse().unwrap_or(0),
                    "pending" => rec.pending = value == "1",
                    "last" => rec.last = Some(State::parse(value)),
                    "nudged" => rec.nudged = value == "1",
                    _ => {}
                }
            }
        }
    }
    map
}

fn save(a: *mut ANativeActivity, map: &BTreeMap<&'static str, Rec>) {
    let Some(dir) = files_dir(a) else {
        return;
    };
    let mut text = String::new();
    for (permission, rec) in map {
        text.push_str(&format!(
            "{permission}\tasks={}\tdenials={}\tpending={}\tlast={}\tnudged={}\n",
            rec.asks,
            rec.denials,
            rec.pending as u8,
            rec.last.unwrap_or(State::Unknown).as_str(),
            rec.nudged as u8
        ));
    }
    if let Err(e) = std::fs::write(dir.join(STATE_FILE), text) {
        crate::log(&format!("permissions|WARN|state write failed: {e}"));
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
    // `NewObjectArray` takes a *class name*, not a descriptor: passing
    // "java/lang/String;" makes FindClass abort the whole process with
    //   JNI DETECTED ERROR: illegal class name 'java/lang/String;'
    // measured 2026-11-26 on the device, before any dialog could appear.
    let array = env
        .new_object_array(requested.len() as i32, "java/lang/String", JObject::null())
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

/// A `Toast` on the activity's own thread. This is the user-facing channel for a
/// denial; the log line is only for whoever reads `logcat` afterwards.
unsafe fn toast(env: &mut JNIEnv, activity: &JObject, text: &str) {
    let body = match env.new_string(text) {
        Ok(body) => JObject::from(body),
        Err(e) => return clear_exception(env, "toast new_string", e),
    };
    let built = env
        .call_static_method(
            "android/widget/Toast",
            "makeText",
            "(Landroid/content/Context;Ljava/lang/CharSequence;I)Landroid/widget/Toast;",
            &[JValue::Object(activity), JValue::Object(&body), JValue::Int(LENGTH_LONG)],
        )
        .and_then(|value| value.l());
    match built {
        Ok(toast) => {
            if let Err(e) = env.call_method(&toast, "show", "()V", &[]) {
                clear_exception(env, "Toast.show", e);
            }
        }
        Err(e) => clear_exception(env, "Toast.makeText", e),
    }
    crate::log(&format!("permissions|TOAST|{text}"));
}

/// `android.settings.APPLICATION_DETAILS_SETTINGS` for this package. Needs no
/// custom class: `Intent` and `Uri` are platform classes, and an activity
/// context means no `FLAG_ACTIVITY_NEW_TASK` is required.
unsafe fn open_app_settings(env: &mut JNIEnv, activity: &JObject) -> Result<(), String> {
    let package = env
        .call_method(activity, "getPackageName", "()Ljava/lang/String;", &[])
        .and_then(|value| value.l())
        .map_err(|e| format!("getPackageName: {e}"))?;
    let package: String = {
        // `JavaStr` borrows the handle it came from, so both have to be bound
        // here; inlining `&JString::from(..)` drops the handle first.
        let handle = jni::objects::JString::from(package);
        let text = env
            .get_string(&handle)
            .map_err(|e| format!("getPackageName text: {e}"))?;
        String::from(text)
    };
    let uri_text = env
        .new_string(format!("package:{package}"))
        .map_err(|e| format!("package uri: {e}"))?;
    let uri_arg = JObject::from(uri_text);
    let uri = env
        .call_static_method(
            "android/net/Uri",
            "parse",
            "(Ljava/lang/String;)Landroid/net/Uri;",
            &[JValue::Object(&uri_arg)],
        )
        .and_then(|value| value.l())
        .map_err(|e| format!("Uri.parse: {e}"))?;
    let action = env
        .new_string("android.settings.APPLICATION_DETAILS_SETTINGS")
        .map_err(|e| format!("action string: {e}"))?;
    let action_arg = JObject::from(action);
    let intent = env
        .new_object(
            "android/content/Intent",
            "(Ljava/lang/String;)V",
            &[JValue::Object(&action_arg)],
        )
        .map_err(|e| format!("new Intent: {e}"))?;
    env.call_method(
        &intent,
        "setData",
        "(Landroid/net/Uri;)Landroid/content/Intent;",
        &[JValue::Object(&uri)],
    )
    .map_err(|e| format!("Intent.setData: {e}"))?;
    env.call_method(
        activity,
        "startActivity",
        "(Landroid/content/Intent;)V",
        &[JValue::Object(&intent)],
    )
    .map_err(|e| format!("startActivity: {e}"))?;
    crate::log(&format!("permissions|SETTINGS|opened app details for {package}"));
    Ok(())
}

/// Called from `onResume`, after the Activity is visible, so a dialog or a
/// `Toast` raised here is on screen.
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

    let states = [
        (CAMERA, query(&mut env, &activity, CAMERA)),
        (RECORD_AUDIO, query(&mut env, &activity, RECORD_AUDIO)),
        (
            ACCESS_COARSE_LOCATION,
            query(&mut env, &activity, ACCESS_COARSE_LOCATION),
        ),
    ];
    let mut map = load(a);
    // States *and* the recorded history in one line: "denied but never asked"
    // and "denied after asking" take different actions, and telling them apart
    // from logcat used to need a rebuild.
    crate::log(&format!(
        "permissions|STATE|camera={} microphone={} location={}|asks=camera:{},microphone:{},location:{}|denials=camera:{},microphone:{},location:{}",
        states[0].1.as_str(),
        states[1].1.as_str(),
        states[2].1.as_str(),
        map[CAMERA].asks,
        map[RECORD_AUDIO].asks,
        map[ACCESS_COARSE_LOCATION].asks,
        map[CAMERA].denials,
        map[RECORD_AUDIO].denials,
        map[ACCESS_COARSE_LOCATION].denials
    ));
    let mut changed = false;
    let mut missing: Vec<&'static str> = Vec::new();

    // A resume that lands within SETTLE_MS of the request is the dialog taking
    // focus, not an answer; resolving on it would consume an attempt before the
    // user has seen anything. LAST_REQUEST_MS is 0 in a fresh process, which
    // means any pending left by an earlier process is old and resolves normally.
    let since_request = now_ms().saturating_sub(LAST_REQUEST_MS.load(Ordering::SeqCst));
    let settled = LAST_REQUEST_MS.load(Ordering::SeqCst) == 0 || since_request >= SETTLE_MS;
    let mut settings_opened = false;

    for (permission, state) in states {
        let rec = map.get_mut(permission).expect("seeded above");
        if matches!(state, State::Granted) {
            let was_pending = rec.pending;
            changed |= rec.mark_granted();
            if was_pending {
                crate::log(&format!("permissions|RESULT|{permission}=granted"));
            }
        } else if rec.pending && settled {
            rec.pending = false;
            changed = true;
            match state {
                State::Granted => unreachable!("handled above"),
                State::Denied => {
                    rec.denials += 1;
                    rec.last = Some(State::Denied);
                    crate::log(&format!(
                        "permissions|RESULT|{permission}=denied (denial #{})",
                        rec.denials
                    ));
                    let name = short(permission);
                    if rec.denials == 1 {
                        let consequence = if permission == ACCESS_COARSE_LOCATION {
                            "gpsd 没有可用坐标".to_string()
                        } else {
                            format!("{name}在 Linux 里是黑的")
                        };
                        toast(
                            &mut env,
                            &activity,
                            &format!("Husk 需要{name}权限，否则{consequence}。下次进来还会再问一次。"),
                        );
                    } else if !rec.nudged || !settings_opened {
                        rec.nudged = true;
                        toast(
                            &mut env,
                            &activity,
                            &format!(
                                "Husk 已经问过两次{name}权限了。现在打开本应用的权限页面，把「{name}」改成允许。"
                            ),
                        );
                        // One page for both permissions: two `startActivity`
                        // calls in the same pass stacked the same screen twice.
                        if !settings_opened {
                            settings_opened = true;
                            if let Err(e) = open_app_settings(&mut env, &activity) {
                                clear_exception(&env, "open_app_settings", e);
                            }
                        }
                    }
                }
                State::Unknown => {
                    rec.last = Some(State::Unknown);
                    crate::log(&format!("permissions|RESULT|{permission}=unknown"));
                }
            }
        }
        if state != State::Granted {
            missing.push(permission);
        }
    }

    // Ask at most twice per installation, and never while an answer is still
    // outstanding. After the second refusal the toast and the settings page are
    // the way back; re-opening the dialog on every launch is how a user ends up
    // tapping "deny" out of irritation and never touching the hardware again.
    let to_ask: Vec<&'static str> = missing
        .iter()
        .copied()
        .filter(|permission| map[permission].asks < 2)
        .collect();
    // Never stack a second dialog on top of an unanswered one. Without this the
    // resume that arrives 100 ms after `requestPermissions` (the dialog taking
    // focus) consumed both attempts in one pass - measured 2026-11-26.
    let any_pending = map.values().any(|rec| rec.pending);
    if !to_ask.is_empty() && !any_pending {
        match request(&mut env, &activity, &to_ask) {
            Ok(()) => {
                LAST_REQUEST_MS.store(now_ms(), Ordering::SeqCst);
                for permission in &to_ask {
                    let rec = map.get_mut(permission).expect("seeded above");
                    rec.asks += 1;
                    rec.pending = true;
                }
                changed = true;
                crate::log(&format!("permissions|REQUEST|{}", to_ask.join(",")));
            }
            // Do not record a failed JNI call as if the user had seen a dialog.
            Err(e) => clear_exception(&env, "request", e),
        }
    } else if missing.is_empty() {
        crate::log("permissions|OK|all hardware permissions granted");
    }

    if changed {
        save(a, &map);
    }
}
