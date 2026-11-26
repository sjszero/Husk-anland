/*
 * getgroups()/setgroups() table race.
 *
 * The old implementation returned a count from one lock acquisition, released
 * the lock, validated the caller's buffer against that count, then re-read the
 * shared count while copying. A concurrent setgroups() could grow the table in
 * between and make getgroups() write past a one-entry buffer.
 *
 * A high-rate SIGALRM handler also calls getgroups() only. If the signal lands
 * while that thread owns the process-local group-table lock, a boolean spinlock
 * deadlocks against itself; the fixed owner-TID lock returns a primary-group
 * snapshot instead.
 */
#define _GNU_SOURCE
#include <errno.h>
#include <pthread.h>
#include <signal.h>
#include <stdatomic.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <sys/time.h>
#include <time.h>
#include <unistd.h>

enum { READERS = 2, WRITERS = 1, ITERS = 20000, MAX_GROUPS = 32 };
static pthread_barrier_t barrier;
static atomic_int failures;
static volatile sig_atomic_t signal_calls;

static void on_alarm(int sig) {
    (void)sig;
    unsigned list[MAX_GROUPS];
    int n = getgroups(0, NULL);
    if (n > 0 && n <= MAX_GROUPS) {
        (void)getgroups(n, list); /* may race by design; errno is ignored */
    }
    signal_calls++;
}

struct guarded_slot {
    volatile unsigned before;
    unsigned values[1];
    volatile unsigned after;
};

static void fail(const char *what, int rc, int err) {
    atomic_fetch_add(&failures, 1);
    fprintf(stderr, "GROUPS_RACE_FAIL what=%s rc=%d errno=%d\n", what, rc, err);
}

static void *reader(void *arg) {
    (void)arg;
    pthread_barrier_wait(&barrier);
    const unsigned canary = 0x51a7c0deU;
    for (int i = 0; i < ITERS; i++) {
        struct guarded_slot s;
        s.before = canary;
        s.values[0] = 0;
        s.after = canary;
        __asm__ __volatile__("" ::: "memory");
        errno = 0;
        int rc = getgroups(1, s.values);
        int err = errno;
        __asm__ __volatile__("" ::: "memory");
        if (s.before != canary || s.after != canary) {
            fail("buffer-overrun", rc, err);
            return NULL;
        }
        if (rc < 0 && err != EINVAL) {
            fail("unexpected-errno", rc, err);
            return NULL;
        }
        if (rc > 1) {
            fail("impossible-count", rc, err);
            return NULL;
        }
    }
    return NULL;
}

static void *writer(void *arg) {
    long id = (long)arg;
    pthread_barrier_wait(&barrier);
    unsigned one[1] = {1500};
    unsigned many[MAX_GROUPS];
    for (int i = 0; i < MAX_GROUPS; i++) many[i] = (unsigned)(2000 + i);
    for (int i = 0; i < ITERS; i++) {
        const unsigned *p = ((i + id) & 1) ? one : many;
        size_t n = p == one ? 1 : MAX_GROUPS;
        if (setgroups(n, p) != 0) {
            fail("setgroups", -1, errno);
            return NULL;
        }
    }
    return NULL;
}

static void *watchdog(void *arg) {
    (void)arg;
    struct timespec left = {20, 0};
    while (nanosleep(&left, &left) != 0 && errno == EINTR) {}
    static const char msg[] = "GROUPS_RACE_FAIL watchdog-timeout\n";
    (void)write(2, msg, sizeof(msg) - 1);
    _exit(124);
}

int main(void) {
    pthread_t readers[READERS], writers[WRITERS], dog;
    struct sigaction sa = {0};
    struct itimerval timer = {0};

    sa.sa_handler = on_alarm;
    sigemptyset(&sa.sa_mask);
    if (sigaction(SIGALRM, &sa, NULL) != 0) {
        perror("sigaction");
        return 2;
    }
    timer.it_value.tv_usec = 1000;
    timer.it_interval.tv_usec = 1000;
    if (setitimer(ITIMER_REAL, &timer, NULL) != 0) {
        perror("setitimer");
        return 2;
    }
    if (pthread_create(&dog, NULL, watchdog, NULL) != 0) return 2;
    pthread_barrier_init(&barrier, NULL, READERS + WRITERS);
    for (long i = 0; i < READERS; i++) pthread_create(&readers[i], NULL, reader, NULL);
    for (long i = 0; i < WRITERS; i++) pthread_create(&writers[i], NULL, writer, (void *)i);
    for (int i = 0; i < READERS; i++) pthread_join(readers[i], NULL);
    for (int i = 0; i < WRITERS; i++) pthread_join(writers[i], NULL);

    timer.it_value.tv_sec = 0;
    timer.it_value.tv_usec = 0;
    timer.it_interval.tv_sec = 0;
    timer.it_interval.tv_usec = 0;
    setitimer(ITIMER_REAL, &timer, NULL);
    unsigned one = 1500;
    (void)setgroups(1, &one);
    int f = atomic_load(&failures);
    printf("GROUPS_RACE failures=%d signals=%ld expected=0\n", f, (long)signal_calls);
    return f == 0 && signal_calls != 0 ? 0 : 1;
}
