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

# A valid Linux path may contain more than 32 directory components. The
# resolver must not keep a fixed one-fd-per-component stack and reject it.
deep=/tmp/husk-deep
for i in $(seq 1 64); do deep=$deep/d$i; done
mkdir -p "$deep"
chk deep-path "$(cd "$deep" && pwd -P)" "$deep"

# An oversized absolute path must return promptly, not panic or spin.
printf -v long '%*s' 5000 ''
long=${long// /a}
test -e "/$long"
rc=$?
chk long-path-rejected "$rc" 1

# Final symlinks must be followed through the guest root, not the host root.
ln -s /etc/os-release /tmp/husk-rel/abs
guest_os=$(head -1 /etc/os-release)
chk symlink-follow "$(head -1 /tmp/husk-rel/abs)" "$guest_os"

# Absolute and dot-dot targets that name host paths must remain contained.
ln -s /system/build.prop /tmp/husk-rel/escape-abs
ln -s ../../../../../../system/build.prop /tmp/husk-rel/escape-dots
if cat /tmp/husk-rel/escape-abs >/dev/null 2>&1; then bad escape-abs contained; else ok escape-abs; fi
if cat /tmp/husk-rel/escape-dots >/dev/null 2>&1; then bad escape-dots contained; else ok escape-dots; fi

# Passthrough /proc must keep normal process queries while refusing the procfs
# magic links that point back into the host namespace.
test -e /proc/self/root/system/build.prop
chk proc-root-contained "$?" 1
chk proc-safe-status "$(head -1 /proc/self/status | cut -d: -f1)" Name

# Metadata calls must act on the final inode without letting a final-name lookup
# race. These checks cover chmod, follow/no-follow utimensat, and stat/statx
# symlink semantics through the interposed libc entry points.
echo x > /tmp/husk-rel/target
ln -s target /tmp/husk-rel/target-link
chmod 0601 /tmp/husk-rel/target
chk metadata-chmod "$(stat -c %a /tmp/husk-rel/target)" 601
touch -h -d @1600000000 /tmp/husk-rel/target-link
chk metadata-utime-nofollow "$(stat -c %Y /tmp/husk-rel/target-link)" 1600000000
chk metadata-stat-nofollow "$(stat -c %F /tmp/husk-rel/target-link)" 'symbolic link'
chk metadata-stat-follow "$(stat -L -c %F /tmp/husk-rel/target-link)" 'regular file'
touch -d @1600000100 /tmp/husk-rel/target-link
chk metadata-utime-follow "$(stat -c %Y /tmp/husk-rel/target)" 1600000100

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

# A real dirfd pins a starting inode, not the guest namespace. Exercise
# openat/fstatat/faccessat/mkdirat/readlinkat with ".." escapes and an absolute
# symlink through libc's PLT. Run this while the parent process's cwd is inside
# the guest root; the host must also check that the attempted host directory was
# not created.
if [ -x /dirfd-probe ]; then
    out=$(/dirfd-probe 2>&1)
    rc=$?
    echo "$out"
    if [ "$rc" -eq 0 ]; then ok relative-dirfd-contained; else bad relative-dirfd-contained "probe rc=$rc"; fi
else
    echo "SKIP relative-dirfd-contained -- /dirfd-probe not installed"
fi

rm -rf /tmp/husk-rel /tmp/husk-deep
echo "#checks=$n fail=$f"
[ "$f" -eq 0 ]
