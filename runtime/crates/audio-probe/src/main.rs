//! Guest-side PCM probe for the Husk AAudio bridge.
//!
//! This binary runs inside the glibc guest. It enumerates output devices,
//! exercises Android music-volume control through the app bridge, sends a
//! deterministic PCM16 sine over a dedicated Unix socket, and prints the
//! bionic-side AAudio metrics returned by the app.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::time::Instant;

fn read_line(stream: &mut UnixStream) -> Result<String, String> {
    let mut bytes = Vec::with_capacity(128);
    let mut one = [0u8; 1];
    loop {
        match stream.read(&mut one).map_err(|e| format!("read: {e}"))? {
            0 => return Err("peer closed while reading line".into()),
            _ if one[0] == b'\n' => break,
            _ if one[0] != b'\r' => bytes.push(one[0]),
            _ => {}
        }
        if bytes.len() > 4096 {
            return Err("line too long".into());
        }
    }
    String::from_utf8(bytes).map_err(|_| "non-utf8 response".into())
}

fn send_line(stream: &mut UnixStream, line: &str) -> Result<(), String> {
    stream
        .write_all(line.as_bytes())
        .and_then(|_| stream.write_all(b"\n"))
        .and_then(|_| stream.flush())
        .map_err(|e| format!("send {line:?}: {e}"))
}

fn request(stream: &mut UnixStream, line: &str) -> Result<String, String> {
    send_line(stream, line)?;
    read_line(stream)
}

fn parse_kv(line: &str) -> Vec<(&str, &str)> {
    line.split_whitespace()
        .filter_map(|field| field.split_once('='))
        .collect()
}

fn kv<'a>(fields: &'a [(&str, &str)], key: &str) -> Option<&'a str> {
    fields.iter().find_map(|(k, v)| (*k == key).then_some(*v))
}

fn sine_pcm16(rate: u32, channels: u16, frames: usize, hz: f64, amplitude: f64) -> Vec<u8> {
    let mut pcm = Vec::with_capacity(frames * channels as usize * 2);
    let scale = i16::MAX as f64 * amplitude.clamp(0.0, 1.0);
    for n in 0..frames {
        let sample = (std::f64::consts::TAU * hz * n as f64 / rate as f64).sin();
        let value = (sample * scale) as i16;
        for _ in 0..channels {
            pcm.extend_from_slice(&value.to_le_bytes());
        }
    }
    pcm
}

fn devices(stream: &mut UnixStream) -> Result<(), String> {
    send_line(stream, "DEVICES")?;
    let begin = read_line(stream)?;
    println!("{begin}");
    if begin.starts_with("DEVICES_ERROR") {
        return Err(begin);
    }
    let count = parse_kv(&begin)
        .iter()
        .find_map(|(k, v)| (*k == "count").then_some(*v))
        .and_then(|v| v.parse::<usize>().ok())
        .ok_or_else(|| format!("bad DEVICES_BEGIN: {begin}"))?;
    for _ in 0..count {
        let line = read_line(stream)?;
        println!("{line}");
    }
    let end = read_line(stream)?;
    println!("{end}");
    if !end.starts_with("DEVICES_END") {
        return Err(format!("bad DEVICES_END: {end}"));
    }
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let socket = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| "/data/user/0/com.linuxhub.myth.husk/files/audio-bridge.sock".into());
    let seconds = args
        .get(2)
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(2)
        .clamp(1, 30);
    let rate = 48_000u32;
    let channels = 1u16;
    let frames = rate as usize * seconds as usize;

    let result = (|| -> Result<(), String> {
        let mut stream =
            UnixStream::connect(&socket).map_err(|e| format!("connect {socket}: {e}"))?;
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(10)))
            .map_err(|e| format!("set read timeout: {e}"))?;
        stream
            .set_write_timeout(Some(std::time::Duration::from_secs(10)))
            .map_err(|e| format!("set write timeout: {e}"))?;

        let pong = request(&mut stream, "PING")?;
        println!("{pong}");
        if !pong.starts_with("PONG") {
            return Err(format!("bad PING response: {pong}"));
        }
        devices(&mut stream)?;

        let before = request(&mut stream, "VOLUME GET")?;
        println!("{before}");
        let before_fields = parse_kv(&before);
        let original_index = kv(&before_fields, "index")
            .ok_or_else(|| format!("VOLUME GET missing index: {before}"))?
            .parse::<i32>()
            .map_err(|_| format!("bad original volume index: {before}"))?;
        let set_low = request(&mut stream, "VOLUME SET 30")?;
        println!("{set_low}");
        let verify_low = request(&mut stream, "VOLUME GET")?;
        println!("{verify_low}");
        if !set_low.starts_with("VOLUME OK") || !verify_low.starts_with("VOLUME OK") {
            return Err(format!("volume set failed: {set_low} / {verify_low}"));
        }

        let pcm = sine_pcm16(rate, channels, frames, 440.0, 0.18);
        println!(
            "PCM bytes={} frames={} rate={} channels={} hz=440 amplitude=0.18",
            pcm.len(),
            frames,
            rate,
            channels
        );
        send_line(
            &mut stream,
            &format!("STREAM frames={frames} rate={rate} channels={channels}"),
        )?;
        let ready = read_line(&mut stream)?;
        println!("{ready}");
        if !ready.starts_with("STREAM_READY") {
            return Err(format!("stream open failed: {ready}"));
        }
        let started = Instant::now();
        for (chunk_index, chunk) in pcm.chunks(16 * 1024).enumerate() {
            stream
                .write_all(chunk)
                .map_err(|e| format!("PCM chunk {chunk_index}: {e}"))?;
        }
        stream.flush().map_err(|e| format!("PCM flush: {e}"))?;
        let done = read_line(&mut stream)?;
        println!("{done}");
        println!("guest wall time ms={}", started.elapsed().as_millis());
        if !done.starts_with("DONE status=PASS") {
            return Err(format!("bridge did not pass: {done}"));
        }

        let restore = request(&mut stream, &format!("VOLUME INDEX {original_index}"))?;
        println!("{restore}");
        if !restore.starts_with("VOLUME OK") {
            return Err(format!("volume restore failed: {restore}"));
        }
        let bye = request(&mut stream, "QUIT")?;
        println!("{bye}");
        Ok(())
    })();

    match result {
        Ok(()) => println!("RESULT PASS"),
        Err(e) => {
            eprintln!("RESULT FAIL: {e}");
            std::process::exit(1);
        }
    }
}
