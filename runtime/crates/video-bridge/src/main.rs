//! Guest-side compressed-video sender for the Husk video bridge.
//!
//! This first proof demuxes an MP4 with a pure-Rust reader, extracts one H.264
//! track, and sends each access unit through the shared ring. The app side owns
//! the bionic MediaCodec. The guest keeps demux ownership and never loads a
//! bionic vendor codec library.

use mp4::{MediaType, Mp4Reader, Mp4Sample, TrackType};
use std::fs::File;
use std::io::{BufReader, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};
use video_protocol::{hex_encode, recv_fd, MappedRing, RingLayout};

const START_CODE: [u8; 4] = [0, 0, 0, 1];
const BUFFER_FLAG_KEY_FRAME: u32 = 1;

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
        if bytes.len() > 16 * 1024 {
            return Err("control line too long".into());
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

fn parse_kv(line: &str) -> Vec<(&str, &str)> {
    line.split_whitespace()
        .filter_map(|field| field.split_once('='))
        .collect()
}

fn kv<'a>(fields: &'a [(&str, &str)], key: &str) -> Option<&'a str> {
    fields.iter().find_map(|(k, v)| (*k == key).then_some(*v))
}

fn parse_usize(fields: &[(&str, &str)], key: &str) -> Result<usize, String> {
    kv(fields, key)
        .ok_or_else(|| format!("missing {key}"))?
        .parse::<usize>()
        .map_err(|_| format!("bad {key}"))
}

fn connect_with_retry(path: &str) -> Result<UnixStream, String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match UnixStream::connect(path) {
            Ok(stream) => return Ok(stream),
            Err(e) if Instant::now() < deadline => {
                let _ = e;
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => return Err(format!("connect {path}: {e}")),
        }
    }
}

fn find_avc_track(mp4: &Mp4Reader<BufReader<File>>) -> Result<u32, String> {
    for track in mp4.tracks().values() {
        if track.track_type().map_err(|e| format!("track type: {e}"))? == TrackType::Video
            && track.media_type().map_err(|e| format!("media type: {e}"))? == MediaType::H264
        {
            return Ok(track.track_id());
        }
    }
    Err("no H.264 video track".into())
}

fn avc_csd(track: &mp4::Mp4Track) -> Result<(Vec<u8>, Vec<u8>, usize), String> {
    let avc1 = track
        .trak
        .mdia
        .minf
        .stbl
        .stsd
        .avc1
        .as_ref()
        .ok_or_else(|| "AVC track has no avc1 sample entry".to_string())?;
    let sps = track
        .sequence_parameter_set()
        .map_err(|e| format!("SPS: {e}"))?;
    let pps = track
        .picture_parameter_set()
        .map_err(|e| format!("PPS: {e}"))?;
    let mut csd0 = Vec::with_capacity(4 + sps.len());
    csd0.extend_from_slice(&START_CODE);
    csd0.extend_from_slice(sps);
    let mut csd1 = Vec::with_capacity(4 + pps.len());
    csd1.extend_from_slice(&START_CODE);
    csd1.extend_from_slice(pps);
    Ok((csd0, csd1, avc1.avcc.length_size_minus_one as usize + 1))
}

fn avcc_to_annexb(bytes: &[u8], length_size: usize) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(bytes.len() + 32);
    let mut offset = 0usize;
    while offset < bytes.len() {
        if offset + length_size > bytes.len() {
            return Err("truncated AVC length prefix".into());
        }
        let mut len = 0usize;
        for &b in &bytes[offset..offset + length_size] {
            len = (len << 8) | b as usize;
        }
        offset += length_size;
        if len == 0 || offset + len > bytes.len() {
            return Err(format!("bad AVC NAL length={len} at {offset}"));
        }
        out.extend_from_slice(&START_CODE);
        out.extend_from_slice(&bytes[offset..offset + len]);
        offset += len;
    }
    Ok(out)
}

fn sample_pts_us(sample: &Mp4Sample, timescale: u32) -> i64 {
    let ticks = sample.start_time as i128 + sample.rendering_offset as i128;
    let value = ticks.saturating_mul(1_000_000) / timescale.max(1) as i128;
    value.clamp(i64::MIN as i128, i64::MAX as i128) as i64
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        eprintln!("usage: video-bridge <socket> <file.mp4> [max_frames]");
        std::process::exit(2);
    }
    let socket = &args[0];
    let media = &args[1];
    let max_frames = args
        .get(2)
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(u32::MAX);

    let outcome = (|| -> Result<(), String> {
        let file = File::open(media).map_err(|e| format!("open {media}: {e}"))?;
        let size = file
            .metadata()
            .map_err(|e| format!("stat {media}: {e}"))?
            .len();
        let mut mp4 = Mp4Reader::read_header(BufReader::new(file), size)
            .map_err(|e| format!("read MP4: {e}"))?;
        let track_id = find_avc_track(&mp4)?;
        let (csd0, csd1, length_size) = {
            let track = mp4
                .tracks()
                .get(&track_id)
                .ok_or_else(|| format!("track {track_id} missing"))?;
            avc_csd(track)?
        };
        let (width, height, timescale, sample_count) = {
            let track = mp4
                .tracks()
                .get(&track_id)
                .ok_or_else(|| format!("track {track_id} missing"))?;
            (
                track.width(),
                track.height(),
                track.timescale(),
                track.sample_count(),
            )
        };
        let wanted = sample_count.min(max_frames);
        println!(
            "guest|OPEN|media={media}|track={track_id}|size={width}x{height}|timescale={timescale}|samples={sample_count}|send={wanted}|length_size={length_size}"
        );
        println!(
            "guest|CSD|csd0={}|csd1={}",
            hex_encode(&csd0),
            hex_encode(&csd1)
        );

        let mut stream = connect_with_retry(socket)?;
        stream
            .set_read_timeout(Some(Duration::from_secs(30)))
            .map_err(|e| format!("set read timeout: {e}"))?;
        stream
            .set_write_timeout(Some(Duration::from_secs(30)))
            .map_err(|e| format!("set write timeout: {e}"))?;

        send_line(&mut stream, "PING")?;
        let pong = read_line(&mut stream)?;
        if !pong.starts_with("PONG ") {
            return Err(format!("bad PING: {pong}"));
        }
        println!("{pong}");

        send_line(
            &mut stream,
            &format!(
                "OPEN mime=video/avc width={width} height={height} codec=c2.mtk.avc.decoder csd0={} csd1={}",
                hex_encode(&csd0),
                hex_encode(&csd1)
            ),
        )?;
        let ready = read_line(&mut stream)?;
        println!("{ready}");
        if !ready.starts_with("OPEN_READY ") {
            return Err(format!("OPEN rejected: {ready}"));
        }
        let fields = parse_kv(&ready);
        let slots = parse_usize(&fields, "slots")? as u32;
        let slot_size = parse_usize(&fields, "slot_size")? as u32;
        let bytes = parse_usize(&fields, "bytes")?;
        let layout = RingLayout::new(slots, slot_size).map_err(|e| format!("ring layout: {e}"))?;
        if bytes != layout.bytes {
            return Err(format!(
                "OPEN_READY bytes={bytes}, local layout={}",
                layout.bytes
            ));
        }
        let ring_fd = recv_fd(stream.as_raw_fd()).map_err(|e| format!("recv ring fd: {e}"))?;
        let ring = MappedRing::from_fd(ring_fd, layout)
            .map_err(|e| format!("map ring fd={ring_fd}: {e}"))?;
        video_protocol::close_fd(ring_fd);
        println!(
            "guest|RING|READY|slots={}|slot_size={}|bytes={}",
            ring.slots(),
            ring.slot_size(),
            layout.bytes
        );

        let started = Instant::now();
        let mut sent = 0u32;
        let mut bytes_sent = 0u64;
        for sample_id in 1..=wanted {
            let sample = mp4
                .read_sample(track_id, sample_id)
                .map_err(|e| format!("read sample {sample_id}: {e}"))?
                .ok_or_else(|| format!("sample {sample_id} missing"))?;
            let payload = avcc_to_annexb(&sample.bytes, length_size)?;
            if sent == 0 {
                let mut head = String::new();
                for byte in payload.iter().take(64) {
                    head.push_str(&format!("{byte:02x}"));
                }
                println!("guest|SAMPLE_PREFIX|{head}");
            }
            let pts = sample_pts_us(&sample, timescale);
            let flags = if sample.is_sync {
                BUFFER_FLAG_KEY_FRAME
            } else {
                0
            };
            let seq = sample_id as u64;
            loop {
                match ring.write_sample(seq, pts, pts, flags, &payload) {
                    Ok(()) => break,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    Err(e) => return Err(format!("ring write seq={seq}: {e}")),
                }
            }
            send_line(
                &mut stream,
                &format!(
                    "SAMPLE seq={seq} len={} pts_us={pts} flags={flags}",
                    payload.len()
                ),
            )?;
            let ack = read_line(&mut stream)?;
            if !ack.starts_with(&format!("SAMPLE_OK seq={seq} ")) {
                return Err(format!("sample {seq} rejected: {ack}"));
            }
            sent += 1;
            bytes_sent = bytes_sent.saturating_add(payload.len() as u64);
            if sent <= 3 || sent.is_multiple_of(30) || sent == wanted {
                println!(
                    "guest|SAMPLE|seq={seq}|sent={sent}/{wanted}|len={}|pts_us={pts}|key={}|ack={ack}",
                    payload.len(),
                    sample.is_sync
                );
            }
        }

        send_line(&mut stream, "EOS")?;
        let eos = read_line(&mut stream)?;
        println!("{eos}");
        if !eos.starts_with("EOS_OK ") {
            return Err(format!("EOS rejected: {eos}"));
        }
        send_line(&mut stream, "CLOSE")?;
        let close = read_line(&mut stream)?;
        println!("{close}");
        if !close.starts_with("CLOSE_OK ") {
            return Err(format!("CLOSE rejected: {close}"));
        }
        send_line(&mut stream, "QUIT")?;
        let bye = read_line(&mut stream)?;
        println!("{bye}");
        println!(
            "guest|DONE|samples={sent}|bytes={bytes_sent}|elapsed_ms={}",
            started.elapsed().as_millis()
        );
        Ok(())
    })();

    match outcome {
        Ok(()) => println!("RESULT PASS"),
        Err(e) => {
            eprintln!("guest|FAIL|{e}");
            println!("RESULT FAIL");
            std::process::exit(1);
        }
    }
}
