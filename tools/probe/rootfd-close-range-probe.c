/*
 * Concurrent close_range(rootfd, rootfd) while path operations run.
 *
 * The wrapper must preserve the private rootfd. The probe intentionally uses
 * only fd 1024, which is the runtime's ROOTFD_FLOOR, so it does not race and
 * close unrelated guest or path-resolution descriptors.
 */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <pthread.h>
#include <stdatomic.h>
#include <stdio.h>
#include <sys/stat.h>
#include <unistd.h>

static atomic_int stop_flag;
static atomic_ulong stat_ok, stat_fail;
static atomic_ulong access_ok, access_fail;
static atomic_ulong range_ok, range_fail;
static atomic_int first_errno;

static void note_errno(int e) {
    int expected = 0;
    atomic_compare_exchange_strong(&first_errno, &expected, e);
}

static void *reader(void *arg) {
    (void)arg;
    struct stat st;
    while (!atomic_load_explicit(&stop_flag, memory_order_relaxed)) {
        errno = 0;
        if (stat("/etc/os-release", &st) == 0) {
            atomic_fetch_add_explicit(&stat_ok, 1, memory_order_relaxed);
        } else {
            atomic_fetch_add_explicit(&stat_fail, 1, memory_order_relaxed);
            note_errno(errno);
        }
        errno = 0;
        if (access("/usr/bin/bash", X_OK) == 0) {
            atomic_fetch_add_explicit(&access_ok, 1, memory_order_relaxed);
        } else {
            atomic_fetch_add_explicit(&access_fail, 1, memory_order_relaxed);
            note_errno(errno);
        }
    }
    return NULL;
}

static void *closer(void *arg) {
    (void)arg;
    for (int i = 0; i < 250; i++) {
        errno = 0;
        if (close_range(1024, 1024, 0) == 0) {
            atomic_fetch_add_explicit(&range_ok, 1, memory_order_relaxed);
        } else {
            atomic_fetch_add_explicit(&range_fail, 1, memory_order_relaxed);
            note_errno(errno);
        }
    }
    return NULL;
}

int main(void) {
    pthread_t readers[8], closers[2];
    atomic_init(&stop_flag, 0);
    atomic_init(&stat_ok, 0); atomic_init(&stat_fail, 0);
    atomic_init(&access_ok, 0); atomic_init(&access_fail, 0);
    atomic_init(&range_ok, 0); atomic_init(&range_fail, 0);
    atomic_init(&first_errno, 0);
    for (int i = 0; i < 8; i++) pthread_create(&readers[i], NULL, reader, NULL);
    for (int i = 0; i < 2; i++) pthread_create(&closers[i], NULL, closer, NULL);
    for (int i = 0; i < 2; i++) pthread_join(closers[i], NULL);
    atomic_store_explicit(&stop_flag, 1, memory_order_relaxed);
    for (int i = 0; i < 8; i++) pthread_join(readers[i], NULL);
    printf("ROOTFD_RACE stat_ok=%lu stat_fail=%lu access_ok=%lu access_fail=%lu range_ok=%lu range_fail=%lu first_errno=%d\n",
           atomic_load(&stat_ok), atomic_load(&stat_fail), atomic_load(&access_ok),
           atomic_load(&access_fail), atomic_load(&range_ok), atomic_load(&range_fail),
           atomic_load(&first_errno));
    return (atomic_load(&stat_fail) || atomic_load(&access_fail) || atomic_load(&range_fail)) ? 1 : 0;
}
