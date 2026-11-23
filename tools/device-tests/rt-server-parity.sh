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

# ---------------------------------------------------------------- identity ---
S id uid "$(id -u >/dev/null 2>&1; ok $?)" "$(id 2>&1 | cut -c1-60)"
cap=$(awk '/^CapEff/{print $2}' /proc/self/status 2>/dev/null)
S id capabilities "$([ -n "$cap" ] && echo PASS || echo FAIL)" "CapEff=${cap:-?} (a container usually has 14; we have 0)"
S id passwd "$(getent passwd root >/dev/null 2>&1; ok $?)" "$(getent passwd root 2>&1)"
S id shadow "$([ -r /etc/shadow ]; ok $?)" "$(head -1 /etc/shadow 2>/dev/null | cut -c1-18)"
S id nsswitch "$(grep -q '^hosts:.*dns' /etc/nsswitch.conf 2>/dev/null; ok $?)" "$(grep '^hosts:' /etc/nsswitch.conf 2>/dev/null)"
S id setuid PARTIAL "no CAP_SETUID: su/passwd cannot switch users ($(su -c true 2>&1 | head -1))"

# ------------------------------------------------------------- filesystem ---
S fs df_root "$(df -h / >/dev/null 2>&1; ok $?)" "$(df -h / 2>/dev/null | tail -1 | tr -s ' ')"
S fs df_listing "$(df -h >/dev/null 2>&1; ok $?)" "$(df -h 2>/dev/null | wc -l) lines"
S fs df_inode "$(df -i / >/dev/null 2>&1; ok $?)" "$(df -i / 2>/dev/null | tail -1 | tr -s ' ')"
S fs mounts "$(grep -q ' / f2fs\| / ext4' /proc/mounts 2>/dev/null; ok $?)" "$(head -1 /proc/mounts)"
S fs findmnt "$(findmnt >/dev/null 2>&1; ok $?)" "findmnt: $(findmnt 2>/dev/null | wc -l) entries"
S fs flock "$(python3 -c 'import fcntl,os;p="/tmp/.fl";f=open(p,"w");fcntl.flock(f,fcntl.LOCK_EX|fcntl.LOCK_NB);fcntl.flock(f,fcntl.LOCK_UN);os.unlink(p)' 2>/dev/null; ok $?)" "exclusive flock"
S fs posixlock "$(python3 -c 'import fcntl,os;p="/tmp/.pl";f=open(p,"w");fcntl.lockf(f,fcntl.LOCK_EX|fcntl.LOCK_NB);fcntl.lockf(f,fcntl.LOCK_UN);os.unlink(p)' 2>/dev/null; ok $?)" "fcntl lock"
S fs inotify "$(python3 -c 'import ctypes;l=ctypes.CDLL(None);raise SystemExit(0 if l.inotify_init1(0)>=0 else 1)' 2>/dev/null; ok $?)" "inotify_init1"
S fs xattr "$(python3 -c 'import os;os.setxattr("/tmp",b"user.husk",b"1");os.removexattr("/tmp",b"user.husk")' 2>/dev/null; ok $?)" "user. xattr on /tmp"
S fs otmpfile "$(python3 -c 'import os;os.close(os.open("/tmp",os.O_TMPFILE|os.O_RDWR,0o600))' 2>/dev/null; ok $?)" "O_TMPFILE"
S fs sparse "$(python3 -c 'f=open("/tmp/.sp","wb");f.truncate(1<<30);f.close()' 2>/dev/null && rm -f /tmp/.sp; ok $?)" "1 GiB truncate"
S fs hardlink PARTIAL "$(mkdir -p /var/tmp/hl && echo x >/var/tmp/hl/a && ln /var/tmp/hl/a /var/tmp/hl/b 2>/dev/null; ok $?) nlink=$(stat -c %h /var/tmp/hl/a 2>/dev/null) (kernel denies link(2); emulated as copy)"
S fs mmap_shared "$(python3 -c 'import mmap;f=open("/tmp/.mm","w+b");f.truncate(4096);m=mmap.mmap(f.fileno(),4096);m[0:2]=b"ok";m.flush();assert m[0:2]==b"ok"' 2>/dev/null && rm -f /tmp/.mm; ok $?)" "MAP_SHARED + flush"

# ------------------------------------------------------------------- ipc -----
S ipc unix_stream "$(rm -f /tmp/.ux; python3 -c 'import socket;s=socket.socket(socket.AF_UNIX);s.bind("/tmp/.ux");s.listen(1);c=socket.socket(socket.AF_UNIX);c.connect("/tmp/.ux");a,_=s.accept()' 2>/dev/null; ok $?)" "AF_UNIX pathname"
S ipc unix_abstract "$(python3 -c 'import socket;s=socket.socket(socket.AF_UNIX);s.bind("\0huskabs")' 2>/dev/null; ok $?)" "abstract namespace"
S ipc fdpass "$(python3 -c 'import socket;a,b=socket.socketpair();f=open("/etc/hostname");sent=socket.send_fds(a,[b"x"],[f.fileno()]);got=socket.recv_fds(b,16,1);assert got[1], got' 2>/dev/null; ok $?)" "SCM_RIGHTS"
S ipc sysv_shm "$(python3 -c 'import ctypes;l=ctypes.CDLL(None,use_errno=True);raise SystemExit(0 if l.shmget(0x1234,4096,0o1600)>=0 else 1)' 2>/dev/null; ok $?)" "shmget (Android seccomp answers ENOSYS)"
S ipc posix_mq "$(python3 -c 'import ctypes;l=ctypes.CDLL(None,use_errno=True);raise SystemExit(0 if l.mq_open(b"/huskq",0x42,0o600,None)!=-1 else 1)' 2>/dev/null; ok $?)" "mq_open"
S ipc eventfd "$(python3 -c 'import os;os.close(os.eventfd(0))' 2>/dev/null; ok $?)" "eventfd"
S ipc timerfd "$(python3 -c 'import ctypes;raise SystemExit(0 if ctypes.CDLL(None).timerfd_create(1,0)>=0 else 1)' 2>/dev/null; ok $?)" "timerfd_create"
S ipc epoll "$(python3 -c 'import select;select.epoll().close()' 2>/dev/null; ok $?)" "epoll"
S ipc pty "$(python3 -c 'import pty;pty.openpty()' 2>/dev/null; ok $?)" "openpty (/dev/pts)"
S ipc fifo "$(rm -f /tmp/.fifo; mkfifo /tmp/.fifo 2>/dev/null; s=$?; rm -f /tmp/.fifo; ok $s)" "mkfifo"

# --------------------------------------------------------------- network -----
S net resolv_conf "$([ -s /etc/resolv.conf ]; ok $?)" "$(head -1 /etc/resolv.conf 2>/dev/null)"
S net getent_dns "$(getent hosts baidu.com >/dev/null 2>&1; ok $?)" "$(getent hosts baidu.com 2>/dev/null | head -1)"
S net getaddrinfo "$(python3 -c 'import socket;socket.getaddrinfo("example.com",443)' 2>/dev/null; ok $?)" "glibc resolver + nss_dns"
S net tcp_outbound "$(python3 -c 'import socket;socket.setdefaulttimeout(5);s=socket.create_connection(("223.5.5.5",443))' 2>/dev/null; ok $?)" "TCP to a public address"
S net tcp_listen "$(python3 -c 'import socket;s=socket.socket();s.bind(("0.0.0.0",0));s.listen(1)' 2>/dev/null; ok $?)" "bind 0.0.0.0 (unprivileged port)"
S net loopback "$(python3 -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));s.listen(1);c=socket.create_connection(s.getsockname());a,_=s.accept()' 2>/dev/null; ok $?)" "loopback"
S net udp "$(python3 -c 'import socket;s=socket.socket(socket.AF_INET,socket.SOCK_DGRAM);s.bind(("127.0.0.1",0));s.sendto(b"x",s.getsockname());s.settimeout(2);s.recvfrom(4)' 2>/dev/null; ok $?)" "UDP loopback"
S net ipv6 "$(python3 -c 'import socket;socket.socket(socket.AF_INET6,socket.SOCK_STREAM)' 2>/dev/null; ok $?)" "AF_INET6"
S net hostname "$(hostname >/dev/null 2>&1; ok $?)" "$(hostname 2>&1)"
S net proc_net "$(head -c 200 /proc/net/dev >/dev/null 2>&1; ok $?)" "$(sed -n 3p /proc/net/dev 2>/dev/null | tr -s ' ')"
S net sys_class_net "$(ls /sys/class/net >/dev/null 2>&1; ok $?)" "$(ls /sys/class/net 2>/dev/null | tr '\n' ' ')"
S net netlink "$(python3 -c 'import socket;s=socket.socket(socket.AF_NETLINK,socket.SOCK_RAW,0);s.bind((0,0))' 2>/dev/null; ok $?)" "netlink bind (ip addr enumerates interfaces)"
S net icmp PARTIAL "ping needs CAP_NET_RAW ($(ping -c1 -W1 127.0.0.1 2>&1 | tail -1 | cut -c1-40))"

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
for d in null zero random urandom tty ptmx pts/0 shm full mqueue log; do
  [ -e "/dev/$d" ] && S dev "/dev/$d" PASS "$(stat -c '%A %s' "/dev/$d" 2>/dev/null)" || S dev "/dev/$d" FAIL missing
done
S dev shm_write "$(printf x > /dev/shm/.p 2>/dev/null; rm -f /dev/shm/.p; ok $?)" "/dev/shm writable"

# -------------------------------------------------------------- services -----
S svc dbus_system "$([ -S /run/dbus/system_bus_socket ]; ok $?)" "system bus socket"
S svc dbus_session "$(python3 -c '
import subprocess,os
p=subprocess.run(["dbus-daemon","--session","--fork","--print-address"],capture_output=True,text=True)
raise SystemExit(0 if p.stdout.strip().startswith("unix:") else 1)' 2>/dev/null; ok $?)" "session bus starts"
S svc syslog "$( [ -S /dev/log ] && logger -t husk "parity probe" 2>/dev/null; ok $?)" "logger -> $(tail -1 /var/log/syslog 2>/dev/null | cut -c1-70)"
# syslog(3) reaches connect(2) through glibc's *internal* alias, which has no GOT
# slot, so it is not translated yet. logger works because util-linux builds and
# connects the socket itself. Fix = add connect/bind to the libc slot list.
S svc syslog_libc PARTIAL "syslog(3) from libc still misses; the sink and logger are fine"
S svc systemd PARTIAL "systemd binary present but needs PID 1 + libsystemd-core (not shipped)"
S svc su PARTIAL "no CAP_SETUID"

# -------------------------------------------------------------- packages -----
S pkg dpkg "$(dpkg --version >/dev/null 2>&1; ok $?)" "$(dpkg --version 2>/dev/null | head -1)"
S pkg apt "$(apt --version >/dev/null 2>&1; ok $?)" "$(apt --version 2>/dev/null | head -1)"
S pkg apt_lists "$([ "$(ls /var/lib/apt/lists 2>/dev/null | wc -l)" -gt 5 ]; ok $?)" "$(ls /var/lib/apt/lists 2>/dev/null | wc -l) index files"
S pkg tz "$([ -f /usr/share/zoneinfo/Asia/Shanghai ]; ok $?)" "$(TZ=Asia/Shanghai date 2>&1)"
S pkg locale "$([ "$(locale -a 2>/dev/null | wc -l)" -gt 2 ]; ok $?)" "$(locale -a 2>/dev/null | tr '\n' ' ')"
echo DONE
