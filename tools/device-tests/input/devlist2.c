#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
typedef struct _XDisplay Display;
typedef struct { int deviceid; char *name; int use; int attachment; int enabled;
                 int num_classes; void **classes; } XIDeviceInfo;
extern Display *XOpenDisplay(const char *);
extern int XIQueryVersion(Display *, int *, int *);
extern XIDeviceInfo *XIQueryDevice(Display *, int, int *);
extern void XIFreeDeviceInfo(XIDeviceInfo *);
static void dump(Display *d, int dev, const char *label) {
  int n = 0;
  XIDeviceInfo *i = XIQueryDevice(d, dev, &n);
  printf("query(%s=%d) -> n=%d\n", label, dev, n);
  if (!i) return;
  for (int k = 0; k < n; k++)
    printf("   id=%-3d use=%d attach=%-3d enabled=%d classes=%d %s\n",
           i[k].deviceid, i[k].use, i[k].attachment, i[k].enabled,
           i[k].num_classes, i[k].name ? i[k].name : "(null)");
  XIFreeDeviceInfo(i);
}
int main(void) {
  Display *d = XOpenDisplay(NULL);
  if (!d) return 1;
  int maj = 2, min = 0;
  XIQueryVersion(d, &maj, &min);
  printf("XI %d.%d\n", maj, min);
  dump(d, 3, "XIAllDevices");
  dump(d, 1, "XIAllMasterDevices");
  dump(d, 2, "id2");
  dump(d, 1, "id1");
  dump(d, 4, "id4");
  return 0;
}
