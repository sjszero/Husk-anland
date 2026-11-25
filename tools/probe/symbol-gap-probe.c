/* symbol-gap-probe - 把「path-shim 没钩的符号」逐个放到 guest 里试出后果。
 *
 * 判据是**效果**，不是「符号在不在」。glibc 里很多函数自己的实现最后转调的是
 * 已经被改写的那几个入口（unlink/rmdir/open/...），这种漏网无害；只有绕过
 * 那些入口的才是真缺口。所以这张表必须实测，靠读源码推会错——evidence 213
 * 就是被实测推翻了一半的例子。
 *
 * 编译（guest 内）：gcc -O1 -o /root/symbol-gap-probe symbol-gap-probe.c -ldl -lrt -lpthread
 * 用法：/root/symbol-gap-probe
 *
 * __xstat 家族必须**直接引用**（走 GOT/PLT，会被 loader 改写）才是老二进制的
 * 真实调用方式；用 dlsym 拿到的是 glibc 自己的地址，测不到钩子。
 */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
#include <semaphore.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <sys/types.h>
#include <unistd.h>

extern int __xstat(int, const char *, struct stat *);
extern int __lxstat(int, const char *, struct stat *);
extern int __fxstatat(int, int, const char *, struct stat *, int);

static void rep(const char *what, long rc, int e) {
  if (rc < 0) printf("%-26s FAIL errno=%d (%s)\n", what, e, strerror(e));
  else        printf("%-26s ok   rc=%ld\n", what, rc);
}

int main(void) {
  /* 先把被测文件建出来，否则 __xstat 那一组拿到的 ENOENT 是「文件不存在」，
     不是「路径没翻译」——这个坑实测踩过一次，两条结论完全相反。 */
  if (system("rm -rf /root/gap-tmp; mkdir -p /root/gap-tmp; echo hi > /root/gap-tmp/c") != 0) {
    /* 建不上就照跑，下面的输出自己会说明 */
  }
  int e, fd;
  struct stat st;

  /* 6 个版本化 stat 入口：老二进制（glibc < 2.33 链接）走的就是这些 */
  errno = 0; int r = __xstat(0, "/root/gap-tmp/c", &st); e = errno;
  rep("__xstat", r, e);
  if (r == 0) printf("   size=%lld\n", (long long)st.st_size);
  errno = 0; r = __lxstat(0, "/root/gap-tmp/c", &st); e = errno; rep("__lxstat", r, e);
  errno = 0; r = __fxstatat(0, AT_FDCWD, "/root/gap-tmp/c", &st, 0); e = errno;
  rep("__fxstatat", r, e);

  /* glibc 自己在内部转调已改写入口的那一批 */
  fd = open("/root/gap-tmp/a", O_CREAT | O_WRONLY, 0644);
  if (fd >= 0) close(fd);
  errno = 0; r = remove("/root/gap-tmp/a"); e = errno; rep("remove", r, e);
  FILE *t = fopen("/root/gap-tmp/b", "w");
  errno = 0; FILE *g = t ? freopen("/root/gap-tmp/b2", "a", t) : NULL; e = errno;
  rep("freopen", g ? 0 : -1, e);
  errno = 0; long pc = pathconf("/root/gap-tmp", _PC_NAME_MAX); e = errno;
  rep("pathconf", pc, e);
  fd = open("/root/gap-tmp/c", O_CREAT | O_WRONLY, 0644);
  if (fd >= 0) { char b[100]; memset(b, 'x', sizeof b); if (write(fd, b, 100) != 100) {} close(fd); }
  errno = 0; r = truncate("/root/gap-tmp/c", 0); e = errno; rep("truncate64", r, e);

  /* /dev/shm 的两个：名字是 "/x"，glibc 自己拼成 /dev/shm/x 再 open */
  errno = 0; fd = shm_open("/husk-gap", O_CREAT | O_RDWR, 0600); e = errno;
  rep("shm_open", fd, e);
  if (fd >= 0) close(fd);
  errno = 0; sem_t *sm = sem_open("/husk-gap", O_CREAT, 0600, 1); e = errno;
  rep("sem_open", sm ? 0 : -1, e);

  /* 曾经的真缺口之一：ld.so 自己 open 的 dlmopen */
  errno = 0; void *h = dlmopen(LM_ID_BASE, "/usr/lib/aarch64-linux-gnu/libm.so.6", RTLD_NOW); e = errno;
  rep("dlmopen", h ? 0 : -1, e);

  /* 另一个：execveat。必须**按符号调用**（走 GOT），用 syscall(SYS_execveat)
     是 raw syscall，本来就绕过整个 shim，测不到钩子。成功时进程被替换，
     所以放子进程里跑，用退出码读数。 */
  {
    char *av[] = {"/usr/bin/true", NULL};
    char *ev[] = {NULL};
    pid_t pid = fork();
    if (pid == 0) {
      execveat(AT_FDCWD, "/usr/bin/true", av, ev, 0);
      _exit(97); /* 回到这里说明 exec 失败 */
    }
    int st = 0;
    if (pid > 0) { waitpid(pid, &st, 0); }
    printf("%-26s child exit=%d%s\n", "execveat(/usr/bin/true)",
           WIFEXITED(st) ? WEXITSTATUS(st) : -1,
           (WIFEXITED(st) && WEXITSTATUS(st) == 0) ? "  (exec 成功)" : "  (没跑起来)");
  }

  /* 对照：raw syscall 版本，永远绕过 shim —— 这是设计边界，不是 bug */
  {
    pid_t pid = fork();
    if (pid == 0) {
      syscall(SYS_execveat, AT_FDCWD, "/usr/bin/true",
              (char *[]){"/usr/bin/true", NULL}, (char *[]){NULL}, 0);
      _exit(97);
    }
    int st = 0;
    if (pid > 0) { waitpid(pid, &st, 0); }
    printf("%-26s child exit=%d  (raw syscall，设计边界)\n", "syscall(SYS_execveat)",
           WIFEXITED(st) ? WEXITSTATUS(st) : -1);
  }

  /* 权限本来就不给的那几个：跟钩不钩无关，结论只会是 EPERM */
  errno = 0; r = chroot("/"); e = errno; rep("chroot(/)", r, e);
  errno = 0; r = umount("/root/gap-tmp"); e = errno; rep("umount", r, e);
  errno = 0; r = swapon("/root/gap-tmp/c", 0); e = errno; rep("swapon", r, e);
  errno = 0; r = swapoff("/root/gap-tmp/c"); e = errno; rep("swapoff", r, e);
  return 0;
}
