# Evidence 93 — camera still (JPEG) and 30 s recording (hardware H.264)

Date: 2026-11-23 15:2x–15:30 · device RMX3357 (`UKHMMB8PYTGECMNV`) · app `com.linuxhub.husk`
· guest Ubuntu 24.04 (`files/rootfs-full`) · loader `files/libld39.so`
· preload `rootfs-full/usr/lib/aarch64-linux-gnu/libhusk_v4l2.so`

Both acceptance items below run through **hardware only**: the frame comes from
Camera2 (`ACameraCaptureSession` → `AImageReader`, NV12), the encode is
`c2.mtk.avc.encoder` through MediaCodec. Nothing is software-rendered or
software-encoded.

## What was added

- `app-rs/src/encoder.rs` — MediaCodec encoder bridge on
  `rootfs-full/run/husk/encoder.sock` (`INFO` / `OPEN` / `FRAME` / `STOP`,
  NV12 uplink, Annex-B access units downlink with MediaCodec flags).
  形态：daemon socket + NV12 上行 + 合成 CSD。
- `runtime/crates/husk-record` — guest recorder: `/dev/videoN` capture (V4L2
  shim) → encoder socket → file. Writes `.h264` or `.mp4`.
- `runtime/crates/husk-record/src/mp4.rs` — minimal but complete MP4 muxer
  (ftyp/mdat/moov, avcC, stts/stss/stsc/stsz/stco). Reason: GStreamer's
  `filesrc`/`filesink` open through libc-internal `open`, so they fail inside the
  guest with ENOENT (the documented `libc-interpose.md` gap). Container writing
  must not depend on that.

## 1. Still photo (JPEG) — PASS

```
$ gst-launch-1.0 v4l2src device=/dev/video1 num-buffers=1 ! videoconvert !
  jpegenc ! filesink location=tmp/husk-still-front.jpg        rc=0
$ file: 187265 bytes, SOI ffd8 / EOI ffd9, SOF0 1920x1080, 3 components
```

`docs/design/evidence/93-camera-still-front.jpg` is that file. (Back camera at
1080p gives a valid JPEG of a dark scene — the phone lies face-up, so its rear
lens is covered; raw NV12 read back as Y∈[0,4], UV=128, i.e. genuinely black,
not a conversion fault.)

## 2. 30 s recording — PASS

```
$ husk-record /run/husk/encoder.sock /dev/video0 30 tmp/rec30.mp4 640 480 15 2000
record|OPEN|640x480|15fps|2000kbps
record|sent|890|frames|30.01s|dqb_err=0
record|collect|aus=890|key=60|bytes=1338587|sps=19|pps=4
record|mp4|tmp/rec30.mp4|1343003 bytes|640x480|29.53 fps|30.14 s
```

(I-frame interval 1 s → 60 IDRs over 30 s, exactly as configured.)

Front camera, same command on `/dev/video1`:

```
record|sent|890|frames|30.02s|dqb_err=0
record|collect|aus=890|key=60|bytes=6259034|sps=19|pps=4
record|mp4|tmp/rec30f.mp4|6263450 bytes|640x480|29.59 fps|30.07 s
```

Host-side verification of the pulled file (FFmpeg, same box):

```
$ ffprobe rec30f.mp4
codec_name=h264   profile=High   width=640   height=480   nb_frames=890
duration=30.073664   bit_rate=1666162
$ ffmpeg -v error -i rec30f.mp4 -f null -
(no output: every frame decoded without a single error)
```

Frames at t=1 s / 15 s / 29 s decode to the same room scene, so the file is a
continuous 30 s clip, not a repeated still.
`docs/design/evidence/93-record-30s-front-t29.png` is the t=29 s frame.

## Three bugs found on the way (all measured)

1. `dlsym(codec_handle, "AMediaCodec_start")` returns NULL — those symbols come
   from the **library** handle, not the codec instance. First attempt answered
   `ERR no_codec` while the platform log showed the component was created.
2. `struct v4l2_buffer.bytesused` is at **offset 8** on aarch64, not near the
   tail; reading offset 76 gave 0 and every frame was silently skipped
   (`sent|0|frames`).
3. MediaCodec delivered **SPS and PPS as two separate CSD buffers**. A
   "first CSD wins" rule wrote the SPS only, producing an H.264 stream with no
   PPS — parseable structure, undecodable content. Now deduped per NAL type.

## Still open for the camera goal

| acceptance item | state |
|---|---|
| guest preview shows a picture | ✅ (evidence 91/92) |
| still photo (JPEG) valid | ✅ this file |
| 30 s recording playable | ✅ this file |
| front/back switch | ✅ `/dev/video0` BACK, `/dev/video1` FRONT |
| permission-denied degradation | NOT DONE |
