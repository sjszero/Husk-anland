/* nograb.c — 预载进 gnome-shell：把 mutter 对指针设备的 XIGrabDevice 变成空操作。
 * 目的：这台 X server 的 XI2 主指针抓取不投递 slave(XTEST) 的 motion，
 * 导致 mutter 的指针抓取期收不到 motion，交互式拖窗永远不动。
 * 关键字：只跳过指针设备，键盘设备照常抓（alt+F7 靠它）。 */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef struct _XDisplay Display;
typedef unsigned long Window, Time, XID;

static int (*real_grab)(Display *, int, Window, Time, XID, int, int, int, void *);
static int (*real_ungrab)(Display *, int, Time);

static void logline(const char *fmt, int a, int b) {
  const char *p = getenv("XIGRAB_LOG");
  if (!p) return;
  FILE *f = fopen(p, "a");
  if (!f) return;
  fprintf(f, fmt, a, b);
  fclose(f);
}

int XIGrabDevice(Display *d, int deviceid, Window w, Time t, XID cur, int gm,
                 int pm, int owner, void *mask) {
  if (!real_grab) real_grab = dlsym(RTLD_NEXT, "XIGrabDevice");
  const char *env = getenv("HUSK_NOGRAB_DEV");
  int dev = env ? atoi(env) : 2;
  if (deviceid == dev) {
    logline("XIGrabDevice dev=%d -> SKIP ok=%d\n", deviceid, 1);
    return 0; /* Success，但不真抓 */
  }
  int rc = real_grab ? real_grab(d, deviceid, w, t, cur, gm, pm, owner, mask) : 0;
  logline("XIGrabDevice dev=%d -> passthrough rc=%d\n", deviceid, rc);
  return rc;
}

int XUngrabDevice(Display *d, int deviceid, Time t) {
  if (!real_ungrab) real_ungrab = dlsym(RTLD_NEXT, "XUngrabDevice");
  const char *env = getenv("HUSK_NOGRAB_DEV");
  int dev = env ? atoi(env) : 2;
  if (deviceid == dev) {
    logline("XUngrabDevice dev=%d -> SKIP ok=%d\n", deviceid, 1);
    return 1;
  }
  return real_ungrab ? real_ungrab(d, deviceid, t) : 1;
}
