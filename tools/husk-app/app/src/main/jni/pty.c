// PTY spawner for the Husk shell.
//
// Hard rule: everything that touches the JVM (string copies, array access,
// allocation) happens in the PARENT. After forkpty() the child only calls
// async-signal-safe things - dup2, chdir, execve - and nothing else. Calling into
// ART from a forked child is undefined behaviour and was the reason the guest
// shell died before printing a byte.
#include <jni.h>
#include <pty.h>
#include <unistd.h>
#include <stdlib.h>
#include <string.h>
#include <stdio.h>
#include <fcntl.h>
#include <errno.h>
#include <sys/ioctl.h>
#include <sys/wait.h>
#include <termios.h>

static pid_t g_pid = -1;

static char *dup_jstr(JNIEnv *e, jstring s) {
    if (!s) return NULL;
    const char *c = (*e)->GetStringUTFChars(e, s, NULL);
    if (!c) return NULL;
    char *r = strdup(c);
    (*e)->ReleaseStringUTFChars(e, s, c);
    return r;
}

JNIEXPORT jint JNICALL
Java_com_linuxhub_husk_Pty_spawn(JNIEnv *env, jclass k, jstring jloader, jstring jlibpath,
                                 jstring jexe, jstring jrootfs, jstring jld,
                                 jint rows, jint cols, jstring jhome, jstring jlog,
                                 jstring jsigsyslog, jobjectArray extra) {
    /* ---- parent: materialise every string we will need in the child ---- */
    char *loader  = dup_jstr(env, jloader);
    char *libpath = dup_jstr(env, jlibpath);
    char *exe     = dup_jstr(env, jexe);
    char *rootfs  = dup_jstr(env, jrootfs);
    char *ld      = dup_jstr(env, jld);
    char *home    = dup_jstr(env, jhome);
    char *log     = dup_jstr(env, jlog);
    char *slog    = dup_jstr(env, jsigsyslog);
    if (!loader || !exe) return -1;

    static char *argv[64];
    static char *envp[24];
    int an = 0, en = 0;
    argv[an++] = loader;
    if (libpath && *libpath) { argv[an++] = "--library-path"; argv[an++] = libpath; }
    argv[an++] = exe;
    if (extra) {
        jsize n = (*env)->GetArrayLength(env, extra);
        for (jsize i = 0; i < n && an < 60; i++) {
            jstring s = (jstring)(*env)->GetObjectArrayElement(env, extra, i);
            char *c = dup_jstr(env, s);
            (*env)->DeleteLocalRef(env, s);
            if (c) argv[an++] = c;
        }
    }
    argv[an] = NULL;

    static char env_root[512], env_ld[512], env_slog[512], env_home[512];
    if (rootfs) { snprintf(env_root, sizeof env_root, "NR_ROOTFS=%s", rootfs); envp[en++] = env_root; }
    if (ld)     { snprintf(env_ld,   sizeof env_ld,   "NR_LD=%s", ld);         envp[en++] = env_ld; }
    if (slog)   { snprintf(env_slog, sizeof env_slog, "NR_SIGSYS_LOG=%s", slog); envp[en++] = env_slog; }
    if (home)   { snprintf(env_home, sizeof env_home, "HOME=%s", home); }
    envp[en++] = "TERM=xterm-256color";
    envp[en++] = "LANG=C.UTF-8";
    envp[en++] = strdup(env_home[0] ? env_home : "HOME=/root");
    envp[en++] = "USER=root";
    envp[en++] = "SHELL=/usr/bin/bash";
    envp[en++] = "PATH=/usr/bin:/bin:/usr/sbin:/sbin";
    /* glibc 2.35+ registers rseq at startup; this device's seccomp policy turns
       that into SIGSYS before a single byte is printed. The launcher has to ask
       glibc not to - there is no other hook early enough. */
    envp[en++] = "GLIBC_TUNABLES=glibc.pthread.rseq=0";
    envp[en] = NULL;

    struct winsize ws;
    memset(&ws, 0, sizeof ws);
    ws.ws_row = rows > 0 ? rows : 24;
    ws.ws_col = cols > 0 ? cols : 80;

    /* ---- fork ---- */
    int master = -1;
    pid_t pid = forkpty(&master, NULL, NULL, &ws);
    if (pid < 0) return -1;
    g_pid = pid;

    if (pid == 0) {
        /* child: async-signal-safe only */
        if (log) {
            int lg = open(log, O_WRONLY | O_CREAT | O_TRUNC, 0600);
            if (lg >= 0) { dup2(lg, 2); if (lg != 2) close(lg); }
        }
        if (home) chdir(home);
        execve(loader, argv, envp);
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
    for (int i = 0; i < 30; i++) {
        pid_t r = waitpid(g_pid, &st, WNOHANG);
        if (r == g_pid) return st;
        if (r < 0) return -1;
        usleep(100000);
    }
    return -2;
}
