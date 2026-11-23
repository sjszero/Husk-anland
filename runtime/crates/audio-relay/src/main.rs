//! Forward PulseAudio's `module-pipe-sink` FIFO to the Husk AAudio bridge.
//!
//! This is intentionally a plain Rust data pump: PulseAudio remains the guest-side mixer,
//! while the app process owns the only Android AAudio stream. The control socket is the
//! dedicated audio channel and is never shared with input, clipboard or video.

use std::fs::OpenOptions;
use std::io::{self, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

const O_NONBLOCK: i32 = 0x800;

fn usage() -> ! {
    eprintln!(
        "usage: audio-relay <audio-bridge.sock> <pulse-pipe> [seconds] [rate=48000] [channels=2]"
    );
    std::process::exit(2)
}

fn parse_kv(args: &[String], key: &str, default: u32) -> Result<u32, String> {
    for arg in args {
        if let Some((k, v)) = arg.split_once('=') {
            if k == key {
                return v.parse::<u32>().map_err(|_| format!("bad {key}={v}"));
            }
        }
    }
    Ok(default)
}

fn read_line(stream: &mut UnixStream) -> Result<String, String> {
    let mut out = Vec::with_capacity(160);
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

fn connect_with_retry(path: &str) -> Result<UnixStream, String> {
    let mut last = String::new();
    for _ in 0..50 {
        match UnixStream::connect(path) {
            Ok(stream) => return Ok(stream),
            Err(e) => {
                last = e.to_string();
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
    Err(format!("connect {path}: {last}"))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        usage();
    }
    let socket = &args[0];
    let fifo = &args[1];
    let seconds = args.get(2).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
    let rate = parse_kv(&args, "rate", 48_000).unwrap_or_else(|e| {
        eprintln!("relay|FAIL|{e}");
        std::process::exit(1);
    });
    let channels = parse_kv(&args, "channels", 2).unwrap_or_else(|e| {
        eprintln!("relay|FAIL|{e}");
        std::process::exit(1);
    });
    if !(8_000..=384_000).contains(&rate) || !(1..=8).contains(&channels) {
        eprintln!("relay|FAIL|invalid format rate={rate} channels={channels}");
        std::process::exit(1);
    }

    let result = (|| -> Result<(), String> {
        let mut control = connect_with_retry(socket)?;
        control
            .set_read_timeout(Some(Duration::from_secs(15)))
            .map_err(|e| format!("set socket read timeout: {e}"))?;
        control
            .set_write_timeout(Some(Duration::from_secs(15)))
            .map_err(|e| format!("set socket write timeout: {e}"))?;
        let ready = request(
            &mut control,
            &format!("OPEN rate={rate} channels={channels}"),
        )?;
        println!("relay|OPEN|{ready}");
        if !ready.starts_with("OPEN_READY") {
            return Err(format!("audio bridge rejected OPEN: {ready}"));
        }

        let mut pipe = OpenOptions::new()
            .read(true)
            .custom_flags(O_NONBLOCK)
            .open(fifo)
            .map_err(|e| format!("open FIFO {fifo}: {e}"))?;
        println!(
            "relay|BEGIN|fifo={fifo}|rate={rate}|channels={channels}|seconds={}",
            if seconds == 0 {
                "until-killed".to_string()
            } else {
                seconds.to_string()
            }
        );

        let frame_bytes = channels as usize * 2;
        let deadline = (seconds != 0).then(|| Instant::now() + Duration::from_secs(seconds));
        let started = Instant::now();
        let mut carry = Vec::with_capacity(frame_bytes * 2);
        let mut buf = [0u8; 8192];
        let mut total_frames = 0u64;
        let mut chunks = 0u64;
        let mut last_ack = String::new();

        loop {
            if let Some(deadline) = deadline {
                if Instant::now() >= deadline {
                    break;
                }
            }
            match pipe.read(&mut buf) {
                Ok(0) => std::thread::sleep(Duration::from_millis(10)),
                Ok(n) => {
                    carry.extend_from_slice(&buf[..n]);
                    let usable = carry.len() / frame_bytes * frame_bytes;
                    if usable == 0 {
                        continue;
                    }
                    let pcm = carry[..usable].to_vec();
                    carry.drain(..usable);
                    let frames = usable / frame_bytes;
                    let line = format!("DATA frames={frames}");
                    control
                        .write_all(line.as_bytes())
                        .and_then(|_| control.write_all(b"\n"))
                        .and_then(|_| control.write_all(&pcm))
                        .and_then(|_| control.flush())
                        .map_err(|e| format!("send PCM chunk: {e}"))?;
                    let ack = read_line(&mut control)?;
                    if !ack.starts_with("DATA_OK ") {
                        return Err(format!("PCM chunk rejected: {ack}"));
                    }
                    total_frames += frames as u64;
                    chunks += 1;
                    last_ack = ack;
                    if chunks % 100 == 0 {
                        println!(
                            "relay|PROGRESS|chunks={chunks}|frames={total_frames}|elapsed_ms={}",
                            started.elapsed().as_millis()
                        );
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(e) => return Err(format!("read FIFO: {e}")),
            }
        }

        control
            .write_all(b"CLOSE\n")
            .and_then(|_| control.flush())
            .map_err(|e| format!("send CLOSE: {e}"))?;
        let close = read_line(&mut control)?;
        println!("relay|CLOSE|{close}");
        if !close.starts_with("CLOSE_OK ") {
            return Err(format!("CLOSE rejected: {close}"));
        }
        let _ = request(&mut control, "QUIT");
        println!(
            "relay|PASS|chunks={chunks}|frames={total_frames}|elapsed_ms={}|last_ack={last_ack}",
            started.elapsed().as_millis()
        );
        Ok(())
    })();

    match result {
        Ok(()) => println!("RESULT PASS"),
        Err(e) => {
            eprintln!("relay|FAIL|{e}");
            println!("RESULT FAIL");
            std::process::exit(1);
        }
    }
}
