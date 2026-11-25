// xi2probe.c — 二分"抓取期间的指针 motion 到底丢在哪一侧"。
// 用法: xi2probe <nograb|grab> <秒数>
//   选 XI_Motion/XI_ButtonPress 在一个自建窗口上；grab 模式再对 dev=2
//   (Virtual core pointer) 做 XIGrabDevice。外面用 XTEST 动指针，看这里收不收得到。
// rootfs 里没有 X11 头，结构体与原型按 64 位 ABI 手写。
#define _GNU_SOURCE
#include <stdio.h>
#include <string.h>
#include <stdlib.h>
#include <unistd.h>
#include <time.h>

typedef struct _XDisplay Display;
typedef unsigned long Window, Atom, Time, XID, Bool32;

typedef struct { int deviceid; int mask_len; unsigned char *mask; } XIEventMask;
typedef struct {
  int type; unsigned long serial; int send_event; int pad0;
  Display *display; int extension; int evtype; unsigned int cookie; void *data;
} XCookie;

extern Display *XOpenDisplay(const char *);
extern Window XDefaultRootWindow(Display *);
extern Window XCreateSimpleWindow(Display *, Window, int, int, unsigned, unsigned,
                                  unsigned, unsigned long, unsigned long);
extern int XSelectInput(Display *, Window, long);
extern int XMapWindow(Display *, Window);
extern int XStoreName(Display *, Window, const char *);
extern int XSync(Display *, int);
extern int XPending(Display *);
extern int XNextEvent(Display *, void *);
extern int XISelectEvents(Display *, Window, XIEventMask *, int);
extern int XIGrabDevice(Display *, int, Window, Time, XID, int, int, int, XIEventMask *);
extern int XUngrabDevice(Display *, int, Time);
extern int XGetEventData(Display *, XCookie *);
extern void XFreeEventData(Display *, XCookie *);

#define XI_ButtonPress   4
#define XI_ButtonRelease 5
#define XI_Motion        6
#define XI_Enter         7
#define XI_Leave         8
#define GenericEvent     35

int main(int argc, char **argv) {
  const char *mode = argc > 1 ? argv[1] : "nograb";
  int secs = argc > 2 ? atoi(argv[2]) : 6;
  int do_grab = strcmp(mode, "grab") == 0;

  Display *d = XOpenDisplay(NULL);
  if (!d) { fprintf(stderr, "no display\n"); return 1; }
  Window root = XDefaultRootWindow(d);
  Window w = XCreateSimpleWindow(d, root, 250, 250, 500, 400, 0, 0, 0x808080);
  XStoreName(d, w, "xi2probe");
  XSelectInput(d, w, 4 | 8);              /* ButtonPress|ButtonRelease, core */
  XMapWindow(d, w);
  XSync(d, 0);
  /* 先让窗口真的 viewable：XIGrabDevice 在 NotViewable 上直接返回 3，测出来的
     是"窗口没画完"而不是"抓取拿不到"。 */
  usleep(400000);
  printf("win=0x%lx mode=%s\n", (unsigned long)w, mode);
  fflush(stdout);

  unsigned char m[4] = {0};
  m[0] |= (1 << XI_Motion) | (1 << XI_ButtonPress) | (1 << XI_ButtonRelease);
  XIEventMask sel = { 1 /* XIAllMasterDevices */, 4, m };
  int rc = XISelectEvents(d, w, &sel, 1);
  printf("XISelectEvents rc=%d\n", rc);
  XSync(d, 0);

  if (do_grab) {
    /* 抓在 root 上：mutter 的搬窗 grab 也是 root 级的，窗口级抓取要额外处理
       viewable 时序，跟要测的东西无关。 */
    Window win = getenv("GRAB_ON_WINDOW") ? w : root;
    int gr = XIGrabDevice(d, 2 /* Virtual core pointer */, win, 0, 0,
                          1 /* GrabModeAsync */, 1, 0 /* owner_events false */, &sel);
    printf("XIGrabDevice ret=%d (0=Success 1=AlreadyGrabbed 3=NotViewable)\n", gr);
    XSync(d, 0);
  }
  fflush(stdout);

  int core_motion = 0, core_btn = 0, xi_motion = 0, xi_btn = 0, xi_other = 0;
  time_t end = time(NULL) + secs;
  union { XCookie c; long pad[24]; } ev;
  while (time(NULL) < end) {
    while (XPending(d)) {
      memset(&ev, 0, sizeof(ev));
      XNextEvent(d, &ev);
      int t = ev.c.type;
      if (getenv("XI2_DEBUG")) { fprintf(stderr, "ev type=%d ext=%d evtype=%d\n", t, ev.c.extension, ev.c.evtype); fflush(stderr); }
      if (t == GenericEvent) {
        /* evtype 由 Xlib 在 cookie 里填好，不取 XGetEventData 也能读——
           这一步只是判定"抓取者有没有收到 XI motion"，不需要事件负载。 */
        int et = ev.c.evtype;
        if (et == XI_Motion) xi_motion++;
        else if (et == XI_ButtonPress || et == XI_ButtonRelease) xi_btn++;
        else xi_other++;
      } else if (t == 6) core_motion++;
      else if (t == 4 || t == 5) core_btn++;
    }
    usleep(20000);
  }
  printf("RESULT mode=%s core_motion=%d core_button=%d xi_motion=%d xi_button=%d xi_other=%d\n",
         mode, core_motion, core_btn, xi_motion, xi_btn, xi_other);
  if (do_grab) XUngrabDevice(d, 2, 0);
  XSync(d, 0);
  return 0;
}
