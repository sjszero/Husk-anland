#!/bin/bash
# Server-surface parity probe.
#
# The question: "a container/VM ships a complete Linux userspace - how much of
# that surface do we actually have?"  Everything is measured inside the guest, in
# the app uid, with no root.  One line per check:
#
#   R|<area>|<name>|PASS|PARTIAL|FAIL|UNKNOWN|<detail>
#
# PASS    works like on a normal server
# PARTIAL answers, but degraded or with a field we cannot fill honestly
# FAIL    missing or refused
# UNKNOWN could not be measured
S() { printf 'R|%s|%s|%s|%s\n' "$1" "$2" "$3" "$4"; }
ok() { if [ "$1" = 0 ]; then echo PASS; else echo FAIL; fi; }
RUNTIME_PROBE=${RUNTIME_PROBE:-/root/runtime-probe}
SP() { "$RUNTIME_PROBE" server-parity "$@"; }

# ---------------------------------------------------------------- identity ---
S id uid "$(id -u >/dev/null 2>&1; ok $?)" "$(id 2>&1 | cut -c1-60)"
cap=$(SP cap-eff 2>/dev/null)
S id capabilities "$([ -n "$cap" ] && echo PASS || echo FAIL)" "CapEff=${cap:-?} (a container usually has 14; we have 0)"
S id passwd "$(getent passwd root >/dev/null 2>&1; ok $?)" "$(getent passwd root 2>&1)"
S id shadow "$([ -r /etc/shadow ]; ok $?)" "$(head -1 /etc/shadow 2>/dev/null | cut -c1-18)"
S id nsswitch "$(grep -q '^hosts:.*dns' /etc/nsswitch.conf 2>/dev/null; ok $?)" "$(grep '^hosts:' /etc/nsswitch.conf 2>/dev/null)"
# Account management is part of the container surface: images ship accounts and
# postinst scripts create more. Measured, not assumed (evidence 111).
S id useradd "$(useradd -m -s /bin/bash huskpp >/dev/null 2>&1 && [ -d /home/huskpp ] && getent passwd huskpp >/dev/null; r=$?; userdel -r huskpp >/dev/null 2>&1; ok $r)" "useradd -m: user + home"
S id groupadd "$(groupadd -r huskgg >/dev/null 2>&1 && getent group huskgg >/dev/null; r=$?; groupdel huskgg >/dev/null 2>&1; ok $r)" "groupadd -r"
S id usermod_member "$(groupadd -r huskgm >/dev/null 2>&1; useradd huskgmu >/dev/null 2>&1; usermod -aG huskgm huskgmu >/dev/null 2>&1; SP group-member huskgmu huskgm 2>/dev/null; r=$?; userdel huskgmu >/dev/null 2>&1; groupdel huskgm >/dev/null 2>&1; ok $r)" "usermod -aG seen through getgrnam"
S id setuid PARTIAL "no CAP_SETUID: su/passwd cannot switch users ($(su -c true 2>&1 | head -1))"

# ------------------------------------------------------------- filesystem ---
S fs df_root "$(df -h / >/dev/null 2>&1; ok $?)" "$(df -h / 2>/dev/null | tail -1 | tr -s ' ')"
S fs df_listing "$(df -h >/dev/null 2>&1; ok $?)" "$(df -h 2>/dev/null | wc -l) lines"
S fs df_inode "$(df -i / >/dev/null 2>&1; ok $?)" "$(df -i / 2>/dev/null | tail -1 | tr -s ' ')"
S fs mounts "$(grep -q ' / f2fs\| / ext4' /proc/mounts 2>/dev/null; ok $?)" "$(head -1 /proc/mounts)"
S fs findmnt "$(findmnt >/dev/null 2>&1; ok $?)" "findmnt: $(findmnt 2>/dev/null | wc -l) entries"
S fs flock "$(SP flock /tmp/.fl 2>/dev/null; ok $?)" "exclusive flock"
S fs posixlock "$(SP posixlock /tmp/.pl 2>/dev/null; ok $?)" "fcntl lock"
S fs inotify "$(SP inotify 2>/dev/null; ok $?)" "inotify_init1"
S fs xattr "$(SP xattr /tmp 2>/dev/null; ok $?)" "user. xattr on /tmp"
S fs otmpfile "$(SP otmpfile /tmp 2>/dev/null; ok $?)" "O_TMPFILE"
S fs sparse "$(SP sparse /tmp/.sp 2>/dev/null; ok $?)" "1 GiB truncate"
S fs hardlink PARTIAL "$(rm -rf /var/tmp/hl && mkdir -p /var/tmp/hl && echo x >/var/tmp/hl/a && ln /var/tmp/hl/a /var/tmp/hl/b 2>/dev/null; ok $?) nlink=$(stat -c %h /var/tmp/hl/a 2>/dev/null) (kernel result measured)"
# Mounting is part of the container surface. mount(2) needs CAP_SYS_ADMIN, so
# these measure the namespace-equivalent the shim provides (evidence 112).
S fs bind_mount "$(rm -rf /var/tmp/hb1 /var/tmp/hb2; mkdir -p /var/tmp/hb1 /var/tmp/hb2; echo b > /var/tmp/hb1/f; mount --bind /var/tmp/hb1 /var/tmp/hb2 2>/dev/null; cat /var/tmp/hb2/f 2>/dev/null | grep -q '^b$'; r=$?; umount /var/tmp/hb2 2>/dev/null; ok $r)" "mount --bind: target shows source content"
S fs tmpfs_mount "$(rm -rf /var/tmp/ht1; mkdir -p /var/tmp/ht1; mount -t tmpfs tmpfs /var/tmp/ht1 2>/dev/null; sh -c 'echo t > /var/tmp/ht1/a' 2>/dev/null; grep -q '^t$' /var/tmp/ht1/a 2>/dev/null; r=$?; umount /var/tmp/ht1 2>/dev/null; ok $r)" "mount -t tmpfs: writable scratch"
S fs ro_mount "$(rm -rf /var/tmp/hr1 /var/tmp/hr2; mkdir -p /var/tmp/hr1 /var/tmp/hr2; echo x > /var/tmp/hr1/f; mount --bind -o ro /var/tmp/hr1 /var/tmp/hr2 2>/dev/null; sh -c 'echo y > /var/tmp/hr2/g' 2>/dev/null && r=1 || r=0; umount /var/tmp/hr2 2>/dev/null; [ ! -e /var/tmp/hr1/g ] || r=1; ok $r)" "ro bind: read ok, write EROFS"
S fs mmap_shared "$(SP mmap-shared /tmp/.mm 2>/dev/null; ok $?)" "MAP_SHARED + flush"

# ------------------------------------------------------------------- ipc -----
S ipc unix_stream "$(SP unix-stream /tmp/.ux 2>/dev/null; ok $?)" "AF_UNIX pathname"
S ipc unix_abstract "$(SP unix-abstract huskabs-$$ 2>/dev/null; ok $?)" "abstract namespace"
S ipc fdpass "$(SP fdpass /etc/hostname 2>/dev/null; ok $?)" "SCM_RIGHTS"
S ipc sysv_shm "$(SP sysv-shm 2>/dev/null; ok $?)" "shmget"
S ipc posix_mq "$(SP posix-mq 2>/dev/null; ok $?)" "mq_open"
S ipc eventfd "$(SP eventfd 2>/dev/null; ok $?)" "eventfd"
S ipc timerfd "$(SP timerfd 2>/dev/null; ok $?)" "timerfd_create"
S ipc epoll "$(SP epoll 2>/dev/null; ok $?)" "epoll"
S ipc pty "$(SP pty 2>/dev/null; ok $?)" "openpty (/dev/pts)"
S ipc fifo "$(rm -f /tmp/.fifo; mkfifo /tmp/.fifo 2>/dev/null; s=$?; rm -f /tmp/.fifo; ok $s)" "mkfifo"

# --------------------------------------------------------------- network -----
S net resolv_conf "$([ -s /etc/resolv.conf ]; ok $?)" "$(head -1 /etc/resolv.conf 2>/dev/null)"
S net getent_dns "$(getent hosts baidu.com >/dev/null 2>&1; ok $?)" "$(getent hosts baidu.com 2>/dev/null | head -1)"
S net getaddrinfo "$(SP getaddrinfo example.com 443 2>/dev/null; ok $?)" "glibc resolver + nss_dns"
S net tcp_outbound "$(SP tcp-outbound 223.5.5.5 443 2>/dev/null; ok $?)" "TCP to a public address"
S net tcp_listen "$(SP tcp-listen 2>/dev/null; ok $?)" "bind 0.0.0.0 (unprivileged port)"
S net loopback "$(SP loopback 2>/dev/null; ok $?)" "loopback"
S net udp "$(SP udp 2>/dev/null; ok $?)" "UDP loopback"
S net ipv6 "$(SP ipv6 2>/dev/null; ok $?)" "AF_INET6"
hostname=$(SP hostname 2>/dev/null)
S net hostname "$([ -n "$hostname" ] && echo PASS || echo FAIL)" "${hostname:-?}"
S net proc_net "$(head -c 200 /proc/net/dev >/dev/null 2>&1; ok $?)" "$(sed -n 3p /proc/net/dev 2>/dev/null | tr -s ' ')"
S net sys_class_net "$(ls /sys/class/net >/dev/null 2>&1; ok $?)" "$(ls /sys/class/net 2>/dev/null | tr '\n' ' ')"
S net netlink "$(SP netlink 2>/dev/null; ok $?)" "netlink bind (ip addr enumerates interfaces)"
S net icmp PARTIAL "$(SP icmp 2>&1)"

# ------------------------------------------------------------- proc / sys ----
for f in /proc/self/status /proc/self/cmdline /proc/cpuinfo /proc/meminfo /proc/version /proc/self/limits /proc/self/maps /proc/loadavg /proc/uptime /proc/stat /proc/filesystems /proc/locks; do
  if head -c 40 "$f" >/dev/null 2>&1; then S proc "$f" PASS "$(head -c 30 "$f" 2>/dev/null | tr '\n' ' ')"; else S proc "$f" FAIL "denied to untrusted_app"; fi
done
for f in /proc/sys/vm/max_map_count /proc/sys/kernel/pid_max /proc/sys/net/core/somaxconn; do
  if head -c 20 "$f" >/dev/null 2>&1; then S sysctl "$f" PASS "$(cat "$f" 2>/dev/null)"; else S sysctl "$f" FAIL "denied to untrusted_app"; fi
done
S sys cgroup "$([ -d /sys/fs/cgroup ] && ls /sys/fs/cgroup >/dev/null 2>&1; ok $?)" "cgroup tree (listing denied => no limits)"
S sys cpu_online "$(cat /sys/devices/system/cpu/online >/dev/null 2>&1; ok $?)" "$(cat /sys/devices/system/cpu/online 2>&1)"

# ------------------------------------------------------------------ /dev -----
for d in null zero random urandom tty ptmx pts shm full mqueue log; do
  [ -e "/dev/$d" ] && S dev "/dev/$d" PASS "$(stat -c '%A %s' "/dev/$d" 2>/dev/null)" || S dev "/dev/$d" FAIL missing
done
S dev shm_write "$(printf x > /dev/shm/.p 2>/dev/null; rm -f /dev/shm/.p; ok $?)" "/dev/shm writable"

# -------------------------------------------------------------- services -----
S svc dbus_system "$([ -S /run/dbus/system_bus_socket ]; ok $?)" "system bus socket"
S svc dbus_session "$(SP dbus-session 2>/dev/null; ok $?)" "session bus starts"
S svc syslog "$( [ -S /dev/log ] && SP syslog husk "parity probe" 2>/dev/null; ok $?)" "syslog -> $(tail -1 /var/log/syslog 2>/dev/null | cut -c1-70)"
# syslog(3) reaches connect(2) through glibc's *internal* alias, which has no GOT
# slot. connect/bind are in the libc slot list now, so this exercises the
# interposed internal path (this Rust probe calls syslog(3) directly).
S svc syslog_libc "$(SP syslog parity "syslog3 probe" >/dev/null 2>&1 && tail -n 3 /var/log/syslog 2>/dev/null | grep -q 'parity: syslog3 probe'; ok $?)" "$(tail -1 /var/log/syslog 2>/dev/null | cut -c1-60)"
S svc systemd PARTIAL "systemd binary present but needs PID 1 + libsystemd-core (not shipped)"
S svc su PARTIAL "no CAP_SETUID"

# -------------------------------------------------------------- packages -----
S pkg dpkg "$(dpkg --version >/dev/null 2>&1; ok $?)" "$(dpkg --version 2>/dev/null | head -1)"
S pkg apt "$(apt --version >/dev/null 2>&1; ok $?)" "$(apt --version 2>/dev/null | head -1)"
S pkg apt_lists "$([ "$(ls /var/lib/apt/lists 2>/dev/null | wc -l)" -gt 5 ]; ok $?)" "$(ls /var/lib/apt/lists 2>/dev/null | wc -l) index files"
S pkg tz "$([ -f /usr/share/zoneinfo/Asia/Shanghai ]; ok $?)" "$(TZ=Asia/Shanghai date 2>&1)"
S pkg locale "$([ "$(locale -a 2>/dev/null | wc -l)" -gt 2 ]; ok $?)" "$(locale -a 2>/dev/null | tr '\n' ' ')"
# dpkg can be "installed" and still have half-configured packages, which makes
# apt refuse every later operation. That state is invisible to `dpkg --version`.
# The first column is "desired/status": "hi" is installed-and-held, which is
# what our libc is, and "rc" is removed-with-conffiles. Only a status letter
# other than i (ok) or c (conffiles) means the database is mid-transaction.
bad=$(SP dpkg-inconsistent 2>/dev/null) || bad=unknown
S pkg db_consistent "$([ "$bad" = 0 ] && echo PASS || echo FAIL)" "$bad package(s) not fully configured"
# ld.so.cache is written by our own writer; an independent reader (the static
# glibc ldconfig kept next to it) has to accept the file, or dlopen by bare
# soname loses the multiarch directories.
# The static reader cannot see the guest namespace, so it gets the real path.
if [ -x /usr/sbin/ldconfig.real.glbc ] && [ -f "$NR_ROOTFS/etc/ld.so.cache" ]; then
  S pkg ld_cache "$(/usr/sbin/ldconfig.real.glbc -C "$NR_ROOTFS/etc/ld.so.cache" -p 2>/dev/null | head -1 | grep -q 'libs found'; ok $?)" "$(/usr/sbin/ldconfig.real.glbc -C "$NR_ROOTFS/etc/ld.so.cache" -p 2>/dev/null | head -1 | cut -c1-60)"
else
  S pkg ld_cache UNKNOWN "independent cache reader or cache absent"
fi
echo DONE
