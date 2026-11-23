//! Pump the Husk AAudio capture bridge into PulseAudio's `module-pipe-source`.
//!
//! The mirror image of `audio-relay`: PulseAudio stays the guest-side mixer and
//! owns the FIFO, while the app process owns the only AAudio input stream. The
//! client is deliberately dumb - `READ frames=N`, one `PCM_OK` line, exactly
//! N frames of interleaved PCM16 - so nothing in the guest has to know what
//! Android's audio HAL is doing.

use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::net::UnixStream;
use std::process::Command;
use std::time::{Duration, Instant};

const O_NONBLOCK: i32 = 0x800;

fn usage() -> ! {
    eprintln!(
        "usage: audio-capture <socket> <fifo> [seconds] [rate=48000] [channels=2] \
         [frames=1024] [gate=1] [pactl=/usr/bin/pactl]"
    );
    std::process::exit(2)
}

fn parse_kv(args: &[String], key: &str, default: u32) -> u32 {
    for arg in args {
        if let Some((k, v)) = arg.split_once('=') {
            if k == key {
                return v.parse::<u32>().unwrap_or(default);
            }
        }
    }
    default
}

fn read_line(stream: &mut UnixStream) -> Result<String, String> {
    let mut out = Vec::with_capacity(120);
    let mut one = [0u8; 1];
    loop {
        stream
            .read_exact(&mut one)
            .map_err(|e| format!("read response: {e}"))?;
        if one[0] == b'\n' {
            break;
        }
        if one[0] != b'\r' {
            if out.len() >= 4096 {
                return Err("response line too long".into());
            }
            out.push(one[0]);
        }
    }
    String::from_utf8(out).map_err(|_| "non-UTF-8 response".into())
}

fn request(stream: &mut UnixStream, line: &str) -> Result<String, String> {
    stream
        .write_all(line.as_bytes())
        .and_then(|_| stream.write_all(b"\n"))
        .and_then(|_| stream.flush())
        .map_err(|e| format!("send {line}: {e}"))?;
    read_line(stream)
}

fn field<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    line.split_whitespace()
        .filter_map(|f| f.split_once('='))
        .find_map(|(k, v)| (k == key).then_some(v))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        usage()
    }
    let socket = &args[0];
    let fifo = &args[1];
    let seconds = args.get(2).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
    let rate = parse_kv(&args, "rate", 48_000);
    let channels = parse_kv(&args, "channels", 2);
    let frames = parse_kv(&args, "frames", 1024).clamp(64, 8192);
    let gate = parse_kv(&args, "gate", 0) != 0;
    let mut pactl = "/usr/bin/pactl".to_string();
    for arg in &args {
        if let Some((k, v)) = arg.split_once('=') {
            if k == "pactl" {
                pactl = v.to_string();
            }
        }
    }

    if let Err(e) = run(socket, fifo, seconds, rate, channels, frames, gate, &pactl) {
        eprintln!("capture|FAIL|{e}");
        std::process::exit(1);
    }
}

/// Ask PulseAudio which source index `husk_mic` got. The prior art
/// (DroidNative `session.sh`) does the same with `pactl list short sources`
/// plus `pactl subscribe`; we poll instead of subscribing so the guest needs no
/// shell loop.
fn source_index(pactl: &str) -> Result<Option<String>, String> {
    let out = Command::new(pactl)
        .args(["list", "short", "sources"])
        .output()
        .map_err(|e| format!("spawn {pactl}: {e}"))?;
    if !out.status.success() {
        return Err(format!("{pactl} list sources rc={}", out.status));
    }
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let mut fields = line.split('\t');
        if let (Some(index), Some(name)) = (fields.next(), fields.next()) {
            if name == "husk_mic" {
                return Ok(Some(index.to_string()));
            }
        }
    }
    Ok(None)
}

/// True while any guest stream records from our microphone source.
fn recorder_attached(pactl: &str, source: &Option<String>) -> Result<bool, String> {
    let out = Command::new(pactl)
        .args(["list", "short", "source-outputs"])
        .output()
        .map_err(|e| format!("spawn {pactl}: {e}"))?;
    if !out.status.success() {
        return Err(format!("{pactl} list source-outputs rc={}", out.status));
    }
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let mut fields = line.split('\t');
        let _own = fields.next();
        match (source, fields.next()) {
            // Count only streams attached to our source; a recorder on
            // `husk_pipe.monitor` must not switch the phone microphone on.
            (Some(want), Some(got)) if want == got => return Ok(true),
            (None, Some(_)) => return Ok(true),
            _ => {}
        }
    }
    Ok(false)
}

fn connect_retry(socket: &str) -> Result<UnixStream, String> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match UnixStream::connect(socket) {
            Ok(stream) => return Ok(stream),
            Err(e) if Instant::now() < deadline => {
                let _ = e;
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(format!("connect {socket}: {e}")),
        }
    }
}

fn wait_fifo(fifo: &str) -> Result<std::fs::File, String> {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        match OpenOptions::new()
            .write(true)
            .custom_flags(O_NONBLOCK)
            .open(fifo)
        {
            Ok(file) => return Ok(file),
            Err(_) => {
                if Instant::now() > deadline {
                    return Err(format!("no reader on {fifo}"));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run(
    socket: &str,
    fifo: &str,
    seconds: u64,
    rate: u32,
    channels: u32,
    frames: u32,
    gate: bool,
    pactl: &str,
) -> Result<(), String> {
    let mut stream: Option<UnixStream> = None;
    let mut pipe: Option<std::fs::File> = None;
    let mut started: Option<Instant> = None;
    let mut total_frames: u64 = 0;
    let mut total_bytes: u64 = 0;
    let mut silent_chunks: u64 = 0;
    let mut peak: i32 = 0;
    let mut last_report = Instant::now();
    let mut buf = vec![0u8; frames as usize * channels as usize * 2];
    let mut idle_announced = false;
    let mut probe_failures = 0u32;

    loop {
        if gate {
            let source = source_index(pactl).unwrap_or(None);
            match recorder_attached(pactl, &source) {
                Ok(true) => idle_announced = false,
                Ok(false) => {
                    if let Some(mut open) = stream.take() {
                        match request(&mut open, "CLOSE") {
                            Ok(line) => println!("capture|GATE|close|{line}"),
                            Err(e) => println!("capture|GATE|close_fail|{e}"),
                        }
                    }
                    pipe = None;
                    if !idle_announced {
                        println!("capture|GATE|idle|no recorder on husk_mic");
                        idle_announced = true;
                    }
                    std::thread::sleep(Duration::from_millis(300));
                    continue;
                }
                Err(e) => {
                    // A failing probe must not wedge capture: keep recording.
                    probe_failures += 1;
                    if probe_failures <= 3 {
                        println!("capture|GATE|probe_fail|{e}");
                    }
                }
            }
        }

        if seconds > 0 {
            if let Some(start) = started {
                if start.elapsed() >= Duration::from_secs(seconds) {
                    break;
                }
            }
        }

        if stream.is_none() {
            let mut fresh = connect_retry(socket)?;
            let ready = request(&mut fresh, &format!("OPEN rate={rate} channels={channels}"))?;
            if !ready.starts_with("CAPTURE_READY") {
                return Err(format!("OPEN failed: {ready}"));
            }
            println!("capture|OPEN|{ready}");
            // PulseAudio's module-pipe-source holds the read end; wait for it
            // without spinning on a blocking open (which would look like a hang).
            pipe = Some(wait_fifo(fifo)?);
            stream = Some(fresh);
            if started.is_none() {
                started = Some(Instant::now());
                last_report = Instant::now();
            }
        }

        let open_stream = stream.as_mut().expect("stream opened above");
        let reply = request(open_stream, &format!("READ frames={frames}"))?;
        if !reply.starts_with("PCM_OK") {
            println!("capture|STOP|{reply}");
            break;
        }
        let bytes: usize = field(&reply, "bytes")
            .and_then(|v| v.parse().ok())
            .ok_or_else(|| format!("bad PCM_OK: {reply}"))?;
        if bytes > buf.len() {
            buf.resize(bytes, 0);
        }
        open_stream
            .read_exact(&mut buf[..bytes])
            .map_err(|e| format!("read pcm: {e}"))?;
        total_frames += frames as u64;
        total_bytes += bytes as u64;
        let mut chunk_peak = 0i32;
        for sample in buf[..bytes].chunks_exact(2) {
            let value = i16::from_le_bytes([sample[0], sample[1]]) as i32;
            chunk_peak = chunk_peak.max(value.abs());
        }
        peak = peak.max(chunk_peak);
        if chunk_peak == 0 {
            silent_chunks += 1;
        }
        // A FIFO nobody drains must not wedge capture; drop the chunk instead.
        if let Some(open_pipe) = pipe.as_mut() {
            if let Err(e) = open_pipe.write_all(&buf[..bytes]) {
                if e.kind() != std::io::ErrorKind::WouldBlock {
                    return Err(format!("fifo write: {e}"));
                }
            }
        }
        if last_report.elapsed() >= Duration::from_secs(1) {
            println!(
                "capture|PROGRESS|frames={total_frames}|bytes={total_bytes}|silent={silent_chunks}|peak={peak}"
            );
            last_report = Instant::now();
        }
    }

    if let Some(mut open) = stream {
        match request(&mut open, "CLOSE") {
            Ok(line) => println!("capture|CLOSE|{line}"),
            Err(e) => println!("capture|CLOSE_FAIL|{e}"),
        }
    }
    let elapsed = started.map(|s| s.elapsed().as_millis()).unwrap_or(0);
    println!(
        "capture|PASS|frames={total_frames}|bytes={total_bytes}|silent_chunks={silent_chunks}|peak={peak}|elapsed_ms={elapsed}|gate={gate}"
    );
    Ok(())
}
