# Evidence 69 — Guest compressed-video bridge v1

Date: 2026-11-23
Device: RMX3357 / MediaTek MT6893 / Dimensity 1200 / Mali-G77 MC9 /
Android 13 / kernel 4.19.191+ / 4 KiB pages / arm64
Repository: `$REPO`

## Conclusion

**FACT: v1 data-plane PASS.** A glibc guest rooted at the private
`files/rootfs-full` tree demuxed the 1920x1080@30 AVC test file with the pure-Rust
MP4 reader, transferred 150 access units over the memfd ring, and the app process
decoded them with `c2.mtk.avc.decoder`. The bridge produced 150
`AHardwareBuffer` outputs and completed `EOS`, codec stop/delete, and connection
cleanup without an error. The final guest run completed in 1599 ms.

This closes demux/transport/bionic-hardware-decode transport only. It does not
close `MEM_IMPORT` back into the guest, display of decoded frames, GStreamer or
Chromium integration, A/V sync, or a ten-minute playback gate.

## Proven path

```text
guest rootfs-full
  pure-Rust MP4 demux
    -> H.264 Annex-B access units + CSD/PTS/flags
    -> memfd 4-slot ring (4 x 2 MiB)
    -> AF_UNIX control socket inside rootfs-full
app process
    -> c2.mtk.avc.decoder
    -> AImageReader(AIMAGE_FORMAT_PRIVATE)
    -> 150 AHardwareBuffer outputs
```

The app control socket was moved from `files/video-bridge.sock` to
`files/rootfs-full/tmp/husk-video/bridge.sock`, so normal `rootfs-full` isolation
does not have to be disabled. The current path shim does not translate
`connect(2)` socket paths, so the harness passes the private physical socket path
to the guest; the media path remains the guest-visible
`/tmp/husk-video/avc-1080p30.mp4`.

## Guest result

Raw log: `69-video-guest-bridge-guest.log`.

```text
guest|OPEN|media=/tmp/husk-video/avc-1080p30.mp4|track=1|size=1920x1080|timescale=15360|samples=150|send=150|length_size=4
PONG bridge=video-v1
OPEN_READY slots=4 slot_size=2097152 bytes=8392960 codec=c2.mtk.avc.decoder
guest|RING|READY|slots=4|slot_size=2097152|bytes=8392960
guest|SAMPLE|seq=150|sent=150/150|len=67253|pts_us=4966666|key=false|ack=SAMPLE_OK seq=150 frames=141 images=141
EOS_OK frames=151 images=150 output_bytes=150
CLOSE_OK mime=video/avc codec=c2.mtk.avc.decoder size=1920x1080 frames=151 output_bytes=150 images_last=150 stop_rc=0 delete_rc=0
guest|DONE|samples=150|bytes=9821765|elapsed_ms=1599
RESULT PASS
HOST_RC=0
```

## Repeated app-side sessions

Raw log: `69-video-guest-bridge-app.log`.

The app-side log contains 14 completed sessions, all with the same result:

```text
video|bridge|OPEN|OPEN_READY slots=4 slot_size=2097152 bytes=8392960 codec=c2.mtk.avc.decoder
video|bridge|EOS_OK frames=151 images=150 output_bytes=150
video|bridge|CLOSE_OK ... frames=151 output_bytes=150 images_last=150 stop_rc=0 delete_rc=0
```

## Reproduction

```sh
F=/data/data/com.linuxhub.husk/files
R=$F/rootfs-full
LIB=$(adb -s UKHMMB8PYTGECMNV shell 'run-as com.linuxhub.husk cat files/libdir' | tr -d '\r')

adb -s UKHMMB8PYTGECMNV shell \
  "run-as com.linuxhub.husk sh -c 'mkdir -p $R/tmp/husk-video && cp $F/avc-1080p30.mp4 $R/tmp/husk-video/avc-1080p30.mp4'"

adb -s UKHMMB8PYTGECMNV shell \
  "run-as com.linuxhub.husk sh -c 'cd $F && NR_LD=$LIB/libld.so timeout 20 sh gexec2.sh rootfs-full $R/usr/local/bin/video-bridge $R/tmp/husk-video/bridge.sock /tmp/husk-video/avc-1080p30.mp4 150 2>&1; echo HOST_RC=\$?'"
```

The bridge marker/control socket must exist before the guest starts:

```sh
adb -s UKHMMB8PYTGECMNV shell \
  'run-as com.linuxhub.husk sh -c "touch files/video-bridge; ls -l files/rootfs-full/tmp/husk-video/bridge.sock"'
```

## Artifact hashes

```text
4bc472f33a88d7c0599b80ff91cfb2100bf8610de61f6db76a99baa2c2c3c386  installed/base APK
4bc472f33a88d7c0599b80ff91cfb2100bf8610de61f6db76a99baa2c2c3c386  app-debug.apk
62226163c5e259d9a774296d7e2736e82a96069c8c4f0298defe938d9d5643f6  libhuskapp.so
a7c869f0e695b0a9757f639dbaba191e4f48aefb969e63d9501da32e8e5bd08f  video-bridge
```

## Boundaries

- **FACT**: v1 accepts one AVC track and the exact `c2.mtk.avc.decoder` name.
- **NOT DONE**: HEVC/VP9/AV1 over the bridge. Only the independent app-side
  HEVC probe from evidence 67 passed.
- **NOT DONE**: import the decoded AHB dma-buf into guest PanVK through `MEM_IMPORT`
  and display it through the existing zero-copy SurfaceControl ring.
- **NOT DONE**: GStreamer/VA-API, Chromium `<video>`, A/V sync, dropped-frame
  accounting, ten-minute playback, CPU comparison, and desktop-player integration.
- **CLOSED BY EVIDENCE 70**: `connect(2)` pathname translation in the shim is now
  implemented and verified with the guest-visible `/run/husk/video.sock` path.
- **UNKNOWN**: other devices and 16/64 KiB kernels were not run for this bridge.
