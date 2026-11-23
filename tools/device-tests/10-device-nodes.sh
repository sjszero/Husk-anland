#!/bin/sh
# Device node surface: what works by name, and the three platform walls.
# Run inside the guest:  sh gexec.sh rootfs-full usr/bin/bash tmp/10.sh
# The walls are asserted on purpose: if the platform ever relaxes, this goes red
# and the design note in docs/design/device-nodes.md has to be revisited.
P=0; F=0
ok()  { P=$((P+1)); echo "  PASS  $1"; }
bad() { F=$((F+1)); echo "  FAIL  $1"; }

[ "$(head -c 4 /dev/zero | od -An -tx1 | tr -d ' \n')" = "00000000" ] \
  && ok "/dev/zero reads zeroes" || bad "/dev/zero reads zeroes"

n=$(head -c 8 /dev/urandom | od -An -tx1 | tr -d ' \n')
[ ${#n} -eq 16 ] && ok "/dev/urandom returns 8 bytes" || bad "/dev/urandom returns 8 bytes"

(echo x > /dev/null) 2>/dev/null && ok "/dev/null accepts writes" || bad "/dev/null accepts writes"

[ -d /dev/shm ] && ok "/dev/shm is a directory" || bad "/dev/shm is a directory"
[ -e /dev/pts ] && ok "/dev/pts exists" || bad "/dev/pts exists"

mkfifo /tmp/husk-fifo.$$ 2>/dev/null && [ -p /tmp/husk-fifo.$$ ] \
  && ok "mkfifo creates a FIFO" || bad "mkfifo creates a FIFO"
rm -f /tmp/husk-fifo.$$

# wall 1: /dev cannot be listed from this domain (SELinux)
e=$(ls /dev 2>&1)
case "$e" in
  *"Permission denied"*) ok "/dev listing is refused by the domain (known wall)" ;;
  *) bad "/dev listing changed - revisit docs/design/device-nodes.md ($e)" ;;
esac

# wall 2: /dev/full is not reachable, so no ENOSPC can be produced
e=$( (echo x > /dev/full) 2>&1 )
case "$e" in
  *"Permission denied"*|*"No such file"*) ok "/dev/full is not reachable (documented non-goal)" ;;
  *"No space left"*) bad "/dev/full started answering ENOSPC - update the design note" ;;
  *) bad "/dev/full changed behaviour ($e)" ;;
esac

# wall 3: /dev/tty exists but there is no controlling terminal -> ENXIO
if [ -e /dev/tty ]; then
  e=$( (echo hi > /dev/tty) 2>&1 )
  case "$e" in
    *"No such device"*) ok "/dev/tty: node present, no controlling terminal (ENXIO)" ;;
    *) ok "/dev/tty: node present, write gave: $e" ;;
  esac
else
  ok "/dev/tty not visible from this domain"
fi

echo "PASS=$P FAIL=$F"
[ "$F" -eq 0 ]
