//! Last-known Android location -> standard gpsd JSON over a private Unix socket.
//!
//! Android's location APIs are Java services.  The app deliberately has no
//! custom Java classes (`hasCode=false`), so this bridge only exercises the
//! subset that needs no Java callback: `LocationManager.getLastKnownLocation()`.
//! It does not claim continuous updates, geofencing, GNSS measurements or
//! background location.
//!
//! The guest-facing interface is a gpsd JSON socket, not a private file format:
//! `GPSD_SOCKET=/run/husk/gps.sock` (`VERSION`, `DEVICES`, `WATCH`, `POLL` and
//! `TPV`).  A Unix socket is deliberate: an unauthenticated TCP listener would
//! let another app on the phone query the location behind the user's grant.
//! Diagnostics are also written under `/run/husk/location/` for tests.

use jni::objects::{JObject, JString, JValue};
use jni::sys::JavaVM as RawJavaVM;
use jni::{JNIEnv, JavaVM};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const ACCESS_COARSE_LOCATION: &str = "android.permission.ACCESS_COARSE_LOCATION";
const GPSD_SOCKET_REL: &str = "run/husk/gps.sock";
const GPSD_SOCKET_GUEST: &str = "/run/husk/gps.sock";
const POLL_MS: u64 = 1_000;
const CLIENT_TICK: Duration = Duration::from_millis(250);

static STARTED: AtomicBool = AtomicBool::new(false);
static FIX: Mutex<Option<LocationFix>> = Mutex::new(None);

#[derive(Clone, Debug)]
struct LocationFix {
    provider: String,
    latitude: f64,
    longitude: f64,
    accuracy_m: f64,
    altitude_m: Option<f64>,
    speed_mps: Option<f64>,
    bearing_deg: Option<f64>,
    timestamp_ms: i64,
    mock: bool,
}

/// Start the poller and gpsd subset once per process.  Failures are isolated:
/// desktop and shell operation never depend on location permission.
pub fn start_once() {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| {
        let _ = std::panic::catch_unwind(poll_loop);
    });
    std::thread::spawn(|| {
        let _ = std::panic::catch_unwind(server_loop);
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

unsafe fn activity_object<'local>(env: &mut JNIEnv<'local>) -> Result<JObject<'local>, String> {
    let act = crate::ACTIVITY.load(Ordering::SeqCst);
    if act.is_null() {
        return Err("null ANativeActivity".into());
    }
    let raw = (*act).clazz as jni::sys::jobject;
    if raw.is_null() {
        return Err("null ANativeActivity.clazz".into());
    }
    // `clazz` is the Activity instance's global reference.  Re-wrapping it is
    // borrow-only; we never DeleteGlobalRef it here.
    let _ = env;
    Ok(JObject::from_raw(raw))
}

fn clear(env: &mut JNIEnv) {
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_clear();
    }
}

fn permission_granted(env: &mut JNIEnv, activity: &JObject) -> Result<bool, String> {
    let name = env
        .new_string(ACCESS_COARSE_LOCATION)
        .map_err(|e| format!("new_string permission: {e}"))?;
    let rc = env
        .call_method(
            activity,
            "checkSelfPermission",
            "(Ljava/lang/String;)I",
            &[JValue::Object(&name)],
        )
        .and_then(|v| v.i());
    match rc {
        Ok(0) => Ok(true),
        Ok(_) => Ok(false),
        Err(e) => {
            clear(env);
            Err(format!("checkSelfPermission: {e}"))
        }
    }
}

fn location_manager<'local>(
    env: &mut JNIEnv<'local>,
    activity: &JObject<'local>,
) -> Result<JObject<'local>, String> {
    let service = env
        .new_string("location")
        .map_err(|e| format!("new_string location: {e}"))?;
    let obj = env
        .call_method(
            activity,
            "getSystemService",
            "(Ljava/lang/String;)Ljava/lang/Object;",
            &[JValue::Object(&service)],
        )
        .and_then(|v| v.l())
        .map_err(|e| {
            clear(env);
            format!("getSystemService(location): {e}")
        })?;
    if obj.is_null() {
        Err("LocationManager is null".into())
    } else {
        Ok(obj)
    }
}

fn string_value(env: &mut JNIEnv, obj: &JObject) -> String {
    if obj.is_null() {
        return String::new();
    }
    let s = unsafe { JString::from_raw(obj.as_raw()) };
    env.get_string(&s)
        .map(|v| String::from(v))
        .unwrap_or_default()
}

fn read_fix(env: &mut JNIEnv, loc: &JObject) -> Result<LocationFix, String> {
    let provider = env
        .call_method(loc, "getProvider", "()Ljava/lang/String;", &[])
        .and_then(|v| v.l())
        .map_err(|e| format!("Location.getProvider: {e}"))
        .map(|obj| string_value(env, &obj))
        .unwrap_or_default();
    let latitude = env
        .call_method(loc, "getLatitude", "()D", &[])
        .and_then(|v| v.d())
        .map_err(|e| format!("Location.getLatitude: {e}"))?;
    let longitude = env
        .call_method(loc, "getLongitude", "()D", &[])
        .and_then(|v| v.d())
        .map_err(|e| format!("Location.getLongitude: {e}"))?;
    if !latitude.is_finite()
        || !longitude.is_finite()
        || !(-90.0..=90.0).contains(&latitude)
        || !(-180.0..=180.0).contains(&longitude)
    {
        return Err("Location returned non-finite or out-of-range coordinates".into());
    }
    let accuracy_m = env
        .call_method(loc, "getAccuracy", "()F", &[])
        .and_then(|v| v.f())
        .map(|v| v as f64)
        .unwrap_or(-1.0);
    let timestamp_ms = env
        .call_method(loc, "getTime", "()J", &[])
        .and_then(|v| v.j())
        .unwrap_or(0);
    let altitude_m = env
        .call_method(loc, "hasAltitude", "()Z", &[])
        .and_then(|v| v.z())
        .unwrap_or(false)
        .then(|| {
            env.call_method(loc, "getAltitude", "()D", &[])
                .and_then(|v| v.d())
                .ok()
        })
        .flatten();
    let speed_mps = env
        .call_method(loc, "hasSpeed", "()Z", &[])
        .and_then(|v| v.z())
        .unwrap_or(false)
        .then(|| {
            env.call_method(loc, "getSpeed", "()F", &[])
                .and_then(|v| v.f())
                .ok()
                .map(|v| v as f64)
        })
        .flatten();
    let bearing_deg = env
        .call_method(loc, "hasBearing", "()Z", &[])
        .and_then(|v| v.z())
        .unwrap_or(false)
        .then(|| {
            env.call_method(loc, "getBearing", "()F", &[])
                .and_then(|v| v.f())
                .ok()
                .map(|v| v as f64)
        })
        .flatten();
    let mock = env
        .call_method(loc, "isFromMockProvider", "()Z", &[])
        .and_then(|v| v.z())
        .unwrap_or(false);
    Ok(LocationFix {
        provider,
        latitude,
        longitude,
        accuracy_m,
        altitude_m,
        speed_mps,
        bearing_deg,
        timestamp_ms,
        mock,
    })
}

fn latest_fix(env: &mut JNIEnv, manager: &JObject) -> Result<Option<LocationFix>, String> {
    let mut best: Option<LocationFix> = None;
    for provider in ["network", "fused", "gps"] {
        let name = match env.new_string(provider) {
            Ok(v) => v,
            Err(e) => {
                clear(env);
                return Err(format!("new_string provider: {e}"));
            }
        };
        let loc = match env
            .call_method(
                manager,
                "getLastKnownLocation",
                "(Ljava/lang/String;)Landroid/location/Location;",
                &[JValue::Object(&name)],
            )
            .and_then(|v| v.l())
        {
            Ok(v) => v,
            Err(e) => {
                clear(env);
                crate::log(&format!(
                    "location|WARN|provider={provider} call failed: {e}"
                ));
                continue;
            }
        };
        if loc.is_null() {
            continue;
        }
        match read_fix(env, &loc) {
            Ok(fix) => {
                if best
                    .as_ref()
                    .map(|old| fix.timestamp_ms > old.timestamp_ms)
                    .unwrap_or(true)
                {
                    best = Some(fix);
                }
            }
            Err(e) => {
                clear(env);
                crate::log(&format!(
                    "location|WARN|provider={provider} read failed: {e}"
                ));
            }
        }
    }
    Ok(best)
}

fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("write {tmp:?}: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("rename {tmp:?}: {e}"))
}

fn publish_diagnostics(rootfs: &Path, status: &str, fix: Option<&LocationFix>) {
    let dir = rootfs.join("run/husk/location");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        crate::log(&format!("location|WARN|mkdir {dir:?}: {e}"));
        return;
    }
    if let Err(e) = write_atomic(&dir.join("status"), status) {
        crate::log(&format!("location|WARN|{e}"));
    }
    if let Some(fix) = fix {
        let json = fix_json(Some(fix));
        if let Err(e) = write_atomic(&dir.join("last-known.json"), &(json + "\n")) {
            crate::log(&format!("location|WARN|{e}"));
        }
    } else {
        match std::fs::remove_file(dir.join("last-known.json")) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => crate::log(&format!("location|WARN|remove last-known: {e}")),
        }
    }
}

fn publish_once(
    rootfs: &Path,
    status: &str,
    fix: Option<&LocationFix>,
    last_published: &mut String,
) {
    if status != last_published {
        publish_diagnostics(rootfs, status, fix);
        last_published.clear();
        last_published.push_str(status);
    }
}

fn poll_loop() {
    let Some(files) = files_dir() else {
        crate::log("location|SKIP|no internalDataPath");
        return;
    };
    let rootfs = match crate::guest_rootfs::resolve(&files) {
        Ok(rootfs) => rootfs,
        Err(e) => {
            crate::log(&format!("location|SKIP|{e}"));
            return;
        }
    };
    let Some(vm) = (unsafe { java_vm() }) else {
        crate::log("location|SKIP|no JavaVM");
        return;
    };
    let mut env = match vm.attach_current_thread_as_daemon() {
        Ok(v) => v,
        Err(e) => {
            crate::log(&format!("location|FAIL|attach: {e}"));
            return;
        }
    };
    let activity = match unsafe { activity_object(&mut env) } {
        Ok(v) => v,
        Err(e) => {
            crate::log(&format!("location|FAIL|activity: {e}"));
            return;
        }
    };
    let mut last_state = String::new();
    let mut last_published = String::new();
    loop {
        match permission_granted(&mut env, &activity) {
            Ok(true) => {
                let manager = match location_manager(&mut env, &activity) {
                    Ok(v) => v,
                    Err(e) => {
                        if let Ok(mut slot) = FIX.lock() {
                            *slot = None;
                        }
                        publish_once(
                            &rootfs,
                            &format!("permission=granted\nbackend=unavailable\nerror={e}\n"),
                            None,
                            &mut last_published,
                        );
                        let msg = format!("location|WARN|manager: {e}");
                        if msg != last_state {
                            crate::log(&msg);
                            last_state = msg;
                        }
                        std::thread::sleep(Duration::from_millis(POLL_MS));
                        continue;
                    }
                };
                match latest_fix(&mut env, &manager) {
                    Ok(Some(fix)) => {
                        let status = format!(
                            "permission=granted\nbackend=android-location\ninterface=gpsd-unix://{}\nprovider={}\nupdated_ms={}\nmock={}\n",
                            GPSD_SOCKET_GUEST,
                            fix.provider,
                            fix.timestamp_ms,
                            fix.mock as u8
                        );
                        publish_once(&rootfs, &status, Some(&fix), &mut last_published);
                        if let Ok(mut slot) = FIX.lock() {
                            *slot = Some(fix.clone());
                        }
                        let msg = format!(
                            "location|FIX|provider={} acc={:.1}",
                            fix.provider, fix.accuracy_m
                        );
                        if msg != last_state {
                            crate::log(&msg);
                            last_state = msg;
                        }
                    }
                    Ok(None) => {
                        publish_once(
                            &rootfs,
                            "permission=granted\nbackend=android-location\ninterface=gpsd-unix:///run/husk/gps.sock\nfix=none\n",
                            None,
                            &mut last_published,
                        );
                        if let Ok(mut slot) = FIX.lock() {
                            *slot = None;
                        }
                        let msg = "location|NOFIX|permission granted, no last-known point";
                        if msg != last_state {
                            crate::log(msg);
                            last_state = msg.to_string();
                        }
                    }
                    Err(e) => {
                        if let Ok(mut slot) = FIX.lock() {
                            *slot = None;
                        }
                        publish_once(
                            &rootfs,
                            &format!("permission=granted\nbackend=android-location\nerror={e}\n"),
                            None,
                            &mut last_published,
                        );
                        let msg = format!("location|WARN|read: {e}");
                        if msg != last_state {
                            crate::log(&msg);
                            last_state = msg;
                        }
                    }
                }
            }
            Ok(false) => {
                if let Ok(mut slot) = FIX.lock() {
                    *slot = None;
                }
                publish_once(
                    &rootfs,
                    "permission=denied\nbackend=android-location\ninterface=gpsd-unix:///run/husk/gps.sock\nfix=none\n",
                    None,
                    &mut last_published,
                );
                let msg = "location|DENY|ACCESS_COARSE_LOCATION not granted";
                if msg != last_state {
                    crate::log(msg);
                    last_state = msg.to_string();
                }
            }
            Err(e) => {
                let msg = format!("location|FAIL|permission: {e}");
                if msg != last_state {
                    crate::log(&msg);
                    last_state = msg;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(POLL_MS));
    }
}

fn server_loop() {
    let Some(files) = files_dir() else {
        crate::log("location|SKIP|gpsd no internalDataPath");
        return;
    };
    let rootfs = match crate::guest_rootfs::resolve(&files) {
        Ok(rootfs) => rootfs,
        Err(e) => {
            crate::log(&format!("location|SKIP|gpsd {e}"));
            return;
        }
    };
    let run = rootfs.join("run/husk");
    if let Err(e) = std::fs::create_dir_all(&run) {
        crate::log(&format!("location|WARN|gpsd mkdir {run:?}: {e}"));
        return;
    }
    let path = rootfs.join(GPSD_SOCKET_REL);
    let _ = std::fs::remove_file(&path);
    let listener = match UnixListener::bind(&path) {
        Ok(v) => v,
        Err(e) => {
            crate::log(&format!("location|WARN|gpsd bind {path:?}: {e}"));
            return;
        }
    };
    crate::log(&format!("location|LISTEN|gpsd unix {GPSD_SOCKET_GUEST}"));
    publish_standard_socket(&rootfs);
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                std::thread::spawn(move || {
                    if let Err(e) = serve_client(stream) {
                        crate::log(&format!("location|WARN|gpsd client: {e}"));
                    }
                });
            }
            Err(e) => crate::log(&format!("location|WARN|gpsd accept: {e}")),
        }
    }
}

/// Most gpsd clients default to `/run/gpsd.sock`.  Add that path only when it
/// is absent; never replace a real daemon's socket or an unrelated file.
fn publish_standard_socket(rootfs: &Path) {
    let link = rootfs.join("run/gpsd.sock");
    let target = PathBuf::from("husk/gps.sock");
    match std::fs::symlink_metadata(&link) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if let Err(e) = std::os::unix::fs::symlink(&target, &link) {
                crate::log(&format!("location|WARN|gpsd default symlink {link:?}: {e}"));
            }
        }
        Ok(meta)
            if meta.file_type().is_symlink()
                && std::fs::read_link(&link).ok().as_deref() == Some(target.as_path()) => {}
        Ok(_) => crate::log(&format!(
            "location|INFO|gpsd default path occupied, kept as-is: {link:?}"
        )),
        Err(e) => crate::log(&format!("location|WARN|gpsd default lstat {link:?}: {e}")),
    }
}

fn serve_client(mut stream: UnixStream) -> Result<(), String> {
    stream
        .set_read_timeout(Some(CLIENT_TICK))
        .map_err(|e| format!("set_read_timeout: {e}"))?;
    let reader_stream = stream
        .try_clone()
        .map_err(|e| format!("clone stream: {e}"))?;
    let mut reader = BufReader::new(reader_stream);
    let mut watch = false;
    let mut last_sent = Instant::now() - Duration::from_secs(2);
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {
                let cmd = line.trim();
                if cmd.starts_with("?WATCH=") {
                    watch = !cmd.contains("\"enable\":false");
                    if watch {
                        send_line(&mut stream, &version_json())?;
                        send_line(&mut stream, &devices_json())?;
                        send_line(&mut stream, &watch_json())?;
                        send_line(&mut stream, &fix_json(current_fix().as_ref()))?;
                        last_sent = Instant::now();
                    }
                } else if cmd == "?POLL;" {
                    send_line(&mut stream, &fix_json(current_fix().as_ref()))?;
                } else if cmd == "?VERSION;" {
                    send_line(&mut stream, &version_json())?;
                } else if cmd == "?DEVICES;" {
                    send_line(&mut stream, &devices_json())?;
                }
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(e) => return Err(format!("read: {e}")),
        }
        if watch && last_sent.elapsed() >= Duration::from_secs(1) {
            send_line(&mut stream, &fix_json(current_fix().as_ref()))?;
            last_sent = Instant::now();
        }
    }
    Ok(())
}

fn current_fix() -> Option<LocationFix> {
    FIX.lock().ok().and_then(|v| v.clone())
}

fn send_line(stream: &mut UnixStream, line: &str) -> Result<(), String> {
    stream
        .write_all(line.as_bytes())
        .and_then(|_| stream.write_all(b"\n"))
        .and_then(|_| stream.flush())
        .map_err(|e| format!("write: {e}"))
}

fn version_json() -> String {
    r#"{"class":"VERSION","release":"3.25-husk","rev":"husk-location","proto_major":3,"proto_minor":15}"#.to_string()
}

fn devices_json() -> String {
    r#"{"class":"DEVICES","devices":[{"class":"DEVICE","path":"husk://android-location","driver":"AndroidLocation","flags":1,"native":0,"bps":0,"parity":0,"stopbits":0,"cycle":1.0}]}"#.to_string()
}

fn watch_json() -> String {
    r#"{"class":"WATCH","enable":true,"json":true,"nmea":false,"raw":0,"scaled":false,"timing":false,"split24":false,"pps":false}"#.to_string()
}

fn fix_json(fix: Option<&LocationFix>) -> String {
    let Some(fix) = fix else {
        return r#"{"class":"TPV","device":"husk","mode":1}"#.to_string();
    };
    let mut out = String::with_capacity(256);
    out.push_str(r#"{"class":"TPV","device":"husk","mode":"#);
    out.push(if fix.altitude_m.is_some() { '3' } else { '2' });
    out.push_str(&format!(
        r#","time":"{}","lat":{:.7},"lon":{:.7},"eph":{:.1}"#,
        format_time_ms(fix.timestamp_ms),
        fix.latitude,
        fix.longitude,
        fix.accuracy_m.max(0.0)
    ));
    if let Some(alt) = fix.altitude_m {
        out.push_str(&format!(r#","altHAE":{alt:.1}"#));
    }
    if let Some(speed) = fix.speed_mps {
        out.push_str(&format!(r#","speed":{speed:.2}"#));
    }
    if let Some(track) = fix.bearing_deg {
        out.push_str(&format!(r#","track":{track:.1}"#));
    }
    out.push_str(&format!(
        r#","provider":"{}","mock":{}}}"#,
        json_escape(&fix.provider),
        fix.mock
    ));
    out
}

fn json_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

fn format_time_ms(timestamp_ms: i64) -> String {
    let secs = timestamp_ms.div_euclid(1000);
    let millis = timestamp_ms.rem_euclid(1000);
    let days = secs.div_euclid(86_400);
    let day_secs = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = day_secs / 3600;
    let minute = (day_secs % 3600) / 60;
    let second = day_secs % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    year += (month <= 2) as i64;
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::{civil_from_days, fix_json, format_time_ms, LocationFix};

    #[test]
    fn epoch_formats_as_rfc3339_utc() {
        assert_eq!(format_time_ms(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(civil_from_days(0), (1970, 1, 1));
    }

    #[test]
    fn no_fix_is_mode_one_not_zero_coordinates() {
        assert_eq!(
            fix_json(None),
            r#"{"class":"TPV","device":"husk","mode":1}"#
        );
    }

    #[test]
    fn fix_json_carries_provider_and_coordinates() {
        let json = fix_json(Some(&LocationFix {
            provider: "network".into(),
            latitude: 22.5,
            longitude: 114.0,
            accuracy_m: 35.0,
            altitude_m: None,
            speed_mps: None,
            bearing_deg: None,
            timestamp_ms: 0,
            mock: false,
        }));
        assert!(json.contains(r#""mode":2"#));
        assert!(json.contains(r#""lat":22.5000000"#));
        assert!(json.contains(r#""provider":"network""#));
    }
}
