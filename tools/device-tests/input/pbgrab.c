// pbgrab.c — 被动抓取：在 root 上 XGrabButton(Button1)，看按下时这个客户端能不能抢到
#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
typedef struct _XDisplay Display;
typedef unsigned long Window, Time, XID;
typedef struct { int type; unsigned long serial; int send_event; int pad0;
  Display *display; Window window; Window root; Window subwindow;
  Time time; int x, y, x_root, y_root; unsigned int state; unsigned int button;
  int same_screen; } XButtonEvent;
extern Display *XOpenDisplay(const char *);
extern Window XDefaultRootWindow(Display *);
extern int XGrabButton(Display *, unsigned, unsigned, Window, int, unsigned,
                       int, int, Window, void *);
extern int XSync(Display *, int);
extern int XPending(Display *);
extern int XNextEvent(Display *, void *);
int main(int argc, char **argv) {
  int secs = argc > 1 ? atoi(argv[1]) : 10;
  Display *d = XOpenDisplay(NULL);
  if (!d) { printf("no display\n"); return 1; }
  Window root = XDefaultRootWindow(d);
  /* ButtonPressMask 4 | ButtonReleaseMask 8 | PointerMotionMask 64 = 76 */
  int st = XGrabButton(d, 1, 0 /*AnyModifier*/, root, 0 /*owner_events False*/,
                       76, 1, 1, 0 /*None*/, 0);
  printf("XGrabButton status=%d\n", st);
  fflush(stdout);
  time_t end = time(NULL) + secs;
  while (time(NULL) < end) {
    XSync(d, 0);
    while (XPending(d)) {
      XButtonEvent *e = calloc(1, sizeof(XButtonEvent) * 4);
      XNextEvent(d, e);
      if (e->type == 4) { printf("GOT ButtonPress x=%d y=%d\n", e->x_root, e->y_root); fflush(stdout); }
      else if (e->type == 5) { printf("GOT ButtonRelease\n"); fflush(stdout); }
      else if (e->type == 6) { printf("GOT Motion %d %d\n", e->x_root, e->y_root); fflush(stdout); }
      free(e);
    }
    usleep(100000);
  }
  printf("DONE\n");
  return 0;
}
