#!/bin/bash
# "Is this a real Linux?" - run inside the runtime (tools/device-tests/run-guest.sh demo.sh)
echo "== kernel =="; uname -sr; echo "page=$(getconf PAGESIZE 2>/dev/null || echo n/a)"
echo "== distro =="; head -4 /etc/os-release
echo "== userspace =="; echo "bins in /usr/bin: $(ls /usr/bin | wc -l)"; bash --version | head -1
echo "== toolchain =="; gcc --version | head -1; make --version | head -1
echo "== writes/pipes =="; echo hi > /tmp/x && cat /tmp/x | tr a-z A-Z
echo "== subprocesses =="; /usr/bin/env sh -c 'echo child-ok'; ls /etc | wc -l
echo "== cwd =="; cd /etc && pwd -P && cd .. && pwd -P
