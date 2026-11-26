//! Guest-side end-to-end probe for the Husk AAudio capture bridge.
//!
//! The probe speaks the bridge's small line protocol directly. A PASS means
//! OPEN/READ/CLOSE completed with the requested PCM16LE bytes; `peak` and
//! `silent_chunks` describe the captured samples without exposing audio data.

use std::env;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

const DEFAULT_SOCKET: &str = "/run/husk/audio-in.sock";
const DEFAULT_SECONDS: f64 = 1.0;
const RATE: u32 = 48_000;
const CHANNELS: u32 = 2;
const FRAMES: u32 = 1_024;
const IO_TIMEOUT: Duration = Duration::from_secs(5);
const AUDIO_SOCKETS: [&str; 2] = ["/run/husk/audio-out.sock", "/run/husk/audio-in.sock"];

#[derive(Debug, Default, PartialEq, Eq)]
struct CaptureStats {
    chunks: u64,
    frames: u64,
    bytes: u64,
    silent_chunks: u64,
    peak: i32,
}

fn field<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    line.split_whitespace()
        .filter_map(|item| item.split_once('='))
        .find_map(|(name, value)| (name == key).then_some(value))
}

fn read_line(stream: &mut UnixStream) -> Result<String, String> {
    let mut out = Vec::with_capacity(120);
    let mut byte = [0u8; 1];
    loop {
        stream
            .read_exact(&mut byte)
            .map_err(|e| format!("read response: {e}"))?;
        match byte[0] {
            b'\n' => break,
            b'\r' => {}
            value if out.len() >= 4_096 => {
                return Err(format!("response line too long (next byte 0x{value:02x})"));
            }
            value => out.push(value),
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

fn pcm_peak(pcm: &[u8]) -> i32 {
    pcm.chunks_exact(2)
        .map(|sample| i16::from_le_bytes([sample[0], sample[1]]) as i32)
        .map(i32::abs)
        .max()
        .unwrap_or(0)
}

fn run(socket_path: &str, seconds: f64) -> Result<String, String> {
    if !seconds.is_finite() || seconds <= 0.0 {
        return Err(format!("invalid duration: {seconds}"));
    }
    let mut stream =
        UnixStream::connect(socket_path).map_err(|e| format!("connect {socket_path}: {e}"))?;
    stream
        .set_read_timeout(Some(IO_TIMEOUT))
        .map_err(|e| format!("set read timeout: {e}"))?;
    stream
        .set_write_timeout(Some(IO_TIMEOUT))
        .map_err(|e| format!("set write timeout: {e}"))?;

    let pong = request(&mut stream, "PING")?;
    println!("{pong}");
    if !pong.starts_with("PONG") {
        return Err(format!("bad PING response: {pong}"));
    }

    let ready = request(
        &mut stream,
        &format!("OPEN rate={RATE} channels={CHANNELS}"),
    )?;
    println!("{ready}");
    if !ready.starts_with("CAPTURE_READY") {
        return Err(format!("OPEN failed: {ready}"));
    }

    let started = Instant::now();
    let limit = Duration::from_secs_f64(seconds);
    let mut stats = CaptureStats::default();
    while started.elapsed() < limit {
        let reply = request(&mut stream, &format!("READ frames={FRAMES}"))?;
        println!("{reply}");
        if !reply.starts_with("PCM_OK") {
            return Err(format!("READ failed: {reply}"));
        }
        let byte_count: usize = field(&reply, "bytes")
            .and_then(|value| value.parse().ok())
            .ok_or_else(|| format!("bad PCM_OK bytes: {reply}"))?;
        let frame_count: u64 = field(&reply, "frames")
            .and_then(|value| value.parse().ok())
            .ok_or_else(|| format!("bad PCM_OK frames: {reply}"))?;
        let mut pcm = vec![0u8; byte_count];
        stream
            .read_exact(&mut pcm)
            .map_err(|e| format!("read PCM ({byte_count} bytes): {e}"))?;

        let chunk_peak = pcm_peak(&pcm);
        stats.peak = stats.peak.max(chunk_peak);
        if chunk_peak == 0 {
            stats.silent_chunks += 1;
        }
        stats.chunks += 1;
        stats.frames += frame_count;
        stats.bytes += byte_count as u64;
    }

    let closed = request(&mut stream, "CLOSE")?;
    println!("{closed}");
    if !closed.starts_with("CAPTURE_OK") {
        return Err(format!("CLOSE failed: {closed}"));
    }

    let elapsed_ms = started.elapsed().as_millis();
    Ok(format!(
        "capture-probe|PASS|chunks={}|frames={}|bytes={}|silent_chunks={}|peak={}|elapsed_ms={elapsed_ms}",
        stats.chunks, stats.frames, stats.bytes, stats.silent_chunks, stats.peak
    ))
}

fn run_concurrency(wait: Duration) -> Result<String, String> {
    let mut passed = 0u32;
    let mut failed = 0u32;
    for path in AUDIO_SOCKETS {
        let idle = match UnixStream::connect(path) {
            Ok(stream) => stream,
            Err(e) => {
                println!("  SKIP  {path}: {e}");
                continue;
            }
        };
        match UnixStream::connect(path) {
            Ok(mut second) => {
                second
                    .set_read_timeout(Some(wait))
                    .map_err(|e| format!("set read timeout for {path}: {e}"))?;
                second
                    .write_all(b"PING\n")
                    .and_then(|_| second.flush())
                    .map_err(|e| format!("send PING to {path}: {e}"))?;
                let mut answer = [0u8; 256];
                match second.read(&mut answer) {
                    Ok(n) if answer[..n].windows(4).any(|w| w == b"PONG") => {
                        println!("  PASS  {path}: second client answered while the first was idle");
                        passed += 1;
                    }
                    Ok(n) => {
                        println!(
                            "  FAIL  {path}: odd answer {:?}",
                            String::from_utf8_lossy(&answer[..n])
                        );
                        failed += 1;
                    }
                    Err(e)
                        if matches!(
                            e.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                        ) =>
                    {
                        println!(
                            "  FAIL  {path}: second client blocked {:.0}s (head-of-line blocking)",
                            wait.as_secs_f64()
                        );
                        failed += 1;
                    }
                    Err(e) => {
                        println!("  FAIL  {path}: {e}");
                        failed += 1;
                    }
                }
            }
            Err(e) => {
                println!("  FAIL  {path}: {e}");
                failed += 1;
            }
        }
        drop(idle);
    }
    let summary = format!("audio-bridge: PASS={passed} FAIL={failed}");
    if failed == 0 {
        Ok(summary)
    } else {
        Err(summary)
    }
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--concurrency") {
        let seconds = match args.get(1) {
            Some(value) => match value.parse::<f64>() {
                Ok(value) if value.is_finite() && value > 0.0 => value,
                _ => {
                    eprintln!("audio-capture-probe|FAIL|invalid concurrency timeout {value:?}");
                    std::process::exit(2);
                }
            },
            None => 3.0,
        };
        match run_concurrency(Duration::from_secs_f64(seconds)) {
            Ok(summary) => println!("{summary}"),
            Err(summary) => {
                eprintln!("{summary}");
                std::process::exit(1);
            }
        }
        return;
    }
    let socket = args
        .first()
        .cloned()
        .unwrap_or_else(|| DEFAULT_SOCKET.into());
    let seconds = match args.get(1) {
        Some(value) => match value.parse::<f64>() {
            Ok(value) => value,
            Err(e) => {
                eprintln!("audio-capture-probe|FAIL|invalid duration {value:?}: {e}");
                std::process::exit(2);
            }
        },
        None => DEFAULT_SECONDS,
    };
    match run(&socket, seconds) {
        Ok(summary) => println!("{summary}"),
        Err(e) => {
            eprintln!("audio-capture-probe|FAIL|{e}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{field, pcm_peak};

    #[test]
    fn parses_named_fields() {
        let line = "PCM_OK frames=1024 bytes=4096 gain=1.0";
        assert_eq!(field(line, "frames"), Some("1024"));
        assert_eq!(field(line, "bytes"), Some("4096"));
        assert_eq!(field(line, "missing"), None);
    }

    #[test]
    fn computes_signed_pcm16_peak() {
        let pcm = [0x00, 0x00, 0x01, 0x00, 0x00, 0x80, 0xff, 0x7f];
        assert_eq!(pcm_peak(&pcm), 32_768);
        assert_eq!(pcm_peak(&[0x00, 0x00]), 0);
        assert_eq!(pcm_peak(&[]), 0);
    }
}
