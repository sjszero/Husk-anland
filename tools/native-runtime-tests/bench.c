/* Micro-benchmarks for the native runtime, built and run *inside* the guest.
 *
 * Every measurement runs in this one process, so the shell is out of the loop
 * and the numbers are syscall+resolver cost only.
 *
 *   bench stat-guest    stat("/etc/os-release")              resolver + syscall
 *   bench stat-real     stat("$NR_ROOTFS/etc/os-release")    passthrough (no resolver)
 *   bench stat-deep     stat("/usr/lib/.../libc.so.6")       4 components, no symlinks
 *   bench open-guest    open+close("/etc/os-release")
 *   bench open-real     open+close("$NR_ROOTFS/etc/os-release")
 *   bench syscall       N x getpid(): the per-syscall floor in this process. Any
 *                       trap-and-forward design (ptrace, seccomp notify) pays this
 *                       plus a round trip; here it must be a bare kernel syscall.
 *   bench exec          fork+execve+waitpid("/usr/bin/true"), 100 children
 *
 * Note: bench uses execve(), not execl(). glibc's execl() calls its own internal
 * execve, which the shim cannot see (see the note in path-shim/src/lib.rs), so an
 * execl() here would measure the unshimmed libc path and fail with ENOENT.
 *
 * NR_ROOTFS is what the launcher exports; a path under it is already real and
 * is passed straight to the kernel (under_root() -> passthrough()).
 */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/wait.h>

extern char **environ;
#include <time.h>
#include <unistd.h>

static double now_ns(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (double)ts.tv_sec * 1e9 + (double)ts.tv_nsec;
}

int main(int argc, char **argv) {
    if (argc < 2) { fprintf(stderr, "usage: bench <mode>\n"); return 2; }
    const char *mode = argv[1];
    const char *root = getenv("NR_ROOTFS");
    if (!root) { fprintf(stderr, "NR_ROOTFS unset\n"); return 2; }
    char real[4096];
    long n = 20000, i, ok = 0;
    double t0, dt;

    if (!strcmp(mode, "exec")) {
        n = 100;
        t0 = now_ns();
        for (i = 0; i < n; i++) {
            pid_t p = fork();
            if (p == 0) {
                char *const av[] = {"true", NULL};
                execve("/usr/bin/true", av, environ);
                _exit(127);
            }
            if (p > 0) { int st; if (waitpid(p, &st, 0) == p && WIFEXITED(st) && !WEXITSTATUS(st)) ok++; }
        }
        dt = now_ns() - t0;
        printf("%-28s %8.0f ns/op  (%ld/%ld ok)\n", "fork+execve+wait", dt / n, ok, n);
        return ok == n ? 0 : 1;
    }

    if (!strcmp(mode, "syscall")) {
        volatile pid_t sink = 0;
        t0 = now_ns();
        for (i = 0; i < 200000; i++) sink = getpid();
        dt = now_ns() - t0;
        printf("%-28s %8.1f ns/op  (%ld calls, sink=%d)\n", "getpid", dt / 200000, 200000L, (int)sink);
        return 0;
    }

    if (!strncmp(mode, "stat", 4)) {
        const char *path;
        if (!strcmp(mode, "stat-real")) { snprintf(real, sizeof real, "%s/etc/os-release", root); path = real; }
        else if (!strcmp(mode, "stat-deep")) path = "/usr/lib/aarch64-linux-gnu/libc.so.6";
        else path = "/etc/os-release";
        struct stat st;
        t0 = now_ns();
        for (i = 0; i < n; i++) if (stat(path, &st) == 0) ok++;
        dt = now_ns() - t0;
        printf("%-28s %8.1f ns/op  (%ld/%ld ok)\n", mode, dt / n, ok, n);
        return ok == n ? 0 : 1;
    }

    if (!strncmp(mode, "open", 4)) {
        const char *path;
        if (!strcmp(mode, "open-real")) { snprintf(real, sizeof real, "%s/etc/os-release", root); path = real; }
        else path = "/etc/os-release";
        t0 = now_ns();
        for (i = 0; i < n; i++) { int fd = open(path, O_RDONLY); if (fd >= 0) { close(fd); ok++; } }
        dt = now_ns() - t0;
        printf("%-28s %8.1f ns/op  (%ld/%ld ok)\n", mode, dt / n, ok, n);
        return ok == n ? 0 : 1;
    }

    fprintf(stderr, "unknown mode %s\n", mode);
    return 2;
}
