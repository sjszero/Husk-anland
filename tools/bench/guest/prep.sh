#!/bin/sh
# One-time guest-side fixtures, so every runner measures the same trees.
# Idempotent. Run inside the guest root: sh /tmp/prep.sh
set -u
cd /root || exit 1

# 200-file C project for `make -j4` (dimension 2/4: build throughput).
if [ ! -d /root/bench-make ]; then
  mkdir -p /root/bench-make
  # f0.c carries main(), so the 200-object link is a real link. Without it the
  # fixture fails on every runner with "undefined reference to `main`", which
  # would measure a broken Makefile instead of a build.
  printf '#include <stdio.h>\nint f0(void){return 0;}\nint main(void){return 0;}\n' \
    > /root/bench-make/f0.c
  i=1
  while [ $i -lt 200 ]; do
    printf '#include <stdio.h>\nint f%d(void){return %d;}\n' "$i" "$i" > /root/bench-make/f$i.c
    i=$((i+1))
  done
  {
    printf 'CFLAGS=-O2\n'
    printf 'OBJS='
    i=0
    while [ $i -lt 200 ]; do printf 'f%d.o ' "$i"; i=$((i+1)); done
    printf '\napp: $(OBJS)\n\t$(CC) -o app $(OBJS)\n'
    printf 'clean:\n\trm -f $(OBJS) app\n'
  } > /root/bench-make/Makefile
fi

# 1000-file git repo for clone + status (dimension 3).
if [ ! -d /root/bench-repo/.git ]; then
  mkdir -p /root/bench-repo
  cd /root/bench-repo || exit 1
  i=0
  while [ $i -lt 1000 ]; do printf 'line %d\n' "$i" > "file$i.txt"; i=$((i+1)); done
  git init -q .
  git -c user.name=bench -c user.email=bench@localhost add -A >/dev/null 2>&1
  git -c user.name=bench -c user.email=bench@localhost commit -q -m init >/dev/null 2>&1
fi
echo "prep ok: $(ls /root/bench-make/*.c 2>/dev/null | wc -l) sources, $(ls /root/bench-repo | wc -l) repo files"
