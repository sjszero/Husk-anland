//! Guest-to-app compressed video bridge.
//!
//! The guest owns demux and sends H.264 access units through a memfd ring. This
//! thread owns the only MediaCodec and ImageReader in the process, drives the
//! exact MTK Codec2 decoder requested by the guest, and keeps AHardwareBuffer
//! production on the bionic side.

use super::*;
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use video_protocol::{MappedRing, RingLayout, DEFAULT_SLOTS, DEFAULT_SLOT_SIZE};

const CLIENT_TIMEOUT: Duration = Duration::from_secs(30);
const EOS_WAIT: Duration = Duration::from_secs(20);
const INPUT_WAIT: Duration = Duration::from_secs(3);

static STARTED: AtomicBool = AtomicBool::new(false);

fn sequence_log(files: &Path, line: &str) {
    crate::log(line);
    if let Ok(mut out) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(files.join("video-bridge.result"))
    {
        let _ = writeln!(out, "{line}");
    }
}

fn read_line(stream: &mut UnixStream) -> std::io::Result<String> {
    let mut bytes = Vec::with_capacity(128);
    let mut one = [0u8; 1];
    loop {
        match stream.read(&mut one)? {
            0 => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "peer closed",
                ))
            }
            _ if one[0] == b'\n' => break,
            _ if one[0] != b'\r' => bytes.push(one[0]),
            _ => {}
        }
        if bytes.len() > 16 * 1024 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "control line too long",
            ));
        }
    }
    String::from_utf8(bytes)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "non-utf8 request"))
}

fn send_line(stream: &mut UnixStream, line: &str) -> std::io::Result<()> {
    stream.write_all(line.as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()
}

fn parse_kv(line: &str) -> Vec<(&str, &str)> {
    line.split_whitespace()
        .filter_map(|field| field.split_once('='))
        .collect()
}

fn kv<'a>(fields: &'a [(&str, &str)], key: &str) -> Option<&'a str> {
    fields.iter().find_map(|(k, v)| (*k == key).then_some(*v))
}

fn parse_u32(fields: &[(&str, &str)], key: &str) -> Result<u32, String> {
    kv(fields, key)
        .ok_or_else(|| format!("missing {key}"))?
        .parse::<u32>()
        .map_err(|_| format!("bad {key}"))
}

fn parse_u64(fields: &[(&str, &str)], key: &str) -> Result<u64, String> {
    kv(fields, key)
        .ok_or_else(|| format!("missing {key}"))?
        .parse::<u64>()
        .map_err(|_| format!("bad {key}"))
}

fn parse_i64(fields: &[(&str, &str)], key: &str) -> Result<i64, String> {
    kv(fields, key)
        .ok_or_else(|| format!("missing {key}"))?
        .parse::<i64>()
        .map_err(|_| format!("bad {key}"))
}

struct Session<'a> {
    api: &'a Api,
    codec: *mut Codec,
    codec_started: bool,
    sink: Option<ImageSink>,
    ring: MappedRing,
    next_seq: u64,
    stats: (u64, u64),
    mime: String,
    codec_name: String,
    width: u32,
    height: u32,
}

impl<'a> Session<'a> {
    unsafe fn open(
        api: &'a Api,
        files: &Path,
        line: &str,
    ) -> Result<(Self, std::os::fd::RawFd), String> {
        let fields = parse_kv(line);
        let mime = kv(&fields, "mime")
            .ok_or_else(|| "OPEN missing mime".to_string())?
            .to_string();
        let width = parse_u32(&fields, "width")?;
        let height = parse_u32(&fields, "height")?;
        let wanted = kv(&fields, "codec")
            .ok_or_else(|| "OPEN missing codec".to_string())?
            .to_string();
        if mime != "video/avc" {
            return Err(format!("v1 bridge only supports video/avc, got {mime}"));
        }
        if wanted != "c2.mtk.avc.decoder" {
            return Err(format!(
                "v1 bridge requires c2.mtk.avc.decoder, got {wanted}"
            ));
        }
        if width == 0 || height == 0 || width > 8192 || height > 8192 {
            return Err(format!("bad video size {width}x{height}"));
        }
        let csd0 = match kv(&fields, "csd0") {
            Some(text) => video_protocol::hex_decode(text).map_err(|e| format!("csd0: {e}"))?,
            None => return Err("OPEN missing csd0".into()),
        };
        let csd1 = match kv(&fields, "csd1") {
            Some(text) => video_protocol::hex_decode(text).map_err(|e| format!("csd1: {e}"))?,
            None => return Err("OPEN missing csd1".into()),
        };

        let layout = RingLayout::new(DEFAULT_SLOTS, DEFAULT_SLOT_SIZE)
            .map_err(|e| format!("ring layout: {e}"))?;
        let (ring, ring_fd) =
            MappedRing::create(layout).map_err(|e| format!("create ring: {e}"))?;

        let wanted_z = CString::new(wanted.as_str()).expect("validated codec name");
        let codec = (api.create_by_name)(wanted_z.as_ptr());
        if codec.is_null() {
            video_protocol::close_fd(ring_fd);
            return Err(format!("AMediaCodec_createCodecByName({wanted}) failed"));
        }

        let mut cleanup_codec = true;
        let mut cleanup_started = false;
        let mut sink: Option<ImageSink> = None;
        let outcome = (|| -> Result<(), String> {
            let actual = codec_name(api, codec);
            sequence_log(
                files,
                &format!("video|bridge|codec|requested={wanted}|actual={actual}"),
            );
            if actual != wanted {
                return Err(format!(
                    "hardware decoder mismatch: requested={wanted} actual={actual}"
                ));
            }

            let image_api = ImageApi::load()?;
            let active = image_api.new_sink(width as i32, height as i32)?;
            let mut window = core::ptr::null_mut();
            let rc = (active.api.reader_get_window)(active.reader, &mut window);
            if rc != AMEDIA_OK || window.is_null() {
                (active.api.reader_delete)(active.reader);
                return Err(format!("AImageReader_getWindow rc={rc} window={window:p}"));
            }

            let format = (api.format_new)();
            if format.is_null() {
                (active.api.reader_delete)(active.reader);
                return Err("AMediaFormat_new failed".into());
            }
            let mime_z = CString::new(mime.as_str()).map_err(|e| format!("mime: {e}"))?;
            let max_input = layout.slot_size as i32;
            (api.format_set_string)(format, key(c"mime"), mime_z.as_ptr());
            (api.format_set_i32)(format, key(c"width"), width as i32);
            (api.format_set_i32)(format, key(c"height"), height as i32);
            (api.format_set_i32)(format, key(c"max-input-size"), max_input);
            (api.format_set_buffer)(
                format,
                key(c"csd-0"),
                csd0.as_ptr() as *const c_void,
                csd0.len(),
            );
            (api.format_set_buffer)(
                format,
                key(c"csd-1"),
                csd1.as_ptr() as *const c_void,
                csd1.len(),
            );
            let rc = (api.configure)(codec, format, window, core::ptr::null_mut(), 0);
            let _ = (api.format_delete)(format);
            if rc != AMEDIA_OK {
                (active.api.reader_delete)(active.reader);
                return Err(format!("configure rc={rc}"));
            }
            let rc = (api.start)(codec);
            if rc != AMEDIA_OK {
                (active.api.reader_delete)(active.reader);
                return Err(format!("start rc={rc}"));
            }
            cleanup_started = true;
            sink = Some(active);
            cleanup_codec = false;
            Ok(())
        })();

        if let Err(e) = outcome {
            if !codec.is_null() && cleanup_codec {
                if cleanup_started {
                    let _ = (api.stop)(codec);
                }
                let _ = (api.codec_delete)(codec);
            }
            video_protocol::close_fd(ring_fd);
            return Err(e);
        }

        Ok((
            Self {
                api,
                codec,
                codec_started: true,
                sink,
                ring,
                next_seq: 1,
                stats: (0, 0),
                mime,
                codec_name: wanted,
                width,
                height,
            },
            ring_fd,
        ))
    }

    unsafe fn feed(&mut self, files: &Path, line: &str) -> Result<String, String> {
        let fields = parse_kv(line);
        let seq = parse_u64(&fields, "seq")?;
        let expected_len = parse_u32(&fields, "len")? as usize;
        let expected_pts = parse_i64(&fields, "pts_us")?;
        let expected_flags = parse_u32(&fields, "flags")?;
        if seq != self.next_seq {
            return Err(format!("sample seq={seq} expected={}", self.next_seq));
        }

        let mut payload = Vec::new();
        let meta = self
            .ring
            .read_into(seq, &mut payload)
            .map_err(|e| format!("ring read seq={seq}: {e}"))?;
        if meta.len as usize != expected_len
            || meta.pts_us != expected_pts
            || meta.flags != expected_flags
        {
            return Err(format!(
                "sample metadata mismatch: ring={:?} message len={expected_len} pts={expected_pts} flags={expected_flags}",
                meta
            ));
        }

        let started = Instant::now();
        let input_index = loop {
            let index = (self.api.dequeue_input)(self.codec, 100_000);
            if index >= 0 {
                break index;
            }
            if index != INFO_TRY_AGAIN_LATER {
                return Err(format!("dequeueInputBuffer seq={seq} rc={index}"));
            }
            let _ = drain_outputs(
                self.api,
                self.codec,
                files,
                0,
                &mut self.stats,
                &mut self.sink,
                "bridge",
            )?;
            if started.elapsed() >= INPUT_WAIT {
                return Err(format!("input buffer timeout seq={seq}"));
            }
        };

        let mut capacity = 0usize;
        let dst = (self.api.get_input_buffer)(self.codec, input_index as usize, &mut capacity);
        if dst.is_null() || payload.len() > capacity {
            return Err(format!(
                "input capacity seq={seq} len={} capacity={capacity}",
                payload.len()
            ));
        }
        core::ptr::copy_nonoverlapping(payload.as_ptr(), dst, payload.len());
        let rc = (self.api.queue_input)(
            self.codec,
            input_index as usize,
            0,
            payload.len(),
            meta.pts_us as u64,
            meta.flags,
        );
        if rc != AMEDIA_OK {
            return Err(format!("queueInputBuffer seq={seq} rc={rc}"));
        }
        self.ring
            .commit(seq)
            .map_err(|e| format!("ring commit seq={seq}: {e}"))?;
        self.next_seq += 1;

        let _ = drain_outputs(
            self.api,
            self.codec,
            files,
            0,
            &mut self.stats,
            &mut self.sink,
            "bridge",
        )?;
        let images = self.sink.as_ref().map(|s| s.seen).unwrap_or(0);
        Ok(format!(
            "SAMPLE_OK seq={seq} frames={} images={images}",
            self.stats.0
        ))
    }

    unsafe fn eos(&mut self, files: &Path) -> Result<String, String> {
        let input_index = loop {
            let index = (self.api.dequeue_input)(self.codec, 100_000);
            if index >= 0 {
                break index;
            }
            if index != INFO_TRY_AGAIN_LATER {
                return Err(format!("dequeueInputBuffer(EOS) rc={index}"));
            }
            let _ = drain_outputs(
                self.api,
                self.codec,
                files,
                0,
                &mut self.stats,
                &mut self.sink,
                "bridge",
            )?;
        };
        let rc = (self.api.queue_input)(
            self.codec,
            input_index as usize,
            0,
            0,
            0,
            BUFFER_FLAG_END_OF_STREAM,
        );
        if rc != AMEDIA_OK {
            return Err(format!("queueInputBuffer(EOS) rc={rc}"));
        }
        let deadline = Instant::now() + EOS_WAIT;
        loop {
            if drain_outputs(
                self.api,
                self.codec,
                files,
                100_000,
                &mut self.stats,
                &mut self.sink,
                "bridge",
            )? {
                break;
            }
            if Instant::now() >= deadline {
                return Err("decoder did not signal output EOS".into());
            }
        }
        let images = self.sink.as_ref().map(|s| s.seen).unwrap_or(0);
        if self.stats.0 == 0 || images == 0 {
            return Err(format!(
                "EOS with no decoded output: frames={} images={images}",
                self.stats.0
            ));
        }
        Ok(format!(
            "EOS_OK frames={} images={images} output_bytes={}",
            self.stats.0, self.stats.1
        ))
    }

    unsafe fn close(&mut self) -> String {
        let mut stopped = 0;
        let mut deleted = 0;
        if !self.codec.is_null() {
            if self.codec_started {
                stopped = (self.api.stop)(self.codec);
            }
            deleted = (self.api.codec_delete)(self.codec);
            self.codec = core::ptr::null_mut();
            self.codec_started = false;
        }
        let images = self.sink.as_ref().map(|sink| sink.seen).unwrap_or(0);
        if let Some(sink) = self.sink.take() {
            (sink.api.reader_delete)(sink.reader);
        }
        format!(
            "CLOSE_OK mime={} codec={} size={}x{} frames={} output_bytes={} images_last={images} stop_rc={stopped} delete_rc={deleted}",
            self.mime,
            self.codec_name,
            self.width,
            self.height,
            self.stats.0,
            self.stats.1
        )
    }
}

impl Drop for Session<'_> {
    fn drop(&mut self) {
        if !self.codec.is_null() {
            unsafe {
                if self.codec_started {
                    let _ = (self.api.stop)(self.codec);
                }
                let _ = (self.api.codec_delete)(self.codec);
            }
            self.codec = core::ptr::null_mut();
        }
        if let Some(sink) = self.sink.take() {
            unsafe { (sink.api.reader_delete)(sink.reader) };
        }
    }
}

unsafe fn handle_client(files: &Path, api: &Api, mut stream: UnixStream) {
    let _ = stream.set_read_timeout(Some(CLIENT_TIMEOUT));
    let _ = stream.set_write_timeout(Some(CLIENT_TIMEOUT));
    sequence_log(
        files,
        &format!("video|bridge|client|connected|fd={}", stream.as_raw_fd()),
    );
    let mut session: Option<Session<'_>> = None;
    loop {
        let line = match read_line(&mut stream) {
            Ok(line) => line,
            Err(e) => {
                sequence_log(files, &format!("video|bridge|client|read|{e}"));
                break;
            }
        };
        let cmd = line
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_ascii_uppercase();
        let reply = match cmd.as_str() {
            "PING" => send_line(&mut stream, "PONG bridge=video-v1"),
            "OPEN" => {
                if session.is_some() {
                    send_line(&mut stream, "OPEN_ERROR session already open")
                } else {
                    match Session::open(api, files, &line) {
                        Ok((active, ring_fd)) => {
                            let open_line = format!(
                                "OPEN_READY slots={} slot_size={} bytes={} codec={}",
                                DEFAULT_SLOTS,
                                DEFAULT_SLOT_SIZE,
                                RingLayout::new(DEFAULT_SLOTS, DEFAULT_SLOT_SIZE)
                                    .expect("static ring layout")
                                    .bytes,
                                active.codec_name
                            );
                            let sent = send_line(&mut stream, &open_line).and_then(|_| {
                                video_protocol::send_fd(stream.as_raw_fd(), ring_fd)
                                    .map_err(|e| std::io::Error::new(e.kind(), e))
                            });
                            video_protocol::close_fd(ring_fd);
                            if let Err(e) = sent {
                                sequence_log(files, &format!("video|bridge|OPEN_SEND|{e}"));
                                let _ =
                                    send_line(&mut stream, &format!("OPEN_ERROR send ring: {e}"));
                            } else {
                                sequence_log(files, &format!("video|bridge|OPEN|{open_line}"));
                                session = Some(active);
                            }
                            Ok(())
                        }
                        Err(e) => {
                            sequence_log(files, &format!("video|bridge|OPEN_FAIL|{e}"));
                            send_line(&mut stream, &format!("OPEN_ERROR {e}"))
                        }
                    }
                }
            }
            "SAMPLE" => match session.as_mut() {
                Some(active) => match active.feed(files, &line) {
                    Ok(ok) => send_line(&mut stream, &ok),
                    Err(e) => {
                        sequence_log(files, &format!("video|bridge|SAMPLE_FAIL|{e}"));
                        send_line(&mut stream, &format!("SAMPLE_ERROR {e}"))
                    }
                },
                None => send_line(&mut stream, "SAMPLE_ERROR no open session"),
            },
            "EOS" => match session.as_mut() {
                Some(active) => match active.eos(files) {
                    Ok(ok) => {
                        sequence_log(files, &format!("video|bridge|{ok}"));
                        send_line(&mut stream, &ok)
                    }
                    Err(e) => {
                        sequence_log(files, &format!("video|bridge|EOS_FAIL|{e}"));
                        send_line(&mut stream, &format!("EOS_ERROR {e}"))
                    }
                },
                None => send_line(&mut stream, "EOS_ERROR no open session"),
            },
            "CLOSE" => match session.take() {
                Some(mut active) => {
                    let summary = active.close();
                    sequence_log(files, &format!("video|bridge|{summary}"));
                    send_line(&mut stream, &summary)
                }
                None => send_line(&mut stream, "CLOSE_OK no-open-session"),
            },
            "QUIT" => {
                if let Some(mut active) = session.take() {
                    let summary = active.close();
                    sequence_log(files, &format!("video|bridge|{summary}"));
                }
                let _ = send_line(&mut stream, "BYE");
                break;
            }
            _ => send_line(&mut stream, "ERROR unknown-command"),
        };
        if let Err(e) = reply {
            sequence_log(files, &format!("video|bridge|client|write|{e}"));
            break;
        }
    }
    if let Some(mut active) = session.take() {
        let summary = active.close();
        sequence_log(files, &format!("video|bridge|disconnect|{summary}"));
    }
    sequence_log(files, "video|bridge|client|disconnected");
}

fn serve() {
    let Some(files) = files_dir() else {
        crate::log("video|bridge|SKIP|no files dir");
        return;
    };
    let Some(api) = (unsafe { Api::load().ok() }) else {
        crate::log("video|bridge|FAIL|MediaCodec symbols unavailable");
        return;
    };
    sequence_log(&files, "video|bridge|API|libmediandk.so");
    let marker = files.join("video-bridge");
    let guest_marker = files.join("rootfs-full/run/husk/enabled");
    let enabled = || marker.exists() || guest_marker.exists();
    let mut announced_wait = false;
    loop {
        if !enabled() {
            if !announced_wait {
                sequence_log(
                    &files,
                    &format!(
                        "video|bridge|WAIT|{} or {}",
                        marker.display(),
                        guest_marker.display()
                    ),
                );
                announced_wait = true;
            }
            std::thread::sleep(Duration::from_millis(500));
            continue;
        }
        announced_wait = false;
        // Keep the control socket inside the guest-visible rootfs. If the
        // socket stayed under `files/`, a guest rooted at `rootfs-full` could
        // only reach it by disabling root isolation (`NR_ROOTFS=/`), which is
        // not a valid product path.
        let path = files.join("rootfs-full/run/husk/video.sock");
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                sequence_log(&files, &format!("video|bridge|FAIL|mkdir {e}"));
                std::thread::sleep(Duration::from_secs(1));
                continue;
            }
        }
        let _ = std::fs::remove_file(&path);
        let listener = match UnixListener::bind(&path) {
            Ok(listener) => listener,
            Err(e) => {
                sequence_log(&files, &format!("video|bridge|FAIL|bind {e}"));
                std::thread::sleep(Duration::from_secs(1));
                continue;
            }
        };
        let _ = listener.set_nonblocking(true);
        sequence_log(&files, &format!("video|bridge|LISTEN|{}", path.display()));
        while enabled() {
            match listener.accept() {
                Ok((stream, _)) => unsafe { handle_client(&files, &api, stream) },
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(e) => {
                    sequence_log(&files, &format!("video|bridge|FAIL|accept {e}"));
                    break;
                }
            }
        }
        let _ = std::fs::remove_file(&path);
        sequence_log(&files, "video|bridge|STOP|marker removed");
    }
}

pub(super) fn start() {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| {
        let _ = std::panic::catch_unwind(serve);
    });
}
