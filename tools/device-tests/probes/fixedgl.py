#!/usr/bin/env python3
"""Fixed-function desktop GL probe (immediate mode), no X server.

Why: `glxgears` crashed in PanVK's Bifrost compiler on this device (evidence 232)
and the crash was reproduced without X11 through `gl-probe fixed` (evidence 244).
Both of those go through the desktop-GL fixed-function path, which Zink has to
translate into its own shaders -- the step where the Bifrost backend dies. This
probe drives that same path with ctypes only (libEGL + libGL, no repo code), so
the claim "fixed-function works with PANVK_DXVK_COMPAT=0" can be re-checked on a
any tree without building anything.

usage: fixedgl.py [W] [H] [seconds] [tag] [plain|lit]

`lit` turns on the pieces `glxgears` uses on top of immediate mode -- lighting,
smooth shading, depth test, colour material -- because the crash in evidence
232/244 is not "immediate mode is broken", it is a *specific* fixed-function
program dying in the Bifrost compiler. A plain triangle is the control.
exit: 0 = ran clean, 1 = GL errors, 2 = setup failed, 139-style death = the bug.
"""
import ctypes
import math
import sys
import time

EGL_PLATFORM_SURFACELESS_MESA = 0x31DD
EGL_OPENGL_API = 0x30A2
EGL_SURFACE_TYPE = 0x3033
EGL_PBUFFER_BIT = 0x0001
EGL_RENDERABLE_TYPE = 0x3040
EGL_OPENGL_BIT = 0x0008
EGL_RED_SIZE = 0x3024
EGL_GREEN_SIZE = 0x3023
EGL_BLUE_SIZE = 0x3022
EGL_ALPHA_SIZE = 0x3021
EGL_NONE = 0x3038

GL_FRAMEBUFFER = 0x8D40
GL_COLOR_ATTACHMENT0 = 0x8CE0
GL_FRAMEBUFFER_COMPLETE = 0x8CD5
GL_RGBA8 = 0x8058
GL_TEXTURE_2D = 0x0DE1
GL_COLOR_BUFFER_BIT = 0x4000
GL_TRIANGLES = 0x0004
GL_NO_ERROR = 0


def main():
    W = int(sys.argv[1]) if len(sys.argv) > 1 else 640
    H = int(sys.argv[2]) if len(sys.argv) > 2 else 480
    secs = int(sys.argv[3]) if len(sys.argv) > 3 else 10
    tag = sys.argv[4] if len(sys.argv) > 4 else "fixed"
    mode = sys.argv[5] if len(sys.argv) > 5 else "plain"

    egl = ctypes.CDLL("libEGL.so.1")
    gl = ctypes.CDLL("libGL.so.1")
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
    a, b = ctypes.c_int(), ctypes.c_int()
    if not egl.eglInitialize(dpy, ctypes.byref(a), ctypes.byref(b)):
        print("FATAL eglInitialize 0x%x" % egl.eglGetError())
        return 2
    egl.eglBindAPI.argtypes = [ctypes.c_uint]
    egl.eglBindAPI.restype = ctypes.c_uint
    if not egl.eglBindAPI(EGL_OPENGL_API):
        print("FATAL eglBindAPI(GL) 0x%x" % egl.eglGetError())
        return 2
    attrs = (ctypes.c_int * 15)(EGL_SURFACE_TYPE, EGL_PBUFFER_BIT,
                                EGL_RENDERABLE_TYPE, EGL_OPENGL_BIT,
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
    # No context attributes: the default is the highest compatibility profile the
    # driver offers. Immediate mode needs a compatibility context.
    egl.eglCreateContext.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_void_p,
                                     ctypes.POINTER(ctypes.c_int)]
    egl.eglCreateContext.restype = ctypes.c_void_p
    ctx = egl.eglCreateContext(dpy, cfg, None, None)
    if not ctx:
        print("FATAL eglCreateContext 0x%x" % egl.eglGetError())
        return 2
    egl.eglMakeCurrent.argtypes = [ctypes.c_void_p] * 4
    egl.eglMakeCurrent.restype = ctypes.c_uint
    if not egl.eglMakeCurrent(dpy, None, None, ctx):
        print("FATAL eglMakeCurrent 0x%x" % egl.eglGetError())
        return 2

    gl.glGetString.restype = ctypes.c_char_p
    gl.glGetString.argtypes = [ctypes.c_uint]
    print("TAG %s GL_RENDERER=%s GL_VERSION=%s"
          % (tag, (gl.glGetString(0x1F01) or b"?").decode(),
             (gl.glGetString(0x1F02) or b"?").decode()), flush=True)

    gl.glGenTextures.argtypes = [ctypes.c_int, ctypes.POINTER(ctypes.c_uint)]
    gl.glBindTexture.argtypes = [ctypes.c_uint, ctypes.c_uint]
    gl.glTexImage2D.argtypes = [ctypes.c_uint, ctypes.c_int, ctypes.c_int, ctypes.c_int,
                                ctypes.c_int, ctypes.c_int, ctypes.c_uint, ctypes.c_uint,
                                ctypes.c_void_p]
    gl.glGenFramebuffers.argtypes = [ctypes.c_int, ctypes.POINTER(ctypes.c_uint)]
    gl.glBindFramebuffer.argtypes = [ctypes.c_uint, ctypes.c_uint]
    gl.glFramebufferTexture2D.argtypes = [ctypes.c_uint, ctypes.c_uint, ctypes.c_uint,
                                          ctypes.c_uint, ctypes.c_int]
    gl.glCheckFramebufferStatus.restype = ctypes.c_uint
    gl.glCheckFramebufferStatus.argtypes = [ctypes.c_uint]
    gl.glViewport.argtypes = [ctypes.c_int] * 4
    gl.glClearColor.argtypes = [ctypes.c_float] * 4
    gl.glClear.argtypes = [ctypes.c_uint]
    gl.glBegin.argtypes = [ctypes.c_uint]
    gl.glEnd.argtypes = []
    gl.glVertex2f.argtypes = [ctypes.c_float, ctypes.c_float]
    gl.glFinish.argtypes = []
    gl.glGetError.restype = ctypes.c_uint
    gl.glGenBuffers.argtypes = [ctypes.c_int, ctypes.POINTER(ctypes.c_uint)]
    gl.glEnable.argtypes = [ctypes.c_uint]
    gl.glDisable.argtypes = [ctypes.c_uint]
    gl.glShadeModel.argtypes = [ctypes.c_uint]
    gl.glLightfv.argtypes = [ctypes.c_uint, ctypes.c_uint, ctypes.POINTER(ctypes.c_float)]
    gl.glNormal3f.argtypes = [ctypes.c_float] * 3
    gl.glColor3f.argtypes = [ctypes.c_float] * 3
    gl.glRotatef.argtypes = [ctypes.c_float, ctypes.c_float, ctypes.c_float, ctypes.c_float]
    gl.glScalef.argtypes = [ctypes.c_float] * 3
    gl.glMatrixMode.argtypes = [ctypes.c_uint]
    gl.glLoadIdentity.argtypes = []

    tex = ctypes.c_uint()
    gl.glGenTextures(1, ctypes.byref(tex))
    gl.glBindTexture(GL_TEXTURE_2D, tex)
    gl.glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA8, W, H, 0, 0x1908, 0x1401, None)
    fbo = ctypes.c_uint()
    gl.glGenFramebuffers(1, ctypes.byref(fbo))
    gl.glBindFramebuffer(GL_FRAMEBUFFER, fbo)
    gl.glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0)
    st = gl.glCheckFramebufferStatus(GL_FRAMEBUFFER)
    if st != GL_FRAMEBUFFER_COMPLETE:
        print("FATAL fbo incomplete 0x%x" % st)
        return 2
    gl.glViewport(0, 0, W, H)

    lit = (mode == "lit")
    if lit:
        GL_LIGHTING = 0x0B50
        GL_LIGHT0 = 0x4000
        GL_POSITION = 0x1203
        GL_DEPTH_TEST = 0x0B71
        GL_NORMALIZE = 0x0BA1
        GL_COLOR_MATERIAL = 0x0B57
        GL_SMOOTH = 0x1D01
        GL_PROJECTION = 0x1701
        GL_MODELVIEW = 0x1700
        gl.glEnable(GL_LIGHTING)
        gl.glEnable(GL_LIGHT0)
        gl.glEnable(GL_DEPTH_TEST)
        gl.glEnable(GL_NORMALIZE)
        gl.glEnable(GL_COLOR_MATERIAL)
        gl.glShadeModel(GL_SMOOTH)
        pos = (ctypes.c_float * 4)(0.5, 5.0, 5.0, 0.0)
        gl.glLightfv(GL_LIGHT0, GL_POSITION, pos)
        gl.glMatrixMode(GL_PROJECTION)
        gl.glLoadIdentity()
        gl.glMatrixMode(GL_MODELVIEW)
        print("MODE lit (lighting + smooth shading + depth test)", flush=True)

    t0 = time.time()
    frames = 0
    errors = 0
    while time.time() - t0 < secs:
        r = (frames % 60) / 60.0
        gl.glClearColor(r, 0.2, 1.0 - r, 1.0)
        gl.glClear(GL_COLOR_BUFFER_BIT)
        if lit:
            gl.glLoadIdentity()
            gl.glRotatef(frames % 360, 0.0, 1.0, 0.0)
            gl.glBegin(GL_TRIANGLES)
            for i in range(60):
                ang = i * 6.0 * math.pi / 180.0
                x = 0.8 * math.cos(ang)
                y = 0.8 * math.sin(ang)
                gl.glNormal3f(x, y, 0.6)
                gl.glColor3f(0.2 + 0.4 * (i % 5) / 4.0, 0.6, 0.9 - 0.4 * (i % 3) / 2.0)
                gl.glVertex2f(x * 0.6, y * 0.6)
                gl.glVertex2f(x, y)
                gl.glVertex2f(x * 0.5 + 0.1, y * 0.5 - 0.1)
            gl.glEnd()
        else:
            gl.glBegin(GL_TRIANGLES)
            gl.glVertex2f(-0.6, -0.5)
            gl.glVertex2f(0.6, -0.5)
            gl.glVertex2f(0.0, 0.6)
            gl.glEnd()
        gl.glFinish()
        frames += 1
        if gl.glGetError() != GL_NO_ERROR:
            errors += 1
    dt = time.time() - t0
    print("DONE %s %dx%d frames=%d fps=%.1f errors=%d"
          % (tag, W, H, frames, frames / max(dt, 1e-9), errors), flush=True)
    return 0 if errors == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
