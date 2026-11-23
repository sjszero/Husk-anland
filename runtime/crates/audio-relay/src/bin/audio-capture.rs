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
use std::time::{Duration, Instant};

const O_NONBLOCK: i32 = 0x800;

fn usage() -> ! {
    eprintln!(
        "usage: audio-capture <socket> <fifo> [seconds] [rate=48000] [channels=2] [frames=1024]"
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

    if let Err(e) = run(socket, fifo, seconds, rate, channels, frames) {
        eprintln!("capture|FAIL|{e}");
        std::process::exit(1);
    }
}

fn run(
    socket: &str,
    fifo: &str,
    seconds: u64,
    rate: u32,
    channels: u32,
    frames: u32,
) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut stream = loop {
        match UnixStream::connect(socket) {
            Ok(stream) => break stream,
            Err(e) if Instant::now() < deadline => {
                let _ = e;
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(format!("connect {socket}: {e}")),
        }
    };
    let ready = request(
        &mut stream,
        &format!("OPEN rate={rate} channels={channels}"),
    )?;
    if !ready.starts_with("CAPTURE_READY") {
        return Err(format!("OPEN failed: {ready}"));
    }
    println!("capture|OPEN|{ready}");

    // PulseAudio's module-pipe-source holds the read end; wait for it without
    // spinning on a blocking open (which would look like a hang).
    let mut pipe = loop {
        match OpenOptions::new()
            .write(true)
            .custom_flags(O_NONBLOCK)
            .open(fifo)
        {
            Ok(file) => break file,
            Err(_) => {
                if Instant::now() > deadline + Duration::from_secs(10) {
                    return Err(format!("no reader on {fifo}"));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    };

    let started = Instant::now();
    let mut total_frames: u64 = 0;
    let mut total_bytes: u64 = 0;
    let mut silent_chunks: u64 = 0;
    let mut peak: i32 = 0;
    let mut last_report = Instant::now();
    let mut buf = vec![0u8; frames as usize * channels as usize * 2];
    loop {
        if seconds > 0 && started.elapsed() >= Duration::from_secs(seconds) {
            break;
        }
        let reply = request(&mut stream, &format!("READ frames={frames}"))?;
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
        stream
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
        if let Err(e) = pipe.write_all(&buf[..bytes]) {
            if e.kind() != std::io::ErrorKind::WouldBlock {
                return Err(format!("fifo write: {e}"));
            }
        }
        if last_report.elapsed() >= Duration::from_secs(1) {
            println!(
                "capture|PROGRESS|frames={total_frames}|bytes={total_bytes}|silent={silent_chunks}|peak={peak}"
            );
            last_report = Instant::now();
        }
    }
    match request(&mut stream, "CLOSE") {
        Ok(line) => println!("capture|CLOSE|{line}"),
        Err(e) => println!("capture|CLOSE_FAIL|{e}"),
    }
    println!(
        "capture|PASS|frames={total_frames}|bytes={total_bytes}|silent_chunks={silent_chunks}|peak={peak}|elapsed_ms={}",
        started.elapsed().as_millis()
    );
    Ok(())
}
