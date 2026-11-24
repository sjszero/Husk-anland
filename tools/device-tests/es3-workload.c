/* es3test.c —— 真 ES3 workload：VAO + 实例化 + UBO + 纹理 + MRT，读回校验像素。
 * 无头文件（这棵 rootfs 的 libc6-dev 头不全），EGL/GL 全部 dlopen。
 */
typedef void *VP; typedef void *DPY; typedef void *CFG; typedef void *CTX; typedef void *SFC;
typedef int I32; typedef unsigned U32; typedef unsigned char U8; typedef float F32; typedef unsigned long UL;
typedef char C8;
extern VP dlopen(const char *, I32); extern VP dlsym(VP, const char *);
extern int printf(const char *, ...);
extern int setvbuf(VP, C8 *, I32, UL); extern VP stdout;

#define TEX_2D 0x0DE1
#define RGBA 0x1908
#define UBYTE 0x1401
#define NEAREST 0x2600
#define TEX_MIN 0x2801
#define TEX_MAG 0x2800
#define TEX_WRAP_S 0x2802
#define TEX_WRAP_T 0x2803
#define CLAMP_EDGE 0x812F
#define ARRAY_BUF 0x8892
#define UNIFORM_BUF 0x8A11
#define STATIC_DRAW 0x88E4
#define FLOAT 0x1406
#define VSH 0x8B31
#define FSH 0x8B30
#define COMPILE_STATUS 0x8B81
#define LINK_STATUS 0x8B82
#define INFO_LOG 0x8B84
#define COL_ATT0 0x8CE0
#define COL_ATT1 0x8CE1
#define FRAMEBUF 0x8D40
#define FB_COMPLETE 0x8CD5
#define RGBA8 0x8058
#define COLOR_BUF_BIT 0x4000
#define TRI_STRIP 0x0005
#define TEX0 0x84C0
#define DEPTH_TEST 0x0B71
#define UNPACK_ALIGN 0x0CF5
#define PIXEL_PACK 0x0D02

static VP h; static VP gpa(const char *n) { return dlsym(h, n); }

typedef void  (*PFN_v_u)(U32); typedef void (*PFN_v_2u)(U32,U32); typedef void (*PFN_v_3u)(U32,U32,U32);
typedef void  (*PFN_v_i)(I32); typedef void (*PFN_v_4i)(I32,I32,I32,I32);
typedef void  (*PFN_v_5i)(I32,I32,I32,I32,I32); typedef void (*PFN_v_6i)(I32,I32,I32,I32,I32,I32);
typedef void  (*PFN_v_8i)(I32,I32,I32,I32,I32,I32,I32,I32);
typedef U32   (*PFN_u_v)(void); typedef void (*PFN_v_u2)(U32,U32*);
typedef void  (*PFN_shader_src)(U32,I32,const C8* const*,const I32*);
typedef void  (*PFN_getlog)(U32,I32,I32*,C8*);
typedef void  (*PFN_3u_i)(U32,U32,I32*);
typedef void  (*PFN_v_none)(void);
typedef U32   (*PFN_u_u)(U32);
typedef U32   (*PFN_err)(void);
typedef void  (*PFN_drawbufs)(I32, const U32*);
typedef void  (*PFN_i_i)(I32,I32);
typedef void  (*PFN_2u)(U32,U32);
typedef void  (*PFN_gen)(U32,U32*);
typedef void  (*PFN_4f)(F32,F32,F32,F32);
typedef void  (*PFN_draw4)(U32,I32,I32,I32);
typedef void  (*PFN_bindbuf)(U32,U32); typedef void (*PFN_bindbufbase)(U32,U32,U32);
typedef void  (*PFN_bufdata)(U32,long,const void*,U32);
typedef void  (*PFN_vaa)(I32,I32,I32); typedef void (*PFN_attribptr)(U32,I32,U32,U8,I32,const void*);
typedef void  (*PFN_divisor)(U32,U32); typedef void (*PFN_drawinst)(U32,I32,U32);
typedef void  (*PFN_tex2d)(U32,I32,I32,I32,I32,I32,U32,U32,const void*);
typedef void  (*PFN_texsub)(U32,I32,I32,I32,I32,U32,U32,const void*);
typedef void  (*PFN_texpar)(U32,U32,I32);
typedef void  (*PFN_fbtex)(U32,U32,U32,U32,I32);
typedef U32   (*PFN_checkfb)(U32);
typedef void  (*PFN_readpix)(I32,I32,I32,I32,U32,U32,void*);
typedef const U8* (*PFN_getstr)(U32);
typedef I32   (*PFN_getuniloc)(U32,const C8*);

int main(void) {
    setvbuf(stdout, 0, 2, 0);

    /* ---- EGL ---- */
    VP e = dlopen("libEGL.so.1", 2);
    if (!e) { printf("FAIL dlopen EGL\n"); return 1; }
    VP (*egpd)(U32, const void *, const I32 *) = (VP (*)(U32, const void *, const I32 *))dlsym(e, "eglGetPlatformDisplay");
    DPY d = egpd ? (DPY)egpd(0x31DD, 0, 0) : (DPY)0;
    if (!d) d = (DPY)((VP(*)(VP))dlsym(e, "eglGetDisplay"))(0);
    I32 (*einit)(DPY, I32 *, I32 *) = (I32 (*)(DPY, I32 *, I32 *))dlsym(e, "eglInitialize");
    I32 ma = 0, mi = 0;
    if (!einit(d, &ma, &mi)) { printf("FAIL eglInitialize\n"); return 1; }
    printf("EGL %d.%d\n", ma, mi);
    I32 (*ebind)(U32) = (I32 (*)(U32))dlsym(e, "eglBindAPI");
    I32 (*echoose)(DPY, const I32 *, CFG *, I32, I32 *) = (I32 (*)(DPY, const I32 *, CFG *, I32, I32 *))dlsym(e, "eglChooseConfig");
    CTX (*emkctx)(DPY, CFG, CTX, const I32 *) = (CTX (*)(DPY, CFG, CTX, const I32 *))dlsym(e, "eglCreateContext");
    I32 (*ecur)(DPY, SFC, SFC, CTX) = (I32 (*)(DPY, SFC, SFC, CTX))dlsym(e, "eglMakeCurrent");
    I32 (*eerr)(void) = (I32 (*)(void))dlsym(e, "eglGetError");
    ebind(0x30A0);                                  /* EGL_OPENGL_ES_API */
    CFG c = 0; I32 got = 0;
    /* 先要 ES3 位（0x40）的 config —— 没有它，ES3 上下文必然 EGL_BAD_CONFIG */
    I32 want3[] = { 0x3033, 0x0001, 0x3040, 0x0040, 0x3038 };  /* SURFACE=PBUFFER, RENDERABLE=ES3 */
    I32 es3cfg = (echoose(d, want3, &c, 1, &got) && got) ? 1 : 0;
    printf("ES3 位 config: %s\n", es3cfg ? "有" : "没有");
    if (!es3cfg) {
        I32 want2[] = { 0x3033, 0x0001, 0x3040, 0x0004, 0x3038 };
        if (!echoose(d, want2, &c, 1, &got) || !got) { printf("FAIL eglChooseConfig(ES2 也没有)\n"); return 1; }
        printf("只有 ES2 位 config → 这台设备在当前设置下建不出 ES3 上下文\n");
        return 2;
    }
    I32 cattrs[] = { 0x3098, 3, 0x30FB, 0, 0x3038 }; /* MAJOR=3, MINOR=0 */
    CTX ctx = emkctx(d, c, 0, cattrs);
    if (!ctx) { printf("FAIL eglCreateContext(ES3) err=%#x\n", eerr()); return 2; }
    if (!ecur(d, 0, 0, ctx)) { printf("FAIL eglMakeCurrent err=%#x\n", eerr()); return 2; }
    printf("ES3 上下文建立 OK\n");

    /* ---- GL 函数 ---- */
    PFN_u_u glCreateShader; PFN_shader_src glShaderSource; PFN_v_u glCompileShader;
    PFN_getlog glGetShaderInfoLog; PFN_3u_i glGetShaderiv;
    PFN_u_v glCreateProgram; PFN_v_u2 glAttachShader; PFN_v_u glLinkProgram;
    PFN_getlog glGetProgramInfoLog; PFN_3u_i glGetProgramiv; PFN_v_u glUseProgram;
    PFN_gen glGenVertexArrays; PFN_v_u glBindVertexArray;
    PFN_gen glGenBuffers; PFN_bindbuf glBindBuffer; PFN_bufdata glBufferData;
    PFN_bufdata glBufferSubData; PFN_attribptr glVertexAttribPointer;
    PFN_v_u glEnableVertexAttribArray; PFN_divisor glVertexAttribDivisor;
    PFN_vaa glVertexAttrib1f;
    PFN_gen glGenTextures; PFN_v_u2 glBindTexture; PFN_tex2d glTexImage2D;
    PFN_texpar glTexParameteri; PFN_v_u glActiveTexture;
    PFN_gen glGenFramebuffers; PFN_2u glBindFramebuffer; PFN_fbtex glFramebufferTexture2D;
    PFN_checkfb glCheckFramebufferStatus;
    PFN_err glGetError; PFN_drawbufs glDrawBuffers;
    PFN_v_4i glViewport; PFN_4f glClearColor; PFN_v_u glClear; PFN_v_u glDisable;
    PFN_draw4 glDrawArraysInstanced; PFN_bindbufbase glBindBufferBase;
    PFN_v_none glFinish; PFN_readpix glReadPixels; PFN_v_u glReadBuffer;
    PFN_v_2u glPixelStorei;
    PFN_getstr glGetString; PFN_getuniloc glGetUniformLocation;
    PFN_i_i glUniform1i;

    VP l = dlopen("libGLESv2.so.2", 2);
    if (!l) { printf("FAIL dlopen GLESv2\n"); return 1; }
#define G(name) ((VP)dlsym(l, name))
    glCreateShader = (PFN_u_u)G("glCreateShader"); glShaderSource = (PFN_shader_src)G("glShaderSource");
    glCompileShader = (PFN_v_u)G("glCompileShader"); glGetShaderInfoLog = (PFN_getlog)G("glGetShaderInfoLog");
    glGetShaderiv = (PFN_3u_i)G("glGetShaderiv"); glCreateProgram = (PFN_u_v)G("glCreateProgram");
    glAttachShader = (PFN_v_u2)G("glAttachShader"); glLinkProgram = (PFN_v_u)G("glLinkProgram");
    glGetProgramInfoLog = (PFN_getlog)G("glGetProgramInfoLog"); glGetProgramiv = (PFN_3u_i)G("glGetProgramiv");
    glUseProgram = (PFN_v_u)G("glUseProgram"); glGenVertexArrays = (PFN_gen)G("glGenVertexArrays");
    glBindVertexArray = (PFN_v_u)G("glBindVertexArray"); glGenBuffers = (PFN_gen)G("glGenBuffers");
    glBindBuffer = (PFN_bindbuf)G("glBindBuffer"); glBufferData = (PFN_bufdata)G("glBufferData");
    glBufferSubData = (PFN_bufdata)G("glBufferSubData");
    glVertexAttribPointer = (PFN_attribptr)G("glVertexAttribPointer");
    glEnableVertexAttribArray = (PFN_v_u)G("glEnableVertexAttribArray");
    glVertexAttribDivisor = (PFN_divisor)G("glVertexAttribDivisor");
    glGenTextures = (PFN_gen)G("glGenTextures"); glBindTexture = (PFN_v_u2)G("glBindTexture");
    glTexImage2D = (PFN_tex2d)G("glTexImage2D"); glTexParameteri = (PFN_texpar)G("glTexParameteri");
    glActiveTexture = (PFN_v_u)G("glActiveTexture"); glGenFramebuffers = (PFN_gen)G("glGenFramebuffers");
    glBindFramebuffer = (PFN_2u)G("glBindFramebuffer");
    glFramebufferTexture2D = (PFN_fbtex)G("glFramebufferTexture2D");
    glCheckFramebufferStatus = (PFN_checkfb)G("glCheckFramebufferStatus");
    glGetError = (PFN_err)G("glGetError"); glDrawBuffers = (PFN_drawbufs)G("glDrawBuffers");
    glViewport = (PFN_v_4i)G("glViewport"); glClearColor = (PFN_4f)G("glClearColor");
    glClear = (PFN_v_u)G("glClear"); glDisable = (PFN_v_u)G("glDisable");
    glDrawArraysInstanced = (PFN_draw4)G("glDrawArraysInstanced");
    glBindBufferBase = (PFN_bindbufbase)G("glBindBufferBase");
    glFinish = (PFN_v_none)G("glFinish"); glReadPixels = (PFN_readpix)G("glReadPixels");
    glReadBuffer = (PFN_v_u)G("glReadBuffer"); glPixelStorei = (PFN_v_2u)G("glPixelStorei");
    glGetString = (PFN_getstr)G("glGetString"); glGetUniformLocation = (PFN_getuniloc)G("glGetUniformLocation");
    glUniform1i = (PFN_i_i)G("glUniform1i");
    if (!glCreateShader || !glDrawArraysInstanced || !glGenVertexArrays) { printf("FAIL GL 函数缺失\n"); return 1; }
    glDisable(DEPTH_TEST);
    printf("GL_VERSION=%s | RENDERER=%s | GLSL=%s\n", glGetString(0x1F02), glGetString(0x1F01), glGetString(0x8B8C));

    /* ---- shader ---- */
    static const C8 *vs_src =
        "#version 300 es\n"
        "layout(location=0) in vec2 pos;\n"
        "layout(location=1) in vec2 inst;\n"
        "out vec2 vUV;\n"
        "void main(){ vUV = pos + 0.5; gl_Position = vec4(pos + inst, 0.0, 1.0); }\n";
    static const C8 *fs_src =
        "#version 300 es\n"
        "precision highp float;\n"
        "in vec2 vUV;\n"
        "uniform sampler2D tex;\n"
        "layout(std140) uniform B { vec4 color; vec2 off; vec2 pad; } u;\n"
        "layout(location=0) out vec4 o0;\n"
        "layout(location=1) out vec4 o1;\n"
        "void main(){ vec4 t = texture(tex, vUV); o0 = u.color * t; o1 = vec4(1.0) - o0; }\n";
    U32 vs = glCreateShader(VSH), fs = glCreateShader(FSH);
    glShaderSource(vs, 1, &vs_src, 0); glCompileShader(vs);
    glShaderSource(fs, 1, &fs_src, 0); glCompileShader(fs);
    I32 ok = 0; C8 log[512];
    glGetShaderiv(vs, COMPILE_STATUS, &ok);
    if (!ok) { glGetShaderInfoLog(vs, 511, 0, log); printf("FAIL 顶点着色器: %s\n", log); return 3; }
    glGetShaderiv(fs, COMPILE_STATUS, &ok);
    if (!ok) { glGetShaderInfoLog(fs, 511, 0, log); printf("FAIL 片元着色器: %s\n", log); return 3; }
    U32 p = glCreateProgram();
    glAttachShader(p, vs); glAttachShader(p, fs); glLinkProgram(p);
    glGetProgramiv(p, LINK_STATUS, &ok);
    if (!ok) { glGetProgramInfoLog(p, 511, 0, log); printf("FAIL 链接: %s\n", log); return 3; }
    glUseProgram(p);
    printf("GLSL ES 3.00 着色器编译+链接 OK\n");

    /* ---- 几何：单位四边形，按实例偏移铺 2x2 ---- */
    static const F32 quad[] = { -0.5f,-0.5f, 0.5f,-0.5f, -0.5f,0.5f, 0.5f,0.5f };
    static const F32 inst[] = { -0.5f,-0.5f,  0.5f,-0.5f,  -0.5f,0.5f,  0.5f,0.5f };
    U32 vao = 0; glGenVertexArrays(1, &vao); glBindVertexArray(vao);
    U32 vb[2]; glGenBuffers(2, vb);
    glBindBuffer(ARRAY_BUF, vb[0]);
    glBufferData(ARRAY_BUF, sizeof(quad), quad, STATIC_DRAW);
    glVertexAttribPointer(0, 2, FLOAT, 0, 8, (const void *)0);
    glEnableVertexAttribArray(0);
    glBindBuffer(ARRAY_BUF, vb[1]);
    glBufferData(ARRAY_BUF, sizeof(inst), inst, STATIC_DRAW);
    glVertexAttribPointer(1, 2, FLOAT, 0, 8, (const void *)0);
    glEnableVertexAttribArray(1);
    glVertexAttribDivisor(1, 1);

    /* UBO: std140, 32 字节 */
    U32 ub; glGenBuffers(1, &ub);
    glBindBuffer(UNIFORM_BUF, ub);
    F32 blk[8] = { 0.5f, 0.5f, 0.5f, 1.0f, 0.0f, 0.0f, 0.0f, 0.0f };
    glBufferData(UNIFORM_BUF, sizeof(blk), blk, STATIC_DRAW);
    glBindBufferBase(UNIFORM_BUF, 0, ub);

    /* 2x2 纹理：红 绿 / 蓝 白 */
    static const U8 texels[4 * 4] = {
        255,0,0,255,  0,255,0,255,
        0,0,255,255,  255,255,255,255,
    };
    U32 tx; glGenTextures(1, &tx);
    glActiveTexture(TEX0); glBindTexture(TEX_2D, tx);
    glTexParameteri(TEX_2D, TEX_MIN, NEAREST); glTexParameteri(TEX_2D, TEX_MAG, NEAREST);
    glTexParameteri(TEX_2D, TEX_WRAP_S, CLAMP_EDGE); glTexParameteri(TEX_2D, TEX_WRAP_T, CLAMP_EDGE);
    glPixelStorei(UNPACK_ALIGN, 1);
    glTexImage2D(TEX_2D, 0, RGBA8, 2, 2, 0, RGBA, UBYTE, texels);
    glUniform1i(glGetUniformLocation(p, "tex"), 0);

    /* FBO: 两个 RGBA8 颜色附件 */
    U32 fbo, rb[2], rt[2];
    glGenFramebuffers(1, &fbo); glBindFramebuffer(FRAMEBUF, fbo);
    glGenTextures(2, rt);
    for (I32 i = 0; i < 2; i++) {
        glBindTexture(TEX_2D, rt[i]);
        glTexParameteri(TEX_2D, TEX_MIN, NEAREST); glTexParameteri(TEX_2D, TEX_MAG, NEAREST);
        glTexImage2D(TEX_2D, 0, RGBA8, 16, 16, 0, RGBA, UBYTE, 0);
        glFramebufferTexture2D(FRAMEBUF, i == 0 ? COL_ATT0 : COL_ATT1, TEX_2D, rt[i], 0);
    }
    (void)rb;
    /* 关键：上面那两次 glBindTexture 把 0 号单元的采样纹理换成了 FBO 附件，
       必须把 2x2 纹理重新绑回去，否则着色器采样的是自己（反馈环）。 */
    glActiveTexture(TEX0); glBindTexture(TEX_2D, tx);
    U32 st = glCheckFramebufferStatus(FRAMEBUF);
    if (st != FB_COMPLETE) { printf("FAIL FBO 不完整 status=%#x\n", st); return 4; }
    { U32 bufs[2]; bufs[0] = COL_ATT0; bufs[1] = COL_ATT1; glDrawBuffers(2, bufs); }
    printf("MRT(2 附件) FBO 完整 OK, drawBuffers 已设, err=%#x\n", glGetError());

    /* 阶段 A：只清屏，验证 FBO 的两个附件都能写、都能读 */
    { U8 t0[4] = {9,9,9,9}, t1[4] = {9,9,9,9};
      glViewport(0, 0, 16, 16);
      glClearColor(0.25f, 0.5f, 0.75f, 1.0f);
      glClear(COLOR_BUF_BIT);
      glFinish();
      glPixelStorei(PIXEL_PACK, 1);
      glReadBuffer(COL_ATT0); glReadPixels(8, 8, 1, 1, RGBA, UBYTE, t0);
      glReadBuffer(COL_ATT1); glReadPixels(8, 8, 1, 1, RGBA, UBYTE, t1);
      printf("阶段A 清屏回读: att0=%d,%d,%d,%d  att1=%d,%d,%d,%d （期望 64,128,191,255）\n",
             t0[0],t0[1],t0[2],t0[3], t1[0],t1[1],t1[2],t1[3]);
    }
    glViewport(0, 0, 16, 16);
    glClearColor(0, 0, 0, 0);
    glClear(COLOR_BUF_BIT);
    printf("清除后 err=%#x\n", glGetError());
    glDrawArraysInstanced(TRI_STRIP, 0, 4, 4);
    glFinish();
    printf("绘制后 err=%#x\n", glGetError());

    /* ---- 读回校验 ---- */
    /* 注意：这棵树的 glReadPixels 整块读回只写第一行+第一列（见 evidence 162），
       所以这里逐像素 1x1 读，才不会把驱动/驱动的读回缺陷误当成渲染错误。 */
    static U8 px[2][16 * 16 * 4];
    for (I32 i = 0; i < 2; i++) {
        glReadBuffer(i == 0 ? COL_ATT0 : COL_ATT1);
        glPixelStorei(PIXEL_PACK, 1);
        for (I32 y = 0; y < 16; y++)
            for (I32 x = 0; x < 16; x++)
                glReadPixels(x, y, 1, 1, RGBA, UBYTE, &px[i][(y * 16 + x) * 4]);
    }
    /* 期望：每个实例区（8x8）里出现纹理四个颜色 × 0.5 */
    /* u.color=(0.5,0.5,0.5,1.0)：颜色减半，alpha 保持 1.0 */
    const U8 expect[4][4] = { {128,0,0,255}, {0,128,0,255}, {0,0,128,255}, {128,128,128,255} };
    I32 bad0 = 0, bad1 = 0, seen[4] = {0,0,0,0};
    for (I32 y = 0; y < 16; y++) for (I32 x = 0; x < 16; x++) {
        I32 o = (y * 16 + x) * 4;
        I32 hit = -1;
        for (I32 k = 0; k < 4; k++) {
            I32 d = 0;
            for (I32 ch = 0; ch < 4; ch++) { I32 v = px[0][o + ch] - expect[k][ch]; if (v < 0) v = -v; if (v > d) d = v; }
            if (d <= 3) { hit = k; break; }
        }
        if (hit < 0) { if (bad0 < 3) printf("  att0 异常像素 (%d,%d)=%d,%d,%d,%d\n", x, y, px[0][o], px[0][o+1], px[0][o+2], px[0][o+3]); bad0++; }
        else seen[hit]++;
        for (I32 ch = 0; ch < 4; ch++) {
            I32 want = 255 - px[0][o + ch], dv = px[1][o + ch] - want;
            if (dv < 0) dv = -dv;
            if (dv > 3) { if (bad1 < 3) printf("  att1 与 1-att0 不符 (%d,%d) ch%d %d vs %d\n", x, y, ch, px[1][o+ch], want); bad1++; break; }
        }
    }
    printf("att0 像素异常数=%d  纹理四色命中数=%d/%d/%d/%d\n", bad0, seen[0], seen[1], seen[2], seen[3]);
    printf("att1（MRT 第二附件）与 1-att0 不符数=%d\n", bad1);
    I32 pass = (bad0 == 0 && bad1 == 0 && seen[0] && seen[1] && seen[2] && seen[3]);
    printf("%s\n", pass ? "RESULT PASS" : "RESULT FAIL");
    return pass ? 0 : 5;
}
