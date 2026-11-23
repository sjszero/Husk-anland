//! Video decode probe and guest-to-app bridge running in the Android
//! NativeActivity process.
//!
//! The probe below established the MediaCodec/AHB capability. `mod bridge`
//! owns the product data plane: a guest sends compressed access units through a
//! memfd ring and this process decodes them with the MTK Codec2 hardware path.
//!
//! The glibc guest cannot load the bionic vendor codec stack.  This module is
//! therefore the bionic-side endpoint: it dynamically resolves the public NDK
//! Media APIs, asks for the MTK Codec2 decoder explicitly, decodes a real file,
//! and records which component actually ran plus the output format it chose.
//!
//! The probe is marker-gated so ordinary app starts do not touch media:
//!
//!   files/video-probe          UTF-8 file name relative to files/, default
//!                              `video-probe.mp4`
//!   files/video-probe.result   append-only evidence produced by this module
//!
//! This is the first video milestone, not the final bridge.  It deliberately
//! uses ByteBuffer output to establish the hardware path before Surface/AHB
//! zero-copy work.  All production calls still use the public NDK ABI.

use core::ffi::{c_char, c_int, c_void};

mod bridge;
mod present;
use std::ffi::{CStr, CString};
use std::fs::OpenOptions;
use std::io::Write;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const RTLD_NOW: c_int = 2;
const AMEDIA_OK: c_int = 0;
const INFO_OUTPUT_BUFFERS_CHANGED: isize = -3;
const INFO_OUTPUT_FORMAT_CHANGED: isize = -2;
const INFO_TRY_AGAIN_LATER: isize = -1;
const BUFFER_FLAG_END_OF_STREAM: u32 = 4;
const DEFAULT_PROBE_FILE: &str = "video-probe.mp4";
const INPUT_TIMEOUT_US: i64 = 2_000_000;
const EOS_TIMEOUT: Duration = Duration::from_secs(15);
const AIMAGE_FORMAT_PRIVATE: i32 = 0x22;
const AHARDWAREBUFFER_USAGE_GPU_SAMPLED_IMAGE: u64 = 1 << 8;

static STARTED: AtomicBool = AtomicBool::new(false);

type Codec = c_void;
type Extractor = c_void;
type Format = c_void;
type Crypto = c_void;
type ImageReader = c_void;
type Image = c_void;
type Ahb = c_void;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct AhbDesc {
    width: u32,
    height: u32,
    layers: u32,
    format: u32,
    usage: u64,
    stride: u32,
    rfu0: u32,
    rfu1: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct BufferInfo {
    offset: i32,
    size: i32,
    presentation_time_us: i64,
    flags: u32,
}

type FnCreateByName = unsafe extern "C" fn(*const c_char) -> *mut Codec;
type FnCreateDecoder = unsafe extern "C" fn(*const c_char) -> *mut Codec;
type FnCodecDelete = unsafe extern "C" fn(*mut Codec) -> c_int;
type FnConfigure =
    unsafe extern "C" fn(*mut Codec, *const Format, *mut c_void, *mut Crypto, u32) -> c_int;
type FnCodecSimple = unsafe extern "C" fn(*mut Codec) -> c_int;
type FnGetInputBuffer = unsafe extern "C" fn(*mut Codec, usize, *mut usize) -> *mut u8;
type FnDequeueInput = unsafe extern "C" fn(*mut Codec, i64) -> isize;
type FnQueueInput = unsafe extern "C" fn(*mut Codec, usize, i64, usize, u64, u32) -> c_int;
type FnDequeueOutput = unsafe extern "C" fn(*mut Codec, *mut BufferInfo, i64) -> isize;
type FnReleaseOutput = unsafe extern "C" fn(*mut Codec, usize, bool) -> c_int;
type FnGetOutputFormat = unsafe extern "C" fn(*mut Codec) -> *mut Format;
type FnGetName = unsafe extern "C" fn(*mut Codec, *mut *mut c_char) -> c_int;
type FnReleaseName = unsafe extern "C" fn(*mut Codec, *mut c_char);

type FnExtractorNew = unsafe extern "C" fn() -> *mut Extractor;
type FnExtractorDelete = unsafe extern "C" fn(*mut Extractor) -> c_int;
type FnSetDataSourceFd = unsafe extern "C" fn(*mut Extractor, c_int, i64, i64) -> c_int;
type FnTrackCount = unsafe extern "C" fn(*mut Extractor) -> usize;
type FnTrackFormat = unsafe extern "C" fn(*mut Extractor, usize) -> *mut Format;
type FnSelectTrack = unsafe extern "C" fn(*mut Extractor, usize) -> c_int;
type FnReadSample = unsafe extern "C" fn(*mut Extractor, *mut u8, usize) -> isize;
type FnSampleFlags = unsafe extern "C" fn(*mut Extractor) -> u32;
type FnSampleTime = unsafe extern "C" fn(*mut Extractor) -> i64;
type FnSampleSize = unsafe extern "C" fn(*mut Extractor) -> isize;
type FnAdvance = unsafe extern "C" fn(*mut Extractor) -> bool;

type FnFormatNew = unsafe extern "C" fn() -> *mut Format;
type FnFormatDelete = unsafe extern "C" fn(*mut Format) -> c_int;
type FnFormatToString = unsafe extern "C" fn(*mut Format) -> *const c_char;
type FnFormatGetString =
    unsafe extern "C" fn(*mut Format, *const c_char, *mut *const c_char) -> bool;
type FnFormatGetInt32 = unsafe extern "C" fn(*mut Format, *const c_char, *mut i32) -> bool;
type FnFormatGetBuffer =
    unsafe extern "C" fn(*mut Format, *const c_char, *mut *mut c_void, *mut usize) -> bool;
type FnFormatSetInt32 = unsafe extern "C" fn(*mut Format, *const c_char, i32);
type FnFormatSetString = unsafe extern "C" fn(*mut Format, *const c_char, *const c_char);
type FnFormatSetBuffer = unsafe extern "C" fn(*mut Format, *const c_char, *const c_void, usize);

type FnImageReaderNewWithUsage =
    unsafe extern "C" fn(i32, i32, i32, u64, i32, *mut *mut ImageReader) -> c_int;
type FnImageReaderDelete = unsafe extern "C" fn(*mut ImageReader);
type FnImageReaderGetWindow = unsafe extern "C" fn(*mut ImageReader, *mut *mut c_void) -> c_int;
type FnImageReaderAcquireNext = unsafe extern "C" fn(*mut ImageReader, *mut *mut Image) -> c_int;
type FnImageDelete = unsafe extern "C" fn(*mut Image);
type FnImageGetHardwareBuffer = unsafe extern "C" fn(*const Image, *mut *mut Ahb) -> c_int;
type FnImageGetTimestamp = unsafe extern "C" fn(*const Image, *mut i64) -> c_int;
type FnAhbDescribe = unsafe extern "C" fn(*const Ahb, *mut AhbDesc);
type FnAhbNativeHandle = unsafe extern "C" fn(*const Ahb) -> *const c_void;

unsafe extern "C" {
    fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlerror() -> *const c_char;
}

struct Api {
    _handle: *mut c_void,
    create_by_name: FnCreateByName,
    create_decoder: FnCreateDecoder,
    codec_delete: FnCodecDelete,
    configure: FnConfigure,
    start: FnCodecSimple,
    stop: FnCodecSimple,
    get_input_buffer: FnGetInputBuffer,
    dequeue_input: FnDequeueInput,
    queue_input: FnQueueInput,
    dequeue_output: FnDequeueOutput,
    release_output: FnReleaseOutput,
    get_output_format: FnGetOutputFormat,
    get_name: FnGetName,
    release_name: FnReleaseName,

    extractor_new: FnExtractorNew,
    extractor_delete: FnExtractorDelete,
    set_data_source_fd: FnSetDataSourceFd,
    track_count: FnTrackCount,
    track_format: FnTrackFormat,
    select_track: FnSelectTrack,
    read_sample: FnReadSample,
    sample_flags: FnSampleFlags,
    sample_time: FnSampleTime,
    sample_size: FnSampleSize,
    advance: FnAdvance,

    format_new: FnFormatNew,
    format_delete: FnFormatDelete,
    format_to_string: FnFormatToString,
    format_get_string: FnFormatGetString,
    format_get_i32: FnFormatGetInt32,
    format_get_buffer: FnFormatGetBuffer,
    format_set_i32: FnFormatSetInt32,
    format_set_string: FnFormatSetString,
    format_set_buffer: FnFormatSetBuffer,
}

struct ImageApi {
    _media_handle: *mut c_void,
    _android_handle: *mut c_void,
    reader_new: FnImageReaderNewWithUsage,
    reader_delete: FnImageReaderDelete,
    reader_get_window: FnImageReaderGetWindow,
    reader_acquire_next: FnImageReaderAcquireNext,
    image_delete: FnImageDelete,
    image_get_hardware_buffer: FnImageGetHardwareBuffer,
    image_get_timestamp: FnImageGetTimestamp,
    ahb_describe: FnAhbDescribe,
    ahb_native_handle: FnAhbNativeHandle,
}

struct ImageSink {
    api: ImageApi,
    reader: *mut ImageReader,
    seen: u64,
}

impl Api {
    unsafe fn load() -> Result<Self, String> {
        let handle = dlopen(c"libmediandk.so".as_ptr(), RTLD_NOW);
        if handle.is_null() {
            return Err(format!("dlopen(libmediandk.so): {}", dlerror_text()));
        }
        Ok(Self {
            _handle: handle,
            create_by_name: symbol(handle, c"AMediaCodec_createCodecByName")?,
            create_decoder: symbol(handle, c"AMediaCodec_createDecoderByType")?,
            codec_delete: symbol(handle, c"AMediaCodec_delete")?,
            configure: symbol(handle, c"AMediaCodec_configure")?,
            start: symbol(handle, c"AMediaCodec_start")?,
            stop: symbol(handle, c"AMediaCodec_stop")?,
            get_input_buffer: symbol(handle, c"AMediaCodec_getInputBuffer")?,
            dequeue_input: symbol(handle, c"AMediaCodec_dequeueInputBuffer")?,
            queue_input: symbol(handle, c"AMediaCodec_queueInputBuffer")?,
            dequeue_output: symbol(handle, c"AMediaCodec_dequeueOutputBuffer")?,
            release_output: symbol(handle, c"AMediaCodec_releaseOutputBuffer")?,
            get_output_format: symbol(handle, c"AMediaCodec_getOutputFormat")?,
            get_name: symbol(handle, c"AMediaCodec_getName")?,
            release_name: symbol(handle, c"AMediaCodec_releaseName")?,

            extractor_new: symbol(handle, c"AMediaExtractor_new")?,
            extractor_delete: symbol(handle, c"AMediaExtractor_delete")?,
            set_data_source_fd: symbol(handle, c"AMediaExtractor_setDataSourceFd")?,
            track_count: symbol(handle, c"AMediaExtractor_getTrackCount")?,
            track_format: symbol(handle, c"AMediaExtractor_getTrackFormat")?,
            select_track: symbol(handle, c"AMediaExtractor_selectTrack")?,
            read_sample: symbol(handle, c"AMediaExtractor_readSampleData")?,
            sample_flags: symbol(handle, c"AMediaExtractor_getSampleFlags")?,
            sample_time: symbol(handle, c"AMediaExtractor_getSampleTime")?,
            sample_size: symbol(handle, c"AMediaExtractor_getSampleSize")?,
            advance: symbol(handle, c"AMediaExtractor_advance")?,

            format_new: symbol(handle, c"AMediaFormat_new")?,
            format_delete: symbol(handle, c"AMediaFormat_delete")?,
            format_to_string: symbol(handle, c"AMediaFormat_toString")?,
            format_get_string: symbol(handle, c"AMediaFormat_getString")?,
            format_get_i32: symbol(handle, c"AMediaFormat_getInt32")?,
            format_get_buffer: symbol(handle, c"AMediaFormat_getBuffer")?,
            format_set_i32: symbol(handle, c"AMediaFormat_setInt32")?,
            format_set_string: symbol(handle, c"AMediaFormat_setString")?,
            format_set_buffer: symbol(handle, c"AMediaFormat_setBuffer")?,
        })
    }
}

impl ImageApi {
    unsafe fn load() -> Result<Self, String> {
        let media_handle = dlopen(c"libmediandk.so".as_ptr(), RTLD_NOW);
        if media_handle.is_null() {
            return Err(format!(
                "dlopen(libmediandk.so) for ImageReader: {}",
                dlerror_text()
            ));
        }
        let android_handle = dlopen(c"libandroid.so".as_ptr(), RTLD_NOW);
        if android_handle.is_null() {
            return Err(format!(
                "dlopen(libandroid.so) for AHardwareBuffer: {}",
                dlerror_text()
            ));
        }
        Ok(Self {
            _media_handle: media_handle,
            _android_handle: android_handle,
            reader_new: symbol(media_handle, c"AImageReader_newWithUsage")?,
            reader_delete: symbol(media_handle, c"AImageReader_delete")?,
            reader_get_window: symbol(media_handle, c"AImageReader_getWindow")?,
            reader_acquire_next: symbol(media_handle, c"AImageReader_acquireNextImage")?,
            image_delete: symbol(media_handle, c"AImage_delete")?,
            image_get_hardware_buffer: symbol(media_handle, c"AImage_getHardwareBuffer")?,
            image_get_timestamp: symbol(media_handle, c"AImage_getTimestamp")?,
            ahb_describe: symbol(android_handle, c"AHardwareBuffer_describe")?,
            ahb_native_handle: symbol(android_handle, c"AHardwareBuffer_getNativeHandle")?,
        })
    }

    unsafe fn new_sink(&self, width: i32, height: i32) -> Result<ImageSink, String> {
        let mut reader = core::ptr::null_mut();
        let rc = (self.reader_new)(
            width,
            height,
            AIMAGE_FORMAT_PRIVATE,
            AHARDWAREBUFFER_USAGE_GPU_SAMPLED_IMAGE,
            4,
            &mut reader,
        );
        if rc != AMEDIA_OK || reader.is_null() {
            return Err(format!(
                "AImageReader_newWithUsage({width}x{height}) rc={rc} reader={reader:p}"
            ));
        }
        Ok(ImageSink {
            reader,
            seen: 0,
            api: ImageApi {
                _media_handle: self._media_handle,
                _android_handle: self._android_handle,
                reader_new: self.reader_new,
                reader_delete: self.reader_delete,
                reader_get_window: self.reader_get_window,
                reader_acquire_next: self.reader_acquire_next,
                image_delete: self.image_delete,
                image_get_hardware_buffer: self.image_get_hardware_buffer,
                image_get_timestamp: self.image_get_timestamp,
                ahb_describe: self.ahb_describe,
                ahb_native_handle: self.ahb_native_handle,
            },
        })
    }
}

unsafe fn symbol<T: Copy>(handle: *mut c_void, name: &CStr) -> Result<T, String> {
    let ptr = dlsym(handle, name.as_ptr());
    if ptr.is_null() {
        return Err(format!(
            "dlsym({}): {}",
            name.to_string_lossy(),
            dlerror_text()
        ));
    }
    Ok(std::mem::transmute_copy(&ptr))
}

unsafe fn dlerror_text() -> String {
    let ptr = dlerror();
    if ptr.is_null() {
        "unknown dlerror".into()
    } else {
        CStr::from_ptr(ptr).to_string_lossy().into_owned()
    }
}

fn files_dir() -> Option<PathBuf> {
    let act = crate::ACTIVITY.load(Ordering::SeqCst);
    if act.is_null() {
        return None;
    }
    let path = unsafe { (*act).internalDataPath };
    if path.is_null() {
        return None;
    }
    Some(PathBuf::from(
        unsafe { CStr::from_ptr(path) }
            .to_string_lossy()
            .into_owned(),
    ))
}

fn marker_file(files: &Path) -> Option<String> {
    let value = std::fs::read_to_string(files.join("video-probe")).ok()?;
    let name = value.trim();
    if name.is_empty() {
        Some(DEFAULT_PROBE_FILE.into())
    } else {
        Some(name.into())
    }
}

fn result(files: &Path, line: &str) {
    crate::log(line);
    if let Ok(mut out) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(files.join("video-probe.result"))
    {
        let _ = writeln!(out, "{line}");
    }
}

fn key(name: &'static CStr) -> *const c_char {
    name.as_ptr()
}

unsafe fn format_text(api: &Api, format: *mut Format) -> String {
    if format.is_null() {
        return "<null>".into();
    }
    let ptr = (api.format_to_string)(format);
    if ptr.is_null() {
        "<null-string>".into()
    } else {
        CStr::from_ptr(ptr).to_string_lossy().into_owned()
    }
}

unsafe fn format_mime(api: &Api, format: *mut Format) -> Option<String> {
    let mut out = core::ptr::null();
    if !(api.format_get_string)(format, key(c"mime"), &mut out) || out.is_null() {
        None
    } else {
        Some(CStr::from_ptr(out).to_string_lossy().into_owned())
    }
}

unsafe fn codec_name(api: &Api, codec: *mut Codec) -> String {
    let mut name = core::ptr::null_mut();
    if (api.get_name)(codec, &mut name) == AMEDIA_OK && !name.is_null() {
        let text = CStr::from_ptr(name).to_string_lossy().into_owned();
        (api.release_name)(codec, name);
        text
    } else {
        "<unknown>".into()
    }
}

unsafe fn requested_codec(mime: &str) -> Option<(&'static str, &'static [u8])> {
    match mime {
        "video/avc" => Some(("c2.mtk.avc.decoder", b"video/avc\0")),
        "video/hevc" => Some(("c2.mtk.hevc.decoder", b"video/hevc\0")),
        "video/x-vnd.on2.vp9" => Some(("c2.mtk.vp9.decoder", b"video/x-vnd.on2.vp9\0")),
        "video/av01" => Some(("c2.mtk.av1.decoder", b"video/av01\0")),
        _ => None,
    }
}

unsafe fn choose_video_track(
    api: &Api,
    extractor: *mut Extractor,
) -> Result<(usize, String, *mut Format), String> {
    let count = (api.track_count)(extractor);
    for index in 0..count {
        let format = (api.track_format)(extractor, index);
        if format.is_null() {
            continue;
        }
        match format_mime(api, format) {
            Some(mime) if mime.starts_with("video/") && requested_codec(&mime).is_some() => {
                return Ok((index, mime, format));
            }
            _ => {
                let _ = (api.format_delete)(format);
            }
        }
    }
    Err(format!("no supported video track in {count} track(s)"))
}

unsafe fn consume_image(sink: &mut ImageSink, files: &Path, tag: &str) -> Result<(), String> {
    let mut image = core::ptr::null_mut();
    let rc = (sink.api.reader_acquire_next)(sink.reader, &mut image);
    if rc != AMEDIA_OK || image.is_null() {
        return Err(format!(
            "AImageReader_acquireNextImage rc={rc} image={image:p}"
        ));
    }
    sink.seen += 1;

    let mut buffer = core::ptr::null_mut();
    let rc = (sink.api.image_get_hardware_buffer)(image, &mut buffer);
    if rc != AMEDIA_OK || buffer.is_null() {
        (sink.api.image_delete)(image);
        return Err(format!(
            "AImage_getHardwareBuffer rc={rc} buffer={buffer:p}"
        ));
    }

    let mut timestamp_ns = 0i64;
    let ts_rc = (sink.api.image_get_timestamp)(image, &mut timestamp_ns);
    let mut desc = AhbDesc::default();
    (sink.api.ahb_describe)(buffer, &mut desc);
    let native = (sink.api.ahb_native_handle)(buffer);
    let (handle_version, num_fds, first_fd) = if native.is_null() {
        (-1, -1, -1)
    } else {
        let words = native as *const i32;
        let version = words.read_unaligned();
        let num_fds = words.add(1).read_unaligned();
        let fd = if num_fds > 0 {
            words.add(3).read_unaligned()
        } else {
            -1
        };
        (version, num_fds, fd)
    };

    if sink.seen == 1 || sink.seen % 30 == 0 {
        result(
            files,
            &format!(
                "video|{tag}|ahb|n={}|timestamp_ns={timestamp_ns}|ts_rc={ts_rc}|size={}x{}|layers={}|format={}|usage=0x{:x}|stride={}|native={native:p}|handle_version={handle_version}|num_fds={num_fds}|first_fd={first_fd}",
                sink.seen,
                desc.width,
                desc.height,
                desc.layers,
                desc.format,
                desc.usage,
                desc.stride
            ),
        );
    }
    if native.is_null() || num_fds <= 0 || first_fd < 0 {
        (sink.api.image_delete)(image);
        return Err(format!(
            "AHardwareBuffer has no dmabuf-native-handle: native={native:p} fds={num_fds} fd={first_fd}"
        ));
    }
    let presented = if tag == "bridge" {
        present::present_if_enabled(files, buffer, desc.width, desc.height)
    } else {
        false
    };
    (sink.api.image_delete)(image);
    if sink.seen <= 3 || sink.seen % 30 == 0 {
        result(
            files,
            &format!("video|{tag}|present|n={}|ok={presented}", sink.seen),
        );
    }
    Ok(())
}

unsafe fn drain_outputs(
    api: &Api,
    codec: *mut Codec,
    files: &Path,
    timeout_us: i64,
    stats: &mut (u64, u64),
    sink: &mut Option<ImageSink>,
    tag: &str,
) -> Result<bool, String> {
    loop {
        let mut info = BufferInfo::default();
        let index = (api.dequeue_output)(codec, &mut info, timeout_us);
        match index {
            INFO_TRY_AGAIN_LATER => return Ok(false),
            INFO_OUTPUT_BUFFERS_CHANGED => continue,
            INFO_OUTPUT_FORMAT_CHANGED => {
                let format = (api.get_output_format)(codec);
                let text = format_text(api, format);
                let color = format_i32(api, format, c"color-format");
                let width = format_i32(api, format, c"width");
                let height = format_i32(api, format, c"height");
                let stride = format_i32(api, format, c"stride");
                let slice_height = format_i32(api, format, c"slice-height");
                result(
                    files,
                    &format!(
                        "video|{tag}|output-format|{text}|color={color:?}|width={width:?}|height={height:?}|stride={stride:?}|slice-height={slice_height:?}"
                    ),
                );
                if !format.is_null() {
                    let _ = (api.format_delete)(format);
                }
                continue;
            }
            i if i >= 0 => {
                stats.0 += 1;
                stats.1 = stats.1.saturating_add(info.size.max(0) as u64);
                let eos = info.flags & BUFFER_FLAG_END_OF_STREAM != 0;
                if stats.0 <= 3 || stats.0 % 30 == 0 || eos {
                    result(
                        files,
                        &format!(
                            "video|{tag}|frame|n={}|pts={}|bytes={}|flags=0x{:x}",
                            stats.0, info.presentation_time_us, info.size, info.flags
                        ),
                    );
                }
                let rc = (api.release_output)(codec, i as usize, sink.is_some() && !eos);
                if rc != AMEDIA_OK {
                    return Err(format!("releaseOutputBuffer({i}) rc={rc}"));
                }
                if let Some(active) = sink.as_mut() {
                    if !eos {
                        consume_image(active, files, tag)?;
                    }
                }
                if eos {
                    return Ok(true);
                }
            }
            other => return Err(format!("dequeueOutputBuffer unexpected={other}")),
        }
    }
}

unsafe fn format_buffer_hex(
    api: &Api,
    format: *mut Format,
    name: &'static CStr,
) -> Option<(usize, String)> {
    let mut data = core::ptr::null_mut();
    let mut len = 0usize;
    if !(api.format_get_buffer)(format, name.as_ptr(), &mut data, &mut len) || data.is_null() {
        return None;
    }
    let bytes = core::slice::from_raw_parts(data as *const u8, len);
    let mut text = String::with_capacity(len * 2 + 2);
    text.push('0');
    text.push('x');
    for byte in bytes {
        text.push_str(&format!("{byte:02x}"));
    }
    Some((len, text))
}

unsafe fn format_i32(api: &Api, format: *mut Format, name: &'static CStr) -> Option<i32> {
    if format.is_null() {
        return None;
    }
    let mut value = 0i32;
    if (api.format_get_i32)(format, name.as_ptr(), &mut value) {
        Some(value)
    } else {
        None
    }
}

unsafe fn probe_file(files: &Path, relative: &str, surface_mode: bool) -> Result<(), String> {
    let path = files.join(relative);
    if !path.is_file() {
        return Err(format!("probe media missing: {}", path.display()));
    }
    let api = Api::load()?;
    let file = std::fs::File::open(&path).map_err(|e| format!("open {}: {e}", path.display()))?;
    let len = file
        .metadata()
        .map_err(|e| format!("stat {}: {e}", path.display()))?
        .len();
    result(
        files,
        &format!(
            "video|probe|BEGIN|path={}|bytes={len}|mode={}",
            path.display(),
            if surface_mode {
                "image-reader-ahb"
            } else {
                "byte-buffer"
            }
        ),
    );

    let extractor = (api.extractor_new)();
    if extractor.is_null() {
        return Err("AMediaExtractor_new returned null".into());
    }
    let mut codec = core::ptr::null_mut();
    let mut track_format = core::ptr::null_mut();
    let mut codec_started = false;
    let mut sink: Option<ImageSink> = None;

    let outcome = (|| -> Result<(), String> {
        let rc = (api.set_data_source_fd)(extractor, file.as_raw_fd(), 0, len as i64);
        if rc != AMEDIA_OK {
            return Err(format!("setDataSourceFd rc={rc}"));
        }
        let (track, mime, format) = choose_video_track(&api, extractor)?;
        track_format = format;
        result(
            files,
            &format!("video|probe|track|index={track}|mime={mime}"),
        );
        result(
            files,
            &format!("video|probe|input-format|{}", format_text(&api, format)),
        );
        result(
            files,
            &format!(
                "video|probe|csd|csd0={:?}|csd1={:?}",
                format_buffer_hex(&api, format, c"csd-0"),
                format_buffer_hex(&api, format, c"csd-1")
            ),
        );
        let rc = (api.select_track)(extractor, track);
        if rc != AMEDIA_OK {
            return Err(format!("selectTrack({track}) rc={rc}"));
        }

        let (wanted, mime_z) =
            requested_codec(&mime).ok_or_else(|| format!("unsupported video mime {mime}"))?;
        let wanted_z = CString::new(wanted).map_err(|e| format!("codec name: {e}"))?;
        codec = (api.create_by_name)(wanted_z.as_ptr());
        if codec.is_null() {
            result(
                files,
                &format!("video|probe|create|name={wanted}|FAIL|falling back by MIME"),
            );
            codec = (api.create_decoder)(mime_z.as_ptr() as *const c_char);
        }
        if codec.is_null() {
            return Err(format!("no decoder for {mime}"));
        }
        let actual = codec_name(&api, codec);
        result(
            files,
            &format!("video|probe|codec|requested={wanted}|actual={actual}"),
        );
        if actual != wanted {
            result(
                files,
                &format!("video|probe|codec-warning|hardware decoder fallback selected {actual}"),
            );
        }

        let mut output_surface = core::ptr::null_mut();
        if surface_mode {
            let width = format_i32(&api, format, c"width").unwrap_or(1920);
            let height = format_i32(&api, format, c"height").unwrap_or(1080);
            let image_api = ImageApi::load()?;
            let active = image_api.new_sink(width, height)?;
            let mut window = core::ptr::null_mut();
            let rc = (image_api.reader_get_window)(active.reader, &mut window);
            result(
                files,
                &format!(
                    "video|probe|image-reader|size={width}x{height}|usage=0x{AHARDWAREBUFFER_USAGE_GPU_SAMPLED_IMAGE:x}|window={window:p}|rc={rc}"
                ),
            );
            if rc != AMEDIA_OK || window.is_null() {
                (image_api.reader_delete)(active.reader);
                return Err(format!("AImageReader_getWindow rc={rc} window={window:p}"));
            }
            output_surface = window;
            sink = Some(active);
        }

        let rc = (api.configure)(codec, format, output_surface, core::ptr::null_mut(), 0);
        result(files, &format!("video|probe|configure|rc={rc}"));
        if rc != AMEDIA_OK {
            return Err(format!("configure rc={rc}"));
        }
        let _ = (api.format_delete)(track_format);
        track_format = core::ptr::null_mut();

        let rc = (api.start)(codec);
        result(files, &format!("video|probe|start|rc={rc}"));
        if rc != AMEDIA_OK {
            return Err(format!("start rc={rc}"));
        }
        codec_started = true;

        let begin = Instant::now();
        let mut decoded = (0u64, 0u64);
        let mut samples = 0u64;
        let mut input_bytes = 0u64;
        loop {
            let sample_size = (api.sample_size)(extractor);
            if sample_size < 0 {
                break;
            }
            let pts = (api.sample_time)(extractor);
            let flags = (api.sample_flags)(extractor);
            let input = (api.dequeue_input)(codec, INPUT_TIMEOUT_US);
            if input < 0 {
                return Err(format!(
                    "dequeueInputBuffer before sample {samples}: {input}"
                ));
            }
            let mut capacity = 0usize;
            let dst = (api.get_input_buffer)(codec, input as usize, &mut capacity);
            if dst.is_null() || sample_size as usize > capacity {
                return Err(format!(
                    "input buffer too small: sample={sample_size} capacity={capacity}"
                ));
            }
            let read = (api.read_sample)(extractor, dst, capacity);
            if read < 0 {
                return Err(format!("readSampleData sample={samples} rc={read}"));
            }
            if samples == 0 && read > 0 {
                let head = core::slice::from_raw_parts(dst, (read as usize).min(64));
                let mut hex = String::new();
                for byte in head {
                    hex.push_str(&format!("{byte:02x}"));
                }
                result(files, &format!("video|probe|sample-prefix|{hex}"));
            }
            let rc = (api.queue_input)(codec, input as usize, 0, read as usize, pts as u64, flags);
            if rc != AMEDIA_OK {
                return Err(format!("queueInputBuffer sample={samples} rc={rc}"));
            }
            samples += 1;
            input_bytes = input_bytes.saturating_add(read as u64);
            let _ = drain_outputs(&api, codec, files, 0, &mut decoded, &mut sink, "probe")?;
            let _ = (api.advance)(extractor);
        }

        let input = (api.dequeue_input)(codec, INPUT_TIMEOUT_US);
        if input < 0 {
            return Err(format!("dequeueInputBuffer(EOS) rc={input}"));
        }
        let rc = (api.queue_input)(codec, input as usize, 0, 0, 0, BUFFER_FLAG_END_OF_STREAM);
        if rc != AMEDIA_OK {
            return Err(format!("queueInputBuffer(EOS) rc={rc}"));
        }

        let deadline = Instant::now() + EOS_TIMEOUT;
        let eos = loop {
            if drain_outputs(&api, codec, files, 0, &mut decoded, &mut sink, "probe")? {
                break true;
            }
            if Instant::now() >= deadline {
                break false;
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        let elapsed_ms = begin.elapsed().as_millis();
        result(
            files,
            &format!(
                "video|probe|done|pass={}|samples={samples}|input_bytes={input_bytes}|frames={}|output_bytes={}|images={}|eos={eos}|elapsed_ms={elapsed_ms}",
                eos && decoded.0 > 0,
                decoded.0,
                decoded.1,
                sink.as_ref().map(|s| s.seen).unwrap_or(0)
            ),
        );
        if !eos {
            return Err("decoder did not signal output EOS".into());
        }
        if decoded.0 == 0 {
            return Err("decoder produced no frames".into());
        }
        Ok(())
    })();

    if !track_format.is_null() {
        let _ = (api.format_delete)(track_format);
    }
    if !codec.is_null() {
        if codec_started {
            let rc = (api.stop)(codec);
            result(files, &format!("video|probe|stop|rc={rc}"));
        }
        let rc = (api.codec_delete)(codec);
        result(files, &format!("video|probe|delete-codec|rc={rc}"));
    }
    if let Some(active) = sink.take() {
        (active.api.reader_delete)(active.reader);
        result(files, "video|probe|delete-image-reader");
    }
    let rc = (api.extractor_delete)(extractor);
    result(files, &format!("video|probe|delete-extractor|rc={rc}"));

    if let Err(e) = &outcome {
        result(files, &format!("video|probe|FAIL|{e}"));
    }
    outcome
}

fn serve() {
    let Some(files) = files_dir() else {
        crate::log("video|probe|SKIP|no files dir");
        return;
    };
    let mut last_marker = String::new();
    let mut announced_wait = false;
    loop {
        let Some(relative) = marker_file(&files) else {
            if !announced_wait {
                crate::log(&format!(
                    "video|probe|WAIT|{}",
                    files.join("video-probe").display()
                ));
                announced_wait = true;
            }
            std::thread::sleep(Duration::from_millis(500));
            continue;
        };
        if relative == last_marker {
            std::thread::sleep(Duration::from_millis(500));
            continue;
        }
        announced_wait = false;
        let _ = std::fs::remove_file(files.join("video-probe.result"));
        crate::log(&format!("video|probe|START|{relative}"));
        let marker_value = relative.clone();
        let (surface_mode, relative) = if let Some(rest) = relative.strip_prefix("surface:") {
            (true, rest)
        } else if let Some(rest) = relative.strip_prefix("bytebuffer:") {
            (false, rest)
        } else {
            (false, relative.as_str())
        };
        let _ = std::panic::catch_unwind(|| unsafe { probe_file(&files, relative, surface_mode) });
        last_marker = marker_value;
    }
}

/// Start the single bionic-side MediaCodec probe thread for this process.
pub fn start_once() {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| {
        let _ = std::panic::catch_unwind(serve);
    });
    bridge::start();
}
