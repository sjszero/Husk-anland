#!/bin/bash
# Item 3 of the sole plan: a real C build inside the runtime.
#   gcc -> cc1 -> as -> collect2 -> ld.bfd, then make -j4 / no-op / clean.
# Driven by run-guest.sh; the toolchain itself is installed by provision-toolchain.sh.
echo "### 1 driver"
gcc --version | head -1
echo
echo "### 2 C compile + run"
rm -rf /tmp/acc; mkdir -p /tmp/acc; cd /tmp/acc || exit 1
cat > hello.c <<'C'
#include <stdio.h>
#include <string.h>
int main(void)
{
    char buf[64];
    snprintf(buf, sizeof buf, "%s", "gcc on the native runtime");
    printf("%s (%zu)\n", buf, strlen(buf));
    return 0;
}
C
gcc -O2 -Wall -Wextra -o hello hello.c; echo "gcc rc=$?"
./hello; echo "run rc=$?"
echo
echo "### 3 subprocesses the driver runs"
gcc -### -O2 -o hello hello.c 2>&1 | grep -v "^#" | tail -4
echo
echo "### 4 make (parallel)"
rm -rf /tmp/proj; mkdir -p /tmp/proj; cd /tmp/proj || exit 1
cat > greet.h <<'H'
#ifndef GREET_H
#define GREET_H
int greet(const char *who);
#endif
H
cat > greet.c <<'C'
#include <stdio.h>
#include "greet.h"
int greet(const char *who) { return printf("hello, %s\n", who) < 0; }
C
cat > main.c <<'C'
#include "greet.h"
int main(void) { return greet("make"); }
C
cat > Makefile <<'M'
CC = gcc
CFLAGS = -O2 -Wall -Wextra
OBJS = main.o greet.o
all: app
app: $(OBJS)
	$(CC) $(CFLAGS) -o app $(OBJS)
%.o: %.c greet.h
	$(CC) $(CFLAGS) -c $< -o $@
clean:
	rm -f *.o app
.PHONY: all clean
M
make -j4; echo "make rc=$?"
./app; echo "run rc=$?"
echo "### 5 incremental + clean"
make; echo "noop rc=$?"
make clean; echo "clean rc=$?"
ls
echo "### 6 artefacts"
ls -l /tmp/acc/hello
ls -l /tmp/proj/*.o /tmp/proj/app 2>&1 | head -3
