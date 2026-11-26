#!/usr/bin/env python3
"""GPU soak probe: how does one process behave after minutes on the hardware path?

Round 6 asks for "多应用并发 + 长稳", which is about the *runtime*, not about a
frame rate number. What goes wrong on a driver that is only nearly working is
slow, not loud: descriptors for /dev/mali0 accumulate, mapped buffers stay
referenced, the driver keeps a growing BO cache, or one context dies after N
frames while the rest keep running fine.

This probe therefore reports, once a second:
  * frames rendered in that second (so a stall is visible as a dip, not hidden
    in an average),
  * resident set size,
  * open fd count and how many of them point at /dev/mali0,
  * GL errors,
and a final aggregate. It never reads pixels back -- readback has its own cost
(evidence 291) and would dominate the numbers.

usage: glsoak.py [W] [H] [seconds] [tag]
"""
import ctypes
import os
import sys
import time

GL_TEXTURE_2D = 0x0DE1
GL_RGBA8 = 0x8058
GL_FRAMEBUFFER = 0x8D40
GL_COLOR_ATTACHMENT0 = 0x8CE0
GL_FRAMEBUFFER_COMPLETE = 0x8CD5
GL_FBO_READ = 0x8CA8
GL_COLOR_BUFFER_BIT = 0x4000
GL_NO_ERROR = 0

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


def rss_kb():
    with open("/proc/self/status") as f:
        for line in f:
            if line.startswith("VmRSS:"):
                return int(line.split()[1])
    return -1


def fds():
    total = 0
    mali = 0
    try:
        for name in os.listdir("/proc/self/fd"):
            total += 1
            try:
                if "mali" in os.readlink("/proc/self/fd/" + name):
                    mali += 1
            except OSError:
                pass
    except OSError:
        pass
    return total, mali


def main():
    W = int(sys.argv[1]) if len(sys.argv) > 1 else 2400
    H = int(sys.argv[2]) if len(sys.argv) > 2 else 1080
    secs = int(sys.argv[3]) if len(sys.argv) > 3 else 180
    tag = sys.argv[4] if len(sys.argv) > 4 else "soak"

    egl = ctypes.CDLL("libEGL.so.1")
    gles = ctypes.CDLL("libGLESv2.so.2")
    egl.eglGetProcAddress.restype = ctypes.c_void_p
    egl.eglGetProcAddress.argtypes = [ctypes.c_char_p]
    addr = egl.eglGetProcAddress(b"eglGetPlatformDisplayEXT")
    if not addr:
        print("FATAL no eglGetPlatformDisplayEXT")
        return 2
    proto = ctypes.CFUNCTYPE(ctypes.c_void_p, ctypes.c_uint, ctypes.c_void_p, ctypes.c_void_p)
    dpy = proto(addr)(EGL_PLATFORM_SURFACELESS_MESA, None, None)
    if not dpy:
        print("FATAL no display")
        return 2
    egl.eglInitialize.argtypes = [ctypes.c_void_p, ctypes.POINTER(ctypes.c_int),
                                  ctypes.POINTER(ctypes.c_int)]
    egl.eglInitialize.restype = ctypes.c_uint
    maj, mnr = ctypes.c_int(), ctypes.c_int()
    if not egl.eglInitialize(dpy, ctypes.byref(maj), ctypes.byref(mnr)):
        print("FATAL eglInitialize 0x%x" % egl.eglGetError())
        return 2
    egl.eglBindAPI.argtypes = [ctypes.c_uint]
    egl.eglBindAPI.restype = ctypes.c_uint
    egl.eglBindAPI(EGL_OPENGL_ES_API)
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
        print("FATAL eglChooseConfig")
        return 2
    ctxattrs = (ctypes.c_int * 3)(EGL_CONTEXT_CLIENT_VERSION, 3, EGL_NONE)
    egl.eglCreateContext.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_void_p,
                                     ctypes.POINTER(ctypes.c_int)]
    egl.eglCreateContext.restype = ctypes.c_void_p
    ctx = egl.eglCreateContext(dpy, cfg, None, ctxattrs)
    if not ctx:
        print("FATAL eglCreateContext 0x%x" % egl.eglGetError())
        return 2
    egl.eglMakeCurrent.argtypes = [ctypes.c_void_p] * 4
    egl.eglMakeCurrent.restype = ctypes.c_uint
    if not egl.eglMakeCurrent(dpy, None, None, ctx):
        print("FATAL eglMakeCurrent 0x%x" % egl.eglGetError())
        return 2

    gles.glClearColor.argtypes = [ctypes.c_float] * 4
    gles.glClear.argtypes = [ctypes.c_uint]
    gles.glGetString.restype = ctypes.c_char_p
    gles.glGetString.argtypes = [ctypes.c_uint]
    print("TAG %s GL_RENDERER=%s" % (tag, (gles.glGetString(0x1F01) or b"?").decode()))

    tex = ctypes.c_uint()
    gles.glGenTextures(1, ctypes.byref(tex))
    gles.glBindTexture(GL_TEXTURE_2D, tex)
    gles.glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA8, W, H, 0, 0x1908, 0x1401, None)
    fbo = ctypes.c_uint()
    gles.glGenFramebuffers(1, ctypes.byref(fbo))
    gles.glBindFramebuffer(GL_FRAMEBUFFER, fbo)
    gles.glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0)
    st = gles.glCheckFramebufferStatus(GL_FRAMEBUFFER)
    if st != GL_FRAMEBUFFER_COMPLETE:
        print("FATAL fbo incomplete 0x%x" % st)
        return 2
    gles.glViewport(0, 0, W, H)

    base_fd, base_mali = fds()
    base_rss = rss_kb()
    print("START %s %dx%d for %ds rss=%dkB fd=%d mali_fd=%d"
          % (tag, W, H, secs, base_rss, base_fd, base_mali))

    t0 = time.time()
    last = t0
    frames_sec = 0
    frames_total = 0
    errors = 0
    worst_sec = 0
    while True:
        now = time.time()
        if now - t0 >= secs:
            break
        r = (frames_total * 7) % 256 / 255.0
        gles.glClearColor(r, 0.25, 1.0 - r, 1.0)
        gles.glClear(GL_COLOR_BUFFER_BIT)
        gles.glFinish()
        frames_sec += 1
        frames_total += 1
        if gles.glGetError() != GL_NO_ERROR:
            errors += 1
        now = time.time()
        if now - last >= 1.0:
            fd, mali = fds()
            print("SEC %4d fps=%4d rss=%dkB fd=%d mali_fd=%d errors=%d"
                  % (int(now - t0), frames_sec, rss_kb(), fd, mali, errors))
            worst_sec = max(worst_sec, frames_sec)
            frames_sec = 0
            last = now
    fd, mali = fds()
    print("DONE %s frames=%d avg_fps=%.1f peak_fps=%d rss=%dkB (start %dkB) fd=%d (start %d) "
          "mali_fd=%d (start %d) errors=%d"
          % (tag, frames_total, frames_total / max(1e-9, time.time() - t0), worst_sec,
             rss_kb(), base_rss, fd, base_fd, mali, base_mali, errors))
    return 0 if errors == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
