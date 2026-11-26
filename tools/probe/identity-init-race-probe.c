/*
 * Fresh-process identity initialization stress.
 *
 * Every invocation starts with path-shim globals at their initial values. One
 * thread calls setgid()/setuid() while the rest race the first getuid/getgid
 * calls. The final identity must still be the one the setter published; a
 * stale startup seed must not overwrite it. A high-rate SIGALRM handler also
 * calls getuid()/getgid(), covering same-thread re-entry in the identity
 * publication path.
 */
#define _GNU_SOURCE
#include <pthread.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/time.h>
#include <unistd.h>

static pthread_barrier_t barrier;
static volatile unsigned final_uid, final_gid;
static volatile sig_atomic_t signal_calls;

static void on_alarm(int sig) {
    (void)sig;
    volatile unsigned x = getuid();
    x ^= getgid();
    (void)x;
    signal_calls++;
}

static void *reader(void *arg) {
    (void)arg;
    pthread_barrier_wait(&barrier);
    volatile unsigned x = 0;
    for (int i = 0; i < 20000; i++) {
        x ^= getuid();
        x ^= getgid();
    }
    (void)x;
    return NULL;
}

static void *writer(void *arg) {
    (void)arg;
    pthread_barrier_wait(&barrier);
    if (setgid(1000) != 0 || setuid(1000) != 0) {
        perror("setid");
        _exit(2);
    }
    return NULL;
}

int main(void) {
    enum { READERS = 15 };
    pthread_t r[READERS], w;
    struct sigaction sa = {0};
    struct itimerval timer = {0};
    sa.sa_handler = on_alarm;
    sigemptyset(&sa.sa_mask);
    if (sigaction(SIGALRM, &sa, NULL) != 0) {
        perror("sigaction");
        return 2;
    }
    timer.it_value.tv_usec = 100;
    timer.it_interval.tv_usec = 100;
    if (setitimer(ITIMER_REAL, &timer, NULL) != 0) {
        perror("setitimer");
        return 2;
    }
    pthread_barrier_init(&barrier, NULL, READERS + 1);
    for (int i = 0; i < READERS; i++) pthread_create(&r[i], NULL, reader, NULL);
    pthread_create(&w, NULL, writer, NULL);
    for (int i = 0; i < READERS; i++) pthread_join(r[i], NULL);
    pthread_join(w, NULL);
    timer.it_value.tv_sec = 0;
    timer.it_value.tv_usec = 0;
    timer.it_interval.tv_sec = 0;
    timer.it_interval.tv_usec = 0;
    setitimer(ITIMER_REAL, &timer, NULL);
    final_uid = getuid();
    final_gid = getgid();
    printf("IDENTITY_RACE final=%u:%u expected=1000:1000 signals=%ld\n",
           final_uid, final_gid, (long)signal_calls);
    return final_uid == 1000 && final_gid == 1000 && signal_calls != 0 ? 0 : 1;
}
