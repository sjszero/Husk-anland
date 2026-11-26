#!/bin/bash
# Supplementary-group audit. Run inside the guest as the fake root:
#   DEV=... sh tools/device-tests/run-guest.sh audit-supplementary-groups.sh
# It must finish with PASS=n FAIL=0 before this behavior is claimed fixed.
set -u
PASS=0; FAIL=0
ok()  { PASS=$((PASS+1)); echo "  PASS  $1"; }
bad() { FAIL=$((FAIL+1)); echo "  FAIL  $1"; }

echo "== G-01 supplementary groups survive setgroups and grant group access =="
groupdel huskgroup 2>/dev/null || true
userdel -r huskuser 2>/dev/null || true
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
env NR_FAKE_UID=1500 NR_FAKE_GID=1500 python3 - <<'PY'
import ctypes, os
libc = ctypes.CDLL(None, use_errno=True)
libc.getgroups.argtypes = [ctypes.c_int, ctypes.POINTER(ctypes.c_uint)]
libc.getgroups.restype = ctypes.c_int
libc.setgroups.argtypes = [ctypes.c_int, ctypes.POINTER(ctypes.c_uint)]
libc.setgroups.restype = ctypes.c_int
libc.initgroups.argtypes = [ctypes.c_char_p, ctypes.c_uint]
libc.initgroups.restype = ctypes.c_int

def groups():
    return sorted(os.getgroups())


def call(name, *args):
    fn = getattr(libc, name)
    r = fn(*args)
    if r != 0:
        e = ctypes.get_errno(); raise OSError(e, os.strerror(e))
    return r

print('  AFTER_START', groups())
print('  UID_GID', os.getuid(), os.getgid())
import subprocess
print('  ID_GROUPS', subprocess.run(['id','-G'], text=True, capture_output=True).stdout.strip())
print('  PY_GETGROUPS', os.getgroups())
os.setgroups([4242])
print('  AFTER_SETGROUPS', groups())
assert groups() == [4242], 'setgroups did not replace the supplementary group list'
print('  ACCESS_GROUP_R', os.access('/tmp/husk-group-audit/target', os.R_OK))
print('  ACCESS_GROUP_W', os.access('/tmp/husk-group-audit/target', os.W_OK))
assert os.access('/tmp/husk-group-audit/target', os.R_OK), \
    'group-owned 0660 file is not readable'

os.initgroups('huskuser', 1500)
print('  AFTER_INITGROUPS', groups())
assert 4242 in groups(), 'initgroups did not read /etc/group membership'
print('  PASS  setgroups/initgroups/getgroups/access')
PY
rc=$?
if [ "$rc" -eq 0 ]; then ok "supplementary group child probe"; else bad "supplementary group child probe"; fi

# Cleanup must not hide a failed assertion.
groupdel huskgroup 2>/dev/null || true
groupdel huskuser 2>/dev/null || true
userdel -r huskuser 2>/dev/null || true
rm -rf /tmp/husk-group-audit

echo
echo "PASS=$PASS FAIL=$FAIL"
[ "$FAIL" -eq 0 ]
