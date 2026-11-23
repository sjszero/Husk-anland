# Evidence 84 — `vaEndPicture` now delivers real access units to the app decoder

Date: 2026-11-23
Device: RMX3357 / MT6893 / Mali-G77 / Android 13 / kernel 4.19.191 / 4 KiB / arm64
Domain: `untrusted_app` (`com.linuxhub.husk`)
rootfs: `/data/data/com.linuxhub.husk/files/rootfs-full`

## Conclusion

**FACT: the standard GStreamer path (`vaapih264dec`) now pushes its access units
through Husk's VA driver into the app-side `c2.mtk.avc.decoder`, and
`vaEndPicture` returns SUCCESS instead of UNIMPLEMENTED.**

Guest log (`HUSK_VA_TRACE=1`):

```text
HUSK_VA_END_ACTIVE context=1 surface=1 buffers=26 size=1920x1088 profile=13
HUSK_VA_ACCESS_UNIT ... slices=12 bytes=94924 prefix=000000016588843a0c6001e202215c...
HUSK_VA_BRIDGE_OPEN context=1 size=1920x1088 codec=c2.mtk.avc.decoder
HUSK_VA_BRIDGE_SUBMIT context=1 surface=1 profile=13 bytes=94924
HUSK_VA_BRIDGE_SUBMIT context=1 surface=2 profile=13 bytes=58221
```

App log for the same session:

```text
video|bridge|OPEN|OPEN_READY slots=4 slot_size=2097152 bytes=8392960 codec=c2.mtk.avc.decoder
video|bridge|disconnect|CLOSE_OK mime=video/avc codec=c2.mtk.avc.decoder size=1920x1088 \
  frames=0 expected=2 decoded=0 dropped=2 output_bytes=0 images_last=0 stop_rc=0 delete_rc=0
```

So: the app really received and queued **2** samples (`expected=2`) for a real
`c2.mtk.avc.decoder`, and the guest got `SAMPLE_OK` back twice. The first access
unit starts with `00 00 00 01 65` — an IDR slice, so the stream start was a
keyframe, not a delta frame.

**FACT: `frames=0 decoded=0` here is not a decode failure, it is the guest
aborting first.** `decoded = images.saturating_sub(1)` counts acquired
ImageReader outputs; the whole session lasted ~40 ms and ended when gst-vaapi
failed (below), before the asynchronous decoder could return its first output.

## What still blocks the pipeline (FACT, this run)

```text
ERROR vaapivideomemory gstvaapivideomemory.c:936:allocator_configure_image_info:
      Cannot create VA image
ERROR vaapidecode gstvaapipluginbase.c:660:ensure_srcpad_allocator:<vaapidecode_h264-0>
      failed to create src pad's allocator
WARN  videodecoder Subclass failed to decide allocation
WARN  qtdemux error: Internal data stream error. (reason error (-5))
```

gst-vaapi builds its output pool out of **VA images** (`vaCreateImage`), not out
of surfaces. Until the driver implements the image API, `vaapih264dec` cannot
produce a src pad and tears the pipeline down regardless of how well decoding
works. This is the next concrete item, and it is a data-plane problem, not a
probe problem: the image must expose the pixels the app decoder produced.

## What this changes

- The VA system layer is no longer "collects access units and returns
  UNIMPLEMENTED". Encoding of the picture parameters into Annex-B CSD, opening
  the guest-visible `/run/husk/video.sock`, the ring write, and the ack all work
  on the real device through the real plugin.
- The synthesized SPS/PPS from `h264.rs` was **accepted** by MediaCodec
  (a rejected CSD fails `configure`, and `OPEN_READY` would not have been sent).
  "Accepted" is not "correct": correctness can only be judged once a decoded
  frame is compared against a reference.
- `frames=0` must not be quoted as "the decoder can't do it" — the guest hung up
  first.

## Not done

- VAImage / surface export (`vaCreateImage`, `vaDeriveImage`, `vaMapBuffer`) and
  the decoded-frame delivery back to the guest.
- HEVC / VP9 / AV1 through the same path; Chromium `<video>`.
- Any claim that hardware decoding through VA-API is verified end to end.

## Files

- `84-va-endpicture-bridge.log` — guest + app side of this run.
- driver under test: `libhusk_va_driver.so` sha256
  `92d9b0940998f3b4415d17fc17863a7978ebd66d2f385885162508054cbf5083`
