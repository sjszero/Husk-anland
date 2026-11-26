/* coverage-probe.c - 逐个验证 path-shim 未经钩子的"漏网"假设。
 * 每个用例都写一个 guest 路径，再读回宿主侧结果，用"文件到底落在哪"判定翻译是否发生。
 * 编译：gcc -O1 -o /root/p /root/coverage-probe.c
 */
#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <errno.h>
#include <unistd.h>
#include <fcntl.h>
#include <dirent.h>
#include <linux/netlink.h>
#include <sys/inotify.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <utime.h>

static int exists(const char *p) { struct stat st; return stat(p, &st) == 0; }

static int open_fd_count(void)
{
    DIR *d = opendir("/proc/self/fd");
    if (!d) return -1;
    int n = 0;
    struct dirent *e;
    while ((e = readdir(d))) if (e->d_name[0] != '.') n++;
    closedir(d);
    return n;
}

int main(void)
{
    char line[256];

    /* 0 对照组：直接 unlink（已钩子） */
    { FILE *f = fopen("/root/.p-unlink", "w"); if (f) { fputs("x", f); fclose(f); }
      errno = 0; int r = unlink("/root/.p-unlink");
      printf("CTL_UNLINK rc=%d errno=%d exists_after=%d\n", r, errno, exists("/root/.p-unlink")); }

    /* 1 remove()：glibc 内部调 __unlink/__rmdir，看 body patch 是否覆盖 */
    { FILE *f = fopen("/root/.p-remove", "w"); if (f) { fputs("x", f); fclose(f); }
      errno = 0; int r = remove("/root/.p-remove");
      printf("REMOVE rc=%d errno=%d exists_after=%d\n", r, errno, exists("/root/.p-remove")); }

    /* 2 inotify_add_watch：内核直接收路径，没有 body 可改 */
    { int ifd = inotify_init1(IN_NONBLOCK);
      errno = 0; int wd = inotify_add_watch(ifd, "/root", IN_CREATE | IN_MODIFY);
      printf("INOTIFY fd=%d wd=%d errno=%d %s\n", ifd, wd, errno, strerror(errno));
      if (wd >= 0) {
          FILE *f = fopen("/root/.p-notify", "w"); if (f) { fclose(f); }
          usleep(200000);
          char buf[4096]; ssize_t n = read(ifd, buf, sizeof buf);
          printf("INOTIFY_EVENT n=%zd errno=%d\n", n, errno);
      } }

    /* 3 freopen()：git/nohup 这类把 stdout 重定向到文件的程序会走 */
    { errno = 0;
      FILE *g = freopen("/root/.p-freopen", "w", stderr);
      int ok = g != NULL; int e = errno;
      fprintf(stderr, "FREOPEN_MARK\n"); fflush(stderr);
      printf("FREOPEN ok=%d errno=%d exists=%d\n", ok, e, exists("/root/.p-freopen")); }

    /* 4 pathconf / scandirat：路径参数直接进内核 */
    { errno = 0; long v = pathconf("/root", _PC_NAME_MAX);
      printf("PATHCONF v=%ld errno=%d\n", v, errno); }
    { struct dirent **nl = NULL; errno = 0;
      int n = scandirat(AT_FDCWD, "/root", &nl, NULL, alphasort);
      printf("SCANDIRAT n=%d errno=%d\n", n, errno); if (n > 0) free(nl); }

    /* 5 老 API 别名：truncate64 / utime / umount （truncate/utimes/umount2 已钩子） */
    { FILE *f = fopen("/root/.p-trunc", "w"); if (f) { fputs("0123456789", f); fclose(f); }
      errno = 0; int r = truncate64("/root/.p-trunc", 3);
      struct stat st; long sz = (stat("/root/.p-trunc", &st) == 0) ? (long)st.st_size : -1;
      printf("TRUNCATE64 rc=%d errno=%d size=%ld\n", r, errno, sz); }
    { struct utimbuf ub; ub.actime = 1000000000; ub.modtime = 1000000000;
      errno = 0; int r = utime("/root/.p-trunc", &ub);
      printf("UTIME rc=%d errno=%d\n", r, errno); }

    /* 6 硬链接复制后备：stdio 追加后再 fclose，别名必须看到新内容 */
    { const char *dir = "/root/.p-hardlink";
      const char *a = "/root/.p-hardlink/a";
      const char *b = "/root/.p-hardlink/b";
      char got[32] = {0};
      mkdir(dir, 0700); unlink(a); unlink(b);
      FILE *f = fopen(a, "w"); if (f) { fputs("AAA", f); fclose(f); }
      errno = 0; int lr = link(a, b); int le = errno;
      f = fopen(a, "a"); if (f) { fputs("BBB", f); fclose(f); }
      f = fopen(b, "r"); if (f) { fgets(got, sizeof got, f); fclose(f); }
      printf("HARDLINK_STDIO link_rc=%d link_errno=%d content=%s\n", lr, le, got);
      unlink(a); unlink(b); rmdir(dir); }

    /* 7 raw syscall：已知绕过，作对照 */
    { errno = 0;
      int fd = (int)syscall(SYS_openat, AT_FDCWD, "/root/.p-rawopen", O_WRONLY | O_CREAT, 0644);
      printf("RAW_OPENAT fd=%d errno=%d\n", fd, errno); if (fd >= 0) close(fd); }

    /* 8 close_range：Android seccomp 拦 436，必须由 shim 用用户态枚举兜底。
       同时验证 netlink 私有的 peer 端会随 client 一起注销。 */
    { int before = open_fd_count();
      int fd = socket(AF_NETLINK, SOCK_RAW, NETLINK_ROUTE);
      int opened = open_fd_count();
      errno = 0; int r = close_range((unsigned)fd, (unsigned)fd, 0);
      int after = open_fd_count();
      printf("CLOSE_RANGE_NETLINK before=%d opened=%d rc=%d errno=%d after=%d\n",
             before, opened, r, errno, after); }

    /* close_range 的普通关闭与 CLOEXEC 语义 */
    { int pfd[2]; int pr = pipe(pfd); errno = 0;
      int r = pr == 0 ? close_range((unsigned)pfd[0], (unsigned)pfd[1], 0) : -1;
      int e = errno;
      int a = pr == 0 ? fcntl(pfd[0], F_GETFD) : -1;
      int b = pr == 0 ? fcntl(pfd[1], F_GETFD) : -1;
      printf("CLOSE_RANGE_PIPE pipe=%d rc=%d errno=%d fd0=%d fd1=%d\n", pr, r, e, a, b); }
    { int pfd[2]; int pr = pipe(pfd); errno = 0;
      int r = pr == 0 ? close_range((unsigned)pfd[0], (unsigned)pfd[1], 4) : -1;
      int e = errno;
      int a = pr == 0 ? fcntl(pfd[0], F_GETFD) : -1;
      int b = pr == 0 ? fcntl(pfd[1], F_GETFD) : -1;
      printf("CLOSE_RANGE_CLOEXEC pipe=%d rc=%d errno=%d fd0=%d fd1=%d\n", pr, r, e, a, b);
      if (pr == 0) { close(pfd[0]); close(pfd[1]); } }
    { errno = 0; int r = close_range(3, 3, 2);
      printf("CLOSE_RANGE_UNSHARE rc=%d errno=%d\n", r, errno); }
    { const char *p = "/root/.p-close-range-root"; unlink(p);
      pid_t pid = fork();
      if (pid == 0) {
          int r = close_range(3, ~0U, 0);
          int fd = open(p, O_WRONLY | O_CREAT, 0600);
          _exit(r == 0 && fd >= 0 ? 0 : 3);
      }
      int status = -1; waitpid(pid, &status, 0);
      printf("CLOSE_RANGE_ALL child=%d exit=%d exists=%d\n", pid,
             WIFEXITED(status) ? WEXITSTATUS(status) : -1, exists(p));
      unlink(p); }

    (void)line;
    return 0;
}
