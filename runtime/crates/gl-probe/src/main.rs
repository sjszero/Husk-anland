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
//!
//! Why 1x1 readbacks: this guest's `glReadPixels` writes only the first row and
//! the first column of a block read (evidence 162), so a block read would hide
//! real errors and invent fake ones.

use std::ffi::CString;
use std::io::Write;
use std::os::raw::{c_char, c_int, c_uint, c_ulong, c_void};

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
const EGL_SURFACE_TYPE: c_int = 0x3033;
const EGL_PBUFFER_BIT: c_int = 0x0001;
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
const GL_TEXTURE0: c_uint = 0x84C0;
const GL_DEPTH_TEST: c_uint = 0x0B71;
const GL_UNPACK_ALIGNMENT: c_uint = 0x0CF5;
const GL_PACK_ALIGNMENT: c_uint = 0x0D02;
const GL_RENDERER: c_uint = 0x1F01;
const GL_VERSION: c_uint = 0x1F02;
const GL_SHADING_LANGUAGE_VERSION: c_uint = 0x8B8C;
const GL_NO_ERROR: c_uint = 0;

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
    draw_arrays_instanced: unsafe extern "C" fn(c_uint, c_int, c_int, c_int),
    bind_buffer_base: unsafe extern "C" fn(c_uint, c_uint, c_uint),
    finish: unsafe extern "C" fn(),
    read_pixels: unsafe extern "C" fn(c_int, c_int, c_int, c_int, c_uint, c_uint, *mut c_void),
    read_buffer: unsafe extern "C" fn(c_uint),
    pixel_storei: unsafe extern "C" fn(c_uint, c_int),
    get_string: unsafe extern "C" fn(c_uint) -> *const c_char,
    get_uniform_location: unsafe extern "C" fn(c_uint, *const c_char) -> c_int,
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
                draw_arrays_instanced: cast(p("glDrawArraysInstanced")),
                bind_buffer_base: cast(p("glBindBufferBase")),
                finish: cast(p("glFinish")),
                read_pixels: cast(p("glReadPixels")),
                read_buffer: cast(p("glReadBuffer")),
                pixel_storei: cast(p("glPixelStorei")),
                get_string: cast(p("glGetString")),
                get_uniform_location: cast(p("glGetUniformLocation")),
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

    // Per-pixel 1x1 readback (evidence 162: block reads only fill row 0/col 0).
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
        Some("root") => run_root(),
        _ => {
            eprintln!("usage: gl-probe <es3 | window [WxH [frames]] | pbuf [WxH [frames]] | root [out.ppm]>");
            2
        }
    };
    std::process::exit(code);
}
