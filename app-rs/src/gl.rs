//! EGL surface + one GLES2 pass.  The text is drawn as textured quads from an
//! atlas that fontdue rasterises at startup from the system monospace font, so
//! the renderer has no C and no font file of its own.

use crate::sys::*;
use std::ffi::{c_char, c_void, CString};

const CELL: i32 = 32;
const ATLAS_COLS: i32 = 16;
const ATLAS_ROWS: i32 = 8;
const PX: f32 = 26.0;

#[derive(Clone, Copy, Default)]
struct Glyph {
    u0: f32,
    v0: f32,
    u1: f32,
    v1: f32,
    dx: f32,
    dy: f32,
    w: f32,
    h: f32,
}

pub struct Gfx {
    pub dpy: EGLDisplay,
    pub surf: EGLSurface,
    prog: GLuint,
    ctx: EGLContext,
    a_pos: GLint,
    a_uv: GLint,
    u_res: GLint,
    u_tex: GLint,
    u_color: GLint,
    glyphs: [Glyph; 128],
    atlas_w: i32,
    atlas_h: i32,
    /// Size of ONE atlas cell in pixels. It equals the on-screen cell, so a
    /// glyph quad is blitted 1:1 and the atlas offset (xmin, baseline - bottom)
    /// lands where it was rasterised. Using a fixed 32px atlas cell here put
    /// every glyph twice its bearing too low - commas and dashes ended up in
    /// the row below.
    acw: f32,
    ach: f32,
    pub cell_w: f32,
    pub cell_h: f32,
    verts: Vec<f32>,
}

fn shader(kind: GLenum, src: &str) -> GLuint {
    let s = CString::new(src).unwrap();
    unsafe {
        let sh = glCreateShader(kind);
        let p = s.as_ptr();
        glShaderSource(sh, 1, &p, core::ptr::null());
        glCompileShader(sh);
        let mut ok: GLint = 0;
        glGetShaderiv(sh, GL_COMPILE_STATUS, &mut ok);
        if ok == 0 {
            crate::log("shader compile failed");
        }
        sh
    }
}

impl Gfx {
    pub fn new(window: *mut c_void) -> Option<Gfx> {
        unsafe {
            let dpy = eglGetDisplay(core::ptr::null_mut());
            if dpy.is_null() {
                return None;
            }
            let mut maj = 0;
            let mut min = 0;
            if eglInitialize(dpy, &mut maj, &mut min) == 0 {
                return None;
            }
            let cfg_attribs: [i32; 13] = [
                EGL_SURFACE_TYPE, EGL_WINDOW_BIT,
                EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT,
                EGL_RED_SIZE, 8, EGL_GREEN_SIZE, 8, EGL_BLUE_SIZE, 8, EGL_ALPHA_SIZE, 8,
                EGL_NONE,
            ];
            let mut cfg: EGLConfig = core::ptr::null_mut();
            let mut n = 0;
            if eglChooseConfig(dpy, cfg_attribs.as_ptr(), &mut cfg, 1, &mut n) == 0 || n == 0 {
                return None;
            }
            let ctx_attribs: [i32; 3] = [EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE];
            let ctx = eglCreateContext(dpy, cfg, core::ptr::null_mut(), ctx_attribs.as_ptr());
            if ctx.is_null() {
                return None;
            }
            let surf = eglCreateWindowSurface(dpy, cfg, window, core::ptr::null());
            if surf.is_null() {
                return None;
            }
            if eglMakeCurrent(dpy, surf, surf, ctx) == 0 {
                return None;
            }

            let vs = shader(GL_VERTEX_SHADER, concat!(
                "attribute vec2 aPos;\n",
                "attribute vec2 aUV;\n",
                "uniform vec2 uRes;\n",
                "varying vec2 vUV;\n",
                "void main(){ vUV = aUV;\n",
                "  vec2 p = aPos / uRes * 2.0 - 1.0;\n",
                "  gl_Position = vec4(p.x, -p.y, 0.0, 1.0); }\n"));
            let fs = shader(GL_FRAGMENT_SHADER, concat!(
                "precision mediump float;\n",
                "varying vec2 vUV;\n",
                "uniform sampler2D uTex;\n",
                "uniform vec3 uColor;\n",
                "void main(){ float a = texture2D(uTex, vUV).a;\n",
                "  gl_FragColor = vec4(uColor * a, a); }\n"));
            let prog = glCreateProgram();
            glAttachShader(prog, vs);
            glAttachShader(prog, fs);
            glLinkProgram(prog);
            let mut ok: GLint = 0;
            glGetProgramiv(prog, GL_LINK_STATUS, &mut ok);
            if ok == 0 {
                crate::log("program link failed");
                return None;
            }
            glUseProgram(prog);

            let mut g = Gfx {
                dpy,
                surf,
                ctx,
                prog,
                a_pos: glGetAttribLocation(prog, b"aPos\0".as_ptr() as *const c_char),
                a_uv: glGetAttribLocation(prog, b"aUV\0".as_ptr() as *const c_char),
                u_res: glGetUniformLocation(prog, b"uRes\0".as_ptr() as *const c_char),
                u_tex: glGetUniformLocation(prog, b"uTex\0".as_ptr() as *const c_char),
                u_color: glGetUniformLocation(prog, b"uColor\0".as_ptr() as *const c_char),
                glyphs: [Glyph::default(); 128],
                atlas_w: ATLAS_COLS * CELL,
                atlas_h: ATLAS_ROWS * CELL,
                acw: CELL as f32,
                ach: CELL as f32,
                cell_w: 16.0,
                cell_h: 34.0,
                verts: Vec::with_capacity(6 * 4 * 1000),
            };
            glDisable(GL_BLEND);
            glEnable(GL_BLEND);
            glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
            g.build_atlas();
            Some(g)
        }
    }

    fn build_atlas(&mut self) {
        // The device ships this; if it is gone we simply draw nothing rather
        // than pretending to render text.
        let path = "/system/fonts/DroidSansMono.ttf";
        let data = match std::fs::read(path) {
            Ok(d) => d,
            Err(_) => {
                crate::log("no system monospace font");
                return;
            }
        };
        let font = match fontdue::Font::from_bytes(data, fontdue::FontSettings::default()) {
            Ok(f) => f,
            Err(_) => {
                crate::log("font parse failed");
                return;
            }
        };
        let (m, _) = font.rasterize('M', PX);
        let adv = m.advance_width.max(1.0);
        self.cell_w = adv.ceil();
        self.cell_h = (PX * 1.55).ceil();
        let baseline = self.cell_h - PX * 0.35;

        // One atlas cell == one screen cell. Anything else makes the glyph's
        // own (xmin, baseline - bottom) offset act twice: once when it is
        // painted into the atlas and once when the quad is placed on screen.
        self.acw = self.cell_w;
        self.ach = self.cell_h;
        self.atlas_w = ATLAS_COLS * self.acw as i32;
        self.atlas_h = ATLAS_ROWS * self.ach as i32;
        let acw = self.acw as i32;
        let ach = self.ach as i32;

        let aw = self.atlas_w as usize;
        let ah = self.atlas_h as usize;
        let mut pix = vec![0u8; aw * ah];

        for code in 32u8..=126u8 {
            let ch = code as char;
            let (m, bitmap) = font.rasterize(ch, PX);
            let gx = ((code as i32 - 32) % ATLAS_COLS) * acw;
            let gy = ((code as i32 - 32) / ATLAS_COLS) * ach;
            let dest_x = gx + m.xmin;
            let dest_y = gy + (baseline - m.ymin as f32 - m.height as f32).round() as i32;
            for row in 0..m.height {
                for col in 0..m.width {
                    let v = bitmap[row * m.width + col];
                    if v == 0 {
                        continue;
                    }
                    let x = dest_x + col as i32;
                    let y = dest_y + row as i32;
                    if x < 0 || y < 0 || x >= self.atlas_w || y >= self.atlas_h {
                        continue;
                    }
                    let i = y as usize * aw + x as usize;
                    pix[i] = pix[i].saturating_add(v);
                }
            }
            self.glyphs[code as usize] = Glyph {
                u0: gx as f32 / self.atlas_w as f32,
                v0: gy as f32 / self.atlas_h as f32,
                u1: (gx + acw) as f32 / self.atlas_w as f32,
                v1: (gy + ach) as f32 / self.atlas_h as f32,
                // The offset already lives inside the atlas cell.
                dx: 0.0,
                dy: 0.0,
                w: acw as f32,
                h: ach as f32,
            };
        }

        // Cursor cell: a solid block, so the cursor is the same draw path as text.
        {
            let code = crate::term::BLOCK as i32;
            let gx = ((code - 32) % ATLAS_COLS) * acw;
            let gy = ((code - 32) / ATLAS_COLS) * ach;
            for y in gy..(gy + ach).min(self.atlas_h) {
                for x in gx..(gx + acw).min(self.atlas_w) {
                    pix[y as usize * aw + x as usize] = 255;
                }
            }
            self.glyphs[code as usize] = Glyph {
                u0: gx as f32 / self.atlas_w as f32,
                v0: gy as f32 / self.atlas_h as f32,
                u1: (gx + acw) as f32 / self.atlas_w as f32,
                v1: (gy + ach) as f32 / self.atlas_h as f32,
                dx: 0.0,
                dy: 0.0,
                w: acw as f32,
                h: ach as f32,
            };
        }

        unsafe {
            let mut tex: GLuint = 0;
            glGenTextures(1, &mut tex);
            glActiveTexture(GL_TEXTURE0);
            glBindTexture(GL_TEXTURE_2D, tex);
            glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
            glTexImage2D(
                GL_TEXTURE_2D, 0, GL_ALPHA as GLint, self.atlas_w, self.atlas_h, 0,
                GL_ALPHA, GL_UNSIGNED_BYTE, pix.as_ptr() as *const c_void,
            );
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
            glUniform1i(self.u_tex, 0);
        }
    }

    pub fn cell(&self) -> (f32, f32) {
        (self.cell_w, self.cell_h)
    }

    pub fn size(&self) -> (i32, i32) {
        let (mut w, mut h) = (0, 0);
        unsafe {
            eglQuerySurface(self.dpy, self.surf, EGL_WIDTH, &mut w);
            eglQuerySurface(self.dpy, self.surf, EGL_HEIGHT, &mut h);
        }
        (w, h)
    }

    pub fn draw(&mut self, cells: &[u8], cols: usize, rows: usize, cx: usize, cy: usize, focused: bool) {
        let (w, h) = self.size();
        if w <= 0 || h <= 0 {
            return;
        }
        unsafe {
            glViewport(0, 0, w, h);
            glClearColor(0.04, 0.05, 0.06, 1.0);
            glClear(GL_COLOR_BUFFER_BIT);
            glUseProgram(self.prog);
            glUniform2f(self.u_res, w as f32, h as f32);
            glUniform3f(self.u_color, 0.78, 0.84, 0.90);
        }
        self.verts.clear();
        let glyphs = self.glyphs; // Copy: 128 * 8 floats
        let cell_w = self.cell_w;
        let cell_h = self.cell_h;
        let (cols32, rows32) = (cols as i32, rows as i32);
        let mut verts = core::mem::take(&mut self.verts);
        for r in 0..rows32 {
            for c in 0..cols32 {
                let ch = cells[(r * cols32 + c) as usize];
                if ch == b' ' || ch >= 127 {
                    continue;
                }
                let g = glyphs[ch as usize];
                if g.w == 0.0 {
                    continue;
                }
                let x0 = c as f32 * cell_w + g.dx;
                let y0 = r as f32 * cell_h + g.dy;
                let x1 = x0 + g.w;
                let y1 = y0 + g.h;
                let uv = [(g.u0, g.v0), (g.u1, g.v0), (g.u1, g.v1), (g.u0, g.v1)];
                let pos = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)];
                for i in [0usize, 1, 2, 0, 2, 3] {
                    verts.push(pos[i].0);
                    verts.push(pos[i].1);
                    verts.push(uv[i].0);
                    verts.push(uv[i].1);
                }
            }
        }
        if focused && cy < rows && cx < cols {
            let g = glyphs[crate::term::BLOCK as usize];
            let x0 = cx as f32 * cell_w;
            let y0 = cy as f32 * cell_h;
            let x1 = x0 + cell_w;
            let y1 = y0 + cell_h;
            let uv = [(g.u0, g.v0), (g.u1, g.v0), (g.u1, g.v1), (g.u0, g.v1)];
            let pos = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)];
            for i in [0usize, 1, 2, 0, 2, 3] {
                verts.push(pos[i].0);
                verts.push(pos[i].1);
                verts.push(uv[i].0);
                verts.push(uv[i].1);
            }
        }
        self.verts = verts;
        unsafe {
            if !self.verts.is_empty() {
                let stride = 4 * core::mem::size_of::<f32>() as GLsizei;
                glVertexAttribPointer(self.a_pos as GLuint, 2, GL_FLOAT, 0, stride, self.verts.as_ptr() as *const c_void);
                glEnableVertexAttribArray(self.a_pos as GLuint);
                let off = 2 * core::mem::size_of::<f32>();
                glVertexAttribPointer(self.a_uv as GLuint, 2, GL_FLOAT, 0, stride, (self.verts.as_ptr() as *const u8).add(off) as *const c_void);
                glEnableVertexAttribArray(self.a_uv as GLuint);
                glDrawArrays(GL_TRIANGLES, 0, (self.verts.len() / 4) as GLsizei);
            }
            eglSwapBuffers(self.dpy, self.surf);
        }
    }
}

impl Drop for Gfx {
    fn drop(&mut self) {
        unsafe {
            eglMakeCurrent(self.dpy, core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut());
            eglDestroySurface(self.dpy, self.surf);
            eglDestroyContext(self.dpy, self.ctx);
        }
    }
}
