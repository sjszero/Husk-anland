// movetest.c — 决定性实验：由"我们自己的客户端"发标准的 _NET_WM_MOVERESIZE，
// 把"GTK/CSD 那条链路"整个排除掉，只看 mutter 的交互式移动在这台机器上能不能起来。
// 有头文件吗？没有（rootfs 里没有 X11 头），所以结构体按 64 位 ABI 手写。
#include <stdio.h>
#include <string.h>
#include <unistd.h>

typedef struct _XDisplay Display;
typedef unsigned long Window, Atom, Time, XID;

#define ButtonPressMask        4
#define ButtonReleaseMask      8
#define StructureNotifyMask    0x20000
#define SubstructureNotifyMask 0x100000
#define SubstructureRedirectMask 0x80000
#define ClientMessage          33
#define ButtonPress            4

typedef struct {
  int type;                  // 0
  unsigned long serial;      // 8
  int send_event;            // 16
  int pad0;
  Display *display;          // 24
  Window window;             // 32
  Atom message_type;         // 40
  int format;                // 48
  int pad1;
  union { char b[20]; short s[10]; long l[5]; } data;  // 56
} ClientMsg;                 // 96

extern Display *XOpenDisplay(const char *);
extern Window XDefaultRootWindow(Display *);
extern Window XCreateSimpleWindow(Display *, Window, int, int, unsigned, unsigned,
                                  unsigned, unsigned long, unsigned long);
extern int XSelectInput(Display *, Window, long);
extern int XMapWindow(Display *, Window);
extern int XStoreName(Display *, Window, const char *);
extern Atom XInternAtom(Display *, const char *, int);
extern int XSendEvent(Display *, Window, int, long, void *);
extern int XFlush(Display *);
extern int XSync(Display *, int);
extern int XNextEvent(Display *, void *);
extern int XQueryPointer(Display *, Window, Window *, Window *, int *, int *,
                         int *, int *, unsigned *);

int main(int argc, char **argv) {
  const char *name = argc > 1 ? argv[1] : "movetest";
  Display *d = XOpenDisplay(NULL);
  if (!d) { fprintf(stderr, "no display\n"); return 1; }
  Window root = XDefaultRootWindow(d);
  Window w = XCreateSimpleWindow(d, root, 300, 300, 320, 220, 0, 0, 0xffffff);
  XStoreName(d, w, name);
  XSelectInput(d, w, ButtonPressMask | ButtonReleaseMask | StructureNotifyMask);
  XMapWindow(d, w);
  XSync(d, 0);
  printf("win=0x%lx\n", (unsigned long)w);
  fflush(stdout);

  Atom a_move = XInternAtom(d, "_NET_WM_MOVERESIZE", 0);
  union { ClientMsg m; long pad[24]; } ev;
  for (;;) {
    memset(&ev, 0, sizeof(ev));
    XNextEvent(d, &ev);
    if (ev.m.type != ClientMessage) {   // 这里只关心我们自己窗口的事件
      if (ev.m.type != ButtonPress) continue;
    }
    if (ev.m.type == ButtonPress) {
      int rx = 0, ry = 0; unsigned mask = 0; Window r2, c2; int wx, wy;
      XQueryPointer(d, root, &r2, &c2, &rx, &ry, &wx, &wy, &mask);
      memset(&ev, 0, sizeof(ev));
      ev.m.type = ClientMessage;
      ev.m.serial = 0;
      ev.m.send_event = 1;
      ev.m.display = d;
      ev.m.window = root;             // 按 EWMH：发给 root
      ev.m.message_type = a_move;
      ev.m.format = 32;
      ev.m.data.l[0] = rx;            // x_root
      ev.m.data.l[1] = ry;            // y_root
      ev.m.data.l[2] = 8;             // _NET_WM_MOVERESIZE_MOVE
      ev.m.data.l[3] = 1;             // button 1
      ev.m.data.l[4] = 1;             // source: application
      XSendEvent(d, root, 0, SubstructureRedirectMask | SubstructureNotifyMask, &ev);
      XSync(d, 0);
      printf("sent _NET_WM_MOVERESIZE at %d,%d for 0x%lx\n", rx, ry, (unsigned long)w);
      fflush(stdout);
    }
  }
  return 0;
}
