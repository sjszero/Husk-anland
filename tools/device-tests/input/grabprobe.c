// grabprobe.c — LD_PRELOAD 探针：记录 gnome-shell 里指针/键盘抓取的调用与返回值。
// 目的：回答"交互式拖窗为什么不动"——是没发起抓取，还是发起了但服务器拒绝。
#define _GNU_SOURCE
#include <stdio.h>
#include <stdarg.h>
#include <stdlib.h>
#include <unistd.h>
#include <dlfcn.h>

typedef struct _XDisplay Display;
typedef unsigned long XID;
typedef XID Window;
typedef XID Cursor;
typedef unsigned long Time;
typedef struct { int deviceid; int mask_len; unsigned char *mask; } XIEventMask;

static FILE *L;
static FILE *lg(void){ if(!L){const char*p=getenv("XIGRAB_LOG");L=fopen(p?p:"/tmp/grabprobe.log","a");if(L)setvbuf(L,NULL,_IOLBF,0);} return L; }
static void lo(const char*fmt,...){FILE*f=lg();if(!f)return;va_list ap;va_start(ap,fmt);vfprintf(f,fmt,ap);va_end(ap);}

__attribute__((constructor)) static void hello(void){ lo("PID=%d PRELOAD grabprobe ok\n",(int)getpid()); }

int XGrabPointer(Display*d,Window w,int own,unsigned int em,int pm,int gm,Window conf,Cursor c,Time t){
  static int(*r)(Display*,Window,int,unsigned int,int,int,Window,Cursor,Time);
  if(!r) r=dlsym(RTLD_NEXT,"XGrabPointer");
  int rc=r? r(d,w,own,em,pm,gm,conf,c,t) : -999;
  lo("PID=%d XGrabPointer ret=%d win=0x%lx mask=0x%x\n",(int)getpid(),rc,(unsigned long)w,em);
  return rc;
}
int XUngrabPointer(Display*d,Time t){
  static int(*r)(Display*,Time);
  if(!r) r=dlsym(RTLD_NEXT,"XUngrabPointer");
  lo("PID=%d XUngrabPointer\n",(int)getpid());
  return r? r(d,t) : 0;
}
int XGrabKeyboard(Display*d,Window w,int own,int pm,int gm,Time t){
  static int(*r)(Display*,Window,int,int,int,Time);
  if(!r) r=dlsym(RTLD_NEXT,"XGrabKeyboard");
  int rc=r? r(d,w,own,pm,gm,t) : -999;
  lo("PID=%d XGrabKeyboard ret=%d win=0x%lx\n",(int)getpid(),rc,(unsigned long)w);
  return rc;
}
int XIGrabDevice(Display*d,int dev,Window w,Time t,Cursor c,int gm,int pm,int own,XIEventMask*m){
  typedef int(*fn)(Display*,int,Window,Time,Cursor,int,int,int,XIEventMask*);
  static fn r;
  if(!r) r=(fn)dlsym(RTLD_NEXT,"XIGrabDevice");
  lo("PID=%d XIGrabDevice dev=%d win=0x%lx own=%d mask_len=%d\n",(int)getpid(),dev,(unsigned long)w,own,m?m->mask_len:-1);
  int rc=r? r(d,dev,w,t,c,gm,pm,own,m) : -999;
  lo("PID=%d XIGrabDevice ret=%d dev=%d\n",(int)getpid(),rc,dev);
  return rc;
}

/* X 错误：把 mutter 自己的 handler 包一层，先记一条，再交回去。
   XI2 的抓取失败是异步的，返回值看不出问题，只能靠这里。 */
typedef struct {
  int type; Display *display; unsigned long resourceid, serial;
  unsigned char error_code, request_code, minor_code;
} XErrEvent;
typedef int (*XErrHandler)(Display*, XErrEvent*);
static XErrHandler real_handler;
static XErrHandler (*r_sethandler)(XErrHandler);

static int log_handler(Display *d, XErrEvent *e){
  if (e) lo("PID=%d XERROR code=%d req=%d minor=%d serial=%lu res=0x%lx\n",
            (int)getpid(), (int)e->error_code, (int)e->request_code,
            (int)e->minor_code, e->serial, (unsigned long)e->resourceid);
  if (real_handler) return real_handler(d, e);
  return 0;
}
XErrHandler XSetErrorHandler(XErrHandler h){
  if (!r_sethandler) r_sethandler = (XErrHandler(*)(XErrHandler))dlsym(RTLD_NEXT, "XSetErrorHandler");
  if (!r_sethandler) return 0;
  if (!real_handler) {
    real_handler = r_sethandler(log_handler);
    return real_handler;      /* 返回 mutter 装的那个旧 handler */
  }
  return r_sethandler(h);
}
