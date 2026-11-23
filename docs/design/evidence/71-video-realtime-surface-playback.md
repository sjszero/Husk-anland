# Evidence 71 — MediaCodec AHB real-time Surface playback

Date: 2026-11-23
Device: RMX3357 / MediaTek MT6893 / Dimensity 1200 / Mali-G77 MC9 /
Android 13 / kernel 4.19.191+ / 4 KiB pages / arm64
Repository: `/Users/l/Documents/Codex/2026-08-10/husk`

## Conclusion

**FACT: the app-side decoded-video display path now runs in real time.** The
guest demuxed a 1920x1080@30 AVC file and sent its access units over the
existing memfd ring. The app decoded them with `c2.mtk.avc.decoder`, took every
output `AHardwareBuffer`, and submitted all 150 buffers directly to a persistent
SurfaceControl child layer. Two consecutive sessions each completed
`150/150` samples in about 5.1 seconds, with `150/150` presenter submissions and
`150/150` SurfaceFlinger completion/release callbacks.

The two screenshots were captured from separate sessions and show different
video timestamps and moving markers. This is a moving-video path, not a
single-frame still.

This evidence closes **hardware decode output -> SurfaceFlinger presentation**.
It does **not** close export of the decoded AHB through kbase `MEM_IMPORT` back
into the glibc guest, guest-side GL/X11/Wayland WSI, GStreamer/VA-API, Chromium,
A/V sync, or the ten-minute acceptance gate.

## Proven path

```text
guest rootfs-full
  pure-Rust MP4 demux + PTS pacing
    -> H.264 access units
    -> memfd ring + AF_UNIX control socket
app process
  -> c2.mtk.avc.decoder
  -> AImageReader(AIMAGE_FORMAT_PRIVATE)
  -> AHardwareBuffer
  -> persistent SurfaceControl child "husk-video"
  -> ASurfaceTransaction_setBuffer / setGeometry / setOnComplete
  -> SurfaceFlinger
```

Each submitted AHB is retained with `AHardwareBuffer_acquire`. The completion
callback waits on the previous SurfaceFlinger release fence before calling
`AHardwareBuffer_release`. Video is letterboxed to the current NativeActivity
window; the measured 1920x1080 output rectangle is
`(0,896)-(1080,1504)` on the 1080x2400 display.

## Real-time result

Guest logs: `71-video-realtime-guest-1.log` and
`71-video-realtime-guest-2.log`.

```text
guest|OPEN|media=/tmp/husk-video/avc-1080p30.mp4|track=1|size=1920x1080|timescale=15360|samples=150|send=150|length_size=4
OPEN_READY slots=4 slot_size=2097152 bytes=8392960 codec=c2.mtk.avc.decoder
EOS_OK frames=151 images=150 output_bytes=150
CLOSE_OK mime=video/avc codec=c2.mtk.avc.decoder size=1920x1080 frames=151 output_bytes=150 images_last=150 stop_rc=0 delete_rc=0
guest|DONE|samples=150|bytes=9821765|elapsed_ms=5099
RESULT PASS
```

The second session reported `elapsed_ms=5094` and the same `150` samples,
`150` images, clean stop/delete, and `RESULT PASS`.

## Presenter result

Raw app log: `71-video-realtime-app.log`.

```text
video|present|control|PASS|parent=...|control=...
video|present|geometry|screen=1080x2400|video=1920x1080|dest=(0,896)-(1080,1504)
video|present|submit|n=150|1920x1080|previous=true|completed=149
video|present|clear|submitted=150|completed=150|released=150
video|present|complete|n=150|slot=149|released=true|submitted=150|released_count=150
video|bridge|CLOSE_OK ... frames=151 output_bytes=150 images_last=150 stop_rc=0 delete_rc=0
```

The app log contains two consecutive sessions. The second session begins with
`video|present|clear|submitted=150|completed=150|released=150`, then counter
reset, and repeats `150` submissions/completions. No `FATAL`, `FORTIFY`, or
fatal signal appears in these sessions.

## Screenshots

- `71-video-realtime-surface-1.png`: first independent session.
- `71-video-realtime-surface-2.png`: second independent session.

Both show the full seven-colour test pattern and changing frame time/marker
positions. Their SHA-256 values differ, and the files are stored without
post-processing.

## Resource observation

Raw observation: `71-video-realtime-resources.log`.

Across two further consecutive 5-second sessions, app fd count moved
`135 -> 133 -> 135` and RSS stayed approximately 155 MiB. Across the earlier
four-session run the same 133-135 fd range repeated. This is evidence of a
bounded range over eight short sessions, **not** a ten-minute leak gate.

## Test media

```text
/tmp/husk-video-test/avc-1080p30.mp4
1920x1080, 30 fps, 150 frames, AVC
sha256 a87c06a79b338a9d28f68d1676c9aac7e2b5e72c811e6b1bcd55e2f6dd3cc858
```

## Reproduction

```sh
LIB=$(adb -s UKHMMB8PYTGECMNV shell 'run-as com.linuxhub.husk cat files/libdir' | tr -d '\r')
R=/data/user/0/com.linuxhub.husk/files/rootfs-full

adb -s UKHMMB8PYTGECMNV shell \
  "run-as com.linuxhub.husk sh -c 'cd /data/user/0/com.linuxhub.husk/files && \
   HUSK_VIDEO_REALTIME=1 NR_LD=$LIB/libld.so timeout 30 sh gexec2.sh rootfs-full \
   $R/usr/local/bin/video-bridge /run/husk/video.sock \
   /tmp/husk-video/avc-1080p30.mp4 150'"
```

Required markers/tree:

```text
files/rootfs-full/run/husk/enabled
files/rootfs-full/run/husk/present
files/rootfs-full/run/husk/video.sock
```

## Artifact hashes

```text
285e4f7135f329f31feee0655bf02a6277fba00429138cce5cc1d6ce25667968  app-debug.apk
05e39bfff905bc0e8225cf5934cb434191fb44a8f67c393b749bea46d38a300f  libhuskapp.so
e8dadebd659daa399c416ea5b1677dfdb993f5dc8cc32894739043f7bcfbc18e  video-bridge
3f94651ff4d09f5c23d5226ac0b591fa5bdfe0f320d141044bc532c5b87ae0fb  71-video-realtime-surface-1.png
654d59e098bcbac17c06f33af43e02af322d4877e3e01a0595e91c664c30592c  71-video-realtime-surface-2.png
```

## Boundaries / next work

- **NOT DONE**: export/import the decoded AHB into guest PanVK through
  `MEM_IMPORT`; current presentation happens in the app process, not in guest
  GL/X11/Wayland.
- **CLOSED BY EVIDENCE 72**: presenter now refreshes the JNI SurfaceControl,
  retires the old child layer by callback generation, rebuilds under the new
  parent, and resumes video after Home/foreground recovery.
- **NOT DONE**: rotation/DPI/multi-window gate and a ten-minute playback soak.
- **NOT DONE**: HEVC/VP9/AV1 over the bridge. Only the independent app-side
  HEVC decode probe from evidence 67 passed.
- **NOT DONE**: GStreamer/VA-API, Chromium `<video>`, A/V synchronization,
  dropped-frame accounting, and hardware-vs-software CPU comparison.
- **UNKNOWN**: relevance of the 133-135 fd oscillation without a full
  ten-minute trace; this evidence does not classify it as a leak or as leak-free.
