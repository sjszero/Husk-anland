// grabrecv.c — 抓住指针(root)后，到底还收不收到 XI_Motion
#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <time.h>
typedef struct _XDisplay Display;
typedef unsigned long Window, Time, XID;
typedef struct { int deviceid; int mask_len; unsigned char *mask; } XIEventMask;
typedef struct {
  int type; unsigned long serial; int send_event; int pad0;
  Display *display; int extension; int evtype; unsigned int cookie; void *data;
} XCookie;
typedef struct {
  int type; unsigned long serial; int send_event; int pad0;
  Display *display; int extension; int evtype; unsigned int cookie; void *data;
} XGenericEventCookie;
extern Display *XOpenDisplay(const char *);
extern Window XDefaultRootWindow(Display *);
extern int XISelectEvents(Display *, Window, XIEventMask *, int);
extern int XIGrabDevice(Display *, int, Window, Time, XID, int, int, int, XIEventMask *);
extern int XSync(Display *, int);
extern int XPending(Display *);
extern int XNextEvent(Display *, void *);
extern int XGetEventData(Display *, void *);
extern void XFreeEventData(Display *, void *);
int main(int argc, char **argv) {
  int secs = argc > 1 ? atoi(argv[1]) : 8;
  Display *d = XOpenDisplay(NULL);
  if (!d) { printf("no display\n"); return 1; }
  Window root = XDefaultRootWindow(d);
  unsigned char m[4] = {0,0,0,0};
  XIEventMask mask = {2, 4, m};
  mask.mask[0] = 1 << 6;  /* XI_Motion bit */
  int sel = XISelectEvents(d, root, &mask, 1);
  int st = XIGrabDevice(d, 2, root, 0, 0, 1, 1, 0, &mask);
  printf("select=%d grab-status=%d\n", sel, st);
  fflush(stdout);
  time_t end = time(NULL) + secs;
  int n_motion = 0, n_btn = 0;
  while (time(NULL) < end) {
    XSync(d, 0);
    while (XPending(d)) {
      XGenericEventCookie ev;
      memset(&ev, 0, sizeof ev);
      XNextEvent(d, &ev);
      if (ev.type == 35 && ev.extension) {
        if (!XGetEventData(d, &ev)) continue;
        if (ev.evtype == 6) n_motion++;
        else if (ev.evtype == 4 || ev.evtype == 5) n_btn++;
        XFreeEventData(d, &ev);
      }
    }
    usleep(100000);
  }
  printf("RESULT motion=%d button=%d\n", n_motion, n_btn);
  return 0;
}
