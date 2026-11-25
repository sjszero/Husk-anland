// grabstatus.c — 看拖拽期间有没有别的客户端（mutter）占着指针抓取。
// XIGrabDevice 返回：0=Success 1=AlreadyGrabbed 2=InvalidTime 3=NotViewable 4=Frozen
#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
typedef struct _XDisplay Display;
typedef unsigned long Window, Time, XID;
typedef struct { int deviceid; int mask_len; unsigned char *mask; } XIEventMask;
extern Display *XOpenDisplay(const char *);
extern Window XDefaultRootWindow(Display *);
extern int XIGrabDevice(Display *, int, Window, Time, XID, int, int, int, XIEventMask *);
extern int XUngrabDevice(Display *, int, Time);
extern int XSync(Display *, int);
int main(int argc, char **argv) {
  int dev = argc > 1 ? atoi(argv[1]) : 2;
  Display *d = XOpenDisplay(NULL);
  if (!d) { printf("no display\n"); return 1; }
  Window root = XDefaultRootWindow(d);
  unsigned char m[4] = {0,0,0,0};
  XIEventMask mask = {dev, 4, m};
  int st = XIGrabDevice(d, dev, root, 0, 0, 1, 1, 0, &mask);
  printf("grab dev=%d status=%d\n", dev, st);
  if (st == 0) XUngrabDevice(d, dev, 0);
  XSync(d, 0);
  return 0;
}
