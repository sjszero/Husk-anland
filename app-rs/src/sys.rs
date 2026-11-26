//! Raw FFI. No bindgen, no C: every declaration here is written by hand so the
//! app has exactly one non-Rust input, the platform's own ABI.
#![allow(non_camel_case_types, non_snake_case, dead_code)]

use core::ffi::{c_char, c_int, c_long, c_void};

// ------------------------------------------------------------------ libc ---
extern "C" {
    pub fn posix_openpt(flags: c_int) -> c_int;
    pub fn grantpt(fd: c_int) -> c_int;
    pub fn unlockpt(fd: c_int) -> c_int;
    pub fn ptsname_r(fd: c_int, buf: *mut c_char, len: usize) -> c_int;
    pub fn fork() -> c_int;
    pub fn pipe(fds: *mut c_int) -> c_int;
    pub fn getpid() -> c_int;
    pub fn setsid() -> c_int;
    pub fn open(path: *const c_char, flags: c_int, ...) -> c_int;
    pub fn ioctl(fd: c_int, req: usize, ...) -> c_int;
    pub fn dup2(old: c_int, new: c_int) -> c_int;
    pub fn close(fd: c_int) -> c_int;
    pub fn read(fd: c_int, buf: *mut c_void, n: usize) -> isize;
    pub fn write(fd: c_int, buf: *const c_void, n: usize) -> isize;
    pub fn execve(
        path: *const c_char,
        argv: *const *const c_char,
        envp: *const *const c_char,
    ) -> c_int;
    pub fn waitpid(pid: c_int, status: *mut c_int, opts: c_int) -> c_int;
    pub fn chdir(path: *const c_char) -> c_int;
    pub fn fcntl(fd: c_int, cmd: c_int, ...) -> c_int;
    pub fn _exit(code: c_int) -> !;
    pub fn __errno() -> *mut c_int;
    pub fn dladdr(addr: *const c_void, info: *mut DlInfo) -> c_int;
    pub fn usleep(us: u32) -> c_int;
    pub fn getuid() -> u32;
    pub fn getgid() -> u32;
    pub fn getgroups(size: c_int, list: *mut u32) -> c_int;
}

#[repr(C)]
pub struct DlInfo {
    pub dli_fname: *const c_char,
    pub dli_fbase: *mut c_void,
    pub dli_sname: *const c_char,
    pub dli_saddr: *mut c_void,
}

pub const O_RDWR: c_int = 2;
pub const O_NOCTTY: c_int = 0o400;
pub const O_NONBLOCK: c_int = 0o4000;
pub const F_SETFL: c_int = 4;
pub const F_DUPFD_CLOEXEC: c_int = 1030;
pub const TIOCSCTTY: usize = 0x540E;
pub const TIOCSWINSZ: usize = 0x5414;

#[repr(C)]
pub struct Winsize {
    pub rows: u16,
    pub cols: u16,
    pub xpixel: u16,
    pub ypixel: u16,
}

// ------------------------------------------------------ native activity ---

#[repr(C)]
pub struct ANativeActivityCallbacks {
    pub onStart: Option<unsafe extern "C" fn(*mut ANativeActivity)>,
    pub onResume: Option<unsafe extern "C" fn(*mut ANativeActivity)>,
    pub onSaveInstanceState:
        Option<unsafe extern "C" fn(*mut ANativeActivity, *mut c_void, usize) -> *mut c_void>,
    pub onPause: Option<unsafe extern "C" fn(*mut ANativeActivity)>,
    pub onStop: Option<unsafe extern "C" fn(*mut ANativeActivity)>,
    pub onDestroy: Option<unsafe extern "C" fn(*mut ANativeActivity)>,
    pub onWindowFocusChanged: Option<unsafe extern "C" fn(*mut ANativeActivity, c_int)>,
    pub onNativeWindowCreated: Option<unsafe extern "C" fn(*mut ANativeActivity, *mut c_void)>,
    pub onNativeWindowResized: Option<unsafe extern "C" fn(*mut ANativeActivity, *mut c_void)>,
    pub onNativeWindowRedrawNeeded: Option<unsafe extern "C" fn(*mut ANativeActivity, *mut c_void)>,
    pub onNativeWindowDestroyed: Option<unsafe extern "C" fn(*mut ANativeActivity, *mut c_void)>,
    pub onInputQueueCreated: Option<unsafe extern "C" fn(*mut ANativeActivity, *mut c_void)>,
    pub onInputQueueDestroyed: Option<unsafe extern "C" fn(*mut ANativeActivity, *mut c_void)>,
    pub onContentRectChanged: Option<unsafe extern "C" fn(*mut ANativeActivity, *const c_void)>,
    pub onConfigurationChanged: Option<unsafe extern "C" fn(*mut ANativeActivity)>,
    pub onLowMemory: Option<unsafe extern "C" fn(*mut ANativeActivity)>,
}

/// android.graphics.Rect as the framework hands it to onContentRectChanged.
#[repr(C)]
pub struct ARect {
    pub left: c_int,
    pub top: c_int,
    pub right: c_int,
    pub bottom: c_int,
}

#[repr(C)]
pub struct ANativeActivity {
    pub callbacks: *mut ANativeActivityCallbacks,
    pub vm: *mut c_void,
    pub env: *mut c_void,
    pub clazz: *mut c_void,
    pub internalDataPath: *const c_char,
    pub externalDataPath: *const c_char,
    pub sdkVersion: i32,
    pub instance: *mut c_void,
    pub assetManager: *mut c_void,
    pub obbPath: *const c_char,
}

extern "C" {
    pub fn ANativeActivity_showSoftInput(activity: *mut ANativeActivity, flags: u32);
    pub fn ANativeActivity_finish(activity: *mut ANativeActivity);
    pub fn ANativeWindow_getWidth(window: *mut c_void) -> i32;
    pub fn ANativeWindow_getHeight(window: *mut c_void) -> i32;
    pub fn ANativeActivity_setWindowFlags(
        activity: *mut ANativeActivity,
        addFlags: u32,
        removeFlags: u32,
    );

    pub fn ALooper_prepare(opts: c_int) -> *mut c_void;
    pub fn ALooper_wake(looper: *mut c_void);
    pub fn ALooper_removeFd(looper: *mut c_void, fd: c_int) -> c_int;

    pub fn ALooper_addFd(
        looper: *mut c_void,
        fd: c_int,
        ident: c_int,
        events: c_int,
        callback: *mut c_void,
        data: *mut c_void,
    ) -> c_int;
    pub fn ALooper_pollOnce(
        timeoutMillis: c_int,
        outFd: *mut c_int,
        outEvents: *mut c_int,
        outData: *mut *mut c_void,
    ) -> c_int;

    pub fn AInputQueue_attachLooper(
        queue: *mut c_void,
        looper: *mut c_void,
        ident: c_int,
        callback: *mut c_void,
        data: *mut c_void,
    );
    pub fn AInputQueue_getEvent(queue: *mut c_void, outEvent: *mut *mut c_void) -> c_int;
    pub fn AInputQueue_preDispatchEvent(queue: *mut c_void, event: *mut c_void) -> c_int;
    pub fn AInputQueue_finishEvent(queue: *mut c_void, event: *mut c_void, handled: c_int);

    pub fn AInputEvent_getType(event: *const c_void) -> i32;
    pub fn AKeyEvent_getAction(event: *const c_void) -> i32;
    pub fn AKeyEvent_getKeyCode(event: *const c_void) -> i32;
    pub fn AKeyEvent_getMetaState(event: *const c_void) -> i32;
    pub fn AMotionEvent_getAction(event: *const c_void) -> i32;
    pub fn AMotionEvent_getX(event: *const c_void, pointerIndex: usize) -> f32;
    pub fn AMotionEvent_getY(event: *const c_void, pointerIndex: usize) -> f32;
    pub fn AMotionEvent_getPointerCount(event: *const c_void) -> usize;
}

pub const ALOOPER_PREPARE_ALLOW_NON_CALLBACKS: c_int = 1;
pub const ALOOPER_EVENT_INPUT: c_int = 1;
pub const AINPUT_EVENT_TYPE_KEY: i32 = 1;
pub const AINPUT_EVENT_TYPE_MOTION: i32 = 2;
pub const AKEY_EVENT_ACTION_DOWN: i32 = 0;
pub const AMOTION_EVENT_ACTION_MASK: i32 = 0xff;
pub const AMOTION_EVENT_ACTION_DOWN: i32 = 0;
pub const AMOTION_EVENT_ACTION_MOVE: i32 = 2;
pub const AMOTION_EVENT_ACTION_UP: i32 = 1;

pub const AKEYCODE_BACK: i32 = 4;
pub const AKEYCODE_DEL: i32 = 67;
pub const AKEYCODE_ENTER: i32 = 66;
pub const AKEYCODE_DPAD_UP: i32 = 19;
pub const AKEYCODE_DPAD_DOWN: i32 = 20;
pub const AKEYCODE_DPAD_LEFT: i32 = 21;
pub const AKEYCODE_DPAD_RIGHT: i32 = 22;
pub const AKEYCODE_TAB: i32 = 61;
pub const AKEYCODE_SPACE: i32 = 62;
pub const AKEYCODE_0: i32 = 7;
pub const AKEYCODE_9: i32 = 16;
pub const AKEYCODE_A: i32 = 29;
pub const AKEYCODE_Z: i32 = 54;
pub const AKEYCODE_COMMA: i32 = 55;
pub const AKEYCODE_PERIOD: i32 = 56;
pub const AKEYCODE_GRAVE: i32 = 68;
pub const AKEYCODE_MINUS: i32 = 69;
pub const AKEYCODE_EQUALS: i32 = 70;
pub const AKEYCODE_LEFT_BRACKET: i32 = 71;
pub const AKEYCODE_RIGHT_BRACKET: i32 = 72;
pub const AKEYCODE_BACKSLASH: i32 = 73;
pub const AKEYCODE_SEMICOLON: i32 = 74;
pub const AKEYCODE_APOSTROPHE: i32 = 75;
pub const AKEYCODE_SLASH: i32 = 76;
pub const AKEYCODE_ESCAPE: i32 = 111;
pub const AMETA_SHIFT_ON: i32 = 1;

// ------------------------------------------------------------ EGL / GLES ---
pub type EGLDisplay = *mut c_void;
pub type EGLConfig = *mut c_void;
pub type EGLSurface = *mut c_void;
pub type EGLContext = *mut c_void;
pub type GLuint = u32;
pub type GLint = i32;
pub type GLenum = u32;
pub type GLsizei = i32;
pub type GLfloat = f32;

extern "C" {
    pub fn eglGetDisplay(display_id: *mut c_void) -> EGLDisplay;
    pub fn eglInitialize(dpy: EGLDisplay, major: *mut i32, minor: *mut i32) -> u32;
    pub fn eglChooseConfig(
        dpy: EGLDisplay,
        attribs: *const i32,
        configs: *mut EGLConfig,
        n: i32,
        num: *mut i32,
    ) -> u32;
    pub fn eglCreateWindowSurface(
        dpy: EGLDisplay,
        cfg: EGLConfig,
        win: *mut c_void,
        attribs: *const i32,
    ) -> EGLSurface;
    pub fn eglCreateContext(
        dpy: EGLDisplay,
        cfg: EGLConfig,
        share: EGLContext,
        attribs: *const i32,
    ) -> EGLContext;
    pub fn eglMakeCurrent(
        dpy: EGLDisplay,
        draw: EGLSurface,
        read: EGLSurface,
        ctx: EGLContext,
    ) -> u32;
    pub fn eglSwapBuffers(dpy: EGLDisplay, surf: EGLSurface) -> u32;
    pub fn eglQuerySurface(dpy: EGLDisplay, surf: EGLSurface, attr: i32, value: *mut i32) -> u32;
    pub fn eglDestroySurface(dpy: EGLDisplay, surf: EGLSurface) -> u32;
    pub fn eglTerminate(dpy: EGLDisplay) -> u32;
    pub fn eglDestroyContext(dpy: EGLDisplay, ctx: EGLContext) -> u32;
    pub fn eglGetProcAddress(name: *const c_char) -> *mut c_void;
    pub fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;

    pub fn glViewport(x: i32, y: i32, w: i32, h: i32);
    pub fn glClearColor(r: f32, g: f32, b: f32, a: f32);
    pub fn glClear(mask: u32);
    pub fn glCreateShader(kind: GLenum) -> GLuint;
    pub fn glShaderSource(s: GLuint, count: GLsizei, str: *const *const c_char, len: *const i32);
    pub fn glCompileShader(s: GLuint);
    pub fn glGetShaderiv(s: GLuint, pname: GLenum, out: *mut GLint);
    pub fn glCreateProgram() -> GLuint;
    pub fn glAttachShader(p: GLuint, s: GLuint);
    pub fn glLinkProgram(p: GLuint);
    pub fn glGetProgramiv(p: GLuint, pname: GLenum, out: *mut GLint);
    pub fn glUseProgram(p: GLuint);
    pub fn glGetAttribLocation(p: GLuint, name: *const c_char) -> GLint;
    pub fn glGetUniformLocation(p: GLuint, name: *const c_char) -> GLint;
    pub fn glVertexAttribPointer(
        idx: GLuint,
        size: GLint,
        ty: GLenum,
        norm: u8,
        stride: GLsizei,
        ptr: *const c_void,
    );
    pub fn glEnableVertexAttribArray(idx: GLuint);
    pub fn glGenTextures(n: GLsizei, out: *mut GLuint);
    pub fn glBindTexture(target: GLenum, tex: GLuint);
    pub fn glActiveTexture(unit: GLenum);
    pub fn glTexImage2D(
        target: GLenum,
        level: GLint,
        ifmt: GLint,
        w: GLsizei,
        h: GLsizei,
        border: GLint,
        fmt: GLenum,
        ty: GLenum,
        data: *const c_void,
    );
    pub fn glTexParameteri(target: GLenum, pname: GLenum, v: GLint);
    pub fn glPixelStorei(pname: GLenum, v: GLint);
    pub fn glUniform1i(loc: GLint, v: GLint);
    pub fn glUniform2f(loc: GLint, a: f32, b: f32);
    pub fn glUniform3f(loc: GLint, a: f32, b: f32, c: f32);
    pub fn glDrawArrays(mode: GLenum, first: GLint, count: GLsizei);
    pub fn glDeleteTextures(n: GLsizei, textures: *const GLuint);
    pub fn glDisable(cap: GLenum);
    pub fn glEnable(cap: GLenum);
    pub fn glBlendFunc(s: GLenum, d: GLenum);
}

pub const EGL_SURFACE_TYPE: i32 = 0x3033;
pub const EGL_WINDOW_BIT: i32 = 0x0004;
pub const EGL_RENDERABLE_TYPE: i32 = 0x3040;
pub const EGL_OPENGL_ES2_BIT: i32 = 0x0004;
pub const EGL_RED_SIZE: i32 = 0x3024;
pub const EGL_GREEN_SIZE: i32 = 0x3023;
pub const EGL_BLUE_SIZE: i32 = 0x3022;
pub const EGL_ALPHA_SIZE: i32 = 0x3021;
pub const EGL_NONE: i32 = 0x3038;
pub const EGL_WIDTH: i32 = 0x3057;
pub const EGL_HEIGHT: i32 = 0x3056;
pub const EGL_CONTEXT_CLIENT_VERSION: i32 = 0x3098;
pub const EGL_NATIVE_BUFFER_ANDROID: u32 = 0x3140;
pub const EGL_IMAGE_PRESERVED_KHR: i32 = 0x30D2;

pub const GL_TEXTURE_2D: u32 = 0x0DE1;
pub const GL_TEXTURE0: u32 = 0x84C0;
pub const GL_TEXTURE_MIN_FILTER: u32 = 0x2801;
pub const GL_TEXTURE_MAG_FILTER: u32 = 0x2800;
pub const GL_NEAREST: i32 = 0x2600;
pub const GL_LINEAR: i32 = 0x2601;
pub const GL_TEXTURE_WRAP_S: u32 = 0x2802;
pub const GL_TEXTURE_WRAP_T: u32 = 0x2803;
pub const GL_CLAMP_TO_EDGE: i32 = 0x812F;
pub const GL_ALPHA: u32 = 0x1906;
pub const GL_UNSIGNED_BYTE: u32 = 0x1401;
pub const GL_UNPACK_ALIGNMENT: u32 = 0x0CF5;
pub const GL_VERTEX_SHADER: u32 = 0x8B31;
pub const GL_FRAGMENT_SHADER: u32 = 0x8B30;
pub const GL_COMPILE_STATUS: u32 = 0x8B81;
pub const GL_LINK_STATUS: u32 = 0x8B82;
pub const GL_FLOAT: u32 = 0x1406;
pub const GL_TRIANGLES: u32 = 0x0004;
pub const GL_COLOR_BUFFER_BIT: u32 = 0x4000;
pub const GL_BLEND: u32 = 0x0BE2;
pub const GL_SRC_ALPHA: u32 = 0x0302;
pub const GL_ONE_MINUS_SRC_ALPHA: u32 = 0x0303;

extern "C" {
    pub fn prctl(option: c_int, ...) -> c_int;
    pub fn sigprocmask(how: c_int, set: *const c_void, oldset: *mut c_void) -> c_int;
    pub fn ptrace(request: c_int, ...) -> c_long;
    pub fn syscall(num: c_long, ...) -> c_long;
    pub fn kill(pid: c_int, sig: c_int) -> c_int;
}

/// Constants bionic does not export as Rust macros.
pub const PR_SET_CHILD_SUBREAPER: c_int = 36;
pub const SIG_SETMASK: c_int = 2;
pub const SIGHUP: c_int = 1;
pub const SIGTERM: c_int = 15;
pub const SIGKILL: c_int = 9;
pub const SIGSYS: c_int = 31;
pub const WNOHANG: c_int = 1;
pub const EINTR: c_int = 4;
pub const EIO: c_int = 5;
pub const EAGAIN: c_int = 11;

/// wait(2) status decoding. Linux layout, same on every arch we ship.
pub fn wifexited(st: c_int) -> bool {
    st & 0x7f == 0
}
pub fn wexitstatus(st: c_int) -> c_int {
    (st >> 8) & 0xff
}
pub fn wifsignaled(st: c_int) -> bool {
    (st & 0x7f) != 0 && (st & 0x7f) != 0x7f
}
pub fn wtermsig(st: c_int) -> c_int {
    st & 0x7f
}

pub fn sig_name(sig: c_int) -> &'static str {
    match sig {
        1 => "SIGHUP",
        2 => "SIGINT",
        3 => "SIGQUIT",
        4 => "SIGILL",
        5 => "SIGTRAP",
        6 => "SIGABRT",
        7 => "SIGBUS",
        8 => "SIGFPE",
        9 => "SIGKILL",
        11 => "SIGSEGV",
        13 => "SIGPIPE",
        14 => "SIGALRM",
        15 => "SIGTERM",
        17 => "SIGCHLD",
        18 => "SIGCONT",
        19 => "SIGSTOP",
        31 => "SIGSYS",
        _ => "?",
    }
}

// ---------------------------------------------------------------- ptrace ---
pub const PTRACE_TRACEME: c_int = 0;
pub const PTRACE_SYSCALL: c_int = 24;
pub const PTRACE_SETOPTIONS: c_int = 0x4200;
pub const PTRACE_GETREGSET: c_int = 0x4204;
pub const PTRACE_PEEKDATA: c_int = 2;
pub const PTRACE_O_TRACESYSGOOD: c_long = 0x1;
pub const NT_PRSTATUS: c_long = 1;
pub const SIGTRAP: c_int = 5;

#[repr(C)]
pub struct IoVec {
    pub base: *mut c_void,
    pub len: usize,
}

/// arm64 `struct user_pt_regs`: x0..x30, sp, pc, pstate.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PtRegs {
    pub regs: [u64; 31],
    pub sp: u64,
    pub pc: u64,
    pub pstate: u64,
}

pub fn wstopsig(st: c_int) -> c_int {
    (st >> 8) & 0xff
}
