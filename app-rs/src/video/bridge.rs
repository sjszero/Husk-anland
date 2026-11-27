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
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use video_protocol::{MappedRing, RingLayout, DEFAULT_SLOTS, DEFAULT_SLOT_SIZE};

const CLIENT_TIMEOUT: Duration = Duration::from_secs(30);
const EOS_WAIT: Duration = Duration::from_secs(20);
const INPUT_WAIT: Duration = Duration::from_secs(3);
/// How long a CPU-copy session waits for the decoder to hand back one frame
/// after an input sample. The decoder is asynchronous; the guest asked for the
/// pixels of this picture, so the wait is bounded and reported in the log.
const FRAME_WAIT: Duration = Duration::from_secs(5);

static STARTED: AtomicBool = AtomicBool::new(false);

/// Only one decoder session may exist at a time.
///
/// The presenter, the event bridge and the single Codec2 instance are all
/// process-wide singletons, so a second concurrent `OPEN` used to succeed and
/// then produce nothing: `FRAME` returned NONE forever while the client saw a
/// healthy socket. Refusing it with a named error is the honest behaviour -
/// and it is what a guest tunnel through one surface can actually support.
static SESSION_ACTIVE: AtomicBool = AtomicBool::new(false);
/// Wall-clock ms of the last command on the active session. A session whose
/// thread is wedged inside MediaCodec stops refreshing this, and refusing new
/// clients forever after one stuck thread is worse than taking the session
/// over - measured: one deadlocked client made every later `OPEN` fail.
static SESSION_ACTIVITY_MS: AtomicU64 = AtomicU64::new(0);
const SESSION_STALE_MS: u64 = 45_000;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Claim the single session slot. Returns false when a live session holds it.
fn claim_session() -> bool {
    if SESSION_ACTIVE
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
    {
        SESSION_ACTIVITY_MS.store(now_ms(), Ordering::SeqCst);
        return true;
    }
    let last = SESSION_ACTIVITY_MS.load(Ordering::SeqCst);
    if last != 0 && now_ms().saturating_sub(last) > SESSION_STALE_MS {
        // The holder has not touched the socket for longer than any legal
        // wait, so it is dead or wedged; reclaim the slot.
        SESSION_ACTIVE.store(false, Ordering::SeqCst);
        if SESSION_ACTIVE
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            SESSION_ACTIVITY_MS.store(now_ms(), Ordering::SeqCst);
            return true;
        }
    }
    false
}

fn release_session() {
    SESSION_ACTIVITY_MS.store(0, Ordering::SeqCst);
    SESSION_ACTIVE.store(false, Ordering::SeqCst);
}

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

struct OutFormat {
    color: i32,
    stride: i32,
    slice: i32,
    width: i32,
    height: i32,
}

struct Frame {
    y: Vec<u8>,
    u: Vec<u8>,
    v: Vec<u8>,
    width: u32,
    height: u32,
    color: i32,
}

struct PendingFrame {
    pts_us: i64,
    nv12: Vec<u8>,
    width: u32,
    height: u32,
}

struct Session<'a> {
    api: &'a Api,
    codec: *mut Codec,
    codec_started: bool,
    sink: Option<ImageSink>,
    fixed: Option<fixed_ring::FixedRing>,
    /// Decode to ByteBuffer and hand the pixels back over the control socket.
    /// This is the path the VA-API system layer needs: it must own the pixels.
    cpu: bool,
    out: Option<OutFormat>,
    /// Decoded frames waiting for the guest to claim them, keyed by the PTS the
    /// guest sent in. Bounded: an unclaimed frame is dropped and counted, never
    /// silently reused.
    pending: std::collections::VecDeque<PendingFrame>,
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
        let mut fixed: Option<fixed_ring::FixedRing> = None;
        // `video-cpu` wins over `video-fixed`: the VA-API system layer has to
        // own the pixels, so it needs them on the control socket, not in an AHB.
        let cpu_mode = files.join("video-cpu").exists();
        let fixed_mode = !cpu_mode && files.join("video-fixed").exists();
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
            let mut active: Option<ImageSink> = None;
            let output_surface = if cpu_mode {
                sequence_log(files, "video|bridge|output|cpu-i420-over-socket");
                core::ptr::null_mut()
            } else if fixed_mode {
                sequence_log(files, "video|bridge|output|fixed-bytebuffer-yv12-ring");
                fixed = Some(fixed_ring::FixedRing::new(width, height, files)?);
                core::ptr::null_mut()
            } else {
                let reader = image_api.new_sink(width as i32, height as i32)?;
                let mut window = core::ptr::null_mut();
                let rc = (reader.api.reader_get_window)(reader.reader, &mut window);
                if rc != AMEDIA_OK || window.is_null() {
                    (reader.api.reader_delete)(reader.reader);
                    return Err(format!("AImageReader_getWindow rc={rc} window={window:p}"));
                }
                active = Some(reader);
                window
            };

            let format = (api.format_new)();
            if format.is_null() {
                if let Some(reader) = active.take() {
                    (reader.api.reader_delete)(reader.reader);
                }
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
            let rc = (api.configure)(codec, format, output_surface, core::ptr::null_mut(), 0);
            let _ = (api.format_delete)(format);
            if rc != AMEDIA_OK {
                if let Some(reader) = active.take() {
                    (reader.api.reader_delete)(reader.reader);
                }
                return Err(format!("configure rc={rc}"));
            }
            let rc = (api.start)(codec);
            if rc != AMEDIA_OK {
                if let Some(reader) = active.take() {
                    (reader.api.reader_delete)(reader.reader);
                }
                return Err(format!("start rc={rc}"));
            }
            cleanup_started = true;
            sink = active;
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
                fixed,
                cpu: cpu_mode,
                out: None,
                pending: std::collections::VecDeque::new(),
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

    unsafe fn drain(&mut self, files: &Path, timeout_us: i64) -> Result<bool, String> {
        if let Some(fixed) = self.fixed.as_mut() {
            fixed_ring::drain_outputs(
                self.api,
                self.codec,
                files,
                timeout_us,
                &mut self.stats,
                fixed,
                "bridge",
            )
        } else {
            drain_outputs(
                self.api,
                self.codec,
                files,
                timeout_us,
                &mut self.stats,
                &mut self.sink,
                "bridge",
            )
        }
    }

    unsafe fn feed(
        &mut self,
        files: &Path,
        line: &str,
    ) -> Result<(String, Option<Vec<u8>>), String> {
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
            let _ = self.drain(files, 0)?;
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

        if self.cpu {
            // Decoding is asynchronous: a codec may need several input samples
            // before it emits the first picture, and B-frame reordering means
            // output order is not input order. So the sample is acknowledged as
            // soon as it is queued, and the guest claims pixels later by PTS.
            self.drain_frames(files, 0)?;
            return Ok((
                format!(
                    "SAMPLE_OK seq={seq} pts={} frames={} pending={}",
                    meta.pts_us,
                    self.stats.0,
                    self.pending.len()
                ),
                None,
            ));
        }

        let _ = self.drain(files, 0)?;
        let images = self
            .sink
            .as_ref()
            .map(|s| s.seen)
            .unwrap_or(if self.fixed.is_some() {
                self.stats.0
            } else {
                0
            });
        Ok((
            format!(
                "SAMPLE_OK seq={seq} frames={} images={images}",
                self.stats.0
            ),
            None,
        ))
    }

    /// Drain whatever the decoder has ready right now into the pending queue.
    unsafe fn drain_frames(&mut self, files: &Path, wait_us: i64) -> Result<(), String> {
        loop {
            let mut info = BufferInfo::default();
            let index = (self.api.dequeue_output)(self.codec, &mut info, wait_us);
            match index {
                INFO_TRY_AGAIN_LATER => return Ok(()),
                INFO_OUTPUT_BUFFERS_CHANGED => {}
                INFO_OUTPUT_FORMAT_CHANGED => {
                    let format = (self.api.get_output_format)(self.codec);
                    let color = format_i32(self.api, format, c"color-format");
                    let stride = format_i32(self.api, format, c"stride");
                    let slice = format_i32(self.api, format, c"slice-height");
                    let fmt_width = format_i32(self.api, format, c"width");
                    let fmt_height = format_i32(self.api, format, c"height");
                    if !format.is_null() {
                        let _ = (self.api.format_delete)(format);
                    }
                    // Trust the decoder's own geometry. On this MTK codec the
                    // ByteBuffer output is the visible size (e.g. 1920x1080 with
                    // stride 1920), not the padded surface size, so forcing the
                    // VA surface height here overruns the buffer.
                    if let Some(color) = color {
                        let width = fmt_width.unwrap_or(self.width as i32).max(1);
                        let height = fmt_height.unwrap_or(self.height as i32).max(1);
                        let out = OutFormat {
                            color,
                            stride: stride.unwrap_or(width).max(width),
                            slice: slice.unwrap_or(height).max(height),
                            width,
                            height,
                        };
                        sequence_log(
                            files,
                            &format!(
                                "video|bridge|cpu-format|color={} stride={} slice-height={} size={}x{}",
                                out.color, out.stride, out.slice, out.width, out.height
                            ),
                        );
                        self.out = Some(out);
                    }
                }
                i if i >= 0 => {
                    let mut capacity = 0usize;
                    let ptr = (self.api.get_output_buffer)(self.codec, i as usize, &mut capacity);
                    let pts = info.presentation_time_us;
                    let size = info.size;
                    let flags = info.flags;
                    let rc = (self.api.release_output)(self.codec, i as usize, false);
                    if rc != AMEDIA_OK {
                        return Err(format!("releaseOutputBuffer({i}) rc={rc}"));
                    }
                    if ptr.is_null() {
                        return Err(format!("getOutputBuffer({i}) returned null"));
                    }
                    let Some(out) = self.out.as_ref() else {
                        return Err(format!("frame {i} arrived before output format"));
                    };
                    self.stats.0 += 1;
                    // A codec-config buffer carries parameter sets, not pixels.
                    if flags & BUFFER_FLAG_CODEC_CONFIG != 0 || size == 0 {
                        sequence_log(
                            files,
                            &format!("video|bridge|cpu-config|flags=0x{flags:x} size={size}"),
                        );
                        continue;
                    }
                    let frame = repack_i420(out, ptr as *const u8, capacity)?;
                    let mut nv12 = frame.y;
                    let chroma = frame.u.len();
                    let mut uv = Vec::with_capacity(2 * chroma);
                    for index in 0..chroma {
                        uv.push(frame.u[index]);
                        uv.push(frame.v[index]);
                    }
                    nv12.extend_from_slice(&uv);
                    if self.pending.len() >= 8 {
                        let dropped = self.pending.pop_front();
                        sequence_log(
                            files,
                            &format!(
                                "video|bridge|cpu-drop|pts={}",
                                dropped.map(|f| f.pts_us).unwrap_or(-1)
                            ),
                        );
                    }
                    self.pending.push_back(PendingFrame {
                        pts_us: pts,
                        nv12,
                        width: frame.width,
                        height: frame.height,
                    });
                }
                other => return Err(format!("dequeueOutputBuffer unexpected={other}")),
            }
        }
    }

    /// Wait for the frame carrying `pts_us` (or, when the client does not know
    /// the PTS, the oldest pending frame).
    unsafe fn take_frame(
        &mut self,
        files: &Path,
        pts_us: i64,
        deadline: Instant,
    ) -> Result<Option<PendingFrame>, String> {
        loop {
            if let Some(position) = self
                .pending
                .iter()
                .position(|frame| pts_us < 0 || frame.pts_us == pts_us)
            {
                return Ok(self.pending.remove(position));
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            // Short slices, not one 50 ms block: the picture is usually a few
            // milliseconds away, and a 50 ms block quantises every frame to
            // 20 fps or worse (measured 13 fps on 1080p30 before this change).
            self.drain_frames(files, 2_000)?;
        }
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
            let _ = self.drain(files, 0)?;
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
            if self.drain(files, 100_000)? {
                break;
            }
            if Instant::now() >= deadline {
                return Err("decoder did not signal output EOS".into());
            }
        }
        let images = self
            .sink
            .as_ref()
            .map(|s| s.seen)
            .unwrap_or(if self.fixed.is_some() {
                self.stats.0
            } else {
                0
            });
        if self.stats.0 == 0 || images == 0 {
            return Err(format!(
                "EOS with no decoded output: frames={} images={images}",
                self.stats.0
            ));
        }
        let expected = self.next_seq.saturating_sub(1);
        let decoded = images.saturating_sub(1);
        let dropped = expected.saturating_sub(decoded);
        Ok(format!(
            "EOS_OK frames={} images={images} expected={expected} decoded={decoded} dropped={dropped} output_bytes={}",
            self.stats.0, self.stats.1
        ))
    }

    unsafe fn close(&mut self) -> String {
        super::present::finish_session();
        release_session();
        super::ebridge::shutdown();
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
        let images = self
            .sink
            .as_ref()
            .map(|sink| sink.seen)
            .unwrap_or(if self.fixed.is_some() {
                self.stats.0
            } else {
                0
            });
        if let Some(sink) = self.sink.take() {
            (sink.api.reader_delete)(sink.reader);
        }
        let expected = self.next_seq.saturating_sub(1);
        let decoded = images.saturating_sub(1);
        let dropped = expected.saturating_sub(decoded);
        format!(
            "CLOSE_OK mime={} codec={} size={}x{} frames={} expected={expected} decoded={decoded} dropped={dropped} output_bytes={} images_last={images} stop_rc={stopped} delete_rc={deleted}",
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
        release_session();
        super::ebridge::shutdown();
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

/// Repack the decoder's padded I420/NV12 ByteBuffer into tight I420.
///
/// MTK pads to macroblock multiples (1920x1088 for a 1920x1080 stream), so the
/// guest would otherwise inherit the padding. Only the two planar families the
/// NDK actually returns are handled; anything else is an error, not a guess.
fn repack_i420(out: &OutFormat, base: *const u8, capacity: usize) -> Result<Frame, String> {
    let width = out.width.max(1) as usize;
    let height = out.height.max(1) as usize;
    let stride = out.stride.max(1) as usize;
    let slice = out.slice.max(1) as usize;
    let chroma_w = width.div_ceil(2);
    let chroma_h = height.div_ceil(2);
    let luma_bytes = stride * slice;

    let mut y = vec![0u8; width * height];
    let (u, v) = match out.color {
        19 => {
            // COLOR_FormatYUV420Planar: three separate planes.
            let chroma_stride = (stride / 2).max(chroma_w);
            let chroma_slice = (slice / 2).max(chroma_h);
            let chroma_bytes = chroma_stride * chroma_slice;
            if capacity < luma_bytes + 2 * chroma_bytes {
                return Err(format!(
                    "I420 output too small: cap={capacity} need={}",
                    luma_bytes + 2 * chroma_bytes
                ));
            }
            let u_src = unsafe { base.add(luma_bytes) };
            let v_src = unsafe { u_src.add(chroma_bytes) };
            let mut u = vec![0u8; chroma_w * chroma_h];
            let mut v = vec![0u8; chroma_w * chroma_h];
            for row in 0..chroma_h {
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        u_src.add(row * chroma_stride),
                        u.as_mut_ptr().add(row * chroma_w),
                        chroma_w,
                    );
                    core::ptr::copy_nonoverlapping(
                        v_src.add(row * chroma_stride),
                        v.as_mut_ptr().add(row * chroma_w),
                        chroma_w,
                    );
                }
            }
            (u, v)
        }
        21 => {
            // COLOR_FormatYUV420SemiPlanar: interleaved UV.
            let chroma_stride = stride.max(2 * chroma_w);
            let chroma_slice = (slice / 2).max(chroma_h);
            let chroma_bytes = chroma_stride * chroma_slice;
            if capacity < luma_bytes + chroma_bytes {
                return Err(format!(
                    "NV12 output too small: cap={capacity} need={}",
                    luma_bytes + chroma_bytes
                ));
            }
            let uv_src = unsafe { base.add(luma_bytes) };
            let mut u = vec![0u8; chroma_w * chroma_h];
            let mut v = vec![0u8; chroma_w * chroma_h];
            for row in 0..chroma_h {
                for col in 0..chroma_w {
                    let pair = unsafe { uv_src.add(row * chroma_stride + col * 2) };
                    u[row * chroma_w + col] = unsafe { *pair };
                    v[row * chroma_w + col] = unsafe { *pair.add(1) };
                }
            }
            (u, v)
        }
        other => {
            return Err(format!(
                "unsupported decoder output color-format {other}; refusing to fabricate pixels"
            ))
        }
    };

    for row in 0..height {
        unsafe {
            core::ptr::copy_nonoverlapping(
                base.add(row * stride),
                y.as_mut_ptr().add(row * width),
                width,
            );
        }
    }

    Ok(Frame {
        y,
        u,
        v,
        width: width as u32,
        height: height as u32,
        color: out.color,
    })
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
        // Keep the session lease alive so a wedged peer can be taken over.
        SESSION_ACTIVITY_MS.store(now_ms(), Ordering::SeqCst);
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
                } else if !claim_session() {
                    sequence_log(files, "video|bridge|OPEN_REFUSED|another session is active");
                    send_line(&mut stream, "OPEN_ERROR another video session is active")
                } else {
                    match Session::open(api, files, &line) {
                        Ok((active, ring_fd)) => {
                            super::present::begin_session();
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
                            release_session();
                            sequence_log(files, &format!("video|bridge|OPEN_FAIL|{e}"));
                            send_line(&mut stream, &format!("OPEN_ERROR {e}"))
                        }
                    }
                }
            }
            "SAMPLE" => match session.as_mut() {
                Some(active) => match active.feed(files, &line) {
                    Ok((ok, payload)) => {
                        let mut outcome = send_line(&mut stream, &ok);
                        if let (Ok(()), Some(bytes)) = (&outcome, payload) {
                            outcome = stream.write_all(&bytes).and_then(|_| stream.flush());
                        }
                        outcome
                    }
                    Err(e) => {
                        sequence_log(files, &format!("video|bridge|SAMPLE_FAIL|{e}"));
                        send_line(&mut stream, &format!("SAMPLE_ERROR {e}"))
                    }
                },
                None => send_line(&mut stream, "SAMPLE_ERROR no open session"),
            },
            "FRAME" => match session.as_mut() {
                Some(active) => {
                    let fields = parse_kv(&line);
                    let pts = parse_i64(&fields, "pts").unwrap_or(-1);
                    let deadline = Instant::now() + FRAME_WAIT;
                    match active.take_frame(files, pts, deadline) {
                        Ok(Some(frame)) => {
                            let line = format!(
                                "FRAME_OK pts={} w={} h={} bytes={}",
                                frame.pts_us,
                                frame.width,
                                frame.height,
                                frame.nv12.len()
                            );
                            let sent = send_line(&mut stream, &line).and_then(|_| {
                                stream.write_all(&frame.nv12).and_then(|_| stream.flush())
                            });
                            sent
                        }
                        Ok(None) => {
                            // A miss is worth a line: "the client asked and the
                            // codec had nothing" is the single most confusing
                            // failure mode on this path (RC=0, zero pictures).
                            sequence_log(
                                files,
                                &format!(
                                    "video|bridge|FRAME_NONE|pts={pts}|pending={}|frames={}|decoded={}",
                                    active.pending.len(),
                                    active.stats.0,
                                    active.stats.1
                                ),
                            );
                            send_line(&mut stream, "FRAME_NONE")
                        }
                        Err(e) => {
                            sequence_log(files, &format!("video|bridge|FRAME_FAIL|{e}"));
                            send_line(&mut stream, &format!("FRAME_ERROR {e}"))
                        }
                    }
                }
                None => send_line(&mut stream, "FRAME_ERROR no open session"),
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
    // Leak once: connection threads need a `'static` table. It is a few
    // primitive words plus the dlopen handle, and this loop runs at most a
    // couple of times per process.
    let api: &'static Api = Box::leak(Box::new(api));
    let rootfs = match crate::guest_rootfs::resolve(&files) {
        Ok(rootfs) => rootfs,
        Err(e) => {
            sequence_log(&files, &format!("video|bridge|SKIP|{e}"));
            return;
        }
    };
    sequence_log(&files, "video|bridge|API|libmediandk.so");
    let marker = files.join("video-bridge");
    let guest_marker = rootfs.join("run/husk/enabled");
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
        // Keep the control socket inside the guest-visible active rootfs. If
        // the socket stayed under `files/`, the guest could only reach it by
        // disabling root isolation (`NR_ROOTFS=/`), which is not a valid
        // product path.
        let path = rootfs.join("run/husk/video.sock");
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
        // One thread per connection: a client that is refused must get a named
        // error, not an invisible EAGAIN because the accept loop is busy
        // streaming for the previous one.
        while enabled() {
            match listener.accept() {
                Ok((stream, _)) => {
                    let files = files.clone();
                    std::thread::spawn(move || unsafe { handle_client(&files, api, stream) });
                }
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
