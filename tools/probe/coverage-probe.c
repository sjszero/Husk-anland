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

static int sock_family(int fd)
{
    struct sockaddr_storage ss;
    socklen_t len = sizeof ss;
    if (getsockname(fd, (struct sockaddr *)&ss, &len) != 0) return -1;
    return ((struct sockaddr *)&ss)->sa_family;
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

    /* 6.0 O_CREAT 必须按 caller 给的 mode 落地，不能为了后续重开偷加 0600 */
    { const char *p = "/root/.p-create-mode"; struct stat st;
      unlink(p); mode_t old_umask = umask(0);
      errno = 0; int fd = open(p, O_WRONLY | O_CREAT | O_EXCL, 0440);
      int e = errno;
      int mode = stat(p, &st) == 0 ? (int)(st.st_mode & 0777) : -1;
      int wr = fd >= 0 ? (int)write(fd, "C", 1) : -1;
      if (fd >= 0) close(fd);
      umask(old_umask);
      printf("CREATE_MODE open_fd=%d errno=%d mode=%o write=%d expected=440\n",
             fd, e, mode, wr);
      unlink(p); }

    /* 6.1 假 root 放宽写权限后，原 inode 模式必须立即恢复 */
    { const char *p = "/root/.p-owner-restore"; struct stat st;
      unlink(p);
      int c = open(p, O_WRONLY | O_CREAT | O_TRUNC, 0600);
      if (c >= 0) { write(c, "A", 1); close(c); }
      chmod(p, 0440);
      errno = 0; int fd = open(p, O_WRONLY);
      int e = errno;
      int mode_open = stat(p, &st) == 0 ? (int)(st.st_mode & 0777) : -1;
      int wr = fd >= 0 ? (int)write(fd, "B", 1) : -1;
      if (fd >= 0) close(fd);
      int mode_final = stat(p, &st) == 0 ? (int)(st.st_mode & 0777) : -1;
      printf("OWNER_RESTORE open_fd=%d errno=%d mode_open=%o write=%d mode_final=%o\n",
             fd, e, mode_open, wr, mode_final);
      unlink(p); }

    /* 6.2 假 root 的 access：0000 可读写但不可执行；任何执行位存在才可执行 */
    { const char *p = "/root/.p-access-root";
      unlink(p);
      int fd = open(p, O_WRONLY | O_CREAT | O_TRUNC, 0600);
      if (fd >= 0) close(fd);
      chmod(p, 0000);
      errno = 0; int rd = access(p, R_OK); int re = errno;
      errno = 0; int wr = access(p, W_OK); int we = errno;
      errno = 0; int x0 = access(p, X_OK); int xe = errno;
      chmod(p, 0555);
      errno = 0; int x1 = access(p, X_OK); int x1e = errno;
      printf("ACCESS_ROOT read=%d/%d write=%d/%d exec0=%d/%d exec5=%d/%d\n",
             rd, re, wr, we, x0, xe, x1, x1e);
      unlink(p); }

    /* 6.3 非 root：最终模式与父目录路径权限都要按 guest 身份判断。
       /tmp 本身可搜索；deny 目录只拒绝父目录 X，own 目录验证虚构 owner 正向。 */
    { const char *dd = "/tmp/.p-access-deny";
      const char *df = "/tmp/.p-access-deny/file";
      const char *od = "/tmp/.p-access-own";
      const char *of = "/tmp/.p-access-own/owned";
      unlink(df); unlink(of); rmdir(dd); rmdir(od);
      mkdir(dd, 0700); chmod(dd, 0700);
      mkdir(od, 0755); chmod(od, 0755);
      int fd = open(df, O_WRONLY | O_CREAT | O_TRUNC, 0600);
      if (fd >= 0) close(fd);
      chmod(df, 0644);
      fd = open(of, O_WRONLY | O_CREAT | O_TRUNC, 0600);
      if (fd >= 0) { fchown(fd, 1000, 1000); close(fd); }
      chmod(of, 0600);
      setgid(1000); setuid(1000);
      errno = 0; int parent = access(df, R_OK); int pe = errno;
      errno = 0; int ownread = access(of, R_OK); int oe = errno;
      setuid(0); setgid(0);
      printf("ACCESS_USER parent=%d/%d owned=%d/%d\n", parent, pe, ownread, oe);
      unlink(df); unlink(of); rmdir(dd); rmdir(od); }

    /* 7 raw syscall：已知绕过，作对照 */
    { errno = 0;
      int fd = (int)syscall(SYS_openat, AT_FDCWD, "/root/.p-rawopen", O_WRONLY | O_CREAT, 0644);
      printf("RAW_OPENAT fd=%d errno=%d\n", fd, errno); if (fd >= 0) close(fd); }

    /* 8 close_range：Android seccomp 拦 436，必须由 shim 用用户态枚举兜底。
       同时验证 netlink 私有的 peer 端会随 client 一起注销。 */
    { int before = open_fd_count();
      int fd = socket(AF_NETLINK, SOCK_RAW, NETLINK_ROUTE);
      int dupfd = fd >= 0 ? dup(fd) : -1;
      int dup2fd = fd >= 0 ? dup2(fd, 63) : -1;
      int fcntlfd = fd >= 0 ? fcntl(fd, F_DUPFD_CLOEXEC, 64) : -1;
      int f0 = sock_family(fd);
      int f1 = sock_family(dupfd);
      int f2 = sock_family(dup2fd);
      int f3 = sock_family(fcntlfd);
      if (fd >= 0) close(fd);
      int f4 = sock_family(dupfd);
      int f5 = sock_family(dup2fd);
      int f6 = sock_family(fcntlfd);
      if (dupfd >= 0) close(dupfd);
      int f7 = sock_family(dup2fd);
      int f8 = sock_family(fcntlfd);
      if (dup2fd >= 0) close(dup2fd);
      int f9 = sock_family(fcntlfd);
      if (fcntlfd >= 0) close(fcntlfd);
      int after = open_fd_count();
      printf("NETLINK_DUP before=%d opened=%d families=%d,%d,%d,%d dup=%d,%d,%d final=%d,%d,%d after=%d\n",
             before, fcntlfd >= 0 ? before + 5 : -1,
             f0, f1, f2, f3, f4, f5, f6, f7, f8, f9, after); }

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
