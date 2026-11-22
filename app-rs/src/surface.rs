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
use jni::objects::{GlobalRef, JObject, JValue};
use jni::{JNIEnv, JavaVM};
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::sys::ANativeActivity;

static VM: AtomicPtr<c_void> = AtomicPtr::new(core::ptr::null_mut());
static SURFACE_VIEW: Mutex<Option<GlobalRef>> = Mutex::new(None);
static NATIVE_CONTROL: AtomicUsize = AtomicUsize::new(0);
static READ_ERROR_LOGGED: AtomicBool = AtomicBool::new(false);

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
        return Some(cached);
    }
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(ptr) = read_control_once() {
            NATIVE_CONTROL.store(ptr, Ordering::SeqCst);
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
    if let Ok(mut slot) = SURFACE_VIEW.lock() {
        *slot = None;
    }
}
