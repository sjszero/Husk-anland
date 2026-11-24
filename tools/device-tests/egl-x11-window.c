/* egl-x11-window.c —— 窗口化 EGL/GLES2 探针：在真实 X11 窗口上建 EGL surface，连续 swap。
 * 目的：把「mutter 合成器在 zink 下崩」这一条缩到最小可复现单元。
 * 无头文件（rootfs 的 libc6-dev 头不全），X11/EGL/GL 全部 dlopen。
 * 每个阶段都打一行，崩在哪一步一看就知道。
 */
typedef void *VP; typedef void *DPY; typedef void *CFG; typedef void *CTX; typedef void *SFC;
typedef int I32; typedef unsigned U32; typedef unsigned char U8; typedef unsigned long UL;
typedef char C8; typedef unsigned long WID; typedef unsigned long XID;
extern VP dlopen(const char *, I32); extern VP dlsym(VP, const char *);
extern int printf(const char *, ...); extern int setvbuf(VP, C8 *, I32, UL); extern VP stdout;

#define RGBA 0x1908
#define UBYTE 0x1401
#define NO_ERROR 0x3000
#define EGL_PLATFORM_X11 0x31D5

typedef I32  (*PFN_swap)(DPY, SFC);
typedef I32  (*PFN_err)(void);
typedef void (*PFN_clearcol)(float, float, float, float);
typedef void (*PFN_clear)(U32);
typedef void (*PFN_viewport)(I32, I32, I32, I32);
typedef void (*PFN_readpix)(I32, I32, I32, I32, U32, U32, void *);

int main(void) {
    setvbuf(stdout, 0, 2, 0);

    VP x = dlopen("libX11.so.6", 2);
    if (!x) { printf("FAIL dlopen X11\n"); return 1; }
    VP (*xopen)(const C8 *) = (VP (*)(const C8 *))dlsym(x, "XOpenDisplay");
    WID (*xmkwin)(VP, WID, I32, I32, U32, U32, U32, UL, UL, I32, U32, VP) =
        (WID (*)(VP, WID, I32, I32, U32, U32, U32, UL, UL, I32, U32, VP))dlsym(x, "XCreateSimpleWindow");
    I32 (*xmap)(VP, WID) = (I32 (*)(VP, WID))dlsym(x, "XMapWindow");
    I32 (*xsync)(VP, I32) = (I32 (*)(VP, I32))dlsym(x, "XSync");
    I32 (*xflush)(VP) = (I32 (*)(VP))dlsym(x, "XFlush");

    VP dpy = xopen(0);
    if (!dpy) { printf("FAIL XOpenDisplay DISPLAY=%s\n", (const C8 *)"?"); return 1; }
    printf("stage 1: XOpenDisplay OK\n");
    WID (*xroot)(VP) = (WID (*)(VP))dlsym(x, "XDefaultRootWindow");
    WID root = xroot ? xroot(dpy) : 0;
    printf("stage 1b: root window=%lu\n", (UL)root);
    WID win = xmkwin(dpy, root, 0, 0, 640, 480, 0, 0, 0, 0, 0, 0);
    xmap(dpy, win); xsync(dpy, 0);
    printf("stage 2: window mapped id=%lu\n", (UL)win);

    VP e = dlopen("libEGL.so.1", 2);
    if (!e) { printf("FAIL dlopen EGL\n"); return 1; }
    VP (*egpd)(U32, VP, const I32 *) = (VP (*)(U32, VP, const I32 *))dlsym(e, "eglGetPlatformDisplay");
    DPY d = egpd ? (DPY)egpd(EGL_PLATFORM_X11, dpy, 0) : (DPY)0;
    if (!d) { printf("FAIL eglGetPlatformDisplay X11\n"); return 1; }
    I32 (*einit)(DPY, I32 *, I32 *) = (I32 (*)(DPY, I32 *, I32 *))dlsym(e, "eglInitialize");
    I32 ma = 0, mi = 0;
    if (!einit(d, &ma, &mi)) { printf("FAIL eglInitialize\n"); return 1; }
    printf("stage 3: eglInitialize %d.%d\n", ma, mi);
    I32 (*ebind)(U32) = (I32 (*)(U32))dlsym(e, "eglBindAPI");
    ebind(0x30A0); /* EGL_OPENGL_ES_API */
    I32 (*echoose)(DPY, const I32 *, CFG *, I32, I32 *) = (I32 (*)(DPY, const I32 *, CFG *, I32, I32 *))dlsym(e, "eglChooseConfig");
    I32 cfgattr[] = { 0x3040, 0x0004, /* EGL_RENDERABLE_TYPE = ES2 */
                      0x3024, 8, 0x3023, 8, 0x3022, 8, 0x3021, 8, 0x3033, 0x0001, 0x3038 };
    CFG cfg = 0; I32 n = 0;
    if (!echoose(d, cfgattr, &cfg, 1, &n) || n < 1) { printf("FAIL eglChooseConfig n=%d\n", n); return 1; }
    printf("stage 4: config OK\n");
    SFC (*emkwin)(DPY, CFG, WID, const I32 *) = (SFC (*)(DPY, CFG, WID, const I32 *))dlsym(e, "eglCreateWindowSurface");
    SFC sfc = emkwin(d, cfg, win, 0);
    I32 (*eerr)(void) = (I32 (*)(void))dlsym(e, "eglGetError");
    if (!sfc) { printf("FAIL eglCreateWindowSurface err=0x%x\n", eerr()); return 1; }
    printf("stage 5: window surface OK\n");
    CTX (*emkctx)(DPY, CFG, CTX, const I32 *) = (CTX (*)(DPY, CFG, CTX, const I32 *))dlsym(e, "eglCreateContext");
    I32 ctxattr[] = { 0x3098, 2, 0x3038 };
    CTX ctx = emkctx(d, cfg, 0, ctxattr);
    if (!ctx) { printf("FAIL eglCreateContext err=0x%x\n", eerr()); return 1; }
    printf("stage 6: context OK\n");
    I32 (*ecur)(DPY, SFC, SFC, CTX) = (I32 (*)(DPY, SFC, SFC, CTX))dlsym(e, "eglMakeCurrent");
    if (!ecur(d, sfc, sfc, ctx)) { printf("FAIL eglMakeCurrent err=0x%x\n", eerr()); return 1; }
    printf("stage 7: makeCurrent OK\n");

    VP g = dlopen("libGLESv2.so.2", 2);
    if (!g) { printf("FAIL dlopen GLESv2\n"); return 1; }
    PFN_clearcol clc = (PFN_clearcol)dlsym(g, "glClearColor");
    PFN_clear cl = (PFN_clear)dlsym(g, "glClear");
    PFN_viewport vp = (PFN_viewport)dlsym(g, "glViewport");
    PFN_readpix rp = (PFN_readpix)dlsym(g, "glReadPixels");
    PFN_swap sw = (PFN_swap)dlsym(e, "eglSwapBuffers");
    const U8 *(*gstr)(U32) = (const U8 *(*)(U32))dlsym(g, "glGetString");
    printf("renderer=%s\n", gstr(0x1F01));
    vp(0, 0, 640, 480);

    int i;
    for (i = 0; i < 60; i++) {
        clc((float)(i % 2), (float)((i / 2) % 2), (float)((i / 4) % 2), 1.0f);
        cl(0x4000);
        if (i == 0) printf("stage 8: first draw OK\n");
        if (!sw(d, sfc)) { printf("FAIL eglSwapBuffers at frame %d err=0x%x\n", i, eerr()); return 1; }
        if (i == 0) printf("stage 9: first swap OK\n");
    }
    printf("stage 10: 60 swaps OK\n");
    U8 px[4] = { 0, 0, 0, 0 };
    clc(0.0f, 1.0f, 0.0f, 1.0f); cl(0x4000); sw(d, sfc);
    rp(10, 10, 1, 1, RGBA, UBYTE, px);
    printf("readback px=%d,%d,%d,%d err=0x%x\n", px[0], px[1], px[2], px[3], eerr());
    printf("RESULT %s\n", (px[0] < 40 && px[1] > 200) ? "PASS" : "SUSPECT");
    return 0;
}
