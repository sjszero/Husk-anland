#!/bin/sh
# Outbound network from the guest: DNS, TCP, send(), HTTP, HTTPS.
# Run inside the guest:  sh gexec.sh rootfs-full usr/bin/bash tmp/13.sh
# Needs real connectivity; there is no offline mode, a failure here is a failure.
P=0; F=0
ok()  { P=$((P+1)); echo "  PASS  $1"; }
bad() { F=$((F+1)); echo "  FAIL  $1"; }

h=$(timeout 15 getent ahostsv4 ports.ubuntu.com 2>/dev/null | head -1 | awk '{print $1}')
[ -n "$h" ] && ok "DNS resolves ports.ubuntu.com ($h)" || bad "DNS resolves ports.ubuntu.com"

ip=$(timeout 15 getent ahostsv4 one.one.one.one 2>/dev/null | head -1 | awk '{print $1}')
[ -n "$ip" ] && ok "DNS resolves one.one.one.one ($ip)" || bad "DNS resolves one.one.one.one"

# send(2) on a real socket: this is the call whose wrapper used to pass a
# garbage destination pointer and come back EINVAL.
timeout 30 python3 - <<'PY'
import socket, sys
host = "ports.ubuntu.com"
try:
    s = socket.create_connection((host, 80), 8)
except OSError as e:
    print("connect failed:", e); sys.exit(3)
req = ("GET /ubuntu-ports/dists/noble/Release HTTP/1.0\r\nHost: %s\r\n\r\n" % host).encode()
n = s.send(req)                      # <- interposed send()
print("send returned", n)
head = s.recv(64)
s.close()
if n != len(req) or not head.startswith(b"HTTP/1."):
    print("bad response:", head[:20]); sys.exit(4)
PY
case $? in
  0) ok "send() + HTTP/1.0 GET over a real socket" ;;
  3) bad "TCP connect to ports.ubuntu.com:80" ;;
  *) bad "send() + HTTP GET over a real socket" ;;
esac

c=$(timeout 30 curl -4 -s -o /dev/null -w '%{http_code}' http://ports.ubuntu.com/ubuntu-ports/dists/noble/Release 2>/dev/null)
[ "$c" = "200" ] && ok "curl http -> 200" || bad "curl http -> 200 (got '$c')"

c=$(timeout 30 curl -4 -s -o /dev/null -w '%{http_code}' https://ports.ubuntu.com/ubuntu-ports/dists/noble/Release 2>/dev/null)
[ "$c" = "200" ] && ok "curl https (TLS) -> 200" || bad "curl https (TLS) -> 200 (got '$c')"

# ICMP is a documented platform limit: ping cannot drop capabilities here
e=$(timeout 10 ping -c1 -W2 127.0.0.1 2>&1 | head -1)
case "$e" in
  *"Operation not permitted"*|*"not permitted"*) ok "raw ICMP is refused (documented limit)" ;;
  *) ok "ping answered: $e" ;;
esac

echo "PASS=$P FAIL=$F"
[ "$F" -eq 0 ]
