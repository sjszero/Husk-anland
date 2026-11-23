//! Fixed application-owned YUV AHardwareBuffer ring for the video bridge.
//!
//! The MediaCodec surface/ImageReader path allocates a fresh producer buffer
//! for every output frame on this MTK device.  That makes a Vulkan import ring
//! impossible even when the application holds only three decoded images.  This
//! module takes the other public NDK path: decode to ByteBuffer, copy the
//! I420 planes into three application-owned YV12 AHardwareBuffers, and hand the
//! same stable dma-buf identities back to the guest.  The copy is explicit and
//! measurable; it buys a stable ring and lets the guest cache Vulkan resources.

use super::*;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

const FIXED_SLOTS: usize = 3;
const AHB_FORMAT_YV12: u32 = 0x3231_5659;
const AHB_USAGE_CPU_WRITE_OFTEN: u64 = 1 << 5;
const AHB_USAGE_GPU_SAMPLED_IMAGE: u64 = 1 << 8;
const AHB_USAGE_GPU_FRAMEBUFFER: u64 = 1 << 9;
const COLOR_FORMAT_YUV420_PLANAR: i32 = 19;

struct Slot {
    buffer: *mut Ahb,
    buffer_key: u64,
    dma_key: u64,
    desc: AhbDesc,
}

pub(super) struct FixedFrame {
    pub buffer: *mut Ahb,
    pub desc: AhbDesc,
    pub planes: AhbPlanes,
    pub buffer_key: u64,
    pub dma_key: u64,
    pub timestamp_ns: i64,
    pub plane_hashes: [u64; 4],
    pub plane_bytes: [usize; 4],
}

pub(super) struct FixedRing {
    pub api: ImageApi,
    slots: Vec<Slot>,
    free: Vec<usize>,
    release_tx: Sender<usize>,
    released_rx: Receiver<usize>,
    pub width: u32,
    pub height: u32,
    source_stride: usize,
    source_slice_height: usize,
    source_color: i32,
}

impl FixedRing {
    pub(super) unsafe fn new(width: u32, height: u32, files: &Path) -> Result<Self, String> {
        let api = ImageApi::load()?;
        let (release_tx, released_rx) = mpsc::channel();
        let mut slots: Vec<Slot> = Vec::with_capacity(FIXED_SLOTS);
        let requested = AhbDesc {
            width,
            height,
            layers: 1,
            format: AHB_FORMAT_YV12,
            usage: AHB_USAGE_CPU_WRITE_OFTEN
                | AHB_USAGE_GPU_SAMPLED_IMAGE
                | AHB_USAGE_GPU_FRAMEBUFFER,
            ..AhbDesc::default()
        };

        for index in 0..FIXED_SLOTS {
            let mut buffer = core::ptr::null_mut();
            let rc = (api.ahb_allocate)(&requested, &mut buffer);
            if rc != 0 || buffer.is_null() {
                for slot in &slots {
                    (api.ahb_release)(slot.buffer);
                }
                return Err(format!(
                    "AHardwareBuffer_allocate slot={index} rc={rc} ptr={buffer:p}"
                ));
            }
            let mut actual = AhbDesc::default();
            (api.ahb_describe)(buffer, &mut actual);
            let native = (api.ahb_native_handle)(buffer);
            let dma_key = dmabuf_key_from_native(native).unwrap_or(0);
            let mut ahb_id = 0u64;
            let ahb_id_rc = api
                .ahb_get_id
                .map(|get_id| get_id(buffer, &mut ahb_id))
                .unwrap_or(-1);
            let buffer_key = if ahb_id_rc == 0 && ahb_id != 0 {
                ahb_id
            } else {
                dma_key
            };
            result(
                files,
                &format!(
                    "video|fixed-ring|allocate|slot={index}|rc={rc}|size={}x{}|stride={}|format={:#x}|usage=0x{:x}|id_rc={ahb_id_rc}|id={ahb_id:016x}|dma={dma_key:016x}|key={buffer_key:016x}",
                    actual.width,
                    actual.height,
                    actual.stride,
                    actual.format,
                    actual.usage
                ),
            );
            if actual.format != AHB_FORMAT_YV12
                || actual.width != width
                || actual.height != height
                || dma_key == 0
                || buffer_key == 0
            {
                (api.ahb_release)(buffer);
                for slot in &slots {
                    (api.ahb_release)(slot.buffer);
                }
                return Err(format!(
                    "bad fixed AHB slot={index}: size={}x{} format={:#x} dma={dma_key:016x} key={buffer_key:016x}",
                    actual.width, actual.height, actual.format
                ));
            }
            slots.push(Slot {
                buffer,
                buffer_key,
                dma_key,
                desc: actual,
            });
        }

        result(
            files,
            &format!("video|fixed-ring|READY|slots={}", slots.len()),
        );
        Ok(Self {
            api,
            slots,
            free: (0..FIXED_SLOTS).collect(),
            release_tx,
            released_rx,
            width,
            height,
            source_stride: width as usize,
            source_slice_height: height as usize,
            source_color: COLOR_FORMAT_YUV420_PLANAR,
        })
    }

    pub(super) fn release_sender(&self) -> Sender<usize> {
        self.release_tx.clone()
    }

    pub(super) fn acquire(&mut self) -> Option<usize> {
        loop {
            while let Ok(slot) = self.released_rx.try_recv() {
                if !self.free.contains(&slot) {
                    self.free.push(slot);
                }
            }
            if let Some(slot) = self.free.pop() {
                return Some(slot);
            }
            match self.released_rx.recv_timeout(Duration::from_millis(100)) {
                Ok(slot) => {
                    if !self.free.contains(&slot) {
                        self.free.push(slot);
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => return None,
            }
        }
    }

    fn release_slot(&mut self, slot: usize) {
        if slot < self.slots.len() && !self.free.contains(&slot) {
            self.free.push(slot);
        }
    }

    pub(super) fn set_source_format(
        &mut self,
        color: Option<i32>,
        stride: Option<i32>,
        slice_height: Option<i32>,
    ) -> Result<(), String> {
        if let Some(color) = color {
            self.source_color = color;
        }
        if self.source_color != COLOR_FORMAT_YUV420_PLANAR {
            return Err(format!(
                "unsupported MediaCodec output color-format={} (expected I420/19)",
                self.source_color
            ));
        }
        if let Some(stride) = stride {
            if stride > 0 {
                self.source_stride = stride as usize;
            }
        }
        if let Some(slice_height) = slice_height {
            if slice_height > 0 {
                self.source_slice_height = slice_height as usize;
            }
        }
        Ok(())
    }

    unsafe fn take_frame(
        &mut self,
        slot_index: usize,
        source: *const u8,
        source_size: usize,
        pts_us: i64,
    ) -> Result<FixedFrame, String> {
        let slot = &self.slots[slot_index];
        let width = self.width as usize;
        let height = self.height as usize;
        let source_stride = self.source_stride.max(width);
        let source_slice_height = self.source_slice_height.max(height);
        let chroma_width = width.div_ceil(2);
        let chroma_height = height.div_ceil(2);
        let chroma_stride = source_stride.div_ceil(2);
        let y_size = source_stride
            .checked_mul(source_slice_height)
            .ok_or_else(|| "source Y size overflow".to_string())?;
        let chroma_size = chroma_stride
            .checked_mul(chroma_height)
            .ok_or_else(|| "source chroma size overflow".to_string())?;
        let required = y_size
            .checked_add(chroma_size.saturating_mul(2))
            .ok_or_else(|| "source total size overflow".to_string())?;
        if source.is_null() || source_size < required {
            return Err(format!(
                "output buffer too small: size={source_size} required={required}"
            ));
        }

        let mut planes = AhbPlanes::default();
        let lock_rc = (self.api.ahb_lock_planes)(
            slot.buffer,
            AHB_USAGE_CPU_WRITE_OFTEN,
            -1,
            core::ptr::null(),
            &mut planes,
        );
        if lock_rc != 0 || planes.plane_count < 3 {
            let _ = (self.api.ahb_unlock)(slot.buffer, core::ptr::null_mut());
            return Err(format!(
                "AHardwareBuffer_lockPlanes rc={lock_rc} planes={}",
                planes.plane_count
            ));
        }
        if planes.planes[0].data.is_null()
            || planes.planes[1].data.is_null()
            || planes.planes[2].data.is_null()
            || (planes.planes[0].row_stride as usize) < width
            || (planes.planes[1].row_stride as usize) < chroma_width
            || (planes.planes[2].row_stride as usize) < chroma_width
        {
            let _ = (self.api.ahb_unlock)(slot.buffer, core::ptr::null_mut());
            return Err("fixed AHB returned invalid plane layout".into());
        }

        copy_rows(
            source,
            source_stride,
            planes.planes[0].data,
            planes.planes[0].row_stride as usize,
            width,
            height,
        );
        // AHardwareBuffer YV12 stores Y, V, U. MediaCodec color-format 19 is
        // I420 (Y, U, V), so the two chroma planes are intentionally crossed.
        copy_rows(
            source.add(y_size),
            chroma_stride,
            planes.planes[2].data,
            planes.planes[2].row_stride as usize,
            chroma_width,
            chroma_height,
        );
        copy_rows(
            source.add(y_size + chroma_size),
            chroma_stride,
            planes.planes[1].data,
            planes.planes[1].row_stride as usize,
            chroma_width,
            chroma_height,
        );

        let mut hashes = [0u64; 4];
        let mut bytes = [0usize; 4];
        for index in 0..3 {
            let plane = planes.planes[index];
            let (plane_width, plane_height) = if index == 0 {
                (width, height)
            } else {
                (chroma_width, chroma_height)
            };
            bytes[index] = plane_width.saturating_mul(plane_height);
            hashes[index] = hash_plane_rows(
                plane.data as *const u8,
                plane.row_stride as usize,
                plane_width,
                plane_height,
            );
        }
        let unlock_rc = (self.api.ahb_unlock)(slot.buffer, core::ptr::null_mut());
        if unlock_rc != 0 {
            return Err(format!("AHardwareBuffer_unlock rc={unlock_rc}"));
        }

        Ok(FixedFrame {
            buffer: slot.buffer,
            desc: slot.desc,
            planes,
            buffer_key: slot.buffer_key,
            dma_key: slot.dma_key,
            timestamp_ns: pts_us.saturating_mul(1000),
            plane_hashes: hashes,
            plane_bytes: bytes,
        })
    }
}

impl Drop for FixedRing {
    fn drop(&mut self) {
        unsafe {
            for slot in &self.slots {
                (self.api.ahb_release)(slot.buffer);
            }
        }
        self.slots.clear();
    }
}

unsafe fn copy_rows(
    source: *const u8,
    source_stride: usize,
    destination: *mut c_void,
    destination_stride: usize,
    width: usize,
    height: usize,
) {
    for row in 0..height {
        core::ptr::copy_nonoverlapping(
            source.add(row * source_stride),
            (destination as *mut u8).add(row * destination_stride),
            width,
        );
    }
}

/// Drain MediaCodec output into the fixed AHB ring. Returns true on EOS.
pub(super) unsafe fn drain_outputs(
    api: &Api,
    codec: *mut Codec,
    files: &Path,
    timeout_us: i64,
    stats: &mut (u64, u64),
    ring: &mut FixedRing,
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
                let color = format_i32(api, format, c"color-format");
                let stride = format_i32(api, format, c"stride");
                let slice_height = format_i32(api, format, c"slice-height");
                result(
                    files,
                    &format!(
                        "video|{tag}|fixed-format|color={color:?}|stride={stride:?}|slice-height={slice_height:?}"
                    ),
                );
                let format_ok = ring.set_source_format(color, stride, slice_height);
                if !format.is_null() {
                    let _ = (api.format_delete)(format);
                }
                format_ok?;
                continue;
            }
            output if output >= 0 => {
                stats.0 = stats.0.saturating_add(1);
                stats.1 = stats.1.saturating_add(info.size.max(0) as u64);
                let eos = info.flags & BUFFER_FLAG_END_OF_STREAM != 0;
                if eos {
                    let rc = (api.release_output)(codec, output as usize, false);
                    if rc != AMEDIA_OK {
                        return Err(format!("releaseOutputBuffer(EOS) rc={rc}"));
                    }
                    return Ok(true);
                }

                let Some(slot) = ring.acquire() else {
                    let rc = (api.release_output)(codec, output as usize, false);
                    if rc != AMEDIA_OK {
                        return Err(format!("releaseOutputBuffer(no-slot) rc={rc}"));
                    }
                    return Err("fixed AHB ring has no free slot".into());
                };
                let mut output_size = 0usize;
                let output_ptr = (api.get_output_buffer)(codec, output as usize, &mut output_size);
                let offset = info.offset.max(0) as usize;
                if output_ptr.is_null() || offset >= output_size {
                    ring.release_slot(slot);
                    let _ = (api.release_output)(codec, output as usize, false);
                    return Err(format!(
                        "getOutputBuffer index={output} rc=null size={output_size} offset={offset}"
                    ));
                }
                let frame = match ring.take_frame(
                    slot,
                    output_ptr.add(offset),
                    output_size - offset,
                    info.presentation_time_us,
                ) {
                    Ok(frame) => frame,
                    Err(error) => {
                        ring.release_slot(slot);
                        let _ = (api.release_output)(codec, output as usize, false);
                        return Err(error);
                    }
                };
                let rc = (api.release_output)(codec, output as usize, false);
                if rc != AMEDIA_OK {
                    ring.release_slot(slot);
                    return Err(format!("releaseOutputBuffer index={output} rc={rc}"));
                }
                if !ebridge::export_fixed_if_enabled(
                    files,
                    &ring.api,
                    frame.buffer,
                    &frame.desc,
                    &frame.planes,
                    frame.buffer_key,
                    frame.dma_key,
                    stats.0,
                    frame.timestamp_ns,
                    &frame.plane_hashes,
                    &frame.plane_bytes,
                    slot,
                    ring.release_sender(),
                ) {
                    ring.release_slot(slot);
                    return Err("fixed AHB export failed".into());
                }
                if stats.0 <= 3 || stats.0 % 30 == 0 {
                    result(
                        files,
                        &format!(
                            "video|{tag}|fixed-frame|n={}|slot={slot}|key={:016x}|pts={}",
                            stats.0, frame.buffer_key, info.presentation_time_us
                        ),
                    );
                }
            }
            other => return Err(format!("dequeueOutputBuffer unexpected={other}")),
        }
    }
}
