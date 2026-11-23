# Evidence 83 — One hardware capability matrix for app and guest

Date: 2026-11-23
Device: RMX3357 / MediaTek MT6893 / Dimensity 1200 / Mali-G77 MC9 /
Android 13 / kernel 4.19.191 / 4 KiB pages / arm64
Repository: `/Users/l/Documents/Codex/2026-08-10/husk`

## Conclusion

**FACT: the app now measures, from inside its own untrusted_app process, what
the platform exposes for every hardware class the goal names, and writes it to
one file both sides can read.**

`files/husk-capabilities.txt` (canonical) and
`rootfs-full/run/husk/capabilities.txt` (guest-visible mirror) carry the same
rows. Nothing is inferred from a node existing: `present=` is what the framework
API returned in this process, `husk=` is a separate claim about our own code.

This does **not** implement any of the missing hardware paths. It replaces
guessing with a measured baseline, which is what the remaining work has to be
planned against.

## What is measured, and how

| class | source of `present=` |
|---|---|
| gpu | `android.os.Build.SOC_MODEL/HARDWARE` identity (node evidence is separate) |
| display | `SurfaceView` install state + SurfaceControl handle cache state |
| video-decode / video-encode | `MediaCodecList(ALL_CODECS).getCodecInfos()`, `isHardwareAccelerated()`, `getSupportedTypes()` |
| audio-out / audio-in | `AudioManager.getDevices(GET_DEVICES_OUTPUTS/INPUTS)` |
| camera | `CameraManager.getCameraIdList()` |
| input | `InputManager.getInputDeviceIds()` + `getName()` / `getSources()` |
| sensors | `SensorManager.getSensorList(TYPE_ALL)` |
| platform-peripherals | `PackageManager.hasSystemFeature(...)` for usb.host / bluetooth(+le) / location.gps / nfc / fingerprint / camera.any / sensor.* |

All calls target platform classes through JNI. No Java of ours, no dex, no C.

Two traps found while wiring it:

- `MediaCodecList.getCodecCount()` is **static** in AOSP; the instance lookup
  raised `NoSuchMethodError: no non-static method
  "Landroid/media/MediaCodecList;.getCodecCount()I"` on this ROM. The supported
  instance API `getCodecInfos()` is used instead.
- The SurfaceControl handle is read lazily on the first guest present, so a `0`
  there is *not* "display bridge missing". The row now says
  `surfaceview=installed, control-cache=not-fetched (lazy: ...)` instead of
  claiming the bridge was absent.

## Real-device result

Raw: `83-capability-matrix.log`.

```text
caps|gpu|present=yes|husk=degraded|backend=/dev/mali0 (kbase)|detail=model=RMX3357 device=RE5469 soc=MT6893 board=RM6893 sdk=33 abis=arm64-v8a,armeabi-v7a,armeabi
caps|display|present=yes|husk=ready|backend=AHardwareBuffer -> dma-buf -> SurfaceControl|detail=surfaceview=installed, control-cache=not-fetched (lazy: read on first guest present)
caps|video-decode|present=yes|husk=degraded|backend=c2.mtk.avc.decoder|detail=hw_dec=37 sw_dec=33 hw_enc=10 sw_enc=17 hw_dec_types=video/avc
caps|video-encode|present=yes|husk=absent|backend=c2.mtk.avc.encoder|detail=types=video/avc
caps|audio-out|present=yes|husk=degraded|backend=AAudio + PulseAudio pipe|detail=devices=1,2,18,20,20,20,20 (types)
caps|audio-in|present=yes|husk=absent|backend=-|detail=devices=15,15,16,28,18,25 (types)
caps|camera|present=yes|husk=absent|backend=-|detail=ids=0,1
caps|input|present=yes|husk=unknown|backend=-|detail=devices=-1:Virtual(0x301),2:mtk-kpd(0x101),3:touchpanel(0x1103),4:ACCDET(0x80000101)
caps|sensors|present=yes|husk=absent|backend=-|detail=count=31 (accel, mag, gyro, proximity, light, step, ...)
caps|platform-peripherals|present=yes|husk=absent|backend=-|detail=usb-host=y bluetooth=y bluetooth-le=y gps=y nfc=y fingerprint=y camera-any=y sensor-*=y gles=y vulkan=y
```

Counts are FACTs from this device: 37 hardware decoders, 10 hardware encoders,
2 cameras, 31 sensors, 6 audio input and 7 audio output device entries, 4 input
devices, and every peripheral feature the goal lists reports present.

## What this changes in the plan

Before this file, "does the device even have NFC / a hardware encoder / a rear
ALS" was a question. It is now a table. Consequences:

- Video encode is **not** blocked by the device (hardware `c2.mtk.avc.encoder`
  exists); it is blocked by us having no encoder path.
- Audio input devices exist; only our capture path is missing.
- Camera 0 and 1 exist; only our path is missing.
- `input` is the one row marked `husk=unknown` rather than `absent`: the devices
  are visible (`touchpanel`, `mtk-kpd`), but nothing routes them into the guest
  yet, so "absent" would be an assertion we have not made.

## Guest-side read

`rootfs-full/run/husk/capabilities.txt` exists with the same 3034-byte content
(see log). Guest read of `/run/husk/capabilities.txt` is **INFERENCE**, not
verified this round: evidence 70 already proved path-shim maps guest
`/run/husk/<name>` onto `rootfs-full/run/husk/<name>` for `connect(2)`, and this
file sits at exactly that path, but no guest process has `cat`-ed it yet.

## Not done

Nothing in this evidence drives GPU, display, video, audio, camera, input,
sensors or peripherals. It is a measurement layer only; every `husk=absent` row
above is still an open work item.
