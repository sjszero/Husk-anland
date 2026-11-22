#!/bin/bash
# Focused path-boundary regression checks for the loader-embedded shim.
# Run inside the guest under a host-side timeout; a stuck resolver is a failure.
set -u
n=0
f=0
ok(){ echo "PASS $1"; n=$((n+1)); }
bad(){ echo "FAIL $1 -- $2"; f=$((f+1)); }
chk(){ if [ "$2" = "$3" ]; then ok "$1"; else bad "$1" "got [$2] want [$3]"; fi; }

rm -rf /tmp/husk-rel; mkdir -p /tmp/husk-rel

# Missing-path resolution must not retain temporary directory descriptors.
before=$(ls /proc/self/fd | wc -l | tr -d ' ')
for i in $(seq 1 1000); do test -e /definitely/missing/$i/x; done
after=$(ls /proc/self/fd | wc -l | tr -d ' ')
chk fd-stable "$after" "$before"

# An oversized absolute path must return promptly, not panic or spin.
printf -v long '%*s' 5000 ''
long=${long// /a}
test -e "/$long"
rc=$?
chk long-path-rejected "$rc" 1

# Final symlinks must be followed through the guest root, not the host root.
ln -s /etc/os-release /tmp/husk-rel/abs
chk symlink-follow "$(head -1 /tmp/husk-rel/abs)" 'PRETTY_NAME="Ubuntu 24.04.4 LTS"'

# Absolute and dot-dot targets that name host paths must remain contained.
ln -s /system/build.prop /tmp/husk-rel/escape-abs
ln -s ../../../../../../system/build.prop /tmp/husk-rel/escape-dots
if cat /tmp/husk-rel/escape-abs >/dev/null 2>&1; then bad escape-abs contained; else ok escape-abs; fi
if cat /tmp/husk-rel/escape-dots >/dev/null 2>&1; then bad escape-dots contained; else ok escape-dots; fi

# Optional guest-side Rust probe: exercises faccessat() through the shim's GOT
# interpose, including dangling symlinks and a relative dirfd.
if [ -x /path-probe ]; then
    out=$(/path-probe 2>&1)
    rc=$?
    echo "$out"
    if [ "$rc" -eq 0 ]; then ok faccessat-nofollow; else bad faccessat-nofollow "probe rc=$rc"; fi
else
    echo "SKIP faccessat-nofollow -- /path-probe not installed"
fi

rm -rf /tmp/husk-rel
echo "#checks=$n fail=$f"
[ "$f" -eq 0 ]
