#!/bin/sh
# Run inside the guest (via gexec.sh) as root-of-the-guest:
#   sh gexec.sh rootfs-full usr/bin/bash tmp/09-network-surface.sh
# Checks the netlink surface: reads must answer, writes must be refused
# honestly (EPERM), never acknowledged with a bare success.
P=0; F=0
ok()  { P=$((P+1)); echo "  PASS  $1"; }
bad() { F=$((F+1)); echo "  FAIL  $1"; }

A=$(ip addr 2>&1)
case "$A" in
  *127.0.0.1*) ok "ip addr shows the loopback address" ;;
  *) bad "ip addr shows the loopback address" ;;
esac
case "$A" in
  *"inet 192."*|*"inet 10."*|*"inet 172."*) ok "ip addr shows a LAN address" ;;
  *) bad "ip addr shows a LAN address" ;;
esac

R=$(ip route 2>&1)
case "$R" in
  *"127.0.0.0/8"*) ok "ip route has the loopback prefix" ;;
  *) bad "ip route has the loopback prefix" ;;
esac

L=$(ip link show lo 2>&1)
case "$L" in
  *"link/loopback"*) ok "ip link show lo resolves by name" ;;
  *) bad "ip link show lo resolves by name" ;;
esac

I=$(hostname -I 2>&1)
[ -n "$(echo "$I" | tr -d ' \n')" ] && ok "hostname -I is non-empty" || bad "hostname -I is non-empty"

G=$(cat /proc/net/route 2>&1)
case "$G" in
  *0000007F*) ok "/proc/net/route has the loopback row in hex" ;;
  *) bad "/proc/net/route has the loopback row in hex" ;;
esac

D=$(cat /proc/net/dev 2>&1)
case "$D" in
  *"  lo:"*) ok "/proc/net/dev lists lo" ;;
  *) bad "/proc/net/dev lists lo" ;;
esac

H=$(getent ahostsv4 localhost 2>&1)
case "$H" in
  *127.0.0.1*) ok "getent ahostsv4 localhost -> 127.0.0.1" ;;
  *) bad "getent ahostsv4 localhost -> 127.0.0.1" ;;
esac

W=$(ip addr add 10.99.99.99/24 dev lo 2>&1)
case "$W" in
  *"not permitted"*) ok "ip addr add is refused with EPERM" ;;
  *) bad "ip addr add is refused with EPERM (got: $W)" ;;
esac
A2=$(ip addr 2>&1)
case "$A2" in
  *10.99.99.99*) bad "the refused address did not appear" ;;
  *) ok "the refused address did not appear" ;;
esac

W2=$(ip route get 8.8.8.8 2>&1)
case "$W2" in
  *"not supported"*) ok "ip route get is refused with EOPNOTSUPP" ;;
  *) bad "ip route get is refused with EOPNOTSUPP (got: $W2)" ;;
esac

echo "PASS=$P FAIL=$F"
[ "$F" -eq 0 ]
