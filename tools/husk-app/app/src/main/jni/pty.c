// Minimal PTY spawner for the Husk shell.
//
// The guest loader is executed under a freshly allocated pseudo-terminal, so the
// guest gets a real tty: job control, ^C, and full-screen programs all behave.
// All the environment the runtime needs is set here, in the forked child, before
// exec - nothing is inherited from the Android app process.
#include <jni.h>
#include <pty.h>
#include <unistd.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/wait.h>
#include <errno.h>
#include <termios.h>
#include <stdio.h>
#include <fcntl.h>

static pid_t g_pid = -1;
static const char *jstr(JNIEnv *e, jstring s) {
    return s ? (*e)->GetStringUTFChars(e, s, NULL) : NULL;
}

JNIEXPORT jint JNICALL
Java_com_linuxhub_husk_Pty_spawn(JNIEnv *env, jclass k, jstring loader, jstring libpath,
                                 jstring exe, jstring rootfs, jstring ld,
                                 jint rows, jint cols, jstring home, jstring logpath) {
    const char *c_loader = jstr(env, loader);
    const char *c_libpath = jstr(env, libpath);
    const char *c_exe = jstr(env, exe);
    const char *c_root = jstr(env, rootfs);
    const char *c_ld = jstr(env, ld);
    const char *c_home = jstr(env, home);
    const char *c_log = jstr(env, logpath);
    if (!c_loader || !c_exe) return -1;

    struct winsize ws;
    memset(&ws, 0, sizeof ws);
    ws.ws_row = rows > 0 ? rows : 24;
    ws.ws_col = cols > 0 ? cols : 80;

    int master = -1;
    pid_t pid = forkpty(&master, NULL, NULL, &ws);
    if (pid < 0) return -1;
    g_pid = pid;

    if (pid == 0) {
        // child: keep stderr in a file so a failure is diagnosable even when the
        // pty master is not being read yet
        if (c_log) {
            int lg = open(c_log, O_WRONLY | O_CREAT | O_TRUNC, 0600);
            if (lg >= 0) { dup2(lg, 2); if (lg != 2) close(lg); }
        }
        // child: build the runtime environment, then become the guest shell
        setenv("NR_ROOTFS", c_root, 1);
        setenv("NR_LD", c_ld, 1);
        setenv("PATH", "/usr/bin:/bin:/usr/sbin:/sbin", 1);
        setenv("TERM", "xterm-256color", 1);
        setenv("LANG", "C.UTF-8", 1);
        if (c_home) { setenv("HOME", c_home, 1); chdir(c_home); }
        else { setenv("HOME", "/root", 1); chdir("/"); }

        dprintf(2, "husk: exec %s --library-path %s %s\n",
                c_loader, c_libpath ? c_libpath : "(none)", c_exe);
        if (c_libpath && *c_libpath) {
            execl(c_loader, c_loader, "--library-path", c_libpath, c_exe, (char *)NULL);
        } else {
            execl(c_loader, c_loader, c_exe, (char *)NULL);
        }
        dprintf(2, "husk: exec failed: %s\n", strerror(errno));
        _exit(127);
    }
    return master;
}

JNIEXPORT jint JNICALL
Java_com_linuxhub_husk_Pty_resize(JNIEnv *env, jclass k, jint fd, jint rows, jint cols) {
    struct winsize ws;
    memset(&ws, 0, sizeof ws);
    ws.ws_row = rows; ws.ws_col = cols;
    return ioctl(fd, TIOCSWINSZ, &ws);
}

JNIEXPORT jint JNICALL
Java_com_linuxhub_husk_Pty_waitExit(JNIEnv *env, jclass k, jint fd) {
    int st = 0;
    if (g_pid <= 0) return -1;
    if (waitpid(g_pid, &st, 0) < 0) return -1;
    return st;
}
