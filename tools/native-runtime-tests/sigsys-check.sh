#!/bin/bash
# Does the in-loader SIGSYS handler work as a fallback?
#
# Android's app seccomp policy answers a few *optional* syscalls with
# SECCOMP_RET_TRAP. This ROM's policy at least covers set_robust_list(99) and
# rseq(293) (evidence/14: glibc's own startup calls them and dies without the
# build-time nop). A direct call proves both modes without relying on glibc:
#
#   NR_SIGSYS=log    -> the process must die with 128+31 = 159, nothing else
#   NR_SIGSYS=enosys -> the call must return -1/ENOSYS and the process must live
#
# Note: getrandom(2) is NOT trapped here (measured: it returns 8 bytes), which
# corrects a claim in evidence/16 - the temp-name code uses auxv AT_RANDOM
# anyway, but the reason given there was wrong.
cd /tmp
cat > sys.c <<'C'
#include <errno.h>
#include <stdio.h>
#include <sys/syscall.h>
#include <unistd.h>

static void probe(const char *name, long nr) {
    errno = 0;
    long r = syscall(nr, 0, 0, 0, 0, 0, 0);
    printf("%-16s nr=%ld -> %ld errno=%d %s\n", name, nr, r, errno,
           (r == -1 && errno == ENOSYS) ? "=ENOSYS (no kernel support)" : "(kernel answered)");
    fflush(stdout);
}
int main(void) {
    probe("set_robust_list", SYS_set_robust_list);
    probe("rseq", SYS_rseq);
    printf("survived both\n");
    return 0;
}
C
gcc -O2 -o sys sys.c || exit 1
./sys; rc=$?
echo "rc=$rc  (159 = 128+SIGSYS, i.e. the trap was not handled)"
