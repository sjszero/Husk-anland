#!/bin/sh
# Install the guest's own ldconfig into a rootfs on the device.
#
# Ubuntu's /sbin/ldconfig.real is a *static* binary: it carries no dynamic
# symbols, so the loader's path shim cannot translate anything it asks for and
# it resolves /lib, /etc, /usr against the Android host namespace. Inside Husk
# that means
#
#   /sbin/ldconfig.real: Can't stat /lib: No such file or directory
#   /sbin/ldconfig.real: Can't create temporary cache file /etc/ld.so.cache~:
#                        Read-only file system
#
# and an exit code of 1, which `dpkg --configure libc-bin` treats as a real
# failure - apt then refuses to touch anything.
#
# The cache is written by husk-ldconfig instead: a normal guest binary that runs
# under the runtime, so the paths it reads and writes are namespace paths. The
# original static binary is kept beside it as ldconfig.real.glbc.
#
#   ./install-ldconfig.sh            # build + push
#   SKIP_BUILD=1 ./install-ldconfig.sh
set -eu
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
. "$HERE/../bench/bench.env"
R="adb -s $DEV shell run-as $PKG"

: "${ROOTFS:=files/linux/distributions/ubuntu-24.04/rootfs}"

if [ "${SKIP_BUILD:-0}" != 1 ]; then
  "$HERE/build-guest-tools.sh" >/dev/null
fi

BIN=$ROOT/runtime/target/aarch64-unknown-linux-gnu/release/husk-ldconfig
[ -s "$BIN" ] || { echo "missing $BIN" >&2; exit 1; }

adb -s "$DEV" shell "run-as $PKG sh -c 'cat > $ROOTFS/usr/bin/husk-ldconfig'" < "$BIN"
adb -s "$DEV" shell "run-as $PKG sh -c 'cat > $ROOTFS/sbin/ldconfig.real.husk'" <<'SCRIPT'
#!/bin/sh
# husk: Ubuntu's ldconfig.real is a static-pie binary, so the loader's path shim
# cannot translate the paths it uses. The ld.so cache is written by husk-ldconfig
# (a normal guest binary) instead; the original static binary is kept next to
# this script as ldconfig.real.glbc.
exec /usr/bin/husk-ldconfig "$@"
SCRIPT
adb -s "$DEV" shell "run-as $PKG sh -c '
  R=$ROOTFS
  chmod 755 \$R/usr/bin/husk-ldconfig \$R/sbin/ldconfig.real.husk
  if [ -f \$R/sbin/ldconfig.real ] && [ ! -f \$R/sbin/ldconfig.real.glbc ]; then
    cp \$R/sbin/ldconfig.real \$R/sbin/ldconfig.real.glbc
  fi
  cp \$R/sbin/ldconfig.real.husk \$R/sbin/ldconfig.real
  rm -f \$R/sbin/ldconfig.real.husk
  ls -l \$R/sbin/ldconfig.real \$R/sbin/ldconfig.real.glbc \$R/usr/bin/husk-ldconfig'"
echo "ok: ldconfig.real -> husk-ldconfig (original kept as ldconfig.real.glbc)"
