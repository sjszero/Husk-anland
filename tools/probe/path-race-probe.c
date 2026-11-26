/* Concurrent final-symlink replacement probe.
 *
 * A shim that resolves a symlink and then lets the kernel look the final name
 * up again can be raced into following an absolute host path. This probe
 * alternates the final name between a guest-relative target and an absolute
 * path outside the guest root while workers chmod, timestamp, stat and
 * truncate it. The caller verifies the host sentinel afterwards.
 */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <pthread.h>
#include <stdatomic.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <time.h>
#include <unistd.h>

#define TOGGLE_ROUNDS 60000
#define WORKER_ROUNDS 30000
#define WORKERS 4

static const char *base = "/tmp/husk-path-race";
static char race_link[256];
static char guest_target[256];
static const char *host_target;
static atomic_int stop_workers;
static atomic_ulong toggles;
static atomic_ulong operations;
static atomic_ulong successes;

static void *toggle_thread(void *unused)
{
    (void)unused;
    for (int i = 0; i < TOGGLE_ROUNDS && !atomic_load(&stop_workers); i++) {
        char tmp[256];
        snprintf(tmp, sizeof(tmp), "%s/.next-%d", base, i & 1);
        unlink(tmp);
        const char *target = (i & 1) ? host_target : "guest-victim";
        if (symlink(target, tmp) == 0 && rename(tmp, race_link) == 0)
            atomic_fetch_add(&toggles, 1);
    }
    return NULL;
}

static void *worker_thread(void *arg)
{
    unsigned seed = (unsigned)(uintptr_t)arg + 1;
    for (int i = 0; i < WORKER_ROUNDS; i++) {
        struct timespec ts[2] = {
            { 1600000000 + (time_t)(i & 31), (long)(i % 1000) * 1000000L },
            { 1600000000 + (time_t)((i + 1) & 31), (long)((i + 1) % 1000) * 1000000L },
        };
        int r;

        r = chmod(race_link, (i & 1) ? 0600 : 0644);
        if (r == 0) atomic_fetch_add(&successes, 1);
        atomic_fetch_add(&operations, 1);

        if ((i + seed) % 3 == 0) {
            r = utimensat(AT_FDCWD, race_link, ts, 0);
            if (r == 0) atomic_fetch_add(&successes, 1);
            atomic_fetch_add(&operations, 1);
        }

        if ((i + seed) % 5 == 0) {
            int fd = open(race_link, O_WRONLY | O_TRUNC | O_CLOEXEC);
            if (fd >= 0) {
                atomic_fetch_add(&successes, 1);
                close(fd);
            }
            atomic_fetch_add(&operations, 1);
        }

        if ((i + seed) % 7 == 0) {
            struct stat st;
            r = lstat(race_link, &st);
            if (r == 0) atomic_fetch_add(&successes, 1);
            atomic_fetch_add(&operations, 1);
        }

        if ((i + seed) % 11 == 0) {
            r = chown(race_link, (uid_t)-1, (gid_t)-1);
            if (r == 0) atomic_fetch_add(&successes, 1);
            atomic_fetch_add(&operations, 1);
        }
    }
    return NULL;
}

int main(int argc, char **argv)
{
    pthread_t toggler;
    pthread_t workers[WORKERS];
    struct stat st;

    if (argc != 2) {
        fprintf(stderr, "usage: %s <host-sentinel-absolute-path>\n", argv[0]);
        return 64;
    }
    host_target = argv[1];
    if (lstat(host_target, &st) == 0) {
        printf("FAIL host sentinel is visible inside guest: %s\n", host_target);
        return 65;
    }
    printf("PASS host-sentinel-not-visible errno=%d\n", errno);

    snprintf(race_link, sizeof(race_link), "%s/race-link", base);
    snprintf(guest_target, sizeof(guest_target), "%s/guest-victim", base);
    unlink(race_link);
    unlink(guest_target);
    rmdir(base);
    if (mkdir(base, 0700) != 0 && errno != EEXIST) {
        perror("mkdir");
        return 66;
    }
    int fd = open(guest_target, O_WRONLY | O_CREAT | O_TRUNC | O_CLOEXEC, 0644);
    if (fd < 0) {
        perror("open guest target");
        return 67;
    }
    if (write(fd, "guest\n", 6) != 6) {
        perror("write guest target");
        close(fd);
        return 68;
    }
    close(fd);
    if (symlink("guest-victim", race_link) != 0) {
        perror("initial symlink");
        return 69;
    }

    atomic_store(&stop_workers, 0);
    if (pthread_create(&toggler, NULL, toggle_thread, NULL) != 0) {
        perror("pthread_create toggler");
        return 70;
    }
    for (unsigned i = 0; i < WORKERS; i++) {
        if (pthread_create(&workers[i], NULL, worker_thread, (void *)(uintptr_t)i) != 0) {
            perror("pthread_create worker");
            atomic_store(&stop_workers, 1);
            pthread_join(toggler, NULL);
            return 71;
        }
    }
    for (unsigned i = 0; i < WORKERS; i++)
        pthread_join(workers[i], NULL);
    atomic_store(&stop_workers, 1);
    pthread_join(toggler, NULL);

    unsigned long n_toggles = atomic_load(&toggles);
    unsigned long n_ops = atomic_load(&operations);
    unsigned long n_ok = atomic_load(&successes);
    char next0[256], next1[256];
    snprintf(next0, sizeof(next0), "%s/.next-0", base);
    snprintf(next1, sizeof(next1), "%s/.next-1", base);
    unlink(next0);
    unlink(next1);
    unlink(race_link);
    unlink(guest_target);
    rmdir(base);

    printf("STATS toggles=%lu operations=%lu successes=%lu\n", n_toggles, n_ops, n_ok);
    if (n_toggles < 1000) {
        printf("FAIL insufficient toggle interleavings\n");
        return 72;
    }
    if (n_ops < 1000) {
        printf("FAIL insufficient metadata operations\n");
        return 73;
    }
    printf("PATH_RACE_DONE\n");
    return 0;
}
