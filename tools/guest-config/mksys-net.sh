#!/bin/sh
# Materialise /sys/class/net inside a guest root (loopback only).
#
# Android refuses netlink and the sysfs listing to untrusted_app (measured), so
# passthrough would show the *host* interfaces - names the guest cannot use. The
# runtime presents its own view instead, and the shim resolves /sys/class/net
# inside the guest root rather than through /sys. Per-interface entries for
# wlan0/rmnet come from the platform (NetworkInterface) when the runtime passes
# them in; the shim does not invent them.
#
#   run-as <pkg> sh mksys-net.sh <rootfs-dir>
# network view; loopback is the one interface this runtime can honestly describe
# without platform input.
cd "$1" || exit 1
mkdir -p sys/class/net/lo/statistics
printf '00:00:00:00:00:00\n' > sys/class/net/lo/address
printf '6\n'                  > sys/class/net/lo/addr_len
printf '772\n'                > sys/class/net/lo/type
printf '0x9\n'                > sys/class/net/lo/flags
printf '65536\n'              > sys/class/net/lo/mtu
printf '1\n'                  > sys/class/net/lo/carrier
printf 'unknown\n'            > sys/class/net/lo/operstate
printf '0\n'                  > sys/class/net/lo/ifindex
for f in rx_bytes rx_packets rx_errors rx_dropped tx_bytes tx_packets tx_errors tx_dropped; do
  printf '0\n' > sys/class/net/lo/statistics/$f
done
ls -l sys/class/net/lo | head -4
