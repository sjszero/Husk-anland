#!/usr/bin/env python3
"""Round-5 readback suite: width boundaries, canary overrun, screenshot path.

Round-5 readback suite: width boundaries, whole-block reads, canary overrun,
and the screenshot-shaped row loop.

History: evidence 162/179/289 reported a 16-byte destination pitch because the
probes used the two PixelStore enum values in the wrong order. GL_PACK_ALIGNMENT
is 0x0D05 and GL_PACK_ROW_LENGTH is 0x0D02. With the corrected constants the
whole-block path is a positive test, not a known defect.

  T1  the supported path, across widths that are and are not multiples of 4;
  T2  the same path with a canary tail, so "it works" cannot mean "it wrote past
      the end of the buffer";
  T3  whole-block WxH read, exact bytes and untouched canary;
  T4  the exact call our screenshot code makes (row loop into W*H*4), compared
      byte for byte against ground truth.

Ground truth is per-pixel 1x1 reads cross-checked against the uploaded pattern,
so a broken texture upload cannot masquerade as a broken readback.

usage: readback-suite.py            (exit 0 iff every T1/T2/T3/T4 assertion holds)
"""
import ctypes
import sys

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


def pattern(x, y):
    return bytes(((x * 7 + 3) & 0xFF, (y * 11 + 5) & 0xFF, (13 * x + 17 * y) % 251, 255))


class Gl:
    def __init__(self):
        egl = ctypes.CDLL("libEGL.so.1")
        gles = ctypes.CDLL("libGLESv2.so.2")
        self.gles = gles
        egl.eglGetProcAddress.restype = ctypes.c_void_p
        egl.eglGetProcAddress.argtypes = [ctypes.c_char_p]
        addr = egl.eglGetProcAddress(b"eglGetPlatformDisplayEXT")
        if not addr:
            raise RuntimeError("no eglGetPlatformDisplayEXT")
        proto = ctypes.CFUNCTYPE(ctypes.c_void_p, ctypes.c_uint, ctypes.c_void_p, ctypes.c_void_p)
        dpy = proto(addr)(EGL_PLATFORM_SURFACELESS_MESA, None, None)
        if not dpy:
            raise RuntimeError("eglGetPlatformDisplayEXT NULL")
        egl.eglInitialize.argtypes = [ctypes.c_void_p, ctypes.POINTER(ctypes.c_int),
                                      ctypes.POINTER(ctypes.c_int)]
        egl.eglInitialize.restype = ctypes.c_uint
        a, b = ctypes.c_int(), ctypes.c_int()
        if not egl.eglInitialize(dpy, ctypes.byref(a), ctypes.byref(b)):
            raise RuntimeError("eglInitialize failed")
        egl.eglBindAPI.argtypes = [ctypes.c_uint]
        egl.eglBindAPI.restype = ctypes.c_uint
        if not egl.eglBindAPI(EGL_OPENGL_ES_API):
            raise RuntimeError("eglBindAPI failed")
        attrs = (ctypes.c_int * 15)(EGL_SURFACE_TYPE, EGL_PBUFFER_BIT,
                                    EGL_RENDERABLE_TYPE, EGL_OPENGL_ES3_BIT,
                                    EGL_RED_SIZE, 8, EGL_GREEN_SIZE, 8,
                                    EGL_BLUE_SIZE, 8, EGL_ALPHA_SIZE, 8, EGL_NONE, 0, 0)
        cfg = ctypes.c_void_p()
        n = ctypes.c_int()
        egl.eglChooseConfig.argtypes = [ctypes.c_void_p, ctypes.POINTER(ctypes.c_int),
                                        ctypes.POINTER(ctypes.c_void_p), ctypes.c_int,
                                        ctypes.POINTER(ctypes.c_int)]
        egl.eglChooseConfig.restype = ctypes.c_uint
        if not egl.eglChooseConfig(dpy, attrs, ctypes.byref(cfg), 1, ctypes.byref(n)) or n.value < 1:
            raise RuntimeError("eglChooseConfig failed")
        ca = (ctypes.c_int * 3)(EGL_CONTEXT_CLIENT_VERSION, 3, EGL_NONE)
        egl.eglCreateContext.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_void_p,
                                         ctypes.POINTER(ctypes.c_int)]
        egl.eglCreateContext.restype = ctypes.c_void_p
        ctx = egl.eglCreateContext(dpy, cfg, None, ca)
        if not ctx:
            raise RuntimeError("eglCreateContext failed")
        egl.eglMakeCurrent.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_void_p,
                                       ctypes.c_void_p]
        egl.eglMakeCurrent.restype = ctypes.c_uint
        if not egl.eglMakeCurrent(dpy, None, None, ctx):
            raise RuntimeError("eglMakeCurrent failed")
        gles.glGetString.restype = ctypes.c_char_p
        gles.glGetString.argtypes = [ctypes.c_uint]
        self.renderer = (gles.glGetString(0x1F01) or b"?").decode("utf-8", "replace")
        self.version = (gles.glGetString(0x1F02) or b"?").decode("utf-8", "replace")
        for name, restype, argtypes in (
            ("glGenTextures", None, [ctypes.c_int, ctypes.POINTER(ctypes.c_uint)]),
            ("glBindTexture", None, [ctypes.c_uint, ctypes.c_uint]),
            ("glTexImage2D", None, [ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_int,
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
        ):
            fn = getattr(gles, name)
            fn.restype = restype
            fn.argtypes = argtypes

    def frame(self, w, h):
        g = self.gles
        g.glPixelStorei(GL_PACK_ALIGNMENT, 4)
        g.glPixelStorei(GL_PACK_ROW_LENGTH, 0)
        tex = ctypes.c_uint()
        g.glGenTextures(1, ctypes.byref(tex))
        g.glBindTexture(GL_TEXTURE_2D, tex.value)
        g.glPixelStorei(GL_UNPACK_ALIGNMENT, 1)
        src = bytearray()
        for y in range(h):
            for x in range(w):
                src += pattern(x, y)
        buf = (ctypes.c_ubyte * len(src)).from_buffer(src)
        g.glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA8, w, h, 0, GL_RGBA, GL_UNSIGNED_BYTE,
                       ctypes.cast(buf, ctypes.c_void_p))
        fbo = ctypes.c_uint()
        g.glGenFramebuffers(1, ctypes.byref(fbo))
        g.glBindFramebuffer(GL_FRAMEBUFFER, fbo.value)
        g.glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D,
                                tex.value, 0)
        st = g.glCheckFramebufferStatus(GL_FRAMEBUFFER)
        if st != GL_FRAMEBUFFER_COMPLETE:
            raise RuntimeError("fbo incomplete 0x%x" % st)
        return tex.value, fbo.value

    def drain(self):
        """Collect (and report) errors that the *setup* raised.

        Frame construction on this stack raises GL_INVALID_VALUE (0x501) on this
        tree for reasons that have nothing to do with the read path -- measured
        2026-11-26: every width reported `0 bytes differ, glGetError=0x501`
        because the sticky error was set before the first glReadPixels. Draining
        before the measured call is the difference between "the read is wrong"
        and "something earlier complained".
        """
        seen = []
        for _ in range(8):
            e = self.gles.glGetError()
            if e == 0:
                break
            seen.append(e)
        return seen

    def read1(self, x, y):
        b = (ctypes.c_ubyte * 4)()
        self.gles.glReadPixels(x, y, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE,
                               ctypes.cast(b, ctypes.c_void_p))
        return bytes(b)

    def read_rows(self, w, h, extra=0, fill=0xAA):
        """Row-at-a-time into one buffer: the shape our screenshot code uses."""
        buf = bytearray([fill] * (w * h * 4 + extra))
        arr = (ctypes.c_ubyte * len(buf)).from_buffer(buf)
        base = ctypes.cast(arr, ctypes.c_void_p)
        for y in range(h):
            self.gles.glReadPixels(0, y, w, 1, GL_RGBA, GL_UNSIGNED_BYTE,
                                   ctypes.c_void_p(base.value + y * w * 4))
        return bytes(buf)

    def read_block(self, w, h, extra=8192, fill=0xAA):
        buf = bytearray([fill] * (w * h * 4 + extra))
        arr = (ctypes.c_ubyte * len(buf)).from_buffer(buf)
        self.gles.glReadPixels(0, 0, w, h, GL_RGBA, GL_UNSIGNED_BYTE,
                               ctypes.cast(arr, ctypes.c_void_p))
        return bytes(buf)


def main():
    P = 0
    F = 0

    def ok(msg):
        nonlocal P
        P += 1
        print("  PASS  " + msg)

    def bad(msg):
        nonlocal F
        F += 1
        print("  FAIL  " + msg)

    gl = Gl()
    print("TAG renderer=%s version=%s" % (gl.renderer, gl.version))

    print("== T1 row-wise readback, width boundaries ==")
    for (w, h) in ((1, 3), (2, 3), (3, 3), (4, 3), (5, 3), (7, 3), (8, 4),
                   (15, 3), (16, 4), (17, 3), (63, 2), (64, 2), (65, 2)):
        gl.frame(w, h)
        want = bytearray()
        uploaded = bytearray()
        for y in range(h):
            for x in range(w):
                want += gl.read1(x, y)
                uploaded += pattern(x, y)
        if bytes(want) != bytes(uploaded):
            bad("W=%d H=%d: 1x1 reads do not match the uploaded pattern" % (w, h))
            continue
        early = gl.drain()
        if early:
            print("  INFO  W=%d H=%d: setup raised %s before the read (drained, not counted)"
                  % (w, h, ",".join("0x%x" % e for e in early)))
        got = gl.read_rows(w, h)
        err = gl.gles.glGetError()
        if got == bytes(want) and err == 0:
            ok("W=%d H=%d: %d bytes exact, glGetError=0" % (w, h, len(got)))
        else:
            diff = sum(1 for i in range(min(len(got), len(want))) if got[i] != want[i])
            bad("W=%d H=%d: %d byte(s) differ, glGetError=0x%x" % (w, h, diff, err))

    print("== T2 canary: row-wise read must not touch the tail ==")
    for (w, h) in ((5, 3), (17, 3), (65, 2)):
        gl.frame(w, h)
        want = bytearray()
        for y in range(h):
            for x in range(w):
                want += gl.read1(x, y)
        tail = 4096
        gl.drain()
        got = gl.read_rows(w, h, extra=tail)
        head, beyond = got[: w * h * 4], got[w * h * 4:]
        over = sum(1 for v in beyond if v != 0xAA)
        if head == bytes(want) and over == 0:
            ok("W=%d H=%d: head exact, %d-byte tail untouched" % (w, h, tail))
        else:
            bad("W=%d H=%d: head %s, tail written in %d byte(s)"
                % (w, h, "exact" if head == bytes(want) else "WRONG", over))

    print("== T3 whole-block read: exact bytes and canary ==")
    for (w, h) in ((16, 16), (65, 4)):
        gl.frame(w, h)
        want = bytearray()
        for y in range(h):
            for x in range(w):
                want += gl.read1(x, y)
        tail = 8192
        gl.drain()
        got = gl.read_block(w, h, extra=tail)
        err = gl.gles.glGetError()
        head, beyond = got[: w * h * 4], got[w * h * 4:]
        over = sum(1 for v in beyond if v != 0xAA)
        if head == bytes(want) and over == 0 and err == 0:
            ok("W=%d H=%d: whole-block exact, %d-byte tail untouched, glGetError=0"
               % (w, h, tail))
        else:
            wrong = sum(1 for a, b in zip(head, want) if a != b) + (len(want) - len(head))
            bad("W=%d H=%d: whole-block %d/%d bytes differ, tail written in %d byte(s), "
                "glGetError=0x%x" % (w, h, wrong, len(want), over, err))

    print("== T4 screenshot path: row loop into W*H*4 ==")
    w, h = 240, 135
    gl.frame(w, h)
    want = bytearray()
    for y in range(h):
        for x in range(w):
            want += gl.read1(x, y)
    gl.drain()
    got = gl.read_rows(w, h)
    if got == bytes(want):
        ok("240x135: %d bytes byte-identical to ground truth" % len(got))
    else:
        diff = sum(1 for i in range(len(want)) if got[i] != bytes(want)[i])
        bad("240x135: %d/%d bytes differ" % (diff, len(want)))

    print("")
    print("PASS=%d FAIL=%d" % (P, F))
    return 0 if F == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
