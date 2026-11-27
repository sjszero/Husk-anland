#!/bin/bash
# Supplementary-group audit. Run inside the guest as the fake root:
#   DEV=... sh tools/device-tests/run-guest.sh audit-supplementary-groups.sh
# It must finish with PASS=n FAIL=0 before this behavior is claimed fixed.
set -u
PASS=0; FAIL=0
RUNTIME_PROBE=${RUNTIME_PROBE:-/root/runtime-probe}
ok()  { PASS=$((PASS+1)); echo "  PASS  $1"; }
bad() { FAIL=$((FAIL+1)); echo "  FAIL  $1"; }

echo "== G-01 supplementary groups survive setgroups and grant group access =="
userdel -r huskuser 2>/dev/null || true
groupdel huskgroup 2>/dev/null || true
groupdel huskuser 2>/dev/null || true
groupadd -g 1500 huskuser || { bad "primary groupadd"; exit 1; }
groupadd -g 4242 huskgroup || { bad "groupadd"; exit 1; }
useradd -u 1500 -g 1500 -m -s /bin/bash huskuser || { bad "useradd"; exit 1; }
usermod -aG huskgroup huskuser || { bad "usermod -aG"; exit 1; }
echo "  GROUP_ENTRY $(grep '^huskgroup:' /etc/group || true)"
rm -rf /tmp/husk-group-audit
mkdir -p /tmp/husk-group-audit
printf 'group-data\n' > /tmp/husk-group-audit/target
chown 0:4242 /tmp/husk-group-audit/target
chmod 0660 /tmp/husk-group-audit/target

# The child is a new process with the guest user's fake identity. The loader
# seeds uid/gid from the environment; setgroups/initgroups must update the same
# process-local group list that access(2) consults.
env NR_FAKE_UID=1500 NR_FAKE_GID=1500 "$RUNTIME_PROBE" supplementary-groups
rc=$?
if [ "$rc" -eq 0 ]; then ok "supplementary group child probe"; else bad "supplementary group child probe"; fi

# Cleanup must not hide a failed assertion. Remove the user before its primary
# group, otherwise userdel leaves an orphan group behind.
userdel -r huskuser 2>/dev/null || true
groupdel huskgroup 2>/dev/null || true
groupdel huskuser 2>/dev/null || true
rm -rf /tmp/husk-group-audit

echo
echo "PASS=$PASS FAIL=$FAIL"
[ "$FAIL" -eq 0 ]
