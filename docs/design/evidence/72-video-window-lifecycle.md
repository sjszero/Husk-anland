# Evidence 72 — Video survives rotation and SurfaceControl replacement

Date: 2026-11-23
Device: RMX3357 / MediaTek MT6893 / Dimensity 1200 / Mali-G77 MC9 /
Android 13 / kernel 4.19.191+ / 4 KiB pages / arm64
Repository: `$REPO`

## Conclusion

**FACT: active hardware-decoded video survives rotation and a real parent
SurfaceControl replacement without restarting the app or the decode session.**

- Rotation from 1080x2400 portrait to 2280x1080 landscape happened while the
  5-second AVC stream was playing. The guest still completed `150/150` samples
  with `RESULT PASS`; the presenter re-laid the video from
  `(0,896)-(1080,1504)` to `(180,0)-(2100,1080)` and continued submitting frames.
- A Home/background/foreground cycle destroyed the old NativeWindow. The guest
  continued decoding, but frames arriving without a live window were not
  submitted. On return, the JNI parent refresh detected a new BLAST
  SurfaceControl, and the presenter rebuilt its child layer:

```text
surface: parent|CHANGED|old=0xb400007aa62f19a0|new=0xb400007aca281060
video|present|rebuild|PASS|old_parent=0xb400007aa62f19a0|new_parent=0xb400007aca281060|control=0xb400007a2212a3c0|retired=0
```

The rebuilt layer immediately accepted new frames and the resumed session
ended with codec stop/delete and `RESULT PASS`.

This closes the previous evidence-71 boundary: **parent SurfaceControl changes
no longer reject every later frame**. It does not close guest-side
GL/X11/Wayland WSI, ten-minute playback, AV sync, or `MEM_IMPORT`.

## Rotation during playback

Raw logs:

- `72-video-rotation-guest.log`
- `72-video-rotation-app.log`

Guest result:

```text
guest|SAMPLE|seq=150|sent=150/150|pts_us=4966666|ack=SAMPLE_OK seq=150 frames=149 images=149
EOS_OK frames=151 images=150 output_bytes=150
CLOSE_OK ... frames=151 output_bytes=150 images_last=150 stop_rc=0 delete_rc=0
guest|DONE|samples=150|bytes=9821765|elapsed_ms=5097
RESULT PASS
```

Presenter geometry transition:

```text
video|present|geometry|screen=1080x2400|video=1920x1080|dest=(0,896)-(1080,1504)
cb: onNativeWindow win=... 2280x1080
video|present|geometry|screen=2280x1080|video=1920x1080|dest=(180,0)-(2100,1080)
video|present|complete|n=150|slot=149|released=true|submitted=150|released_count=150
```

The screenshots `72-video-rotation-landscape.png` and
`72-video-rotation-finished.png` show different video timestamps in landscape.
No `FORTIFY`, fatal signal, or app restart occurred.

## SurfaceControl replacement during playback

Raw logs:

- `72-video-surface-rebuild-guest.log`
- `72-video-surface-rebuild-app.log`

Screenshot after returning to the foreground:
`72-video-surface-rebuild-resumed.png`.

Relevant sequence:

```text
cb: onWindowFocusChanged focused=0
cb: onNativeWindowDestroyed win=...
video|present|SUBMIT|FAIL
cb: onNativeWindow win=... 1080x2400
cb: onWindowFocusChanged focused=1
surface: parent|CHANGED|old=...|new=...
video|present|rebuild|PASS|old_parent=...|new_parent=...|control=...|retired=0
video|present|geometry|screen=1080x2400|video=1920x1080|dest=(0,896)-(1080,1504)
```

The guest still completed the file and returned `RESULT PASS`. The presenter
statistics at session close were `submitted=119`, because decoded frames that
arrived while the app had no live Android window were intentionally skipped.
This is graceful frame dropping, not a crash and not a claim that every frame
was displayed while backgrounded.

## Repeated rebuild / resource check

Raw logs:

- `72-video-repeated-rebuild-guest.log`
- `72-video-repeated-rebuild-app.log`

This run exercised the rebuild path again on the same app process. The next
session detected another parent generation and rebuilt without retaining the
previous child:

```text
surface: parent|CHANGED|old=0xb400007aca281060|new=0xb400007aca281100
video|present|rebuild|PASS|old_parent=0xb400007aca281060|new_parent=0xb400007aca281100|control=0xb400007aa54a96a0|retired=0
```

App fd count was 137 before and 137 after; RSS was approximately 158 MiB at
the end. This is a short repeated-rebuild check, not a ten-minute leak gate.

## Implementation

- `app-rs/src/surface.rs` now refreshes the JNI SurfaceControl pointer at most
  once per second instead of caching the first pointer for the process lifetime.
- `app-rs/src/video/present.rs` tracks callback generations. On parent change it
  creates a child under the new parent, clears the old layer, moves the old
  `SurfaceControl` into a retired list, and releases it only after all callbacks
  that reference that generation have completed.
- `CallbackDone` decrements the generation counter on every callback exit,
  including panic unwinding caught by `catch_unwind`.
- Geometry logging now includes window size, so resize events are visible even
  when the decoded frame size is unchanged.

## Reproduction

```sh
LIB=$(adb -s UKHMMB8PYTGECMNV shell 'run-as com.linuxhub.husk cat files/libdir' | tr -d '\r')
R=/data/user/0/com.linuxhub.husk/files/rootfs-full

adb -s UKHMMB8PYTGECMNV shell cmd window user-rotation lock 0
adb -s UKHMMB8PYTGECMNV shell am start \
  -n com.linuxhub.husk/android.app.NativeActivity

adb -s UKHMMB8PYTGECMNV shell \
  "run-as com.linuxhub.husk sh -c 'cd /data/user/0/com.linuxhub.husk/files && \
   HUSK_VIDEO_REALTIME=1 NR_LD=$LIB/libld.so timeout 30 sh gexec2.sh rootfs-full \
   $R/usr/local/bin/video-bridge /run/husk/video.sock \
   /tmp/husk-video/avc-1080p30.mp4 150'"
```

During the command, run either:

```sh
adb -s UKHMMB8PYTGECMNV shell cmd window user-rotation lock 1
```

or press Home and bring the Activity back to the foreground.

## Artifact hashes

```text
e1e6a98ba863fd63baa0aa400a78f4b0c189de13328faad0c8a59f293681bfe1  app-debug.apk
bfc6c56ef06655386494a8c46ccb89708e25994946e062a657e666b4710e8217  libhuskapp.so
e8dadebd659daa399c416ea5b1677dfdb993f5dc8cc32894739043f7bcfbc18e  video-bridge
bda5daab0c1bae965b69bfd6bfb624d057ea2245dca3351cec92c44af97dac45  72-video-rotation-landscape.png
a22aad8a369e1384296c05075a1d1ca2756e55f472c0ae27776c2aa89a64758e  72-video-rotation-finished.png
84ac8d34a3ecf3f0436dfc331e4e2ba7068b57020e36594a4d9cdfc427dc4ae7  72-video-surface-rebuild-resumed.png
```

## Remaining boundaries

- **NOT DONE**: guest-side GL/GBM/Wayland/X11 WSI and swapchain.
- **NOT DONE**: ten-minute playback with dropped-frame and fd/RSS curves.
- **NOT DONE**: AV sync, GStreamer/VA-API, Chromium, HEVC/VP9/AV1 bridge.
- **NOT DONE**: import the decoded AHB into the guest through kbase `MEM_IMPORT`.
- **UNKNOWN**: multi-window and split-screen behavior was not exercised in this
  evidence.
