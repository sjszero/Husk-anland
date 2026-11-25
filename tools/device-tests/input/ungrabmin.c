// ungrabmin.c — 把「XIUngrabDevice 之后 SIGSEGV」拆到最小（evidence 208 §3 / 209）。
//
// 用法: ungrabmin <xi|core|xi-grab> [秒]
//   xi       只做 XOpenDisplay -> XIUngrabDevice(dev=2, 0) -> XSync
//   core     只做 XOpenDisplay -> XUngrabDevice(0=CurrentTime)  -> XSync   （核心协议，2 个参数）
//   xi-grab  XIGrabDevice(root) 之后再 XIUngrabDevice
//
// 为什么要区分 xi 和 core：xi2probe.c 里那行 `XUngrabDevice(d, 2, 0)` 调的是**核心协议**
// 的 XUngrabDevice（原型只有 2 个参数），它根本不会释放 XI2 的抓取。所以 208 记的
// 「崩点在 XIUngrabDevice」需要重测——那个程序从来没调用过 XIUngrabDevice。
//
// rootfs 里没有 X11 头，原型按 X.Org 的头文件手写；链接直接用运行时的 .so.6。
#define _GNU_SOURCE
#include <stdio.h>
#include <string.h>
#include <stdlib.h>
#include <unistd.h>

typedef struct _XDisplay Display;
typedef unsigned long Window, Time, XID;

typedef struct { int deviceid; int mask_len; unsigned char *mask; } XIEventMask;

extern Display *XOpenDisplay(const char *);
extern Window XDefaultRootWindow(Display *);
extern int XSync(Display *, int);
extern int XCloseDisplay(Display *);
extern int XSelectInput(Display *, Window, long);
/* 核心协议：int XUngrabDevice(Display *dpy, Time time); */
extern int XUngrabDevice(Display *, Time);
/* 核心协议：int XUngrabPointer(Display *dpy, Time time); */
extern int XUngrabPointer(Display *, Time);
/* XI2：int XIUngrabDevice(Display *dpy, int deviceid, Time time); */
extern int XIUngrabDevice(Display *, int, Time);
/* XI2：int XIGrabDevice(Display *dpy, int deviceid, Window grab_window, Time time,
 *                       Cursor cursor, int grab_mode, int paired_device_mode,
 *                       Bool owner_events, XIEventMask *mask); */
extern int XIGrabDevice(Display *, int, Window, Time, XID, int, int, int, XIEventMask *);
extern int XISelectEvents(Display *, Window, XIEventMask *, int);

#define XI_ButtonPress 4
#define XI_ButtonRelease 5
#define XI_Motion 6

int main(int argc, char **argv) {
    const char *mode = argc > 1 ? argv[1] : "xi";
    Display *d = XOpenDisplay(NULL);
    if (!d) { fprintf(stderr, "no display\n"); return 1; }
    printf("step 1: XOpenDisplay ok (%s)\n", mode);
    fflush(stdout);

    if (!strcmp(mode, "xi-grab")) {
        Window root = XDefaultRootWindow(d);
        unsigned char m[4] = {0};
        m[0] |= (1 << XI_Motion) | (1 << XI_ButtonPress) | (1 << XI_ButtonRelease);
        XIEventMask sel = { 1 /* XIAllMasterDevices */, 4, m };
        printf("step 2: XISelectEvents rc=%d\n", XISelectEvents(d, root, &sel, 1));
        XSync(d, 0);
        printf("step 3: XIGrabDevice rc=%d\n",
               XIGrabDevice(d, 2, root, 0, 0, 1, 1, 0, &sel));
        XSync(d, 0);
    }

    if (!strcmp(mode, "core-pt")) {
        printf("step 4: XUngrabPointer(dpy, CurrentTime) rc=%d\n", XUngrabPointer(d, 0));
    } else if (!strcmp(mode, "core")) {
        printf("step 4: XUngrabDevice(dpy, CurrentTime) rc=%d\n", XUngrabDevice(d, 0));
    } else {
        printf("step 4: XIUngrabDevice(dpy, 2, CurrentTime) rc=%d\n", XIUngrabDevice(d, 2, 0));
    }
    fflush(stdout);

    XSync(d, 0);
    printf("step 5: XSync ok\n");
    fflush(stdout);
    XCloseDisplay(d);
    printf("step 6: XCloseDisplay ok\n");
    fflush(stdout);
    return 0;
}
