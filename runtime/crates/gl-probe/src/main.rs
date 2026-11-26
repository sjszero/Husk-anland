//! Guest-side GL probes. Two subcommands, both resolved at run time through
//! `dlopen` so the guest needs no headers and no dev packages:
//!
//!   gl-probe es3      real ES3 workload: instancing + std140 UBO + texture +
//!                     MRT, every pixel read back 1x1 and checked. Prints
//!                     `RESULT PASS|FAIL`. (was tools/device-tests/es3-workload.c)
//!   gl-probe window   EGL window surface on $DISPLAY, 60 clears + swaps, then
//!                     a 1x1 readback. Prints a `stage N:` line per step so a
//!                     crash says exactly where it died.
//!                     (was tools/device-tests/egl-x11-window.c)
//!   gl-probe fixed    surfaceless desktop OpenGL compatibility context with a
//!                     fixed-function gear draw. This exercises the legacy GL
//!                     lowering path that `glxgears` uses without requiring an
//!                     X server, so a PanVK/Bifrost compiler crash is separable
//!                     from the window/presentation layer.
//!
//! The `readback` subcommand tests the whole-block path. The older 1x1 reads in
//! the workload probes remain useful for localizing one bad pixel, but are no
//! longer a workaround for a block-read defect: evidence 314 showed that the
//! former 16-byte pitch was caused by swapped PixelStore enum constants in the
//! probes themselves.

use std::ffi::CString;
use std::io::Write;
use std::os::raw::{c_char, c_double, c_int, c_uint, c_ulong, c_void};

// ---------------------------------------------------------------- dlopen glue

struct Lib(*mut c_void);

impl Lib {
    fn open(name: &str) -> Option<Lib> {
        let path = CString::new(name).ok()?;
        let h = unsafe { dlopen(path.as_ptr(), 2) };
        if h.is_null() {
            None
        } else {
            Some(Lib(h))
        }
    }

    fn sym(&self, name: &str) -> *mut c_void {
        let Ok(c) = CString::new(name) else {
            return std::ptr::null_mut();
        };
        unsafe { dlsym(self.0, c.as_ptr()) }
    }
}

unsafe fn cast<T: Copy>(p: *mut c_void) -> T {
    debug_assert!(!p.is_null());
    std::mem::transmute_copy(&p)
}

extern "C" {
    fn dlopen(path: *const c_char, mode: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

macro_rules! say {
    ($($arg:tt)*) => {{
        println!($($arg)*);
        let _ = std::io::stdout().flush();
    }};
}

// ------------------------------------------------------------- EGL constants

const EGL_OPENGL_ES_API: c_uint = 0x30A0;
const EGL_OPENGL_API: c_uint = 0x30A2;
const EGL_SURFACE_TYPE: c_int = 0x3033;
const EGL_PBUFFER_BIT: c_int = 0x0001;
const EGL_OPENGL_BIT: c_int = 0x0008;
const EGL_RENDERABLE_TYPE: c_int = 0x3040;
const EGL_OPENGL_ES2_BIT: c_int = 0x0004;
const EGL_OPENGL_ES3_BIT: c_int = 0x0040;
const EGL_NONE: c_int = 0x3038;
const EGL_CONTEXT_MAJOR_VERSION: c_int = 0x3098;
const EGL_CONTEXT_MINOR_VERSION: c_int = 0x30FB;
const EGL_PLATFORM_SURFACELESS_MESA: c_uint = 0x31DD;
const EGL_PLATFORM_X11_KHR: c_uint = 0x31D5;
const EGL_WIDTH: c_int = 0x3057;
const EGL_HEIGHT: c_int = 0x3056;
const EGL_RED_SIZE: c_int = 0x3024;
const EGL_GREEN_SIZE: c_int = 0x3023;
const EGL_BLUE_SIZE: c_int = 0x3022;
const EGL_ALPHA_SIZE: c_int = 0x3021;

// -------------------------------------------------------------- GL constants

const GL_TEXTURE_2D: c_uint = 0x0DE1;
const GL_RGBA: c_uint = 0x1908;
const GL_UNSIGNED_BYTE: c_uint = 0x1401;
const GL_NEAREST: c_int = 0x2600;
const GL_TEXTURE_MIN_FILTER: c_uint = 0x2801;
const GL_TEXTURE_MAG_FILTER: c_uint = 0x2800;
const GL_TEXTURE_WRAP_S: c_uint = 0x2802;
const GL_TEXTURE_WRAP_T: c_uint = 0x2803;
const GL_CLAMP_TO_EDGE: c_int = 0x812F;
const GL_ARRAY_BUFFER: c_uint = 0x8892;
const GL_UNIFORM_BUFFER: c_uint = 0x8A11;
const GL_STATIC_DRAW: c_uint = 0x88E4;
const GL_FLOAT: c_uint = 0x1406;
const GL_VERTEX_SHADER: c_uint = 0x8B31;
const GL_FRAGMENT_SHADER: c_uint = 0x8B30;
const GL_COMPILE_STATUS: c_uint = 0x8B81;
const GL_LINK_STATUS: c_uint = 0x8B82;
const GL_COLOR_ATTACHMENT0: c_uint = 0x8CE0;
const GL_COLOR_ATTACHMENT1: c_uint = 0x8CE1;
const GL_FRAMEBUFFER: c_uint = 0x8D40;
const GL_FRAMEBUFFER_COMPLETE: c_uint = 0x8CD5;
const GL_RGBA8: c_int = 0x8058;
const GL_COLOR_BUFFER_BIT: c_uint = 0x4000;
const GL_TRIANGLE_STRIP: c_uint = 0x0005;
const GL_TRIANGLES: c_uint = 0x0004;
const GL_TEXTURE0: c_uint = 0x84C0;
const GL_DEPTH_TEST: c_uint = 0x0B71;
const GL_UNPACK_ALIGNMENT: c_uint = 0x0CF5;
const GL_PACK_ALIGNMENT: c_uint = 0x0D05;
const GL_PACK_ROW_LENGTH: c_uint = 0x0D02;
const GL_RENDERER: c_uint = 0x1F01;
const GL_VERSION: c_uint = 0x1F02;
const GL_SHADING_LANGUAGE_VERSION: c_uint = 0x8B8C;
const GL_NO_ERROR: c_uint = 0;
const GL_DEPTH_BUFFER_BIT: c_uint = 0x0100;
const GL_LEQUAL: c_uint = 0x0203;
const GL_SMOOTH: c_uint = 0x1D01;
const GL_QUADS: c_uint = 0x0007;
const GL_TRIANGLE_FAN: c_uint = 0x0006;
const GL_PROJECTION: c_uint = 0x1701;
const GL_MODELVIEW: c_uint = 0x1700;
const GL_LIGHTING: c_uint = 0x0B50;
const GL_LIGHT0: c_uint = 0x4000;
const GL_POSITION: c_uint = 0x1203;
const GL_COLOR_MATERIAL: c_uint = 0x0B57;
const GL_FRONT_AND_BACK: c_uint = 0x0408;
const GL_AMBIENT_AND_DIFFUSE: c_uint = 0x1602;
const GL_NORMALIZE: c_uint = 0x0BA1;

type Display = c_void;
type Config = *mut c_void;
type Context = *mut c_void;
type Surface = c_void;
type Window = c_ulong;

struct Egl {
    get_platform_display: unsafe extern "C" fn(c_uint, *mut c_void, *const c_int) -> *mut Display,
    get_display: unsafe extern "C" fn(*mut c_void) -> *mut Display,
    initialize: unsafe extern "C" fn(*mut Display, *mut c_int, *mut c_int) -> c_uint,
    bind_api: unsafe extern "C" fn(c_uint) -> c_uint,
    choose_config:
        unsafe extern "C" fn(*mut Display, *const c_int, *mut Config, c_int, *mut c_int) -> c_uint,
    create_context: unsafe extern "C" fn(*mut Display, Config, Context, *const c_int) -> Context,
    create_pbuffer_surface:
        unsafe extern "C" fn(*mut Display, Config, *const c_int) -> *mut Surface,
    create_window_surface:
        unsafe extern "C" fn(*mut Display, Config, Window, *const c_int) -> *mut Surface,
    make_current: unsafe extern "C" fn(*mut Display, *mut Surface, *mut Surface, Context) -> c_uint,
    swap_buffers: unsafe extern "C" fn(*mut Display, *mut Surface) -> c_uint,
    get_error: unsafe extern "C" fn() -> c_int,
    destroy_context: unsafe extern "C" fn(*mut Display, Context) -> c_uint,
    destroy_surface: unsafe extern "C" fn(*mut Display, *mut Surface) -> c_uint,
    terminate: unsafe extern "C" fn(*mut Display) -> c_uint,
}

impl Egl {
    fn load() -> Option<Egl> {
        let egl = Lib::open("libEGL.so.1")?;
        let p = |n: &str| egl.sym(n);
        if p("eglInitialize").is_null() {
            return None;
        }
        Some(unsafe {
            Egl {
                get_platform_display: cast(p("eglGetPlatformDisplay")),
                get_display: cast(p("eglGetDisplay")),
                initialize: cast(p("eglInitialize")),
                bind_api: cast(p("eglBindAPI")),
                choose_config: cast(p("eglChooseConfig")),
                create_context: cast(p("eglCreateContext")),
                create_pbuffer_surface: cast(p("eglCreatePbufferSurface")),
                create_window_surface: cast(p("eglCreateWindowSurface")),
                make_current: cast(p("eglMakeCurrent")),
                swap_buffers: cast(p("eglSwapBuffers")),
                get_error: cast(p("eglGetError")),
                destroy_context: cast(p("eglDestroyContext")),
                destroy_surface: cast(p("eglDestroySurface")),
                terminate: cast(p("eglTerminate")),
            }
        })
    }
}

struct Gl {
    create_shader: unsafe extern "C" fn(c_uint) -> c_uint,
    shader_source: unsafe extern "C" fn(c_uint, c_int, *const *const c_char, *const c_int),
    compile_shader: unsafe extern "C" fn(c_uint),
    get_shader_info_log: unsafe extern "C" fn(c_uint, c_int, *mut c_int, *mut c_char),
    get_shader_iv: unsafe extern "C" fn(c_uint, c_uint, *mut c_int),
    create_program: unsafe extern "C" fn() -> c_uint,
    attach_shader: unsafe extern "C" fn(c_uint, c_uint),
    link_program: unsafe extern "C" fn(c_uint),
    get_program_info_log: unsafe extern "C" fn(c_uint, c_int, *mut c_int, *mut c_char),
    get_program_iv: unsafe extern "C" fn(c_uint, c_uint, *mut c_int),
    use_program: unsafe extern "C" fn(c_uint),
    gen_vertex_arrays: unsafe extern "C" fn(c_int, *mut c_uint),
    bind_vertex_array: unsafe extern "C" fn(c_uint),
    gen_buffers: unsafe extern "C" fn(c_int, *mut c_uint),
    bind_buffer: unsafe extern "C" fn(c_uint, c_uint),
    buffer_data: unsafe extern "C" fn(c_uint, isize, *const c_void, c_uint),
    vertex_attrib_pointer: unsafe extern "C" fn(c_uint, c_int, c_uint, u8, c_int, *const c_void),
    enable_vertex_attrib_array: unsafe extern "C" fn(c_uint),
    vertex_attrib_divisor: unsafe extern "C" fn(c_uint, c_uint),
    gen_textures: unsafe extern "C" fn(c_int, *mut c_uint),
    bind_texture: unsafe extern "C" fn(c_uint, c_uint),
    tex_image_2d: unsafe extern "C" fn(
        c_uint,
        c_int,
        c_int,
        c_int,
        c_int,
        c_int,
        c_uint,
        c_uint,
        *const c_void,
    ),
    tex_parameteri: unsafe extern "C" fn(c_uint, c_uint, c_int),
    active_texture: unsafe extern "C" fn(c_uint),
    gen_framebuffers: unsafe extern "C" fn(c_int, *mut c_uint),
    bind_framebuffer: unsafe extern "C" fn(c_uint, c_uint),
    framebuffer_texture_2d: unsafe extern "C" fn(c_uint, c_uint, c_uint, c_uint, c_int),
    check_framebuffer_status: unsafe extern "C" fn(c_uint) -> c_uint,
    get_error: unsafe extern "C" fn() -> c_uint,
    draw_buffers: unsafe extern "C" fn(c_int, *const c_uint),
    viewport: unsafe extern "C" fn(c_int, c_int, c_int, c_int),
    clear_color: unsafe extern "C" fn(f32, f32, f32, f32),
    clear: unsafe extern "C" fn(c_uint),
    disable: unsafe extern "C" fn(c_uint),
    draw_arrays: unsafe extern "C" fn(c_uint, c_int, c_int),
    draw_arrays_instanced: unsafe extern "C" fn(c_uint, c_int, c_int, c_int),
    bind_buffer_base: unsafe extern "C" fn(c_uint, c_uint, c_uint),
    finish: unsafe extern "C" fn(),
    read_pixels: unsafe extern "C" fn(c_int, c_int, c_int, c_int, c_uint, c_uint, *mut c_void),
    read_buffer: unsafe extern "C" fn(c_uint),
    pixel_storei: unsafe extern "C" fn(c_uint, c_int),
    get_string: unsafe extern "C" fn(c_uint) -> *const c_char,
    get_uniform_location: unsafe extern "C" fn(c_uint, *const c_char) -> c_int,
    get_attrib_location: unsafe extern "C" fn(c_uint, *const c_char) -> c_int,
    uniform1i: unsafe extern "C" fn(c_int, c_int),
}

impl Gl {
    fn load() -> Option<Gl> {
        let gl = Lib::open("libGLESv2.so.2")?;
        let p = |n: &str| gl.sym(n);
        if p("glCreateShader").is_null() || p("glDrawArraysInstanced").is_null() {
            return None;
        }
        Some(unsafe {
            Gl {
                create_shader: cast(p("glCreateShader")),
                shader_source: cast(p("glShaderSource")),
                compile_shader: cast(p("glCompileShader")),
                get_shader_info_log: cast(p("glGetShaderInfoLog")),
                get_shader_iv: cast(p("glGetShaderiv")),
                create_program: cast(p("glCreateProgram")),
                attach_shader: cast(p("glAttachShader")),
                link_program: cast(p("glLinkProgram")),
                get_program_info_log: cast(p("glGetProgramInfoLog")),
                get_program_iv: cast(p("glGetProgramiv")),
                use_program: cast(p("glUseProgram")),
                gen_vertex_arrays: cast(p("glGenVertexArrays")),
                bind_vertex_array: cast(p("glBindVertexArray")),
                gen_buffers: cast(p("glGenBuffers")),
                bind_buffer: cast(p("glBindBuffer")),
                buffer_data: cast(p("glBufferData")),
                vertex_attrib_pointer: cast(p("glVertexAttribPointer")),
                enable_vertex_attrib_array: cast(p("glEnableVertexAttribArray")),
                vertex_attrib_divisor: cast(p("glVertexAttribDivisor")),
                gen_textures: cast(p("glGenTextures")),
                bind_texture: cast(p("glBindTexture")),
                tex_image_2d: cast(p("glTexImage2D")),
                tex_parameteri: cast(p("glTexParameteri")),
                active_texture: cast(p("glActiveTexture")),
                gen_framebuffers: cast(p("glGenFramebuffers")),
                bind_framebuffer: cast(p("glBindFramebuffer")),
                framebuffer_texture_2d: cast(p("glFramebufferTexture2D")),
                check_framebuffer_status: cast(p("glCheckFramebufferStatus")),
                get_error: cast(p("glGetError")),
                draw_buffers: cast(p("glDrawBuffers")),
                viewport: cast(p("glViewport")),
                clear_color: cast(p("glClearColor")),
                clear: cast(p("glClear")),
                disable: cast(p("glDisable")),
                draw_arrays: cast(p("glDrawArrays")),
                draw_arrays_instanced: cast(p("glDrawArraysInstanced")),
                bind_buffer_base: cast(p("glBindBufferBase")),
                finish: cast(p("glFinish")),
                read_pixels: cast(p("glReadPixels")),
                read_buffer: cast(p("glReadBuffer")),
                pixel_storei: cast(p("glPixelStorei")),
                get_string: cast(p("glGetString")),
                get_uniform_location: cast(p("glGetUniformLocation")),
                get_attrib_location: cast(p("glGetAttribLocation")),
                uniform1i: cast(p("glUniform1i")),
            }
        })
    }

    fn string(&self, which: c_uint) -> String {
        let p = unsafe { (self.get_string)(which) };
        if p.is_null() {
            return "?".into();
        }
        unsafe { std::ffi::CStr::from_ptr(p) }
            .to_string_lossy()
            .into_owned()
    }
}

/// Minimal desktop OpenGL 2.1 entry points. Kept separate from `Gl` so the ES3
/// probe keeps linking only GLESv2; this path deliberately loads `libGL.so.1`
/// and uses fixed-function state, matching the lowering path used by glxgears.
struct LegacyGl {
    viewport: unsafe extern "C" fn(c_int, c_int, c_int, c_int),
    clear_color: unsafe extern "C" fn(f32, f32, f32, f32),
    clear: unsafe extern "C" fn(c_uint),
    enable: unsafe extern "C" fn(c_uint),
    depth_func: unsafe extern "C" fn(c_uint),
    shade_model: unsafe extern "C" fn(c_uint),
    matrix_mode: unsafe extern "C" fn(c_uint),
    load_identity: unsafe extern "C" fn(),
    frustum: unsafe extern "C" fn(c_double, c_double, c_double, c_double, c_double, c_double),
    translatef: unsafe extern "C" fn(f32, f32, f32),
    rotatef: unsafe extern "C" fn(f32, f32, f32, f32),
    lightfv: unsafe extern "C" fn(c_uint, c_uint, *const f32),
    color_material: unsafe extern "C" fn(c_uint, c_uint),
    gen_textures: unsafe extern "C" fn(c_int, *mut c_uint),
    bind_texture: unsafe extern "C" fn(c_uint, c_uint),
    tex_image_2d: unsafe extern "C" fn(
        c_uint,
        c_int,
        c_int,
        c_int,
        c_int,
        c_int,
        c_uint,
        c_uint,
        *const c_void,
    ),
    gen_framebuffers: unsafe extern "C" fn(c_int, *mut c_uint),
    bind_framebuffer: unsafe extern "C" fn(c_uint, c_uint),
    framebuffer_texture_2d: unsafe extern "C" fn(c_uint, c_uint, c_uint, c_uint, c_int),
    check_framebuffer_status: unsafe extern "C" fn(c_uint) -> c_uint,
    begin: unsafe extern "C" fn(c_uint),
    end: unsafe extern "C" fn(),
    normal3f: unsafe extern "C" fn(f32, f32, f32),
    vertex3f: unsafe extern "C" fn(f32, f32, f32),
    color3f: unsafe extern "C" fn(f32, f32, f32),
    finish: unsafe extern "C" fn(),
    read_pixels: unsafe extern "C" fn(c_int, c_int, c_int, c_int, c_uint, c_uint, *mut c_void),
    get_error: unsafe extern "C" fn() -> c_uint,
    get_string: unsafe extern "C" fn(c_uint) -> *const c_char,
}

impl LegacyGl {
    fn load() -> Option<LegacyGl> {
        let gl = Lib::open("libGL.so.1")?;
        let p = |n: &str| gl.sym(n);
        if p("glBegin").is_null() || p("glLightfv").is_null() {
            return None;
        }
        Some(unsafe {
            LegacyGl {
                viewport: cast(p("glViewport")),
                clear_color: cast(p("glClearColor")),
                clear: cast(p("glClear")),
                enable: cast(p("glEnable")),
                depth_func: cast(p("glDepthFunc")),
                shade_model: cast(p("glShadeModel")),
                matrix_mode: cast(p("glMatrixMode")),
                load_identity: cast(p("glLoadIdentity")),
                frustum: cast(p("glFrustum")),
                translatef: cast(p("glTranslatef")),
                rotatef: cast(p("glRotatef")),
                lightfv: cast(p("glLightfv")),
                color_material: cast(p("glColorMaterial")),
                gen_textures: cast(p("glGenTextures")),
                bind_texture: cast(p("glBindTexture")),
                tex_image_2d: cast(p("glTexImage2D")),
                gen_framebuffers: cast(p("glGenFramebuffers")),
                bind_framebuffer: cast(p("glBindFramebuffer")),
                framebuffer_texture_2d: cast(p("glFramebufferTexture2D")),
                check_framebuffer_status: cast(p("glCheckFramebufferStatus")),
                begin: cast(p("glBegin")),
                end: cast(p("glEnd")),
                normal3f: cast(p("glNormal3f")),
                vertex3f: cast(p("glVertex3f")),
                color3f: cast(p("glColor3f")),
                finish: cast(p("glFinish")),
                read_pixels: cast(p("glReadPixels")),
                get_error: cast(p("glGetError")),
                get_string: cast(p("glGetString")),
            }
        })
    }

    fn string(&self, which: c_uint) -> String {
        let p = unsafe { (self.get_string)(which) };
        if p.is_null() {
            return "?".into();
        }
        unsafe { std::ffi::CStr::from_ptr(p) }
            .to_string_lossy()
            .into_owned()
    }
}

fn legacy_surfaceless_context(egl: &Egl) -> Option<(*mut Display, Context)> {
    let display = unsafe {
        let d = (egl.get_platform_display)(
            EGL_PLATFORM_SURFACELESS_MESA,
            std::ptr::null_mut(),
            std::ptr::null(),
        );
        if d.is_null() {
            (egl.get_display)(std::ptr::null_mut())
        } else {
            d
        }
    };
    if display.is_null() {
        say!("FAIL eglGetDisplay(desktop GL fixedgl)");
        return None;
    }
    let (mut major, mut minor) = (0, 0);
    if unsafe { (egl.initialize)(display, &mut major, &mut minor) } == 0 {
        say!("FAIL eglInitialize(desktop GL fixedgl)");
        return None;
    }
    if unsafe { (egl.bind_api)(EGL_OPENGL_API) } == 0 {
        say!("FAIL eglBindAPI(OpenGL) err={:#x}", unsafe {
            (egl.get_error)()
        });
        return None;
    }
    let config_attrs = [
        EGL_SURFACE_TYPE,
        EGL_PBUFFER_BIT,
        EGL_RENDERABLE_TYPE,
        EGL_OPENGL_BIT,
        EGL_RED_SIZE,
        8,
        EGL_GREEN_SIZE,
        8,
        EGL_BLUE_SIZE,
        8,
        EGL_ALPHA_SIZE,
        8,
        EGL_NONE,
    ];
    let mut config: Config = std::ptr::null_mut();
    let mut count = 0;
    if unsafe { (egl.choose_config)(display, config_attrs.as_ptr(), &mut config, 1, &mut count) }
        == 0
        || count < 1
    {
        say!("RESULT UNSUPPORTED (no desktop-GL fixedgl config)");
        return None;
    }
    let context =
        unsafe { (egl.create_context)(display, config, std::ptr::null_mut(), std::ptr::null()) };
    if context.is_null() {
        say!("FAIL eglCreateContext(fixedgl) err={:#x}", unsafe {
            (egl.get_error)()
        });
        return None;
    }
    if unsafe { (egl.make_current)(display, std::ptr::null_mut(), std::ptr::null_mut(), context) }
        == 0
    {
        say!("FAIL eglMakeCurrent(fixedgl) err={:#x}", unsafe {
            (egl.get_error)()
        });
        return None;
    }
    Some((display, context))
}

fn run_fixedgl(width: u32, height: u32, seconds: u64, tag: &str, lit: bool) -> i32 {
    let Some(egl) = Egl::load() else {
        say!("FAIL dlopen EGL");
        return 2;
    };
    let Some((display, context)) = legacy_surfaceless_context(&egl) else {
        return 2;
    };
    let Some(gl) = LegacyGl::load() else {
        say!("RESULT UNSUPPORTED (desktop libGL symbols missing)");
        return 2;
    };

    say!(
        "TAG {} GL_RENDERER={} GL_VERSION={}",
        tag,
        gl.string(GL_RENDERER),
        gl.string(GL_VERSION)
    );
    let mut texture = 0u32;
    let mut framebuffer = 0u32;
    unsafe {
        (gl.gen_textures)(1, &mut texture);
        (gl.bind_texture)(GL_TEXTURE_2D, texture);
        (gl.tex_image_2d)(
            GL_TEXTURE_2D,
            0,
            GL_RGBA8,
            width as c_int,
            height as c_int,
            0,
            GL_RGBA,
            GL_UNSIGNED_BYTE,
            std::ptr::null(),
        );
        (gl.gen_framebuffers)(1, &mut framebuffer);
        (gl.bind_framebuffer)(GL_FRAMEBUFFER, framebuffer);
        (gl.framebuffer_texture_2d)(
            GL_FRAMEBUFFER,
            GL_COLOR_ATTACHMENT0,
            GL_TEXTURE_2D,
            texture,
            0,
        );
    }
    if unsafe { (gl.check_framebuffer_status)(GL_FRAMEBUFFER) } != GL_FRAMEBUFFER_COMPLETE {
        say!("FATAL fbo incomplete");
        return 2;
    }
    unsafe {
        (gl.viewport)(0, 0, width as c_int, height as c_int);
        if lit {
            (gl.enable)(GL_LIGHTING);
            (gl.enable)(GL_LIGHT0);
            (gl.enable)(GL_DEPTH_TEST);
            (gl.enable)(GL_NORMALIZE);
            (gl.enable)(GL_COLOR_MATERIAL);
            (gl.shade_model)(GL_SMOOTH);
            let position = [0.5f32, 5.0, 5.0, 0.0];
            (gl.lightfv)(GL_LIGHT0, GL_POSITION, position.as_ptr());
            (gl.matrix_mode)(GL_PROJECTION);
            (gl.load_identity)();
            (gl.matrix_mode)(GL_MODELVIEW);
            say!("MODE lit (lighting + smooth shading + depth test)");
        } else {
            say!("MODE plain");
        }
    }

    let start = std::time::Instant::now();
    let mut frames = 0u64;
    let mut errors = 0u64;
    while start.elapsed().as_secs_f64() < seconds as f64 {
        let phase = (frames % 60) as f32 / 60.0;
        unsafe {
            (gl.clear_color)(phase, 0.2, 1.0 - phase, 1.0);
            (gl.clear)(GL_COLOR_BUFFER_BIT);
            if lit {
                (gl.load_identity)();
                (gl.rotatef)((frames % 360) as f32, 0.0, 1.0, 0.0);
                (gl.begin)(GL_TRIANGLES);
                for i in 0..60 {
                    let angle = i as f32 * 6.0 * std::f32::consts::PI / 180.0;
                    let x = 0.8 * angle.cos();
                    let y = 0.8 * angle.sin();
                    (gl.normal3f)(x, y, 0.6);
                    (gl.color3f)(
                        0.2 + 0.4 * (i % 5) as f32 / 4.0,
                        0.6,
                        0.9 - 0.4 * (i % 3) as f32 / 2.0,
                    );
                    (gl.vertex3f)(x * 0.6, y * 0.6, 0.0);
                    (gl.vertex3f)(x, y, 0.0);
                    (gl.vertex3f)(x * 0.5 + 0.1, y * 0.5 - 0.1, 0.0);
                }
                (gl.end)();
            } else {
                (gl.begin)(GL_TRIANGLES);
                (gl.vertex3f)(-0.6, -0.5, 0.0);
                (gl.vertex3f)(0.6, -0.5, 0.0);
                (gl.vertex3f)(0.0, 0.6, 0.0);
                (gl.end)();
            }
            (gl.finish)();
        }
        frames += 1;
        if unsafe { (gl.get_error)() } != GL_NO_ERROR {
            errors += 1;
        }
    }
    let fps = frames as f64 / start.elapsed().as_secs_f64().max(1e-9);
    say!(
        "DONE {} {}x{} frames={} fps={:.1} errors={}",
        tag,
        width,
        height,
        frames,
        fps,
        errors
    );
    unsafe {
        (egl.make_current)(
            display,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );
        (egl.destroy_context)(display, context);
        (egl.terminate)(display);
    }
    if errors == 0 {
        0
    } else {
        1
    }
}

fn legacy_display_context(egl: &Egl) -> Option<(*mut Display, *mut Surface, Context)> {
    let display = unsafe {
        let d = (egl.get_platform_display)(
            EGL_PLATFORM_SURFACELESS_MESA,
            std::ptr::null_mut(),
            std::ptr::null(),
        );
        if d.is_null() {
            (egl.get_display)(std::ptr::null_mut())
        } else {
            d
        }
    };
    if display.is_null() {
        say!("FAIL eglGetDisplay(desktop GL)");
        return None;
    }
    let (mut major, mut minor) = (0, 0);
    if unsafe { (egl.initialize)(display, &mut major, &mut minor) } == 0 {
        say!("FAIL eglInitialize(desktop GL)");
        return None;
    }
    if unsafe { (egl.bind_api)(EGL_OPENGL_API) } == 0 {
        say!("FAIL eglBindAPI(OpenGL) err={:#x}", unsafe {
            (egl.get_error)()
        });
        return None;
    }
    let config_attrs = [
        EGL_SURFACE_TYPE,
        EGL_PBUFFER_BIT,
        EGL_RENDERABLE_TYPE,
        EGL_OPENGL_BIT,
        EGL_RED_SIZE,
        8,
        EGL_GREEN_SIZE,
        8,
        EGL_BLUE_SIZE,
        8,
        EGL_NONE,
    ];
    let mut config: Config = std::ptr::null_mut();
    let mut count = 0;
    if unsafe { (egl.choose_config)(display, config_attrs.as_ptr(), &mut config, 1, &mut count) }
        == 0
        || count < 1
    {
        say!("RESULT UNSUPPORTED (no desktop-GL EGL config)");
        return None;
    }
    let surface_attrs = [EGL_WIDTH, 64, EGL_HEIGHT, 64, EGL_NONE];
    let surface = unsafe { (egl.create_pbuffer_surface)(display, config, surface_attrs.as_ptr()) };
    if surface.is_null() {
        say!(
            "FAIL eglCreatePbufferSurface(desktop GL) err={:#x}",
            unsafe { (egl.get_error)() }
        );
        return None;
    }
    let ctx_attrs = [
        EGL_CONTEXT_MAJOR_VERSION,
        2,
        EGL_CONTEXT_MINOR_VERSION,
        1,
        EGL_NONE,
    ];
    let context =
        unsafe { (egl.create_context)(display, config, std::ptr::null_mut(), ctx_attrs.as_ptr()) };
    if context.is_null() {
        say!("FAIL eglCreateContext(desktop GL) err={:#x}", unsafe {
            (egl.get_error)()
        });
        return None;
    }
    if unsafe { (egl.make_current)(display, surface, surface, context) } == 0 {
        say!("FAIL eglMakeCurrent(desktop GL) err={:#x}", unsafe {
            (egl.get_error)()
        });
        return None;
    }
    Some((display, surface, context))
}

fn draw_fixed_gear(gl: &LegacyGl, angle_deg: f32) {
    const TEETH: usize = 48;
    const TAU: f32 = std::f32::consts::TAU;
    let step = TAU / TEETH as f32;
    unsafe {
        (gl.load_identity)();
        (gl.translatef)(0.0, 0.0, -3.6);
        (gl.rotatef)(angle_deg, 0.0, 0.0, 1.0);
        (gl.rotatef)(18.0, 1.0, 0.6, 0.0);

        for (z, normal_z, color) in [
            (0.18, 1.0, (0.20, 0.82, 1.0)),
            (-0.18, -1.0, (0.08, 0.28, 0.42)),
        ] {
            (gl.color3f)(color.0, color.1, color.2);
            (gl.begin)(GL_QUADS);
            (gl.normal3f)(0.0, 0.0, normal_z);
            for i in 0..TEETH {
                let a0 = i as f32 * step;
                let a1 = a0 + step * 0.28;
                let a2 = a0 + step * 0.66;
                let a3 = a0 + step * 0.94;
                let ring = |a: f32, r: f32| (a.cos() * r, a.sin() * r);
                let (ix0, iy0) = ring(a0, 0.76);
                let (ox1, oy1) = ring(a1, 1.02);
                let (ox2, oy2) = ring(a2, 1.02);
                let (ix3, iy3) = ring(a3, 0.76);
                if normal_z < 0.0 {
                    (gl.vertex3f)(ix0, iy0, z);
                    (gl.vertex3f)(ix3, iy3, z);
                    (gl.vertex3f)(ox2, oy2, z);
                    (gl.vertex3f)(ox1, oy1, z);
                } else {
                    (gl.vertex3f)(ix0, iy0, z);
                    (gl.vertex3f)(ox1, oy1, z);
                    (gl.vertex3f)(ox2, oy2, z);
                    (gl.vertex3f)(ix3, iy3, z);
                }
            }
            (gl.end)();
        }

        // A filled hub keeps the centre pixel useful even when the gear is
        // rotated; it also gives the compiler several more varying paths.
        (gl.color3f)(0.95, 0.75, 0.20);
        (gl.begin)(GL_TRIANGLE_FAN);
        (gl.normal3f)(0.0, 0.0, 1.0);
        (gl.vertex3f)(0.0, 0.0, 0.19);
        for i in 0..=32 {
            let a = i as f32 * TAU / 32.0;
            (gl.vertex3f)(a.cos() * 0.43, a.sin() * 0.43, 0.19);
        }
        (gl.end)();
    }
}

fn run_fixed(frames: u32) -> i32 {
    let Some(egl) = Egl::load() else {
        say!("FAIL dlopen EGL");
        return 1;
    };
    let Some((display, surface, context)) = legacy_display_context(&egl) else {
        return 77;
    };
    let Some(gl) = LegacyGl::load() else {
        say!("RESULT UNSUPPORTED (desktop libGL symbols missing)");
        return 77;
    };
    say!("FIXED_RENDERER={}", gl.string(GL_RENDERER));
    say!("FIXED_VERSION={}", gl.string(GL_VERSION));

    let light_pos = [2.0f32, 3.0, 4.0, 1.0];
    unsafe {
        (gl.viewport)(0, 0, 64, 64);
        (gl.enable)(GL_DEPTH_TEST);
        (gl.depth_func)(GL_LEQUAL);
        (gl.shade_model)(GL_SMOOTH);
        (gl.enable)(GL_LIGHTING);
        (gl.enable)(GL_LIGHT0);
        (gl.enable)(GL_COLOR_MATERIAL);
        (gl.enable)(GL_NORMALIZE);
        (gl.color_material)(GL_FRONT_AND_BACK, GL_AMBIENT_AND_DIFFUSE);
        (gl.lightfv)(GL_LIGHT0, GL_POSITION, light_pos.as_ptr());
        (gl.matrix_mode)(GL_PROJECTION);
        (gl.load_identity)();
        (gl.frustum)(-1.0, 1.0, -1.0, 1.0, 1.0, 20.0);
        (gl.matrix_mode)(GL_MODELVIEW);
        (gl.clear_color)(0.02, 0.03, 0.05, 1.0);
    }

    // Debugging aid for mapping the driver's crash PC to a module offset. It is
    // opt-in and capped so a typo cannot leave a test process sleeping forever.
    if let Ok(ms) = std::env::var("HUSK_GL_FIXED_DELAY_MS") {
        if let Ok(ms) = ms.parse::<u64>() {
            std::thread::sleep(std::time::Duration::from_millis(ms.min(30_000)));
        }
    }

    let started = std::time::Instant::now();
    for frame in 0..frames {
        unsafe {
            (gl.clear)(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
        }
        draw_fixed_gear(&gl, frame as f32 * 2.7);
        unsafe { (gl.finish)() };
    }
    let elapsed = started.elapsed().as_secs_f64();

    let mut pixel = [0u8; 4];
    unsafe {
        (gl.read_pixels)(
            32,
            32,
            1,
            1,
            GL_RGBA,
            GL_UNSIGNED_BYTE,
            pixel.as_mut_ptr() as *mut c_void,
        );
        (gl.finish)();
    }
    let err = unsafe { (gl.get_error)() };
    let nonblack = pixel[0] as u32 + pixel[1] as u32 + pixel[2] as u32;
    say!(
        "FIXED frames={frames} elapsed={elapsed:.3}s fps={:.1} pixel={},{},{},{} err={err:#x}",
        frames as f64 / elapsed.max(0.000_001),
        pixel[0],
        pixel[1],
        pixel[2],
        pixel[3]
    );

    unsafe {
        (egl.make_current)(
            display,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );
        (egl.destroy_context)(display, context);
        (egl.destroy_surface)(display, surface);
        (egl.terminate)(display);
    }

    if err == GL_NO_ERROR && nonblack > 20 {
        say!("RESULT PASS");
        0
    } else {
        say!("RESULT FAIL");
        5
    }
}

/// Surfaceless EGL display + ES3 pbuffer context, the path `gl-probe es3` uses.
fn es3_context(egl: &Egl) -> Option<(*mut Display, *mut Surface, Context)> {
    let display = unsafe {
        let d = (egl.get_platform_display)(
            EGL_PLATFORM_SURFACELESS_MESA,
            std::ptr::null_mut(),
            std::ptr::null(),
        );
        if d.is_null() {
            (egl.get_display)(std::ptr::null_mut())
        } else {
            d
        }
    };
    if display.is_null() {
        return None;
    }
    let mut major = 0;
    let mut minor = 0;
    if unsafe { (egl.initialize)(display, &mut major, &mut minor) } == 0 {
        return None;
    }
    say!("EGL {major}.{minor}");
    unsafe { (egl.bind_api)(EGL_OPENGL_ES_API) };

    let want3 = [
        EGL_SURFACE_TYPE,
        EGL_PBUFFER_BIT,
        EGL_RENDERABLE_TYPE,
        EGL_OPENGL_ES3_BIT,
        EGL_NONE,
    ];
    let want2 = [
        EGL_SURFACE_TYPE,
        EGL_PBUFFER_BIT,
        EGL_RENDERABLE_TYPE,
        EGL_OPENGL_ES2_BIT,
        EGL_NONE,
    ];
    let mut config: Config = std::ptr::null_mut();
    let mut got = 0;
    let has3 = unsafe { (egl.choose_config)(display, want3.as_ptr(), &mut config, 1, &mut got) }
        != 0
        && got > 0;
    say!("ES3 位 config: {}", if has3 { "有" } else { "没有" });
    if !has3 {
        let mut c2: Config = std::ptr::null_mut();
        let mut n2 = 0;
        let ok2 = unsafe { (egl.choose_config)(display, want2.as_ptr(), &mut c2, 1, &mut n2) } != 0
            && n2 > 0;
        if !ok2 {
            say!("FAIL eglChooseConfig(ES2 也没有)");
            return None;
        }
        say!("只有 ES2 位 config → 这台设备在当前设置下建不出 ES3 上下文");
        return None;
    }
    let surface_attrs = [EGL_WIDTH, 16, EGL_HEIGHT, 16, EGL_NONE];
    let surface = unsafe { (egl.create_pbuffer_surface)(display, config, surface_attrs.as_ptr()) };
    if surface.is_null() {
        say!("FAIL eglCreatePbufferSurface err={:#x}", unsafe {
            (egl.get_error)()
        });
        return None;
    }
    let ctx_attrs = [
        EGL_CONTEXT_MAJOR_VERSION,
        3,
        EGL_CONTEXT_MINOR_VERSION,
        0,
        EGL_NONE,
    ];
    let context =
        unsafe { (egl.create_context)(display, config, std::ptr::null_mut(), ctx_attrs.as_ptr()) };
    if context.is_null() {
        say!("FAIL eglCreateContext(ES3) err={:#x}", unsafe {
            (egl.get_error)()
        });
        return None;
    }
    if unsafe { (egl.make_current)(display, surface, surface, context) } == 0 {
        say!("FAIL eglMakeCurrent err={:#x}", unsafe {
            (egl.get_error)()
        });
        return None;
    }
    say!("ES3 上下文建立 OK");
    Some((display, surface, context))
}

fn shader(gl: &Gl, kind: c_uint, src: &str) -> Option<c_uint> {
    let shader = unsafe { (gl.create_shader)(kind) };
    if shader == 0 {
        return None;
    }
    let c = CString::new(src).ok()?;
    let ptr = c.as_ptr();
    unsafe {
        (gl.shader_source)(shader, 1, &ptr, std::ptr::null());
        (gl.compile_shader)(shader);
    }
    let mut ok = 0;
    unsafe { (gl.get_shader_iv)(shader, GL_COMPILE_STATUS, &mut ok) };
    if ok == 0 {
        let mut log = [0 as c_char; 512];
        unsafe { (gl.get_shader_info_log)(shader, 511, std::ptr::null_mut(), log.as_mut_ptr()) };
        let text = unsafe { std::ffi::CStr::from_ptr(log.as_ptr()) }
            .to_string_lossy()
            .into_owned();
        say!("FAIL 着色器: {text}");
        return None;
    }
    Some(shader)
}

fn run_es3() -> i32 {
    let Some(egl) = Egl::load() else {
        say!("FAIL dlopen EGL");
        return 1;
    };
    let Some((display, surface, context)) = es3_context(&egl) else {
        return 2;
    };
    let Some(gl) = Gl::load() else {
        say!("FAIL dlopen GLESv2");
        return 1;
    };
    unsafe { (gl.disable)(GL_DEPTH_TEST) };
    say!(
        "GL_VERSION={} | RENDERER={} | GLSL={}",
        gl.string(GL_VERSION),
        gl.string(GL_RENDERER),
        gl.string(GL_SHADING_LANGUAGE_VERSION)
    );

    let vs_src = "#version 300 es\n\
        layout(location=0) in vec2 pos;\n\
        layout(location=1) in vec2 inst;\n\
        out vec2 vUV;\n\
        void main(){ vUV = pos + 0.5; gl_Position = vec4(pos + inst, 0.0, 1.0); }\n";
    let fs_src = "#version 300 es\n\
        precision highp float;\n\
        in vec2 vUV;\n\
        uniform sampler2D tex;\n\
        layout(std140) uniform B { vec4 color; vec2 off; vec2 pad; } u;\n\
        layout(location=0) out vec4 o0;\n\
        layout(location=1) out vec4 o1;\n\
        void main(){ vec4 t = texture(tex, vUV); o0 = u.color * t; o1 = vec4(1.0) - o0; }\n";
    let (Some(vs), Some(fs)) = (
        shader(&gl, GL_VERTEX_SHADER, vs_src),
        shader(&gl, GL_FRAGMENT_SHADER, fs_src),
    ) else {
        return 3;
    };
    let program = unsafe { (gl.create_program)() };
    unsafe {
        (gl.attach_shader)(program, vs);
        (gl.attach_shader)(program, fs);
        (gl.link_program)(program);
    }
    let mut linked = 0;
    unsafe { (gl.get_program_iv)(program, GL_LINK_STATUS, &mut linked) };
    if linked == 0 {
        let mut log = [0 as c_char; 512];
        unsafe { (gl.get_program_info_log)(program, 511, std::ptr::null_mut(), log.as_mut_ptr()) };
        let text = unsafe { std::ffi::CStr::from_ptr(log.as_ptr()) }
            .to_string_lossy()
            .into_owned();
        say!("FAIL 链接: {text}");
        return 3;
    }
    unsafe { (gl.use_program)(program) };
    say!("GLSL ES 3.00 着色器编译+链接 OK");

    // Geometry: unit quad, instanced into a 2x2 grid.
    let quad: [f32; 8] = [-0.5, -0.5, 0.5, -0.5, -0.5, 0.5, 0.5, 0.5];
    let inst: [f32; 8] = [-0.5, -0.5, 0.5, -0.5, -0.5, 0.5, 0.5, 0.5];
    let mut vao = 0u32;
    unsafe {
        (gl.gen_vertex_arrays)(1, &mut vao);
        (gl.bind_vertex_array)(vao);
    }
    let mut vb = [0u32; 2];
    unsafe {
        (gl.gen_buffers)(2, vb.as_mut_ptr());
        (gl.bind_buffer)(GL_ARRAY_BUFFER, vb[0]);
        (gl.buffer_data)(
            GL_ARRAY_BUFFER,
            std::mem::size_of_val(&quad) as isize,
            quad.as_ptr() as *const c_void,
            GL_STATIC_DRAW,
        );
        (gl.vertex_attrib_pointer)(0, 2, GL_FLOAT, 0, 8, std::ptr::null());
        (gl.enable_vertex_attrib_array)(0);
        (gl.bind_buffer)(GL_ARRAY_BUFFER, vb[1]);
        (gl.buffer_data)(
            GL_ARRAY_BUFFER,
            std::mem::size_of_val(&inst) as isize,
            inst.as_ptr() as *const c_void,
            GL_STATIC_DRAW,
        );
        (gl.vertex_attrib_pointer)(1, 2, GL_FLOAT, 0, 8, std::ptr::null());
        (gl.enable_vertex_attrib_array)(1);
        (gl.vertex_attrib_divisor)(1, 1);
    }

    // std140 block: vec4 color + vec2 off + vec2 pad = 32 bytes.
    let block: [f32; 8] = [0.5, 0.5, 0.5, 1.0, 0.0, 0.0, 0.0, 0.0];
    let mut ub = 0u32;
    unsafe {
        (gl.gen_buffers)(1, &mut ub);
        (gl.bind_buffer)(GL_UNIFORM_BUFFER, ub);
        (gl.buffer_data)(
            GL_UNIFORM_BUFFER,
            std::mem::size_of_val(&block) as isize,
            block.as_ptr() as *const c_void,
            GL_STATIC_DRAW,
        );
        (gl.bind_buffer_base)(GL_UNIFORM_BUFFER, 0, ub);
    }

    // 2x2 texture: red green / blue white.
    let texels: [u8; 16] = [
        255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
    ];
    let mut tex = 0u32;
    unsafe {
        (gl.gen_textures)(1, &mut tex);
        (gl.active_texture)(GL_TEXTURE0);
        (gl.bind_texture)(GL_TEXTURE_2D, tex);
        (gl.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
        (gl.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
        (gl.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
        (gl.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
        (gl.pixel_storei)(GL_UNPACK_ALIGNMENT, 1);
        (gl.tex_image_2d)(
            GL_TEXTURE_2D,
            0,
            GL_RGBA8,
            2,
            2,
            0,
            GL_RGBA,
            GL_UNSIGNED_BYTE,
            texels.as_ptr() as *const c_void,
        );
        let name = CString::new("tex").unwrap();
        (gl.uniform1i)((gl.get_uniform_location)(program, name.as_ptr()), 0);
    }

    // MRT framebuffer: two RGBA8 attachments.
    let mut fbo = 0u32;
    let mut rt = [0u32; 2];
    unsafe {
        (gl.gen_framebuffers)(1, &mut fbo);
        (gl.bind_framebuffer)(GL_FRAMEBUFFER, fbo);
        (gl.gen_textures)(2, rt.as_mut_ptr());
        for (i, texture) in rt.iter().enumerate() {
            (gl.bind_texture)(GL_TEXTURE_2D, *texture);
            (gl.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
            (gl.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
            (gl.tex_image_2d)(
                GL_TEXTURE_2D,
                0,
                GL_RGBA8,
                16,
                16,
                0,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                std::ptr::null(),
            );
            let attachment = if i == 0 {
                GL_COLOR_ATTACHMENT0
            } else {
                GL_COLOR_ATTACHMENT1
            };
            (gl.framebuffer_texture_2d)(GL_FRAMEBUFFER, attachment, GL_TEXTURE_2D, *texture, 0);
        }
        // The two binds above replaced unit 0's texture with an FBO attachment;
        // rebind the 2x2 texture or the shader samples itself (feedback loop).
        (gl.active_texture)(GL_TEXTURE0);
        (gl.bind_texture)(GL_TEXTURE_2D, tex);
    }
    let status = unsafe { (gl.check_framebuffer_status)(GL_FRAMEBUFFER) };
    if status != GL_FRAMEBUFFER_COMPLETE {
        say!("FAIL FBO 不完整 status={status:#x}");
        return 4;
    }
    let buffers = [GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1];
    unsafe { (gl.draw_buffers)(2, buffers.as_ptr()) };
    say!(
        "MRT(2 附件) FBO 完整 OK, drawBuffers 已设, err={:#x}",
        unsafe { (gl.get_error)() }
    );

    // Phase A: clear only -- both attachments must be writable and readable.
    let (mut a0, mut a1) = ([9u8; 4], [9u8; 4]);
    unsafe {
        (gl.viewport)(0, 0, 16, 16);
        (gl.clear_color)(0.25, 0.5, 0.75, 1.0);
        (gl.clear)(GL_COLOR_BUFFER_BIT);
        (gl.finish)();
        (gl.pixel_storei)(GL_PACK_ALIGNMENT, 1);
        (gl.read_buffer)(GL_COLOR_ATTACHMENT0);
        (gl.read_pixels)(
            8,
            8,
            1,
            1,
            GL_RGBA,
            GL_UNSIGNED_BYTE,
            a0.as_mut_ptr() as *mut c_void,
        );
        (gl.read_buffer)(GL_COLOR_ATTACHMENT1);
        (gl.read_pixels)(
            8,
            8,
            1,
            1,
            GL_RGBA,
            GL_UNSIGNED_BYTE,
            a1.as_mut_ptr() as *mut c_void,
        );
    }
    say!(
        "阶段A 清屏回读: att0={},{},{},{}  att1={},{},{},{} （期望 64,128,191,255）",
        a0[0],
        a0[1],
        a0[2],
        a0[3],
        a1[0],
        a1[1],
        a1[2],
        a1[3]
    );

    unsafe {
        (gl.viewport)(0, 0, 16, 16);
        (gl.clear_color)(0.0, 0.0, 0.0, 0.0);
        (gl.clear)(GL_COLOR_BUFFER_BIT);
    }
    say!("清除后 err={:#x}", unsafe { (gl.get_error)() });
    unsafe {
        (gl.draw_arrays_instanced)(GL_TRIANGLE_STRIP, 0, 4, 4);
        (gl.finish)();
    }
    say!("绘制后 err={:#x}", unsafe { (gl.get_error)() });

    // Per-pixel 1x1 readback keeps failures attributable to one attachment/pixel.
    // Whole-block correctness is covered separately by `gl-probe readback`.
    let mut px = [[0u8; 16 * 16 * 4]; 2];
    for (i, attachment) in [GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1]
        .iter()
        .enumerate()
    {
        unsafe {
            (gl.read_buffer)(*attachment);
            (gl.pixel_storei)(GL_PACK_ALIGNMENT, 1);
        }
        for y in 0..16 {
            for x in 0..16 {
                let offset = ((y * 16 + x) * 4) as usize;
                unsafe {
                    (gl.read_pixels)(
                        x,
                        y,
                        1,
                        1,
                        GL_RGBA,
                        GL_UNSIGNED_BYTE,
                        px[i][offset..].as_mut_ptr() as *mut c_void,
                    );
                }
            }
        }
    }
    let expect: [[u8; 4]; 4] = [
        [128, 0, 0, 255],
        [0, 128, 0, 255],
        [0, 0, 128, 255],
        [128, 128, 128, 255],
    ];
    let (mut bad0, mut bad1) = (0, 0);
    let mut seen = [0i32; 4];
    for y in 0..16 {
        for x in 0..16 {
            let o = (y * 16 + x) * 4;
            let mut hit = -1i32;
            for (k, want) in expect.iter().enumerate() {
                let mut worst = 0i32;
                for ch in 0..4 {
                    let diff = (px[0][o + ch] as i32 - want[ch] as i32).abs();
                    worst = worst.max(diff);
                }
                if worst <= 3 {
                    hit = k as i32;
                    break;
                }
            }
            if hit < 0 {
                if bad0 < 3 {
                    say!(
                        "  att0 异常像素 ({},{})={},{},{},{}",
                        x,
                        y,
                        px[0][o],
                        px[0][o + 1],
                        px[0][o + 2],
                        px[0][o + 3]
                    );
                }
                bad0 += 1;
            } else {
                seen[hit as usize] += 1;
            }
            for ch in 0..4 {
                let want = 255 - px[0][o + ch] as i32;
                if (px[1][o + ch] as i32 - want).abs() > 3 {
                    if bad1 < 3 {
                        say!(
                            "  att1 与 1-att0 不符 ({},{}) ch{} {} vs {}",
                            x,
                            y,
                            ch,
                            px[1][o + ch],
                            want
                        );
                    }
                    bad1 += 1;
                    break;
                }
            }
        }
    }
    say!(
        "att0 像素异常数={}  纹理四色命中数={}/{}/{}/{}",
        bad0,
        seen[0],
        seen[1],
        seen[2],
        seen[3]
    );
    say!("att1（MRT 第二附件）与 1-att0 不符数={bad1}");
    let pass = bad0 == 0 && bad1 == 0 && seen.iter().all(|n| *n > 0);

    unsafe {
        (egl.make_current)(
            display,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );
        (egl.destroy_context)(display, context);
        (egl.destroy_surface)(display, surface);
        (egl.terminate)(display);
    }
    say!("{}", if pass { "RESULT PASS" } else { "RESULT FAIL" });
    if pass {
        // A readback that silently returns zeros would otherwise pass.
        assert_ne!(a0[3], 0);
        0
    } else {
        5
    }
}

/// Pure X pixel transport, no GL at all: a client 32-bit buffer of `width`x
/// `height` goes to a window of the same size with `XPutImage` + `XSync`,
/// `frames` times. This is the copy half of the presentation cost on its own;
/// `window - pbuf - xput` is what the GPU readback adds on top.
fn run_xput(width: u32, height: u32, frames: u32) -> i32 {
    let x11 = match Lib::open("libX11.so.6") {
        Some(l) => l,
        None => {
            say!("FAIL dlopen X11");
            return 1;
        }
    };
    unsafe {
        let open: unsafe extern "C" fn(*const c_char) -> *mut c_void =
            cast(x11.sym("XOpenDisplay"));
        let root_of: unsafe extern "C" fn(*mut c_void) -> Window =
            cast(x11.sym("XDefaultRootWindow"));
        let def_screen: unsafe extern "C" fn(*mut c_void) -> c_int =
            cast(x11.sym("XDefaultScreen"));
        let def_depth: unsafe extern "C" fn(*mut c_void, c_int) -> c_int =
            cast(x11.sym("XDefaultDepth"));
        let def_visual: unsafe extern "C" fn(*mut c_void, c_int) -> *mut c_void =
            cast(x11.sym("XDefaultVisual"));
        let create_image: unsafe extern "C" fn(
            *mut c_void,
            *mut c_void,
            c_uint,
            c_int,
            c_int,
            *mut c_char,
            c_uint,
            c_uint,
            c_int,
            c_int,
        ) -> *mut c_void = cast(x11.sym("XCreateImage"));
        let create_gc: unsafe extern "C" fn(
            *mut c_void,
            Window,
            c_ulong,
            *mut c_void,
        ) -> *mut c_void = cast(x11.sym("XCreateGC"));
        let put_image: unsafe extern "C" fn(
            *mut c_void,
            Window,
            *mut c_void,
            *mut c_void,
            c_int,
            c_int,
            c_int,
            c_int,
            c_uint,
            c_uint,
        ) = cast(x11.sym("XPutImage"));
        let create: unsafe extern "C" fn(
            *mut c_void,
            Window,
            c_int,
            c_int,
            c_uint,
            c_uint,
            c_uint,
            c_ulong,
            c_ulong,
            c_int,
            c_uint,
            *mut c_void,
        ) -> Window = cast(x11.sym("XCreateSimpleWindow"));
        let map: unsafe extern "C" fn(*mut c_void, Window) -> c_int = cast(x11.sym("XMapWindow"));
        let sync: unsafe extern "C" fn(*mut c_void, c_int) -> c_int = cast(x11.sym("XSync"));

        let display = open(std::ptr::null());
        if display.is_null() {
            say!("FAIL XOpenDisplay");
            return 1;
        }
        let root = root_of(display);
        let screen = def_screen(display);
        let depth = def_depth(display, screen);
        let visual = def_visual(display, screen);
        let window = create(
            display,
            root,
            0,
            0,
            width,
            height,
            0,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
        );
        map(display, window);
        sync(display, 0);
        say!("stage 2: window mapped id={window}");

        // XCreateImage with a caller buffer: no allocation, no copy on our side.
        let stride = (width as usize) * 4;
        let mut pixels = vec![0u8; stride * height as usize];
        for (i, b) in pixels.iter_mut().enumerate() {
            *b = (i % 251) as u8;
        }
        let image = create_image(
            display,
            visual,
            depth as c_uint,
            2, // ZPixmap
            0,
            pixels.as_mut_ptr() as *mut c_char,
            width,
            height,
            32,
            stride as c_int,
        );
        if image.is_null() {
            say!("FAIL XCreateImage");
            return 1;
        }
        let gc = create_gc(display, window, 0, std::ptr::null_mut());
        if gc.is_null() {
            say!("FAIL XCreateGC");
            return 1;
        }
        for _ in 0..3 {
            put_image(display, window, gc, image, 0, 0, 0, 0, width, height);
            sync(display, 0);
        }
        let t0 = std::time::Instant::now();
        for _ in 0..frames {
            put_image(display, window, gc, image, 0, 0, 0, 0, width, height);
            sync(display, 0);
        }
        let dt = t0.elapsed().as_secs_f64();
        say!(
            "stage 10: {frames} XPutImage OK ({:.1} ms, {:.1} fps) {}x{} depth{depth}",
            dt * 1000.0,
            frames as f64 / dt,
            width,
            height
        );
    }
    0
}

/// GPU-bound benchmark: a full-screen triangle strip whose fragment shader runs
/// 48 dependent `sin()` per pixel, drawn `passes` times per frame into a pbuffer,
/// with `glFinish` per frame. A clear (what `pbuf` measures) is the worst case for
/// a GPU - the work is trivial and the per-command overhead dominates. This is the
/// shape where hardware acceleration is supposed to show, so this is what the
/// README quotes.
fn run_fill(width: u32, height: u32, passes: u32, frames: u32) -> i32 {
    let Some(egl) = Egl::load() else {
        say!("FAIL dlopen EGL");
        return 1;
    };
    unsafe {
        let display = (egl.get_platform_display)(
            EGL_PLATFORM_SURFACELESS_MESA,
            std::ptr::null_mut(),
            std::ptr::null(),
        );
        if display.is_null() {
            say!("FAIL eglGetPlatformDisplay(surfaceless)");
            return 1;
        }
        let mut major = 0;
        let mut minor = 0;
        if (egl.initialize)(display, &mut major, &mut minor) == 0 {
            say!("FAIL eglInitialize");
            return 1;
        }
        (egl.bind_api)(EGL_OPENGL_ES_API);
        let config_attrs = [
            EGL_RENDERABLE_TYPE,
            EGL_OPENGL_ES2_BIT,
            EGL_RED_SIZE,
            8,
            EGL_GREEN_SIZE,
            8,
            EGL_BLUE_SIZE,
            8,
            EGL_ALPHA_SIZE,
            8,
            EGL_SURFACE_TYPE,
            EGL_PBUFFER_BIT,
            EGL_NONE,
        ];
        let mut config: Config = std::ptr::null_mut();
        let mut count = 0;
        if (egl.choose_config)(display, config_attrs.as_ptr(), &mut config, 1, &mut count) == 0
            || count < 1
        {
            say!("FAIL eglChooseConfig n={count}");
            return 1;
        }
        let surface_attrs = [EGL_WIDTH, width as i32, EGL_HEIGHT, height as i32, EGL_NONE];
        let surface = (egl.create_pbuffer_surface)(display, config, surface_attrs.as_ptr());
        if surface.is_null() {
            say!("FAIL eglCreatePbufferSurface err={:#x}", (egl.get_error)());
            return 1;
        }
        let ctx_attrs = [EGL_CONTEXT_MAJOR_VERSION, 2, EGL_NONE];
        let context =
            (egl.create_context)(display, config, std::ptr::null_mut(), ctx_attrs.as_ptr());
        if context.is_null() {
            say!("FAIL eglCreateContext err={:#x}", (egl.get_error)());
            return 1;
        }
        if (egl.make_current)(display, surface, surface, context) == 0 {
            say!("FAIL eglMakeCurrent err={:#x}", (egl.get_error)());
            return 1;
        }
        let Some(gl) = Gl::load() else {
            say!("FAIL dlopen GLESv2");
            return 1;
        };
        say!("renderer={}", gl.string(GL_RENDERER));

        let vs = b"attribute vec2 p; varying vec2 uv;\nvoid main(){ uv = p*0.5+0.5; gl_Position = vec4(p,0.0,1.0); }\n";
        let fs = b"precision highp float; varying vec2 uv;\nvoid main(){ vec3 c = vec3(uv,0.5);\n for (int i=0;i<48;i++){ c = sin(c*3.14159 + vec3(1e-3,-9e-4,7e-4)); }\n gl_FragColor = vec4(c,1.0); }\n";
        let mut ok = true;
        let mut program = 0u32;
        for (kind, src) in [(0x8B31u32, &vs[..]), (0x8B30u32, &fs[..])] {
            let sh = (gl.create_shader)(kind);
            let ptr = src.as_ptr() as *const c_char;
            let len = src.len() as c_int;
            (gl.shader_source)(sh, 1, &ptr, &len);
            (gl.compile_shader)(sh);
            let mut st = 0;
            (gl.get_shader_iv)(sh, 0x8B81, &mut st);
            if st == 0 {
                let mut buf = [0u8; 512];
                (gl.get_shader_info_log)(
                    sh,
                    512,
                    std::ptr::null_mut(),
                    buf.as_mut_ptr() as *mut c_char,
                );
                let msg = std::ffi::CStr::from_ptr(buf.as_ptr() as *const c_char)
                    .to_string_lossy()
                    .to_string();
                say!("FAIL shader compile: {msg}");
                ok = false;
            }
            if program == 0 {
                program = (gl.create_program)();
            }
            (gl.attach_shader)(program, sh);
        }
        if !ok {
            return 1;
        }
        (gl.link_program)(program);
        let mut link = 0;
        (gl.get_program_iv)(program, 0x8B82, &mut link);
        if link == 0 {
            say!("FAIL link");
            return 1;
        }
        (gl.use_program)(program);
        let verts: [f32; 8] = [-1.0, -1.0, 1.0, -1.0, -1.0, 1.0, 1.0, 1.0];
        let mut vb = 0u32;
        (gl.gen_buffers)(1, &mut vb);
        (gl.bind_buffer)(0x8892, vb);
        (gl.buffer_data)(
            0x8892,
            (verts.len() * 4) as isize,
            verts.as_ptr() as *const c_void,
            0x88E4,
        );
        // `p` is an attribute, not a uniform: glGetUniformLocation returns -1 for
        // it, and with no attrib bound the strip collapses and the fragment
        // shader never runs (measured: 0.23 ms/frame on llvmpipe, i.e. nothing).
        let loc = (gl.get_attrib_location)(program, c"p".as_ptr());
        if loc < 0 {
            say!("FAIL glGetAttribLocation(p) = {loc}");
            return 1;
        }
        (gl.enable_vertex_attrib_array)(loc as u32);
        (gl.vertex_attrib_pointer)(loc as u32, 2, 0x1406, 0, 0, std::ptr::null());
        (gl.viewport)(0, 0, width as c_int, height as c_int);
        // one warm-up frame
        for _ in 0..passes {
            (gl.draw_arrays)(5, 0, 4); // GL_TRIANGLE_STRIP
        }
        (gl.finish)();
        // One pixel back, so a collapsed draw cannot pass as a fast one.
        (gl.pixel_storei)(0x0D05 /*GL_PACK_ALIGNMENT*/, 4);
        let mut px = [0u8; 4];
        (gl.read_pixels)(
            0,
            0,
            1,
            1,
            0x1908,
            0x1401,
            px.as_mut_ptr() as *const c_void as *mut c_void,
        );
        say!("pixel(0,0)={},{},{},{}", px[0], px[1], px[2], px[3]);
        if px[0] == 0 && px[1] == 0 && px[2] == 0 {
            say!("WARN 读回 (0,0) 是全黑，draw 可能没落地");
        }
        let t0 = std::time::Instant::now();
        for _ in 0..frames {
            for _ in 0..passes {
                (gl.draw_arrays)(5, 0, 4); // GL_TRIANGLE_STRIP
            }
            (gl.finish)();
        }
        let dt = t0.elapsed().as_secs_f64();
        say!(
            "stage 10: {frames} frames x {passes} passes OK ({:.1} ms, {:.1} fps) {}x{} fill",
            dt * 1000.0,
            frames as f64 / dt,
            width,
            height
        );
    }
    0
}

/// Offscreen EGL: a pbuffer of `width`x`height`, the same clear+swap loop as
/// `window`, no X server in the path. Subtracting this from `window` at the same
/// size is the presentation cost alone (the SHM handoff to the X server), which
/// is the number that decides whether a zero-copy path is worth building.
fn run_pbuf(width: u32, height: u32, frames: u32) -> i32 {
    let Some(egl) = Egl::load() else {
        say!("FAIL dlopen EGL");
        return 1;
    };
    unsafe {
        let display = (egl.get_platform_display)(
            EGL_PLATFORM_SURFACELESS_MESA,
            std::ptr::null_mut(),
            std::ptr::null(),
        );
        if display.is_null() {
            say!("FAIL eglGetPlatformDisplay(surfaceless)");
            return 1;
        }
        let mut major = 0;
        let mut minor = 0;
        if (egl.initialize)(display, &mut major, &mut minor) == 0 {
            say!("FAIL eglInitialize");
            return 1;
        }
        say!("stage 3: eglInitialize {major}.{minor}");
        (egl.bind_api)(EGL_OPENGL_ES_API);
        let config_attrs = [
            EGL_RENDERABLE_TYPE,
            EGL_OPENGL_ES2_BIT,
            EGL_RED_SIZE,
            8,
            EGL_GREEN_SIZE,
            8,
            EGL_BLUE_SIZE,
            8,
            EGL_ALPHA_SIZE,
            8,
            EGL_SURFACE_TYPE,
            EGL_PBUFFER_BIT,
            EGL_NONE,
        ];
        let mut config: Config = std::ptr::null_mut();
        let mut count = 0;
        if (egl.choose_config)(display, config_attrs.as_ptr(), &mut config, 1, &mut count) == 0
            || count < 1
        {
            say!("FAIL eglChooseConfig n={count}");
            return 1;
        }
        let surface_attrs = [EGL_WIDTH, width as i32, EGL_HEIGHT, height as i32, EGL_NONE];
        let surface = (egl.create_pbuffer_surface)(display, config, surface_attrs.as_ptr());
        if surface.is_null() {
            say!("FAIL eglCreatePbufferSurface err={:#x}", (egl.get_error)());
            return 1;
        }
        let ctx_attrs = [EGL_CONTEXT_MAJOR_VERSION, 2, EGL_NONE];
        let context =
            (egl.create_context)(display, config, std::ptr::null_mut(), ctx_attrs.as_ptr());
        if context.is_null() {
            say!("FAIL eglCreateContext err={:#x}", (egl.get_error)());
            return 1;
        }
        if (egl.make_current)(display, surface, surface, context) == 0 {
            say!("FAIL eglMakeCurrent err={:#x}", (egl.get_error)());
            return 1;
        }
        let Some(gl) = Gl::load() else {
            say!("FAIL dlopen GLESv2");
            return 1;
        };
        say!("renderer={}", gl.string(GL_RENDERER));
        (gl.viewport)(0, 0, width as c_int, height as c_int);
        let swap_t0 = std::time::Instant::now();
        for i in 0..frames {
            let color = ((i % 2) as f32, ((i / 2) % 2) as f32, ((i / 4) % 2) as f32);
            (gl.clear_color)(color.0, color.1, color.2, 1.0);
            (gl.clear)(GL_COLOR_BUFFER_BIT);
            // A pbuffer swap is a no-op: without a finish this loop only records
            // Vulkan commands (measured 0.1 ms for 60 frames) instead of running
            // them, and the number would mean nothing.
            (gl.finish)();
            if (egl.swap_buffers)(display, surface) == 0 {
                say!(
                    "FAIL eglSwapBuffers at frame {i} err={:#x}",
                    (egl.get_error)()
                );
                return 1;
            }
        }
        let dt = swap_t0.elapsed().as_secs_f64();
        say!(
            "stage 10: {frames} swaps OK ({:.1} ms, {:.1} fps) {}x{} pbuffer",
            dt * 1000.0,
            frames as f64 / dt,
            width,
            height
        );
    }
    0
}

/// `glReadPixels` 的地址学。evidence 162 记录了「整块读只写第一行 + 第一列」，
/// 但没定原因。这里把一张**每像素颜色都不同**的 RGBA8 纹理挂到 FBO 上，整块读回进
/// 预填 `0xAA` 的缓冲区，然后逐个线性偏移报「谁被写了、写的值是哪一格的」，用值本身
/// 判定是行距算错、x/y 写反、还是每行只拷了 1 像素。
///
/// 输出四段：A 整块、B 逐行（Wx1 × H 次）、C 逐列（1xH × W 次）、D 单点采样。
/// A 段是现象，B/C 段是**能不能绕过去**的答案。
fn run_readback(width: u32, height: u32) -> i32 {
    let Some(egl) = Egl::load() else {
        say!("FAIL dlopen EGL");
        return 1;
    };
    unsafe {
        let display = (egl.get_platform_display)(
            EGL_PLATFORM_SURFACELESS_MESA,
            std::ptr::null_mut(),
            std::ptr::null(),
        );
        if display.is_null() {
            say!("FAIL eglGetPlatformDisplay(surfaceless)");
            return 1;
        }
        let (mut major, mut minor) = (0, 0);
        if (egl.initialize)(display, &mut major, &mut minor) == 0 {
            say!("FAIL eglInitialize");
            return 1;
        }
        (egl.bind_api)(EGL_OPENGL_ES_API);
        let config_attrs = [
            EGL_RENDERABLE_TYPE,
            EGL_OPENGL_ES2_BIT,
            EGL_RED_SIZE,
            8,
            EGL_GREEN_SIZE,
            8,
            EGL_BLUE_SIZE,
            8,
            EGL_ALPHA_SIZE,
            8,
            EGL_SURFACE_TYPE,
            EGL_PBUFFER_BIT,
            EGL_NONE,
        ];
        let mut config: Config = std::ptr::null_mut();
        let mut count = 0;
        if (egl.choose_config)(display, config_attrs.as_ptr(), &mut config, 1, &mut count) == 0
            || count < 1
        {
            say!("FAIL eglChooseConfig n={count}");
            return 1;
        }
        // 1x1 pbuffer 只是为了让上下文 current；读回走 FBO，不受它大小影响。
        let surface_attrs = [EGL_WIDTH, 1, EGL_HEIGHT, 1, EGL_NONE];
        let surface = (egl.create_pbuffer_surface)(display, config, surface_attrs.as_ptr());
        if surface.is_null() {
            say!("FAIL eglCreatePbufferSurface err={:#x}", (egl.get_error)());
            return 1;
        }
        let ctx_attrs = [EGL_CONTEXT_MAJOR_VERSION, 2, EGL_NONE];
        let context =
            (egl.create_context)(display, config, std::ptr::null_mut(), ctx_attrs.as_ptr());
        if context.is_null() {
            say!("FAIL eglCreateContext err={:#x}", (egl.get_error)());
            return 1;
        }
        if (egl.make_current)(display, surface, surface, context) == 0 {
            say!("FAIL eglMakeCurrent err={:#x}", (egl.get_error)());
            return 1;
        }
        let Some(gl) = Gl::load() else {
            say!("FAIL dlopen GLESv2");
            return 1;
        };
        say!("renderer={}", gl.string(GL_RENDERER));

        let (w, h) = (width as usize, height as usize);
        let npix = w * h;
        // 每像素唯一：R 只跟 x 有关，G 只跟 y 有关，B 两者混合，A 固定 255。
        let expect = |x: usize, y: usize| -> [u8; 4] {
            [
                (x * 7 + 3) as u8,
                (y * 11 + 5) as u8,
                ((x * 13 + y * 17) % 251) as u8,
                255,
            ]
        };
        let mut src = vec![0u8; npix * 4];
        for y in 0..h {
            for x in 0..w {
                let e = expect(x, y);
                let o = (y * w + x) * 4;
                src[o..o + 4].copy_from_slice(&e);
            }
        }
        let mut tex = 0u32;
        (gl.gen_textures)(1, &mut tex);
        (gl.bind_texture)(GL_TEXTURE_2D, tex);
        (gl.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
        (gl.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
        (gl.tex_image_2d)(
            GL_TEXTURE_2D,
            0,
            GL_RGBA8,
            w as c_int,
            h as c_int,
            0,
            GL_RGBA,
            GL_UNSIGNED_BYTE,
            src.as_ptr() as *const c_void,
        );
        let mut fbo = 0u32;
        (gl.gen_framebuffers)(1, &mut fbo);
        (gl.bind_framebuffer)(GL_FRAMEBUFFER, fbo);
        (gl.framebuffer_texture_2d)(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
        let st = (gl.check_framebuffer_status)(GL_FRAMEBUFFER);
        say!("stage 3: FBO status={st:#x}（0x8cd5 为完整）");
        if st != GL_FRAMEBUFFER_COMPLETE {
            say!("FAIL FBO incomplete");
            return 1;
        }
        (gl.read_buffer)(GL_COLOR_ATTACHMENT0);
        (gl.pixel_storei)(GL_PACK_ALIGNMENT, 4);

        // ---- A：整块一次读
        let mut buf = vec![0xAAu8; npix * 4];
        (gl.read_pixels)(
            0,
            0,
            w as c_int,
            h as c_int,
            GL_RGBA,
            GL_UNSIGNED_BYTE,
            buf.as_mut_ptr() as *mut c_void,
        );
        let mut written = 0usize;
        let mut hit = 0usize;
        let mut detail = Vec::new();
        for i in 0..npix {
            let px = &buf[i * 4..i * 4 + 4];
            if px.iter().all(|b| *b == 0xAA) {
                continue;
            }
            written += 1;
            let (x, y) = (i % w, i / w);
            if px == &expect(x, y)[..] {
                hit += 1;
            } else if detail.len() < 200 {
                // want_yx：若 x/y 被写反，这个偏移上应该出现对角的那个值
                let yx = expect(y % w, x % h);
                // 从值本身反解「这块数据原本是哪一格」：R=x*7+3、G=y*11+5 在宽度 <37 时唯一
                let sx = (px[0].wrapping_sub(3) as usize).checked_div(7);
                let sy = (px[1].wrapping_sub(5) as usize).checked_div(11);
                let sane = px[0].wrapping_sub(3) % 7 == 0 && px[1].wrapping_sub(5) % 11 == 0;
                detail.push(if sane {
                    format!(
                        "  w idx={i} (x={x},y={y}) src=({},{}) want(x,y)={:02x?}",
                        sx.unwrap_or(0),
                        sy.unwrap_or(0),
                        expect(x, y)
                    )
                } else {
                    format!(
                        "  w idx={i} (x={x},y={y}) got={px:02x?} want(x,y)={:02x?} want(y,x)={yx:02x?}",
                        expect(x, y)
                    )
                });
            }
        }
        say!("=== A 整块 {w}x{h}（{npix} 像素，预填 0xAA）");
        say!("  被写 {written}/{npix}（期望 {npix}），值正确 {hit}/{npix}");
        for d in &detail {
            say!("{d}");
        }

        // ---- A2/A3：换个 pack 参数再整块读，看是不是参数没喂对
        for (label, rowlen, align) in [
            ("PACK_ROW_LENGTH=W", w as c_int, 4),
            ("PACK_ALIGNMENT=1", 0, 1),
        ] {
            (gl.pixel_storei)(GL_PACK_ROW_LENGTH, rowlen);
            (gl.pixel_storei)(GL_PACK_ALIGNMENT, align);
            let mut b2 = vec![0xAAu8; npix * 4];
            (gl.read_pixels)(
                0,
                0,
                w as c_int,
                h as c_int,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                b2.as_mut_ptr() as *mut c_void,
            );
            let mut w2 = 0usize;
            let mut h2 = 0usize;
            for i in 0..npix {
                let px = &b2[i * 4..i * 4 + 4];
                if px.iter().all(|b| *b == 0xAA) {
                    continue;
                }
                w2 += 1;
                let (x, y) = (i % w, i / w);
                if px == &expect(x, y)[..] {
                    h2 += 1;
                }
            }
            say!("=== {label}：被写 {w2}/{npix}，值正确 {h2}/{npix}");
        }
        (gl.pixel_storei)(GL_PACK_ROW_LENGTH, 0);
        (gl.pixel_storei)(GL_PACK_ALIGNMENT, 4);

        // ---- B：逐行读（Wx1 × H 次）
        let mut row = vec![0xAAu8; w * 4];
        let (mut rw, mut rh) = (0usize, 0usize);
        for y in 0..h {
            for b in row.iter_mut() {
                *b = 0xAA;
            }
            (gl.read_pixels)(
                0,
                y as c_int,
                w as c_int,
                1,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                row.as_mut_ptr() as *mut c_void,
            );
            for x in 0..w {
                let px = &row[x * 4..x * 4 + 4];
                if px.iter().all(|b| *b == 0xAA) {
                    continue;
                }
                rw += 1;
                if px == &expect(x, y)[..] {
                    rh += 1;
                }
            }
        }
        say!("=== B 逐行读（{w}x1 × {h} 次）：被写 {rw}/{npix}，值正确 {rh}/{npix}");

        // ---- C：逐列读（1xH × W 次）。缓冲区按整块大小给，读回越界时不会踩坏堆，
        // 越界本身也单独计数（这一项正是 A 段那个行距错位在窄图上的后果）。
        let mut col = vec![0xAAu8; npix * 4];
        let (mut cw, mut ch_) = (0usize, 0usize);
        let mut cover = 0usize;
        let mut reach = 0usize;
        for x in 0..w {
            for b in col.iter_mut() {
                *b = 0xAA;
            }
            (gl.read_pixels)(
                x as c_int,
                0,
                1,
                h as c_int,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                col.as_mut_ptr() as *mut c_void,
            );
            for y in 0..h {
                let px = &col[y * 4..y * 4 + 4];
                if px.iter().all(|b| *b == 0xAA) {
                    continue;
                }
                cw += 1;
                if px == &expect(x, y)[..] {
                    ch_ += 1;
                }
            }
            if let Some(last) = col.iter().rposition(|b| *b != 0xAA) {
                reach = reach.max(last + 1);
            }
            if col[h * 4..].iter().any(|b| *b != 0xAA) {
                cover += 1;
                for b in col.iter_mut() {
                    *b = 0xAA;
                }
            }
        }
        say!(
            "=== C 逐列读（1x{h} × {w} 次）：被写 {cw}/{npix}，值正确 {ch_}/{npix}，越界写到缓冲后段 {cover}/{w} 次（最远触及 {reach} 字节，一次 1x{h} 读本该只写 {h4} 字节）",
            h4 = h * 4
        );

        // ---- D：单点采样
        let pts = [
            (0usize, 0usize),
            (w - 1, 0),
            (0, h - 1),
            (w - 1, h - 1),
            (w / 2, h / 2),
        ];
        let mut dhit = 0usize;
        for (x, y) in pts {
            let mut one = [0xAAu8; 4];
            (gl.read_pixels)(
                x as c_int,
                y as c_int,
                1,
                1,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                one.as_mut_ptr() as *mut c_void,
            );
            let ok = one == expect(x, y);
            if ok {
                dhit += 1;
            }
            say!(
                "  (x={x},y={y}) got={one:02x?} want={:02x?} {}",
                expect(x, y),
                if ok { "ok" } else { "MISMATCH" }
            );
        }
        say!("=== D 单点 1x1 采样：{dhit}/{} 正确", pts.len());

        (egl.make_current)(
            display,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );
        (egl.destroy_context)(display, context);
        (egl.destroy_surface)(display, surface);
        (egl.terminate)(display);
    }
    0
}

fn proc_rss_kb() -> u64 {
    let Ok(status) = std::fs::read_to_string("/proc/self/status") else {
        return 0;
    };
    status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))
        .and_then(|value| value.split_whitespace().next())
        .and_then(|value| value.parse().ok())
        .unwrap_or(0)
}

fn proc_fd_counts() -> (u64, u64) {
    let mut total = 0u64;
    let mut mali = 0u64;
    let Ok(entries) = std::fs::read_dir("/proc/self/fd") else {
        return (0, 0);
    };
    for entry in entries.flatten() {
        total += 1;
        if std::fs::read_link(entry.path())
            .map(|path| path.to_string_lossy().contains("/dev/mali0"))
            .unwrap_or(false)
        {
            mali += 1;
        }
    }
    (total, mali)
}

/// GPU soak: report per-second frame progress, RSS and device fds. This keeps the
/// Python probe's leak signal while using the same EGL/GLES loader path as the
/// rest of the Rust GPU tools.
fn run_soak(width: u32, height: u32, seconds: u64, tag: &str) -> i32 {
    let Some(egl) = Egl::load() else {
        say!("FAIL dlopen EGL");
        return 1;
    };
    unsafe {
        let display = (egl.get_platform_display)(
            EGL_PLATFORM_SURFACELESS_MESA,
            std::ptr::null_mut(),
            std::ptr::null(),
        );
        if display.is_null() {
            say!("FAIL eglGetPlatformDisplay(surfaceless)");
            return 1;
        }
        let (mut major, mut minor) = (0, 0);
        if (egl.initialize)(display, &mut major, &mut minor) == 0 {
            say!("FAIL eglInitialize");
            return 1;
        }
        if (egl.bind_api)(EGL_OPENGL_ES_API) == 0 {
            say!("FAIL eglBindAPI(GL_ES) err={:#x}", (egl.get_error)());
            return 1;
        }
        let config_attrs = [
            EGL_RENDERABLE_TYPE,
            EGL_OPENGL_ES2_BIT,
            EGL_RED_SIZE,
            8,
            EGL_GREEN_SIZE,
            8,
            EGL_BLUE_SIZE,
            8,
            EGL_ALPHA_SIZE,
            8,
            EGL_SURFACE_TYPE,
            EGL_PBUFFER_BIT,
            EGL_NONE,
        ];
        let mut config: Config = std::ptr::null_mut();
        let mut count = 0;
        if (egl.choose_config)(display, config_attrs.as_ptr(), &mut config, 1, &mut count) == 0
            || count < 1
        {
            say!("FAIL eglChooseConfig n={count}");
            return 1;
        }
        let surface_attrs = [EGL_WIDTH, width as i32, EGL_HEIGHT, height as i32, EGL_NONE];
        let surface = (egl.create_pbuffer_surface)(display, config, surface_attrs.as_ptr());
        if surface.is_null() {
            say!("FAIL eglCreatePbufferSurface err={:#x}", (egl.get_error)());
            return 1;
        }
        let ctx_attrs = [EGL_CONTEXT_MAJOR_VERSION, 2, EGL_NONE];
        let context =
            (egl.create_context)(display, config, std::ptr::null_mut(), ctx_attrs.as_ptr());
        if context.is_null() {
            say!("FAIL eglCreateContext err={:#x}", (egl.get_error)());
            return 1;
        }
        if (egl.make_current)(display, surface, surface, context) == 0 {
            say!("FAIL eglMakeCurrent err={:#x}", (egl.get_error)());
            return 1;
        }
        let Some(gl) = Gl::load() else {
            say!("FAIL dlopen GLESv2");
            return 1;
        };
        say!("TAG {tag} GL_RENDERER={}", gl.string(GL_RENDERER));
        (gl.viewport)(0, 0, width as c_int, height as c_int);

        let (base_fd, base_mali) = proc_fd_counts();
        let base_rss = proc_rss_kb();
        say!(
            "START {tag} {width}x{height} for {seconds}s rss={base_rss}kB fd={base_fd} mali_fd={base_mali}"
        );

        let started = std::time::Instant::now();
        let mut last = started;
        let mut frames_total = 0u64;
        let mut frames_second = 0u64;
        let mut peak_fps = 0u64;
        let mut errors = 0u64;
        while started.elapsed() < std::time::Duration::from_secs(seconds) {
            let phase = (frames_total * 7 % 256) as f32 / 255.0;
            (gl.clear_color)(phase, 0.25, 1.0 - phase, 1.0);
            (gl.clear)(GL_COLOR_BUFFER_BIT);
            (gl.finish)();
            frames_total += 1;
            frames_second += 1;
            if (gl.get_error)() != 0 {
                errors += 1;
            }
            if last.elapsed() >= std::time::Duration::from_secs(1) {
                let (fd, mali) = proc_fd_counts();
                say!(
                    "SEC {:4} fps={:4} rss={}kB fd={fd} mali_fd={mali} errors={errors}",
                    started.elapsed().as_secs(),
                    frames_second,
                    proc_rss_kb()
                );
                peak_fps = peak_fps.max(frames_second);
                frames_second = 0;
                last = std::time::Instant::now();
            }
        }
        let (fd, mali) = proc_fd_counts();
        let elapsed = started.elapsed().as_secs_f64();
        say!(
            "DONE {tag} frames={frames_total} avg_fps={:.1} peak_fps={peak_fps} rss={}kB (start {base_rss}kB) fd={fd} (start {base_fd}) mali_fd={mali} (start {base_mali}) errors={errors}",
            frames_total as f64 / elapsed.max(1e-9),
            proc_rss_kb()
        );
        if errors == 0 {
            0
        } else {
            1
        }
    }
}

fn readback_pattern(x: usize, y: usize) -> [u8; 4] {
    [
        (x * 7 + 3) as u8,
        (y * 11 + 5) as u8,
        ((x * 13 + y * 17) % 251) as u8,
        255,
    ]
}

fn drain_gl_errors(gl: &Gl) -> Vec<u32> {
    let mut seen = Vec::new();
    for _ in 0..8 {
        let error = unsafe { (gl.get_error)() };
        if error == 0 {
            break;
        }
        seen.push(error);
    }
    seen
}

/// Round-5 readback suite: width boundaries, canary tails, whole-block reads,
/// and the row-by-row shape used by screenshots.
fn run_readback_suite() -> i32 {
    let Some(egl) = Egl::load() else {
        say!("FAIL dlopen EGL");
        return 1;
    };
    unsafe {
        let display = (egl.get_platform_display)(
            EGL_PLATFORM_SURFACELESS_MESA,
            std::ptr::null_mut(),
            std::ptr::null(),
        );
        if display.is_null() {
            say!("FAIL eglGetPlatformDisplay(surfaceless)");
            return 1;
        }
        let (mut major, mut minor) = (0, 0);
        if (egl.initialize)(display, &mut major, &mut minor) == 0 {
            say!("FAIL eglInitialize");
            return 1;
        }
        if (egl.bind_api)(EGL_OPENGL_ES_API) == 0 {
            say!("FAIL eglBindAPI(GL_ES) err={:#x}", (egl.get_error)());
            return 1;
        }
        let config_attrs = [
            EGL_SURFACE_TYPE,
            EGL_PBUFFER_BIT,
            EGL_RENDERABLE_TYPE,
            EGL_OPENGL_ES3_BIT,
            EGL_RED_SIZE,
            8,
            EGL_GREEN_SIZE,
            8,
            EGL_BLUE_SIZE,
            8,
            EGL_ALPHA_SIZE,
            8,
            EGL_NONE,
        ];
        let mut config: Config = std::ptr::null_mut();
        let mut count = 0;
        if (egl.choose_config)(display, config_attrs.as_ptr(), &mut config, 1, &mut count) == 0
            || count < 1
        {
            say!("FAIL eglChooseConfig n={count}");
            return 1;
        }
        let surface_attrs = [EGL_WIDTH, 1, EGL_HEIGHT, 1, EGL_NONE];
        let surface = (egl.create_pbuffer_surface)(display, config, surface_attrs.as_ptr());
        if surface.is_null() {
            say!("FAIL eglCreatePbufferSurface err={:#x}", (egl.get_error)());
            return 1;
        }
        let ctx_attrs = [EGL_CONTEXT_MAJOR_VERSION, 3, EGL_NONE];
        let context =
            (egl.create_context)(display, config, std::ptr::null_mut(), ctx_attrs.as_ptr());
        if context.is_null() {
            say!("FAIL eglCreateContext err={:#x}", (egl.get_error)());
            return 1;
        }
        if (egl.make_current)(display, surface, surface, context) == 0 {
            say!("FAIL eglMakeCurrent err={:#x}", (egl.get_error)());
            return 1;
        }

        let Some(gl) = Gl::load() else {
            say!("FAIL dlopen GLESv2");
            return 1;
        };
        say!(
            "TAG renderer={} version={}",
            gl.string(GL_RENDERER),
            gl.string(GL_VERSION)
        );

        let mut texture = 0u32;
        (gl.gen_textures)(1, &mut texture);
        (gl.bind_texture)(GL_TEXTURE_2D, texture);
        (gl.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
        (gl.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
        let mut framebuffer = 0u32;
        (gl.gen_framebuffers)(1, &mut framebuffer);
        (gl.bind_framebuffer)(GL_FRAMEBUFFER, framebuffer);
        (gl.pixel_storei)(GL_UNPACK_ALIGNMENT, 1);
        (gl.pixel_storei)(GL_PACK_ALIGNMENT, 4);
        (gl.pixel_storei)(GL_PACK_ROW_LENGTH, 0);

        let frame = |w: usize, h: usize| -> Result<(), String> {
            let mut pixels = vec![0u8; w * h * 4];
            for y in 0..h {
                for x in 0..w {
                    let offset = (y * w + x) * 4;
                    pixels[offset..offset + 4].copy_from_slice(&readback_pattern(x, y));
                }
            }
            (gl.tex_image_2d)(
                GL_TEXTURE_2D,
                0,
                GL_RGBA8,
                w as c_int,
                h as c_int,
                0,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                pixels.as_ptr() as *const c_void,
            );
            (gl.framebuffer_texture_2d)(
                GL_FRAMEBUFFER,
                GL_COLOR_ATTACHMENT0,
                GL_TEXTURE_2D,
                texture,
                0,
            );
            let status = (gl.check_framebuffer_status)(GL_FRAMEBUFFER);
            if status != GL_FRAMEBUFFER_COMPLETE {
                return Err(format!("FBO incomplete for {w}x{h}: {status:#x}"));
            }
            (gl.viewport)(0, 0, w as c_int, h as c_int);
            Ok(())
        };

        let read1 = |x: usize, y: usize| -> [u8; 4] {
            let mut pixel = [0u8; 4];
            (gl.read_pixels)(
                x as c_int,
                y as c_int,
                1,
                1,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                pixel.as_mut_ptr() as *mut c_void,
            );
            pixel
        };
        let read_rows = |w: usize, h: usize, extra: usize| -> Vec<u8> {
            let mut out = vec![0xAAu8; w * h * 4 + extra];
            let base = out.as_mut_ptr();
            for y in 0..h {
                (gl.read_pixels)(
                    0,
                    y as c_int,
                    w as c_int,
                    1,
                    GL_RGBA,
                    GL_UNSIGNED_BYTE,
                    base.add(y * w * 4) as *mut c_void,
                );
            }
            out
        };
        let read_block = |w: usize, h: usize, extra: usize| -> Vec<u8> {
            let mut out = vec![0xAAu8; w * h * 4 + extra];
            (gl.read_pixels)(
                0,
                0,
                w as c_int,
                h as c_int,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                out.as_mut_ptr() as *mut c_void,
            );
            out
        };

        let mut passed = 0u32;
        let mut failed = 0u32;
        let mut passed_case = |message: String| {
            passed += 1;
            say!("  PASS  {message}");
        };
        let mut failed_case = |message: String| {
            failed += 1;
            say!("  FAIL  {message}");
        };

        say!("== T1 row-wise readback, width boundaries ==");
        for (w, h) in [
            (1usize, 3usize),
            (2, 3),
            (3, 3),
            (4, 3),
            (5, 3),
            (7, 3),
            (8, 4),
            (15, 3),
            (16, 4),
            (17, 3),
            (63, 2),
            (64, 2),
            (65, 2),
        ] {
            if let Err(e) = frame(w, h) {
                failed_case(format!("W={w} H={h}: {e}"));
                continue;
            }
            let mut want = Vec::with_capacity(w * h * 4);
            let mut uploaded = Vec::with_capacity(w * h * 4);
            for y in 0..h {
                for x in 0..w {
                    want.extend_from_slice(&read1(x, y));
                    uploaded.extend_from_slice(&readback_pattern(x, y));
                }
            }
            if want != uploaded {
                failed_case(format!(
                    "W={w} H={h}: 1x1 reads do not match the uploaded pattern"
                ));
                continue;
            }
            let early = drain_gl_errors(&gl);
            if !early.is_empty() {
                say!("  INFO  W={w} H={h}: setup raised {early:x?} before the read (drained)");
            }
            let got = read_rows(w, h, 0);
            let error = (gl.get_error)();
            if got == want && error == 0 {
                passed_case(format!(
                    "W={w} H={h}: {} bytes exact, glGetError=0",
                    got.len()
                ));
            } else {
                let different = got
                    .iter()
                    .zip(&want)
                    .filter(|(left, right)| left != right)
                    .count();
                failed_case(format!(
                    "W={w} H={h}: {different} byte(s) differ, glGetError={error:#x}"
                ));
            }
        }

        say!("== T2 canary: row-wise read must not touch the tail ==");
        for (w, h) in [(5usize, 3usize), (17, 3), (65, 2)] {
            if let Err(e) = frame(w, h) {
                failed_case(format!("W={w} H={h}: {e}"));
                continue;
            }
            let mut want = Vec::with_capacity(w * h * 4);
            for y in 0..h {
                for x in 0..w {
                    want.extend_from_slice(&read1(x, y));
                }
            }
            let tail = 4_096usize;
            drain_gl_errors(&gl);
            let got = read_rows(w, h, tail);
            let head = &got[..w * h * 4];
            let beyond = &got[w * h * 4..];
            let overrun = beyond.iter().filter(|byte| **byte != 0xAA).count();
            if head == want && overrun == 0 {
                passed_case(format!(
                    "W={w} H={h}: head exact, {tail}-byte tail untouched"
                ));
            } else {
                failed_case(format!(
                    "W={w} H={h}: head {}, tail written in {overrun} byte(s)",
                    if head == want { "exact" } else { "WRONG" }
                ));
            }
        }

        say!("== T3 whole-block read: exact bytes and canary ==");
        for (w, h) in [(16usize, 16usize), (65, 4)] {
            if let Err(e) = frame(w, h) {
                failed_case(format!("W={w} H={h}: {e}"));
                continue;
            }
            let mut want = Vec::with_capacity(w * h * 4);
            for y in 0..h {
                for x in 0..w {
                    want.extend_from_slice(&read1(x, y));
                }
            }
            let tail = 8_192usize;
            drain_gl_errors(&gl);
            let got = read_block(w, h, tail);
            let error = (gl.get_error)();
            let head = &got[..w * h * 4];
            let beyond = &got[w * h * 4..];
            let overrun = beyond.iter().filter(|byte| **byte != 0xAA).count();
            if head == want && overrun == 0 && error == 0 {
                passed_case(format!(
                    "W={w} H={h}: whole-block exact, {tail}-byte tail untouched, glGetError=0"
                ));
            } else {
                let different = head
                    .iter()
                    .zip(&want)
                    .filter(|(left, right)| left != right)
                    .count();
                failed_case(format!(
                    "W={w} H={h}: {different}/{len} bytes differ, tail written in {overrun} byte(s), glGetError={error:#x}",
                    len = want.len()
                ));
            }
        }

        say!("== T4 screenshot path: row loop into W*H*4 ==");
        let (w, h) = (240usize, 135usize);
        if let Err(e) = frame(w, h) {
            failed_case(format!("W={w} H={h}: {e}"));
        } else {
            let mut want = Vec::with_capacity(w * h * 4);
            for y in 0..h {
                for x in 0..w {
                    want.extend_from_slice(&read1(x, y));
                }
            }
            drain_gl_errors(&gl);
            let got = read_rows(w, h, 0);
            if got == want {
                passed_case(format!(
                    "{w}x{h}: {} bytes byte-identical to ground truth",
                    got.len()
                ));
            } else {
                let different = got
                    .iter()
                    .zip(&want)
                    .filter(|(left, right)| left != right)
                    .count();
                failed_case(format!("{w}x{h}: {different}/{} bytes differ", want.len()));
            }
        }

        say!("");
        say!("PASS={passed} FAIL={failed}");
        if failed == 0 {
            0
        } else {
            1
        }
    }
}

/// Windowed EGL: real X11 window at `width`x`height`, `frames` clear+swap
/// frames, then a 1x1 readback. Defaults (640x480, 60) are the numbers the
/// evidence files were taken with; other sizes are how the presentation cost
/// gets separated from the GPU cost - a copy-bound path scales with pixels, a
/// submission-bound one does not.
fn run_window(width: u32, height: u32, frames: u32) -> i32 {
    let x11 = match Lib::open("libX11.so.6") {
        Some(l) => l,
        None => {
            say!("FAIL dlopen X11");
            return 1;
        }
    };
    unsafe {
        let open: unsafe extern "C" fn(*const c_char) -> *mut c_void =
            cast(x11.sym("XOpenDisplay"));
        let root_of: unsafe extern "C" fn(*mut c_void) -> Window =
            cast(x11.sym("XDefaultRootWindow"));
        let create: unsafe extern "C" fn(
            *mut c_void,
            Window,
            c_int,
            c_int,
            c_uint,
            c_uint,
            c_uint,
            c_ulong,
            c_ulong,
            c_int,
            c_uint,
            *mut c_void,
        ) -> Window = cast(x11.sym("XCreateSimpleWindow"));
        let map: unsafe extern "C" fn(*mut c_void, Window) -> c_int = cast(x11.sym("XMapWindow"));
        let sync: unsafe extern "C" fn(*mut c_void, c_int) -> c_int = cast(x11.sym("XSync"));
        let destroy: unsafe extern "C" fn(*mut c_void, Window) -> c_int =
            cast(x11.sym("XDestroyWindow"));

        let display = open(std::ptr::null());
        if display.is_null() {
            say!("FAIL XOpenDisplay");
            return 1;
        }
        say!("stage 1: XOpenDisplay OK");
        let root = root_of(display);
        say!("stage 1b: root window={root}");
        let window = create(
            display,
            root,
            0,
            0,
            width,
            height,
            0,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
        );
        map(display, window);
        sync(display, 0);
        say!("stage 2: window mapped id={window}");

        let Some(egl) = Egl::load() else {
            say!("FAIL dlopen EGL");
            return 1;
        };
        let egl_display =
            (egl.get_platform_display)(EGL_PLATFORM_X11_KHR, display, std::ptr::null());
        if egl_display.is_null() {
            say!("FAIL eglGetPlatformDisplay(X11)");
            return 1;
        }
        let mut major = 0;
        let mut minor = 0;
        if (egl.initialize)(egl_display, &mut major, &mut minor) == 0 {
            say!("FAIL eglInitialize");
            return 1;
        }
        say!("stage 3: eglInitialize {major}.{minor}");
        (egl.bind_api)(EGL_OPENGL_ES_API);
        let config_attrs = [
            EGL_RENDERABLE_TYPE,
            EGL_OPENGL_ES2_BIT,
            EGL_RED_SIZE,
            8,
            EGL_GREEN_SIZE,
            8,
            EGL_BLUE_SIZE,
            8,
            EGL_ALPHA_SIZE,
            8,
            EGL_SURFACE_TYPE,
            EGL_PBUFFER_BIT,
            EGL_NONE,
        ];
        let mut config: Config = std::ptr::null_mut();
        let mut count = 0;
        if (egl.choose_config)(
            egl_display,
            config_attrs.as_ptr(),
            &mut config,
            1,
            &mut count,
        ) == 0
            || count < 1
        {
            say!("FAIL eglChooseConfig n={count}");
            return 1;
        }
        say!("stage 4: config OK");
        let surface = (egl.create_window_surface)(egl_display, config, window, std::ptr::null());
        if surface.is_null() {
            say!("FAIL eglCreateWindowSurface err={:#x}", (egl.get_error)());
            return 1;
        }
        say!("stage 5: window surface OK");
        let ctx_attrs = [EGL_CONTEXT_MAJOR_VERSION, 2, EGL_NONE];
        let context = (egl.create_context)(
            egl_display,
            config,
            std::ptr::null_mut(),
            ctx_attrs.as_ptr(),
        );
        if context.is_null() {
            say!("FAIL eglCreateContext err={:#x}", (egl.get_error)());
            return 1;
        }
        say!("stage 6: context OK");
        if (egl.make_current)(egl_display, surface, surface, context) == 0 {
            say!("FAIL eglMakeCurrent err={:#x}", (egl.get_error)());
            return 1;
        }
        say!("stage 7: makeCurrent OK");

        let Some(gl) = Gl::load() else {
            say!("FAIL dlopen GLESv2");
            return 1;
        };
        say!("renderer={}", gl.string(GL_RENDERER));
        (gl.viewport)(0, 0, width as c_int, height as c_int);
        let swap_t0 = std::time::Instant::now();
        for i in 0..frames {
            let color = ((i % 2) as f32, ((i / 2) % 2) as f32, ((i / 4) % 2) as f32);
            (gl.clear_color)(color.0, color.1, color.2, 1.0);
            (gl.clear)(GL_COLOR_BUFFER_BIT);
            if i == 0 {
                say!("stage 8: first draw OK");
            }
            if (egl.swap_buffers)(egl_display, surface) == 0 {
                say!(
                    "FAIL eglSwapBuffers at frame {i} err={:#x}",
                    (egl.get_error)()
                );
                return 1;
            }
            if i == 0 {
                say!("stage 9: first swap OK");
            }
        }
        let swap_dt = swap_t0.elapsed().as_secs_f64();
        say!(
            "stage 10: {frames} swaps OK ({:.1} ms, {:.1} fps) {}x{}",
            swap_dt * 1000.0,
            frames as f64 / swap_dt,
            width,
            height
        );
        (gl.clear_color)(0.0, 1.0, 0.0, 1.0);
        (gl.clear)(GL_COLOR_BUFFER_BIT);
        (egl.swap_buffers)(egl_display, surface);
        let mut px = [0u8; 4];
        (gl.read_pixels)(
            10,
            10,
            1,
            1,
            GL_RGBA,
            GL_UNSIGNED_BYTE,
            px.as_mut_ptr() as *mut c_void,
        );
        say!(
            "readback px={},{},{},{} err={:#x}",
            px[0],
            px[1],
            px[2],
            px[3],
            (egl.get_error)()
        );
        let ok = px[0] < 40 && px[1] > 200;
        say!("RESULT {}", if ok { "PASS" } else { "SUSPECT" });
        (egl.make_current)(
            egl_display,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );
        (egl.destroy_context)(egl_display, context);
        (egl.destroy_surface)(egl_display, surface);
        (egl.terminate)(egl_display);
        destroy(display, window);
        let _ = GL_NO_ERROR;
        if ok {
            0
        } else {
            5
        }
    }
}

/// Capture what the X server is actually showing and write it as a PPM.
///
/// Needed because the App draws its own UI on top of the phone screen: `screencap`
/// on the device proves nothing about the guest desktop. This reads the X root
/// window through `XGetImage`, so it is the compositor's own output.
///
///   gl-probe root [out.ppm]
fn run_root() -> i32 {
    let x11 = match Lib::open("libX11.so.6") {
        Some(l) => l,
        None => {
            say!("FAIL dlopen X11");
            return 1;
        }
    };
    let out = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "/tmp/root.ppm".to_string());
    unsafe {
        let open: unsafe extern "C" fn(*const c_char) -> *mut c_void =
            cast(x11.sym("XOpenDisplay"));
        let root_of: unsafe extern "C" fn(*mut c_void) -> Window =
            cast(x11.sym("XDefaultRootWindow"));
        let dw: unsafe extern "C" fn(*mut c_void, c_int) -> c_int = cast(x11.sym("XDisplayWidth"));
        let dh: unsafe extern "C" fn(*mut c_void, c_int) -> c_int = cast(x11.sym("XDisplayHeight"));
        let get_image: unsafe extern "C" fn(
            *mut c_void,
            Window,
            c_int,
            c_int,
            c_uint,
            c_uint,
            c_ulong,
            c_int,
        ) -> *mut c_void = cast(x11.sym("XGetImage"));
        let free: unsafe extern "C" fn(*mut c_void) -> c_int = cast(x11.sym("XFree"));

        let display = open(std::ptr::null());
        if display.is_null() {
            say!("FAIL XOpenDisplay");
            return 1;
        }
        let root = root_of(display);
        let w = dw(display, 0) as c_uint;
        let h = dh(display, 0) as c_uint;
        say!("root {w}x{h} window={root}");
        // AllPlanes, ZPixmap
        let img = get_image(display, root, 0, 0, w, h, c_ulong::MAX, 2);
        if img.is_null() {
            say!("FAIL XGetImage");
            return 1;
        }
        let base = img as *const u8;
        let iw = *(base as *const c_int) as usize;
        let ih = *(base.add(4) as *const c_int) as usize;
        let data = *(base.add(16) as *const *const u8);
        let stride = *(base.add(44) as *const c_int) as i64 as usize;
        let bpp = *(base.add(48) as *const c_int) as i64 as usize;
        say!("image {iw}x{ih} stride={stride} bpp={bpp}");
        if data.is_null() || iw == 0 || ih == 0 {
            say!("FAIL empty image");
            return 1;
        }
        if std::mem::size_of::<usize>() != 8 {
            say!("FAIL unexpected pointer size");
            return 1;
        }
        let step = (bpp / 8).max(4);
        let mut ppm = Vec::with_capacity(iw * ih * 3 + 32);
        ppm.extend_from_slice(format!("P6\n{iw} {ih}\n255\n").as_bytes());
        let mut nonblack = 0usize;
        let mut hist = [0usize; 8];
        for y in 0..ih {
            let row = data.add(y * stride);
            for x in 0..iw {
                let q = row.add(x * step);
                let (b, g, r) = (*q, *q.add(1), *q.add(2));
                ppm.push(r);
                ppm.push(g);
                ppm.push(b);
                if r > 8 || g > 8 || b > 8 {
                    nonblack += 1;
                    let lum = (r as usize + g as usize + b as usize) / 3;
                    hist[(lum * 8 / 256).min(7)] += 1;
                }
            }
        }
        (free)(img as *mut c_void);
        match std::fs::File::create(&out) {
            Ok(mut f) => {
                use std::io::Write;
                if let Err(e) = f.write_all(&ppm) {
                    say!("FAIL write {out}: {e}");
                    return 1;
                }
            }
            Err(e) => {
                say!("FAIL create {out}: {e}");
                return 1;
            }
        }
        let total = iw * ih;
        say!(
            "nonblack={nonblack}/{total} ({:.1}%) histogram={:?}",
            nonblack as f64 * 100.0 / total as f64,
            hist
        );
        say!("wrote {out}");
        0
    }
}

/// `WxH`. Anything outside 1..=7680 is a typo, not a request.
fn parse_size(s: &str) -> Option<(u32, u32)> {
    let (w, h) = s.split_once('x')?;
    let w: u32 = w.parse().ok()?;
    let h: u32 = h.parse().ok()?;
    if (1..=7680).contains(&w) && (1..=7680).contains(&h) {
        Some((w, h))
    } else {
        None
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("es3") => run_es3(),
        Some("fixed") => {
            let frames = args
                .get(1)
                .and_then(|a| a.parse::<u32>().ok())
                .filter(|n| (1..=100_000).contains(n))
                .unwrap_or(60);
            run_fixed(frames)
        }
        Some("fixedgl") => {
            let size = args.get(1).map(String::as_str).unwrap_or("640x480");
            let seconds = args
                .get(2)
                .and_then(|value| value.parse::<u64>().ok())
                .filter(|value| (1..=86_400).contains(value))
                .unwrap_or(10);
            let tag = args.get(3).map(String::as_str).unwrap_or("fixed");
            let lit = matches!(args.get(4).map(String::as_str), Some("lit"));
            match parse_size(size) {
                Some((width, height)) => run_fixedgl(width, height, seconds, tag, lit),
                None => {
                    eprintln!("gl-probe fixedgl: bad size {size:?} (want WxH, e.g. 640x480)");
                    2
                }
            }
        }
        Some("window") => {
            let size = args.get(1).map(String::as_str).unwrap_or("640x480");
            let frames = args
                .get(2)
                .and_then(|a| a.parse::<u32>().ok())
                .filter(|n| (1..=100_000).contains(n))
                .unwrap_or(60);
            match parse_size(size) {
                Some((w, h)) => run_window(w, h, frames),
                None => {
                    eprintln!("gl-probe window: bad size {size:?} (want WxH, e.g. 640x480)");
                    2
                }
            }
        }
        Some("pbuf") => {
            let size = args.get(1).map(String::as_str).unwrap_or("640x480");
            let frames = args
                .get(2)
                .and_then(|a| a.parse::<u32>().ok())
                .filter(|n| (1..=100_000).contains(n))
                .unwrap_or(60);
            match parse_size(size) {
                Some((w, h)) => run_pbuf(w, h, frames),
                None => {
                    eprintln!("gl-probe pbuf: bad size {size:?} (want WxH, e.g. 640x480)");
                    2
                }
            }
        }
        Some("xput") => {
            let size = args.get(1).map(String::as_str).unwrap_or("640x480");
            let frames = args
                .get(2)
                .and_then(|a| a.parse::<u32>().ok())
                .filter(|n| (1..=100_000).contains(n))
                .unwrap_or(60);
            match parse_size(size) {
                Some((w, h)) => run_xput(w, h, frames),
                None => {
                    eprintln!("gl-probe xput: bad size {size:?} (want WxH, e.g. 640x480)");
                    2
                }
            }
        }
        Some("fill") => {
            let size = args.get(1).map(String::as_str).unwrap_or("640x480");
            let passes = args
                .get(2)
                .and_then(|a| a.parse::<u32>().ok())
                .filter(|n| (1..=4096).contains(n))
                .unwrap_or(4);
            let frames = args
                .get(3)
                .and_then(|a| a.parse::<u32>().ok())
                .filter(|n| (1..=100_000).contains(n))
                .unwrap_or(60);
            match parse_size(size) {
                Some((w, h)) => run_fill(w, h, passes, frames),
                None => {
                    eprintln!("gl-probe fill: bad size {size:?} (want WxH, e.g. 640x480)");
                    2
                }
            }
        }
        Some("readback") => {
            let size = args.get(1).map(String::as_str).unwrap_or("16x16");
            match parse_size(size) {
                Some((w, h)) => run_readback(w, h),
                None => {
                    eprintln!("gl-probe readback: bad size {size:?} (want WxH, e.g. 16x16)");
                    2
                }
            }
        }
        Some("readback-suite") => run_readback_suite(),
        Some("soak") => {
            let size = args.get(1).map(String::as_str).unwrap_or("2400x1080");
            let seconds = args
                .get(2)
                .and_then(|value| value.parse::<u64>().ok())
                .filter(|value| (1..=86_400).contains(value))
                .unwrap_or(180);
            let tag = args.get(3).map(String::as_str).unwrap_or("soak");
            match parse_size(size) {
                Some((w, h)) => run_soak(w, h, seconds, tag),
                None => {
                    eprintln!("gl-probe soak: bad size {size:?} (want WxH, e.g. 2400x1080)");
                    2
                }
            }
        }
        Some("root") => run_root(),
        _ => {
            eprintln!("usage: gl-probe <es3 | fixed [frames] | fixedgl [WxH [seconds [tag [plain|lit]]]] | window [WxH [frames]] | pbuf [WxH [frames]] | xput [WxH [frames]] | fill [WxH [passes [frames]]] | readback [WxH] | readback-suite | soak [WxH [seconds [tag]]] | root [out.ppm]>");
            2
        }
    };
    std::process::exit(code);
}
