#!/bin/sh
# The measurement body. Runs *inside* the guest root, once per repetition, and
# prints one "name<TAB>value" line per item. Nothing here knows which runner it
# is under: that is the point.
#
# Environment:
#   BENCH_COLD_APT=1   also drop /var/lib/apt/lists and time a cold `apt-get update`
#                      (expensive; run-all.sh sets it only on the first N reps)
set -u
cd /root 2>/dev/null || cd /tmp
now() { date +%s%N; }
put() { printf '%s\t%s\n' "$1" "$2"; }
timed() {
  name=$1; shift
  t0=$(now)
  if "$@" >/dev/null 2>&1; then
    t1=$(now)
    put "$name" $(( (t1 - t0) / 1000000 ))
  else
    # A missing tool must not look like a fast one.
    put "$name" NA
  fi
}

# --- 2 · process + syscall density ------------------------------------------
timed fork_exec_100  sh -c 'i=0; while [ $i -lt 100 ]; do /bin/true; i=$((i+1)); done'
timed sh_loop_1000   sh -c 'i=0; while [ $i -lt 1000 ]; do i=$((i+1)); done'
timed fork_50_true   sh -c 'i=0; while [ $i -lt 50 ]; do (/bin/true); i=$((i+1)); done'
timed getpid_20k     sh -c 'python3 -c "
import os
for _ in range(20000): os.getpid()
"'
timed stat_20k       sh -c 'python3 -c "
import os
for _ in range(20000): os.stat(\"/etc/hostname\")
"'
timed open_close_20k sh -c 'python3 -c "
for _ in range(20000):
    f = open(\"/etc/hostname\"); f.close()
"'

# --- 3 · file I/O -----------------------------------------------------------
timed find_usr       sh -c 'find /usr -xdev -type f | wc -l'
timed tar_doc        sh -c 'tar -cf - /usr/share/doc 2>/dev/null | wc -c'
timed grep_include   sh -c 'grep -rl pthread /usr/include 2>/dev/null | wc -l'
timed write_256m     sh -c 'dd if=/dev/zero of=/root/bench.bin bs=1M count=256 conv=fsync 2>/dev/null'
timed read_256m      sh -c 'dd if=/root/bench.bin of=/dev/null bs=1M 2>/dev/null'
timed small_files    sh -c 'mkdir -p /root/bench.d && cd /root/bench.d && i=0; while [ $i -lt 1000 ]; do : > f$i; i=$((i+1)); done; cd /; rm -rf /root/bench.d'
rm -f /root/bench.bin

# --- 4 · package management + dev workflow ---------------------------------
timed dpkg_l         sh -c 'dpkg -l | wc -l'
timed apt_simulate   sh -c 'apt-get -s install --no-download acl | tail -1'
timed apt_update_hot sh -c 'apt-get update >/dev/null 2>&1'
if [ "${BENCH_COLD_APT:-0}" = 1 ]; then
  rm -rf /var/lib/apt/lists/* 2>/dev/null
  timed apt_update_cold sh -c 'apt-get update >/dev/null 2>&1'
fi
timed gcc_hello      sh -c 'printf "#include <stdio.h>\nint main(void){puts(\"hi\");return 0;}\n" > /root/h.c && gcc -o /root/h /root/h.c && rm -f /root/h /root/h.c'
timed make_j4        sh -c 'cd /root/bench-make && make -s clean >/dev/null 2>&1; make -s -j4 >/dev/null 2>&1'
timed python_json    sh -c 'python3 -c "import json;json.dumps(list(range(20000)))"'
timed python_numpy   sh -c 'python3 -c "import numpy;print(int(numpy.arange(200000).sum()))"'
timed git_clone      sh -c 'rm -rf /root/bench-clone && git clone -q file:///root/bench-repo /root/bench-clone && git -C /root/bench-clone status --short >/dev/null && rm -rf /root/bench-clone'

# --- 8 · compatibility (availability, not speed) ---------------------------
for c in bash git python3 gcc make vim node nano; do
  if command -v "$c" >/dev/null 2>&1; then put "have_$c" 1; else put "have_$c" 0; fi
done
