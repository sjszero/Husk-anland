# Husk video bridge v1

> Status: v1 data plane **PASS on device** (evidence 69). A guest rooted at the
> private `rootfs-full` tree demuxes H.264, sends access units through the memfd
> ring, and the app produces AHardwareBuffers with `c2.mtk.avc.decoder`. This
> document remains the source of truth for the guest-to-app compressed-frame data
> plane. It intentionally does **not** claim that dma-buf return,
> GStreamer/Chromium integration, display, or AV sync are done.

## Prior art read before implementation

The mandatory reference gate was checked before writing this module:

| Source | Revision / API | Reused idea | Not reused |
|---|---|---|---|
| Android NDK Media (`frameworks/av`) | NDK r29 headers, `AMediaCodec`, `AMediaFormat`, `AImageReader` | Public bionic ABI, hardware Codec2 selection, CSD buffers, compressed access units, PTS/flags | No Java `MediaCodec` object or JNI |
| `GStreamer/gstreamer` | upstream media pipeline model | guest keeps demux and AV sync; VA-API backend becomes the later public integration point | No gst daemon or extra relay process |
| `FFmpeg/FFmpeg` | official upstream | guest demux/clock ownership and compressed-packet boundary | no linked FFmpeg library in this first Rust proof |
| `alfg/mp4-rust` | crates.io `mp4 0.14.0`, MIT | pure-Rust MP4 track/sample reader for the first guest demux proof | not the final general demuxer |

The first proof uses a pure-Rust MP4 reader in the guest only to prove the ownership
boundary and the transport. The final guest pipeline remains free to use GStreamer or
FFmpeg for broad container/codec coverage.

## Architecture

```
guest process (glibc, untrusted_app)
  MP4 / later GStreamer or FFmpeg
    -> compressed access units + CSD + PTS + flags
    -> memfd shared ring (4 slots x 2 MiB)
    -> one notification per sequence on a dedicated AF_UNIX stream

app process (bionic, same uid)
  bridge thread
    -> validates sequence and copies one record out of the ring
    -> AMediaCodec (c2.mtk.avc.decoder first)
    -> AImageReader(AIMAGE_FORMAT_PRIVATE)
    -> AHardwareBuffer + dma-buf fds
```

There is no separate decoder service and no virgl/gst relay process. The socket is only
the control/wakeup channel. Compressed bytes never travel through the socket.

## Protocol v1

All control messages are UTF-8 lines terminated by `\n`.

1. Guest connects to the control socket under
   `rootfs-full/tmp/husk-video/bridge.sock`. The current test harness passes the
   private physical path because `connect(2)` path translation is not yet part of
   the shim; rootfs isolation itself remains enabled.
2. Guest sends `OPEN mime=video/avc width=<n> height=<n> codec=<name> csd0=<hex> csd1=<hex>`.
3. App replies `OPEN_READY ...`, then sends exactly one `SCM_RIGHTS` message carrying the memfd.
4. Guest maps the memfd. The socket is used only for sequence notifications and replies.
5. For each access unit:
   - guest writes one record into the ring;
   - guest sends `SAMPLE seq=<n> len=<bytes> pts_us=<i64> flags=<u32>`;
   - app decodes the record and replies `SAMPLE_OK seq=<n> frames=<n> images=<n>`.
6. Guest sends `EOS`; app queues an empty MediaCodec input buffer with EOS and drains until
   the output EOS flag, then replies `EOS_OK ...`.
7. Guest sends `CLOSE`; app stops/deletes codec and closes the ImageReader. `QUIT` closes the
   connection.

A guest may not advance to sequence `n+1` until it receives `SAMPLE_OK n`. This is the
first backpressure rule: it prevents the ring from being reused while MediaCodec still owns
the prior input bytes.

## Ring layout

`video-protocol` owns both layouts so app and guest cannot drift:

- header: magic, version, slot count, slot size, stride, monotonic `write_seq`, `read_seq`;
- four fixed slots, each with a 64-byte record header followed by up to 2 MiB of payload;
- producer writes payload and metadata, then publishes `write_seq` with release ordering;
- consumer reads `write_seq` with acquire ordering, validates `seq`, copies the payload, then
  publishes `read_seq` with release ordering.

The ring is a `memfd_create(MFD_CLOEXEC)` mapping. Only the descriptor is transferred; no path
under app-private data is exposed through the guest path shim.

## v1 scope and explicit unknowns

- v1 accepts one video track, H.264/AVC only, and requires the exact MTK Codec2 decoder name.
- v1 proves guest demux + transport + hardware decode + AHB production on the
  attached device (14 repeated app-side sessions; evidence 69).
- v1 does not yet import the returned AHB fd through kbase `MEM_IMPORT`.
- v1 does not yet prove HEVC, VP9, AV1, GStreamer VA-API, Chromium, audio/video sync,
  reconnect races, 10-minute playback, or dropped-frame accounting.
- The MP4 reader is a proof input, not the final general-purpose guest demuxer.
