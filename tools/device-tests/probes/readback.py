#!/usr/bin/env python3
"""Independent whole-block glReadPixels probe (ctypes, no Rust, no gl-probe).

Why this exists: evidence 179 attributed the "destination row pitch is always
16 bytes" defect to Mesa's pack layer on the strength of three environments
agreeing -- but all three ran the *same* Rust probe. This one shares no code
with it: ctypes -> libEGL.so.1/libGLESv2.so.2 directly.

Method
  1. EGL surfaceless, ES3 context, no window.
  2. RGBA8 texture whose every pixel is unique: R=x*7+3, G=y*11+5,
     B=(13x+17y)%251, A=255  (R/G invert uniquely for x<37, y<23).
  3. Ground truth = per-pixel 1x1 reads (the path evidence 179 says is good),
     so a broken texture *upload* cannot masquerade as a broken readback.
  4. Whole-block WxH read into a 0xAA-filled buffer, then for every written
     linear offset, decode which source cell landed there.

usage: readback.py [W] [H] [tag]
"""
import ctypes
import sys
import time

GL_RGBA = 0x1908
GL_UNSIGNED_BYTE = 0x1401
GL_TEXTURE_2D = 0x0DE1
GL_RGBA8 = 0x8058
GL_FRAMEBUFFER = 0x8D40
GL_COLOR_ATTACHMENT0 = 0x8CE0
GL_FRAMEBUFFER_COMPLETE = 0x8CD5
GL_PACK_ALIGNMENT = 0x0D05
GL_UNPACK_ALIGNMENT = 0x0CF5
GL_PACK_ROW_LENGTH = 0x0D02

EGL_PLATFORM_SURFACELESS_MESA = 0x31DD
EGL_OPENGL_ES_API = 0x30A0
EGL_SURFACE_TYPE = 0x3033
EGL_PBUFFER_BIT = 0x0001
EGL_RENDERABLE_TYPE = 0x3040
EGL_OPENGL_ES3_BIT = 0x00000040
EGL_RED_SIZE = 0x3024
EGL_GREEN_SIZE = 0x3023
EGL_BLUE_SIZE = 0x3022
EGL_ALPHA_SIZE = 0x3021
EGL_NONE = 0x3038
EGL_CONTEXT_CLIENT_VERSION = 0x3098


def main():
    W = int(sys.argv[1]) if len(sys.argv) > 1 else 16
    H = int(sys.argv[2]) if len(sys.argv) > 2 else 16
    tag = sys.argv[3] if len(sys.argv) > 3 else "default"

    egl = ctypes.CDLL("libEGL.so.1")
    gles = ctypes.CDLL("libGLESv2.so.2")

    egl.eglGetProcAddress.restype = ctypes.c_void_p
    egl.eglGetProcAddress.argtypes = [ctypes.c_char_p]
    addr = egl.eglGetProcAddress(b"eglGetPlatformDisplayEXT")
    if not addr:
        print("FATAL no eglGetPlatformDisplayEXT")
        return 2
    proto = ctypes.CFUNCTYPE(ctypes.c_void_p, ctypes.c_uint, ctypes.c_void_p, ctypes.c_void_p)
    get_display = proto(addr)
    dpy = get_display(EGL_PLATFORM_SURFACELESS_MESA, None, None)
    if not dpy:
        print("FATAL eglGetPlatformDisplayEXT returned NULL")
        return 2

    egl.eglInitialize.argtypes = [ctypes.c_void_p, ctypes.POINTER(ctypes.c_int),
                                  ctypes.POINTER(ctypes.c_int)]
    egl.eglInitialize.restype = ctypes.c_uint
    maj, mnr = ctypes.c_int(), ctypes.c_int()
    if not egl.eglInitialize(dpy, ctypes.byref(maj), ctypes.byref(mnr)):
        print("FATAL eglInitialize failed 0x%x" % egl.eglGetError())
        return 2

    egl.eglBindAPI.argtypes = [ctypes.c_uint]
    egl.eglBindAPI.restype = ctypes.c_uint
    if not egl.eglBindAPI(EGL_OPENGL_ES_API):
        print("FATAL eglBindAPI failed 0x%x" % egl.eglGetError())
        return 2

    attrs = (ctypes.c_int * 15)(
        EGL_SURFACE_TYPE, EGL_PBUFFER_BIT,
        EGL_RENDERABLE_TYPE, EGL_OPENGL_ES3_BIT,
        EGL_RED_SIZE, 8, EGL_GREEN_SIZE, 8, EGL_BLUE_SIZE, 8, EGL_ALPHA_SIZE, 8,
        EGL_NONE, 0, 0)
    cfg = ctypes.c_void_p()
    n = ctypes.c_int()
    egl.eglChooseConfig.argtypes = [ctypes.c_void_p, ctypes.POINTER(ctypes.c_int),
                                    ctypes.POINTER(ctypes.c_void_p), ctypes.c_int,
                                    ctypes.POINTER(ctypes.c_int)]
    egl.eglChooseConfig.restype = ctypes.c_uint
    if not egl.eglChooseConfig(dpy, attrs, ctypes.byref(cfg), 1, ctypes.byref(n)) or n.value < 1:
        print("FATAL eglChooseConfig failed")
        return 2

    ctxattrs = (ctypes.c_int * 3)(EGL_CONTEXT_CLIENT_VERSION, 3, EGL_NONE)
    egl.eglCreateContext.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_void_p,
                                     ctypes.POINTER(ctypes.c_int)]
    egl.eglCreateContext.restype = ctypes.c_void_p
    ctx = egl.eglCreateContext(dpy, cfg, None, ctxattrs)
    if not ctx:
        print("FATAL eglCreateContext failed 0x%x" % egl.eglGetError())
        return 2
    egl.eglMakeCurrent.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_void_p,
                                   ctypes.c_void_p]
    egl.eglMakeCurrent.restype = ctypes.c_uint
    if not egl.eglMakeCurrent(dpy, None, None, ctx):
        print("FATAL eglMakeCurrent failed 0x%x" % egl.eglGetError())
        return 2

    gles.glGetString.restype = ctypes.c_char_p
    gles.glGetString.argtypes = [ctypes.c_uint]
    renderer = gles.glGetString(0x1F01) or b"?"
    version = gles.glGetString(0x1F02) or b"?"
    print("TAG %s  GL_RENDERER=%s  GL_VERSION=%s" %
          (tag, renderer.decode("utf-8", "replace"), version.decode("utf-8", "replace")))

    for name, restype, argtypes in (
        ("glGenTextures", None, [ctypes.c_int, ctypes.POINTER(ctypes.c_uint)]),
        ("glBindTexture", None, [ctypes.c_uint, ctypes.c_uint]),
        ("glTexImage2D", None, [ctypes.c_uint, ctypes.c_int, ctypes.c_int, ctypes.c_int,
                                ctypes.c_int, ctypes.c_int, ctypes.c_uint, ctypes.c_uint,
                                ctypes.c_void_p]),
        ("glGenFramebuffers", None, [ctypes.c_int, ctypes.POINTER(ctypes.c_uint)]),
        ("glBindFramebuffer", None, [ctypes.c_uint, ctypes.c_uint]),
        ("glFramebufferTexture2D", None, [ctypes.c_uint, ctypes.c_uint, ctypes.c_uint,
                                          ctypes.c_uint, ctypes.c_int]),
        ("glCheckFramebufferStatus", ctypes.c_uint, [ctypes.c_uint]),
        ("glPixelStorei", None, [ctypes.c_uint, ctypes.c_int]),
        ("glReadPixels", None, [ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_int,
                                ctypes.c_uint, ctypes.c_uint, ctypes.c_void_p]),
        ("glGetError", ctypes.c_uint, []),
        ("glGetIntegerv", None, [ctypes.c_uint, ctypes.POINTER(ctypes.c_int)]),
    ):
        fn = getattr(gles, name)
        fn.restype = restype
        fn.argtypes = argtypes

    tex = ctypes.c_uint()
    gles.glGenTextures(1, ctypes.byref(tex))
    gles.glBindTexture(GL_TEXTURE_2D, tex.value)
    gles.glPixelStorei(GL_UNPACK_ALIGNMENT, 1)
    src_bytes = bytearray(W * H * 4)
    for y in range(H):
        for x in range(W):
            o = 4 * (y * W + x)
            src_bytes[o] = (x * 7 + 3) & 0xFF
            src_bytes[o + 1] = (y * 11 + 5) & 0xFF
            src_bytes[o + 2] = (13 * x + 17 * y) % 251
            src_bytes[o + 3] = 255
    upload = (ctypes.c_ubyte * len(src_bytes)).from_buffer(src_bytes)
    gles.glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA8, W, H, 0, GL_RGBA,
                      GL_UNSIGNED_BYTE, ctypes.cast(upload, ctypes.c_void_p))

    fbo = ctypes.c_uint()
    gles.glGenFramebuffers(1, ctypes.byref(fbo))
    gles.glBindFramebuffer(GL_FRAMEBUFFER, fbo.value)
    gles.glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D,
                                tex.value, 0)
    st = gles.glCheckFramebufferStatus(GL_FRAMEBUFFER)
    if st != GL_FRAMEBUFFER_COMPLETE:
        print("FATAL fbo incomplete 0x%x" % st)
        return 2

    def read1(x, y):
        b = (ctypes.c_ubyte * 4)()
        gles.glReadPixels(x, y, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, ctypes.cast(b, ctypes.c_void_p))
        return bytes(b)

    # Ground truth: per-pixel 1x1 (known-good path), cross-checked against the
    # pattern we uploaded, so an upload defect cannot poison the comparison.
    mode = sys.argv[4] if len(sys.argv) > 4 else "rgba"

    truth = {}
    upload_bad = 0
    if mode != "cost":
      for y in range(H):
        for x in range(W):
            v = read1(x, y)
            want = bytes(bytearray([(x * 7 + 3) & 0xFF, (y * 11 + 5) & 0xFF,
                                    (13 * x + 17 * y) % 251, 255]))
            truth[(x, y)] = v
            if v != want:
                upload_bad += 1
      print("SOURCE 1x1-read map: %d/%d pixels differ from uploaded pattern "
            "(upload/pack-read identity mismatches)" % (upload_bad, W * H))
    back = {}
    for (x, y), v in truth.items():
        back.setdefault(v, []).append((x, y))

    def whole_read(row_length, align):
        gles.glPixelStorei(GL_PACK_ROW_LENGTH, row_length)
        gles.glPixelStorei(GL_PACK_ALIGNMENT, align)
        while gles.glGetError():
            pass
        buf = bytearray([0xAA] * (W * H * 4 + 8192))  # +8 KiB canary slack
        arr = (ctypes.c_ubyte * len(buf)).from_buffer(buf)
        gles.glReadPixels(0, 0, W, H, GL_RGBA, GL_UNSIGNED_BYTE,
                          ctypes.cast(arr, ctypes.c_void_p))
        err = gles.glGetError()
        return bytes(buf), err

    def analyse(buf):
        """Map every written linear offset back to the source cell it holds."""
        limit = W * H * 4
        placed = {}          # (x, y) -> byte offset it landed at
        dupes = 0
        for off in range(0, limit, 4):
            v = buf[off:off + 4]
            if v == b"\xaa\xaa\xaa\xaa":
                continue
            hits = back.get(v)
            if not hits:
                dupes += 1
                continue
            x, y = hits[0]
            if (x, y) in placed:
                dupes += 1
            else:
                placed[(x, y)] = off
        over = [i for i in range(limit, len(buf)) if buf[i] != 0xAA]
        return placed, dupes, over

    def report(label, row_length, align):
        buf, err = whole_read(row_length, align)
        placed, dupes, over = analyse(buf)
        correct = sum(1 for (x, y), off in placed.items() if off == 4 * (y * W + x))
        print("BLOCK %-20s written=%d/%d correct=%d err=0x%x dup=%d overrun=%d"
              % (label, len(placed), W * H, correct, err, dupes, len(over)))
        if over:
            print("      OVERRUN first=%d last=%d (limit=%d)" % (over[0], over[-1], W * H * 4))
        pitch = placed.get((0, 1), 0) - placed.get((0, 0), 0) if (0, 0) in placed else 0
        rowpx = max((x for (x, y) in placed if y == 0), default=-1) + 1
        rows = sorted({y for (x, y) in placed})
        print("      dest_pitch_bytes=%d (=%d px)  row0_px=%d  rows_present=%d%s"
              % (pitch, pitch // 4, rowpx, len(rows),
                 "  rows=%s" % (rows if len(rows) <= 8 else "%d..%d" % (rows[0], rows[-1]))))
        if W * H <= 512:
            inv = sorted(((off, xy) for xy, off in placed.items()))
            cells = ["%d=%d,%d" % (off, xy[0], xy[1]) for off, xy in inv]
            for i in range(0, len(cells), 8):
                print("      " + " ".join(cells[i:i + 8]))
        return placed

    def pbo_read(label):
        """Same read into a PBO: isolates client-memory packing from the blit."""
        GL_PIXEL_PACK_BUFFER = 0x88EB
        GL_STREAM_READ = 0x88E1
        GL_MAP_READ_BIT = 0x0001
        size = W * H * 4
        for name, restype, argtypes in (
            ("glGenBuffers", None, [ctypes.c_int, ctypes.POINTER(ctypes.c_uint)]),
            ("glBindBuffer", None, [ctypes.c_uint, ctypes.c_uint]),
            ("glBufferData", None, [ctypes.c_uint, ctypes.c_ssize_t, ctypes.c_void_p,
                                    ctypes.c_uint]),
            ("glMapBufferRange", ctypes.c_void_p, [ctypes.c_uint, ctypes.c_ssize_t,
                                                   ctypes.c_ssize_t, ctypes.c_uint]),
            ("glUnmapBuffer", ctypes.c_ubyte, [ctypes.c_uint]),
        ):
            fn = getattr(gles, name)
            fn.restype = restype
            fn.argtypes = argtypes
        b = ctypes.c_uint()
        gles.glGenBuffers(1, ctypes.byref(b))
        gles.glBindBuffer(GL_PIXEL_PACK_BUFFER, b.value)
        gles.glBufferData(GL_PIXEL_PACK_BUFFER, size, None, GL_STREAM_READ)
        gles.glPixelStorei(GL_PACK_ROW_LENGTH, 0)
        gles.glPixelStorei(GL_PACK_ALIGNMENT, 4)
        while gles.glGetError():
            pass
        gles.glReadPixels(0, 0, W, H, GL_RGBA, GL_UNSIGNED_BYTE, ctypes.c_void_p(0))
        err = gles.glGetError()
        p = gles.glMapBufferRange(GL_PIXEL_PACK_BUFFER, 0, size, GL_MAP_READ_BIT)
        if not p:
            print("BLOCK %-20s MAP FAILED err=0x%x" % (label, err))
        else:
            buf = ctypes.string_at(p, size)
            placed, dupes, over = analyse(buf)
            correct = sum(1 for (x, y), off in placed.items() if off == 4 * (y * W + x))
            pitch = placed.get((0, 1), 0) - placed.get((0, 0), 0) if (0, 0) in placed else 0
            print("BLOCK %-20s written=%d/%d correct=%d err=0x%x dest_pitch_bytes=%d"
                  % (label, len(placed), W * H, correct, err, pitch))
            gles.glUnmapBuffer(GL_PIXEL_PACK_BUFFER)
        gles.glBindBuffer(GL_PIXEL_PACK_BUFFER, 0)

    def geti(pname):
        v = ctypes.c_int()
        gles.glGetIntegerv(pname, ctypes.byref(v))
        return v.value

    # What does GL *say* the pack state is? If the getter and the packer read the
    # same struct, these must predict the measured stride.
    def state(label):
        print("STATE %-18s PACK_ALIGNMENT=%d PACK_ROW_LENGTH=%d" %
              (label, geti(GL_PACK_ALIGNMENT), geti(GL_PACK_ROW_LENGTH)))

    gles.glPixelStorei(GL_PACK_ALIGNMENT, 4)
    gles.glPixelStorei(GL_PACK_ROW_LENGTH, 0)
    state("default")
    gles.glPixelStorei(GL_PACK_ROW_LENGTH, 32)
    state("rowlen=32")
    gles.glPixelStorei(GL_PACK_ROW_LENGTH, 0)
    gles.glPixelStorei(GL_PACK_ALIGNMENT, 8)
    state("align=8")
    gles.glPixelStorei(GL_PACK_ALIGNMENT, 4)

    mode = sys.argv[4] if len(sys.argv) > 4 else "rgba"

    if mode == "proc":
        # Bypass libglvnd: take Mesa's own entry points straight out of
        # eglGetProcAddress (what the EGL vendor lib hands to the dispatcher)
        # and repeat the block read. If this is correct while the dispatched
        # call is not, the bug lives in the glvnd dispatch, not in Mesa.
        real = egl.eglGetProcAddress(b"glReadPixels")
        realps = egl.eglGetProcAddress(b"glPixelStorei")
        glvnd = ctypes.cast(gles.glReadPixels, ctypes.c_void_p).value
        print("PTR glvnd_glReadPixels=%#x mesa_glReadPixels=%#x same=%s"
              % (glvnd, real or 0, glvnd == real))
        if not real:
            print("FATAL eglGetProcAddress(glReadPixels) is NULL")
            return 2
        rp = ctypes.CFUNCTYPE(None, ctypes.c_int, ctypes.c_int, ctypes.c_int,
                              ctypes.c_int, ctypes.c_uint, ctypes.c_uint,
                              ctypes.c_void_p)(real)
        ps = ctypes.CFUNCTYPE(None, ctypes.c_uint, ctypes.c_int)(realps) if realps else None
        for align in (1, 4, 8):
            if ps:
                ps(GL_PACK_ROW_LENGTH, 0)
                ps(GL_PACK_ALIGNMENT, align)
            while gles.glGetError():
                pass
            buf = bytearray([0xAA] * (W * H * 4 + 8192))
            arr = (ctypes.c_ubyte * len(buf)).from_buffer(buf)
            rp(0, 0, W, H, GL_RGBA, GL_UNSIGNED_BYTE,
               ctypes.cast(arr, ctypes.c_void_p))
            placed, dupes, over = analyse(bytes(buf))
            pitch = placed.get((0, 1), 0) - placed.get((0, 0), 0) if (0, 0) in placed else 0
            rowpx = max((x for (x, y) in placed if y == 0), default=-1) + 1
            correct = sum(1 for (x, y), off in placed.items() if off == 4 * (y * W + x))
            print("PROC align=%d written=%d/%d correct=%d pitch=%d row0_px=%d"
                  % (align, len(placed), W * H, correct, pitch, rowpx))
        return 0

    if mode == "sweep":
        # Width boundary sweep (round-5 deliverable): does the bogus stride depend
        # on width at all? H=3 keeps it cheap; pitch is derived from the last
        # touched byte, which equals (H-1)*pitch + W*bpp - 1 on a clean write.
        last_fail = []
        for w in range(1, 41):
            b = (ctypes.c_ubyte * (w * 3 * 4 + 4096))()
            for i in range(len(b)):
                b[i] = 0xAA
            gles.glPixelStorei(GL_PACK_ROW_LENGTH, 0)
            gles.glPixelStorei(GL_PACK_ALIGNMENT, 4)
            while gles.glGetError():
                pass
            gles.glReadPixels(0, 0, w, 3, GL_RGBA, GL_UNSIGNED_BYTE,
                              ctypes.cast(b, ctypes.c_void_p))
            err = gles.glGetError()
            last = max((i for i in range(len(b)) if b[i] != 0xAA), default=-1)
            pitch = (last + 1 - w * 4) / 2 if last >= 0 else -1
            ok = abs(pitch - w * 4) < 1
            if not ok:
                last_fail.append(w)
            print("SWEEP w=%2d last_off=%5d implied_pitch=%6.1f expect=%4d %s err=0x%x"
                  % (w, last, pitch, w * 4, "OK" if ok else "BAD", err))
        print("SWEEP broken_widths=%d %s" % (len(last_fail), last_fail))
        return 0

    if mode in ("rgb", "bgr"):
        # Third-party-format check: is the bogus stride specific to the RGBA8
        # pack, or does every 3/4-byte pack use Alignment in place of width?
        #   "width <- Alignment"  -> pitch 4 / 12 / 24  (bpp 3, align 1/4/8)
        #   correct               -> pitch 48 / 48 / 48
        GL_RGB = 0x1907
        bpp = 3
        for align in (1, 4, 8):
            gles.glPixelStorei(GL_PACK_ROW_LENGTH, 0)
            gles.glPixelStorei(GL_PACK_ALIGNMENT, align)
            while gles.glGetError():
                pass
            buf = bytearray([0xAA] * (W * H * bpp + 8192))
            arr = (ctypes.c_ubyte * len(buf)).from_buffer(buf)
            gles.glReadPixels(0, 0, W, H, GL_RGB, GL_UNSIGNED_BYTE,
                              ctypes.cast(arr, ctypes.c_void_p))
            err = gles.glGetError()
            last = max((i for i, b in enumerate(buf) if b != 0xAA), default=-1)
            pitch = (last + 1 - W * bpp) / (H - 1) if last >= 0 and H > 1 else -1
            print("RGB align=%d last_off=%d implied_pitch=%.1f err=0x%x"
                  % (align, last, pitch, err))
        return 0

    if mode == "cost":
        # Split the presentation bill at device resolution, without needing an X
        # server: (a) render leg = clear + glFinish, (b) readback leg = the
        # known-good row-at-a-time glReadPixels + glFinish. evidence 174 could
        # only subtract "window - pbuffer"; this measures the readback directly.
        for name, restype, argtypes in (
            ("glClearColor", None, [ctypes.c_float, ctypes.c_float, ctypes.c_float,
                                    ctypes.c_float]),
            ("glClear", None, [ctypes.c_uint]),
            ("glFinish", None, []),
        ):
            fn = getattr(gles, name)
            fn.restype = restype
            fn.argtypes = argtypes
        frames = int(sys.argv[5]) if len(sys.argv) > 5 else 12
        gles.glClearColor(0.2, 0.4, 0.6, 1.0)
        gles.glFinish()
        for _ in range(2):  # warmup
            gles.glClear(0x4000)
            gles.glFinish()
        t = time.perf_counter()
        for _ in range(frames):
            gles.glClear(0x4000)
            gles.glFinish()
        render_ms = (time.perf_counter() - t) * 1000.0 / frames

        buf = bytearray(W * 4 * H)
        arr = (ctypes.c_ubyte * len(buf)).from_buffer(buf)
        gles.glPixelStorei(GL_PACK_ROW_LENGTH, 0)
        gles.glPixelStorei(GL_PACK_ALIGNMENT, 4)

        def rows_read():
            for y in range(H):
                gles.glReadPixels(0, y, W, 1, GL_RGBA, GL_UNSIGNED_BYTE,
                                  ctypes.cast(arr, ctypes.c_void_p))

        rows_read()  # warm
        gles.glFinish()
        t = time.perf_counter()
        for _ in range(frames):
            rows_read()
            gles.glFinish()
        read_ms = (time.perf_counter() - t) * 1000.0 / frames
        mp = W * H / 1e6
        # One whole-block call per frame: this is what a present/screenshot path
        # actually does internally. The data lands misplaced (see evidence 289),
        # so this times the transfer + pack work, not a correct copy.
        def block_read():
            gles.glReadPixels(0, 0, W, H, GL_RGBA, GL_UNSIGNED_BYTE,
                              ctypes.cast(arr, ctypes.c_void_p))

        block_read()
        gles.glFinish()
        t = time.perf_counter()
        for _ in range(frames):
            block_read()
            gles.glFinish()
        block_ms = (time.perf_counter() - t) * 1000.0 / frames
        print("COST %dx%d (%.3f MP) frames=%d" % (W, H, mp, frames))
        print("COST render      %.2f ms/frame  (%.2f ms/MP)" % (render_ms, render_ms / mp))
        print("COST read-1call  %.2f ms/frame  (%.2f ms/MP)  single glReadPixels WxH"
              % (block_ms, block_ms / mp))
        print("COST read-perrow %.2f ms/frame  (%.2f ms/MP)  %d calls of Wx1"
              % (read_ms, read_ms / mp, H))
        return 0

    if mode == "rgba4":
        # bpp != 4 discriminator: an RGBA4 target whose only legal read type is
        # GL_UNSIGNED_SHORT_4_4_4_4 (2 B/px). Predictions for a 16-wide block:
        #   correct            -> pitch 32
        #   "bpp * Alignment"  -> 8  at Alignment=4
        #   "4 * Alignment"    -> 16 at Alignment=4
        GL_RGBA4 = 0x8056
        GL_USHORT4444 = 0x8033
        while gles.glGetError():
            pass
        impl_fmt = ctypes.c_int()
        impl_typ = ctypes.c_int()
        gles.glGetIntegerv(0x8B9A, ctypes.byref(impl_fmt))
        gles.glGetIntegerv(0x8B9B, ctypes.byref(impl_typ))
        print("IMPL_READ fmt=0x%x type=0x%x" % (impl_fmt.value, impl_typ.value))
        t2 = ctypes.c_uint()
        gles.glGenTextures(1, ctypes.byref(t2))
        gles.glBindTexture(GL_TEXTURE_2D, t2.value)
        gles.glPixelStorei(GL_UNPACK_ALIGNMENT, 1)
        gles.glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA4, W, H, 0, GL_RGBA,
                          GL_UNSIGNED_BYTE, None)
        f2 = ctypes.c_uint()
        gles.glGenFramebuffers(1, ctypes.byref(f2))
        gles.glBindFramebuffer(GL_FRAMEBUFFER, f2.value)
        gles.glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0,
                                    GL_TEXTURE_2D, t2.value, 0)
        st2 = gles.glCheckFramebufferStatus(GL_FRAMEBUFFER)
        if st2 != GL_FRAMEBUFFER_COMPLETE:
            print("FATAL rgba4 fbo incomplete 0x%x" % st2)
            return 2
        bpp = 2
        for align in (1, 4, 8):
            gles.glPixelStorei(GL_PACK_ROW_LENGTH, 0)
            gles.glPixelStorei(GL_PACK_ALIGNMENT, align)
            while gles.glGetError():
                pass
            buf = bytearray([0xAA] * (W * H * bpp + 8192))
            arr = (ctypes.c_ubyte * len(buf)).from_buffer(buf)
            gles.glReadPixels(0, 0, W, H, GL_RGBA, GL_USHORT4444,
                              ctypes.cast(arr, ctypes.c_void_p))
            err = gles.glGetError()
            n = sum(1 for b in buf if b != 0xAA)
            over = sum(1 for b in buf[W * H * bpp:] if b != 0xAA)
            print("RGBA4 align=%d written_bytes=%d overrun=%d err=0x%x"
                  % (align, n, over, err))
        return 0

    if mode == "count":
        # bpp discriminator without needing unique pixel values: read the same
        # WxH block as a pack type with a known bytes-per-pixel and count how
        # many destination bytes are touched. Written bytes = (H-1)*pitch + W*bpp,
        # so the pitch falls out arithmetically.
        #   GL_UNSIGNED_BYTE            4 B/px
        #   GL_UNSIGNED_SHORT_4_4_4_4   2 B/px
        ptype = int(sys.argv[5], 16) if len(sys.argv) > 5 else 0x1401
        bpp = int(sys.argv[6]) if len(sys.argv) > 6 else 4
        for align in (1, 4, 8):
            gles.glPixelStorei(GL_PACK_ROW_LENGTH, 0)
            gles.glPixelStorei(GL_PACK_ALIGNMENT, align)
            while gles.glGetError():
                pass
            buf = bytearray([0xAA] * (W * H * bpp + 8192))
            arr = (ctypes.c_ubyte * len(buf)).from_buffer(buf)
            gles.glReadPixels(0, 0, W, H, GL_RGBA, ptype,
                              ctypes.cast(arr, ctypes.c_void_p))
            err = gles.glGetError()
            n = sum(1 for b in buf if b != 0xAA)
            pitch = (n - W * bpp) / (H - 1) if H > 1 else -1
            print("COUNT type=0x%x bpp=%d align=%d written_bytes=%d implied_pitch=%.1f err=0x%x"
                  % (ptype, bpp, align, n, pitch, err))
        gles.glPixelStorei(GL_PACK_ALIGNMENT, 4)
        print("SIZE %dx%d  tag=%s" % (W, H, tag))
        return 0

    if mode == "float":
        # bpp discriminator. Same read, but as GL_FLOAT (16 B/px on the wire).
        # Written-byte count = (H-1)*pitch + W*bpp, so pitch falls out without
        # decoding pixel identity:
        #   pitch = 16  -> "4 * Alignment"      (alignment used as a width)
        #   pitch = 64  -> "bpp * Alignment"    (alignment used as a row length)
        GL_FLOAT = 0x1406
        bpp = 16
        for align in (1, 4, 8):
            gles.glPixelStorei(GL_PACK_ROW_LENGTH, 0)
            gles.glPixelStorei(GL_PACK_ALIGNMENT, align)
            while gles.glGetError():
                pass
            buf = bytearray([0xAA] * (W * H * bpp + 8192))
            arr = (ctypes.c_ubyte * len(buf)).from_buffer(buf)
            gles.glReadPixels(0, 0, W, H, GL_RGBA, GL_FLOAT,
                              ctypes.cast(arr, ctypes.c_void_p))
            err = gles.glGetError()
            n = sum(1 for b in buf if b != 0xAA)
            pitch = (n - W * bpp) / (H - 1) if H > 1 else -1
            print("FLOAT align=%d written_bytes=%d implied_pitch=%.1f err=0x%x"
                  % (align, n, pitch, err))
        gles.glPixelStorei(GL_PACK_ALIGNMENT, 4)
        print("SIZE %dx%d  tag=%s" % (W, H, tag))
        return 0

    print("SIZE %dx%d  tag=%s" % (W, H, tag))
    report("default", 0, 4)
    report("ROW_LENGTH=%d" % (W + 16), W + 16, 4)
    report("ROW_LENGTH=%d" % W, W, 4)
    report("ALIGN=1", 0, 1)
    report("ALIGN=8", 0, 8)
    gles.glPixelStorei(GL_PACK_ROW_LENGTH, 0)
    gles.glPixelStorei(GL_PACK_ALIGNMENT, 4)
    pbo_read("PBO")
    # Row-at-a-time, the documented workaround.
    ok = 0
    for y in range(H):
        b = (ctypes.c_ubyte * (W * 4))()
        gles.glReadPixels(0, y, W, 1, GL_RGBA, GL_UNSIGNED_BYTE,
                          ctypes.cast(b, ctypes.c_void_p))
        for x in range(W):
            if bytes(b[4 * x:4 * x + 4]) == truth[(x, y)]:
                ok += 1
    print("ROW-AT-A-TIME %d/%d correct" % (ok, W * H))
    return 0


if __name__ == "__main__":
    sys.exit(main())
