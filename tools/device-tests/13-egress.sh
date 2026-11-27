#!/bin/sh
# Outbound network from the guest: DNS, TCP, send(), HTTP, HTTPS/TLS and raw ICMP.
# Run through run-egress-probe.sh; no guest Python, awk, curl or ping is required.
# Needs real connectivity; there is no offline mode, a failure here is a failure.
P=0; F=0
ok()  { P=$((P+1)); echo "  PASS  $1"; }
bad() { F=$((F+1)); echo "  FAIL  $1"; }

if timeout 45 "${RUNTIME_PROBE:-/root/runtime-probe}" egress; then
  ok "DNS + TCP + send() + HTTP + HTTPS/TLS + ICMP boundary"
else
  bad "DNS + TCP + send() + HTTP + HTTPS/TLS + ICMP boundary"
fi

echo "PASS=$P FAIL=$F"
[ "$F" -eq 0 ]
