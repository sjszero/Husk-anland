//! Guest-side camera client: drive the app's camera bridge socket and dump
//! NV12 frames. This is the transport proof *before* the `/dev/videoN` V4L2
//! emulation lands, so the wire protocol can be validated on its own.
//!
//! usage: camera-client <socket> <out-dir> [frames=3] [w=1280] [h=720] [id=1]
//!
//! Protocol (line oriented, then a raw payload):
//!   -> OPEN w=.. h=.. [id=..]
//!   <- OK w=.. h=.. fourcc=NV12 bytes=..
//!   -> FRAME
//!   <- FRAME seq=.. ts=.. w=.. h=.. bytes=..  + payload   | NOFRAME
//!   -> STOP
//!   <- BYE

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Instant;

fn usage() -> ! {
    eprintln!("usage: camera-client <socket> <out-dir> [frames=3] [w=1280] [h=720] [id=1]");
    std::process::exit(2)
}

fn arg_u32(args: &[String], key: &str, default: u32) -> u32 {
    args.iter()
        .find_map(|a| a.split_once('=').filter(|(k, _)| *k == key).map(|(_, v)| v))
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn arg_str<'a>(args: &'a [String], key: &str) -> Option<&'a str> {
    args.iter()
        .find_map(|a| a.split_once('=').filter(|(k, _)| *k == key).map(|(_, v)| v))
}

fn read_line(stream: &mut UnixStream) -> Result<String, String> {
    let mut out = Vec::with_capacity(96);
    let mut one = [0u8; 1];
    loop {
        stream
            .read_exact(&mut one)
            .map_err(|e| format!("read: {e}"))?;
        match one[0] {
            b'\n' => return Ok(String::from_utf8_lossy(&out).into_owned()),
            b'\r' => {}
            _ => out.push(one[0]),
        }
    }
}

fn field<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    line.split_whitespace()
        .filter_map(|f| f.split_once('='))
        .find_map(|(k, v)| (k == key).then_some(v))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        usage();
    }
    let socket = &args[0];
    let out_dir = Path::new(&args[1]);
    let frames = arg_u32(&args, "frames", 3);
    let width = arg_u32(&args, "w", 1280);
    let height = arg_u32(&args, "h", 720);
    let id = arg_str(&args, "id");

    if let Err(e) = run(socket, out_dir, frames, width, height, id) {
        eprintln!("camera-client|FAIL|{e}");
        std::process::exit(1);
    }
}

fn run(
    socket: &str,
    out_dir: &Path,
    frames: u32,
    width: u32,
    height: u32,
    id: Option<&str>,
) -> Result<(), String> {
    std::fs::create_dir_all(out_dir).map_err(|e| format!("mkdir: {e}"))?;
    let mut stream = UnixStream::connect(socket).map_err(|e| format!("connect {socket}: {e}"))?;
    let open = match id {
        Some(id) => format!("OPEN w={width} h={height} id={id}\n"),
        None => format!("OPEN w={width} h={height}\n"),
    };
    stream
        .write_all(open.as_bytes())
        .and_then(|_| stream.write_all(b""))
        .map_err(|e| format!("send OPEN: {e}"))?;
    let ready = read_line(&mut stream)?;
    if !ready.starts_with("OK") {
        return Err(format!("OPEN rejected: {ready}"));
    }
    println!("camera-client|OPEN|{ready}");
    let w: usize = field(&ready, "w")
        .and_then(|v| v.parse().ok())
        .unwrap_or(width as usize);
    let h: usize = field(&ready, "h")
        .and_then(|v| v.parse().ok())
        .unwrap_or(height as usize);
    let bytes: usize = field(&ready, "bytes")
        .and_then(|v| v.parse().ok())
        .unwrap_or(w * h * 3 / 2);

    let started = Instant::now();
    let mut buf = vec![0u8; bytes];
    let mut saved = 0u32;
    let mut seqs = Vec::new();
    while saved < frames {
        stream
            .write_all(b"FRAME\n")
            .map_err(|e| format!("send FRAME: {e}"))?;
        let line = read_line(&mut stream)?;
        if line.starts_with("NOFRAME") {
            println!("camera-client|NOFRAME|{saved}");
            continue;
        }
        if !line.starts_with("FRAME") {
            return Err(format!("unexpected reply: {line}"));
        }
        let n: usize = field(&line, "bytes")
            .and_then(|v| v.parse().ok())
            .ok_or_else(|| format!("bad header: {line}"))?;
        if n != bytes {
            return Err(format!("frame size changed: {n} != {bytes}"));
        }
        stream
            .read_exact(&mut buf)
            .map_err(|e| format!("read payload: {e}"))?;
        let (min, max, mean) = stats(&buf);
        let seq = field(&line, "seq").unwrap_or("?");
        let ts = field(&line, "ts").unwrap_or("?");
        seqs.push(seq.to_string());
        let path = out_dir.join(format!("frame-{saved:02}.nv12"));
        std::fs::write(&path, &buf).map_err(|e| format!("write: {e}"))?;
        println!(
            "camera-client|FRAME|seq={seq}|ts={ts}|{w}x{h}|bytes={n}|min={min}|max={max}|mean={mean:.2}"
        );
        saved += 1;
    }
    stream
        .write_all(b"STOP\n")
        .map_err(|e| format!("send STOP: {e}"))?;
    let bye = read_line(&mut stream).unwrap_or_default();
    println!(
        "camera-client|PASS|frames={saved}|elapsed_ms={}|bye={bye}|seqs={}",
        started.elapsed().as_millis(),
        seqs.join(",")
    );
    Ok(())
}

fn stats(frame: &[u8]) -> (u8, u8, f64) {
    if frame.is_empty() {
        return (0, 0, 0.0);
    }
    let mut min = 255u8;
    let mut max = 0u8;
    let mut sum = 0u64;
    for &b in frame {
        min = min.min(b);
        max = max.max(b);
        sum += b as u64;
    }
    (min, max, sum as f64 / frame.len() as f64)
}
