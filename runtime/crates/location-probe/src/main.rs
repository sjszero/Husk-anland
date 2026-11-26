//! Guest-side probe for the Husk Android-location -> gpsd Unix socket bridge.
//!
//! The probe speaks only the small gpsd JSON subset Husk exposes: `VERSION`,
//! `WATCH`, `POLL` and `TPV`. Coordinates are redacted by default so gate logs
//! and public evidence never contain a real location.

use std::env;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

const DEFAULT_SOCKET: &str = "/run/husk/gps.sock";
const DEADLINE: Duration = Duration::from_secs(8);

#[derive(Debug)]
struct Fix {
    latitude: f64,
    longitude: f64,
    accuracy_m: Option<f64>,
    provider: String,
    mode: i64,
}

fn field_start(line: &str, key: &str) -> Option<usize> {
    let pattern = format!("\"{key}\":");
    line.find(&pattern).map(|at| at + pattern.len())
}

fn json_number(line: &str, key: &str) -> Option<f64> {
    let rest = &line[field_start(line, key)?..];
    let end = rest.find([',', '}']).unwrap_or(rest.len());
    rest[..end].trim().parse().ok()
}

fn json_integer(line: &str, key: &str) -> Option<i64> {
    let rest = &line[field_start(line, key)?..];
    let end = rest.find([',', '}']).unwrap_or(rest.len());
    rest[..end].trim().parse().ok()
}

fn json_string(line: &str, key: &str) -> Option<String> {
    let rest = line.get(field_start(line, key)?..)?.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn parse_fix(line: &str) -> Option<Fix> {
    if json_string(line, "class").as_deref() != Some("TPV") {
        return None;
    }
    let mode = json_integer(line, "mode")?;
    if mode < 2 {
        return None;
    }
    let latitude = json_number(line, "lat")?;
    let longitude = json_number(line, "lon")?;
    if !latitude.is_finite()
        || !longitude.is_finite()
        || !(-90.0..=90.0).contains(&latitude)
        || !(-180.0..=180.0).contains(&longitude)
    {
        return None;
    }
    Some(Fix {
        latitude,
        longitude,
        accuracy_m: json_number(line, "eph"),
        provider: json_string(line, "provider").unwrap_or_else(|| "-".into()),
        mode,
    })
}

fn run(socket: &str, reveal: bool) -> Result<String, String> {
    let mut stream = UnixStream::connect(socket).map_err(|e| format!("connect={socket}: {e}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(1)))
        .map_err(|e| format!("read timeout: {e}"))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .map_err(|e| format!("write timeout: {e}"))?;
    stream
        .write_all(b"?WATCH={\"enable\":true,\"json\":true}\n?POLL;\n")
        .and_then(|_| stream.flush())
        .map_err(|e| format!("request: {e}"))?;

    let deadline = Instant::now() + DEADLINE;
    let mut pending = Vec::new();
    let mut chunk = [0u8; 4096];
    let mut version = false;
    let mut fix = None;
    while Instant::now() < deadline && fix.is_none() {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                pending.extend_from_slice(&chunk[..n]);
                while let Some(end) = pending.iter().position(|byte| *byte == b'\n') {
                    let line: Vec<u8> = pending.drain(..=end).collect();
                    let line = String::from_utf8_lossy(&line);
                    if json_string(&line, "class").as_deref() == Some("VERSION") {
                        version = true;
                    }
                    if let Some(candidate) = parse_fix(&line) {
                        fix = Some(candidate);
                        break;
                    }
                }
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(e) => return Err(format!("read: {e}")),
        }
    }

    let fix = fix.ok_or_else(|| "no TPV fix".to_string())?;
    if !version {
        return Err("no VERSION".into());
    }
    let coordinates = if reveal {
        format!("lat={}|lon={}", fix.latitude, fix.longitude)
    } else {
        "coordinates=redacted".into()
    };
    let accuracy = fix
        .accuracy_m
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".into());
    Ok(format!(
        "location-probe|PASS|{coordinates}|eph={accuracy}|provider={}|mode={}",
        fix.provider, fix.mode
    ))
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let reveal = args.iter().any(|arg| arg == "--reveal");
    let socket = args
        .iter()
        .find(|arg| !arg.starts_with("--"))
        .cloned()
        .or_else(|| env::var("GPSD_SOCKET").ok())
        .unwrap_or_else(|| DEFAULT_SOCKET.into());
    match run(&socket, reveal) {
        Ok(line) => println!("{line}"),
        Err(e) => {
            eprintln!("location-probe|FAIL|{e}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{json_number, json_string, parse_fix};

    #[test]
    fn parses_gpsd_tpv() {
        let fix = parse_fix(
            r#"{"class":"TPV","device":"husk","mode":2,"lat":22.5,"lon":114.0,"eph":35.0,"provider":"network"}"#,
        )
        .expect("valid fix");
        assert_eq!(fix.provider, "network");
        assert_eq!(fix.mode, 2);
        assert_eq!(fix.accuracy_m, Some(35.0));
    }

    #[test]
    fn rejects_no_fix_and_invalid_coordinates() {
        assert!(parse_fix(r#"{"class":"TPV","mode":1}"#).is_none());
        assert!(parse_fix(r#"{"class":"TPV","mode":2,"lat":91,"lon":0}"#).is_none());
    }

    #[test]
    fn extracts_json_scalars() {
        let line = r#"{"class":"TPV","lat":22.5,"provider":"gps"}"#;
        assert_eq!(json_number(line, "lat"), Some(22.5));
        assert_eq!(json_string(line, "class").as_deref(), Some("TPV"));
    }
}
