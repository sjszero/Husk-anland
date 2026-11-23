#!/bin/bash
# Server-surface parity probe.
#
# The question this answers: "a container/VM ships a complete Linux userspace —
# how much of that surface do we actually have?"  Everything is measured inside
# the guest, from the app uid, with no root.  One line per check:
#
#   R|<area>|<name>|PASS|PARTIAL|FAIL|<detail>
#
# PASS   = the interface exists and behaves like on a normal server
# PARTIAL= it answers, but with something wrong (wrong numbers, degraded path)
# FAIL   = missing or refused
S() { printf 'R|%s|%s|%s|%s\n' "$1" "$2" "$3" "$4"; }

# ---------------------------------------------------------------- identity ---
S id uid "$(id -u >/dev/null 2>&1 && echo PASS || echo FAIL)" "$(id 2>&1)"
cap=$(awk '/^CapEff/{print $2}' /proc/self/status 2>/dev/null)
S id capability "$([ -n "$cap" ] && echo PASS || echo FAIL)" "CapEff=$cap"
S id passwd "$(grep -q '^root:' /etc/passwd 2>/dev/null && echo PASS || echo FAIL)" "$(getent passwd root 2>/dev/null)"
S id shadow "$([ -r /etc/shadow ] && echo PASS || echo FAIL)" "$(head -1 /etc/shadow 2>/dev/null | cut -c1-20)"
( /usr/bin/passwd --version >/dev/null 2>&1; echo $? >/tmp/.suidret ) ; S id setuid PARTIAL "passwd rc=$(cat /tmp/.suidret 2>/dev/null)"
S id nsswitch "$(grep -q '^hosts:' /etc/nsswitch.conf 2>/dev/null && echo PASS || echo FAIL)" "$(grep '^hosts:' /etc/nsswitch.conf 2>/dev/null)"

# ---------------------------------------------------------------- filesystem -
S fs df_root PARTIAL "$(df -h / 2>&1 | tail -1 | tr -s ' ')"
S fs df_inode PARTIAL "$(df -i / 2>&1 | tail -1 | tr -s ' ')"
S fs df_shm "$(df -h /dev/shm 2>/dev/null | tail -1 | tr -s ' ')"
S fs statvfs_clash PARTIAL "guest-root statvfs=$(python3 -c 'import os;s=os.statvfs("/");print(int(s.f_blocks*s.f_frsize/1048576),"MB")' 2>&1)"
S fs flock "$(python3 -c 'import fcntl,os;p="/tmp/.fl";f=open(p,"w");fcntl.flock(f,fcntl.LOCK_EX|fcntl.LOCK_NB);fcntl.flock(f,fcntl.LOCK_UN);os.unlink(p);print("ok")' 2>&1)"
S fs posixlock "$(python3 -c 'import fcntl,os;p="/tmp/.pl";f=open(p,"w");fcntl.lockf(f,fcntl.LOCK_EX|fcntl.LOCK_NB);fcntl.lockf(f,fcntl.LOCK_UN);os.unlink(p);print("ok")' 2>&1)"
S fs inotify "$(python3 -c 'import ctypes;l=ctypes.CDLL(None);fd=l.inotify_init1(0);print("fd",fd) if fd>=0 else print("err",ctypes.get_errno())' 2>&1)"
S fs xattr "$(python3 -c 'import os;os.setxattr("/tmp","user.husk","1");v=os.getxattr("/tmp","user.husk");os.removexattr("/tmp","user.husk");print(v)' 2>&1)"
S fs otmpfile "$(python3 -c 'import os;fd=os.open("/tmp",os.O_TMPFILE|os.O_RDWR,0o600);os.close(fd);print("ok")' 2>&1)"
S fs sparse "$(python3 -c 'f=open("/tmp/.sp","wb");f.truncate(1<<30);f.close();print("1GiB")' 2>&1 && rm -f /tmp/.sp)"
S fs hardlink "$(echo a >/tmp/.h1 && ln /tmp/.h1 /tmp/.h2 && echo ok && rm -f /tmp/.h1 /tmp/.h2)"
S fs mmap_shared "$(python3 -c 'import mmap,os;f=open("/tmp/.mm","w+b");f.truncate(4096);m=mmap.mmap(f.fileno(),4096);m[0:2]=b"ok";m.flush();print(m[0:2])' 2>&1 && rm -f /tmp/.mm)"
S fs tmp_sticky "$(ls -ld /tmp 2>&1 | awk '{print $1}')"
S fs own_usage UNKNOWN "guest df total=$(df -h / 2>/dev/null | tail -1 | awk '{print $2}')"

# ---------------------------------------------------------------- ipc --------
S ipc unix_stream "$(python3 -c 'import socket,os;s=socket.socket(socket.AF_UNIX);s.bind("/tmp/.ux");s.listen(1);c=socket.socket(socket.AF_UNIX);c.connect("/tmp/.ux");a,_=s.accept();print("ok")' 2>&1)"
S ipc unix_abstract "$(python3 -c 'import socket;s=socket.socket(socket.AF_UNIX);s.bind("\0huskabs");print("ok")' 2>&1)"
S ipc fdpass "$(python3 -c 'import socket,array;a,b=socket.socketpair();f=open("/etc/hostname");socket.send_fds(a,b"x",[f.fileno()]);print("ok")' 2>&1)"
S ipc sysv_shm "$(python3 -c 'import ctypes,os;l=ctypes.CDLL(None,use_errno=True);i=l.shmget(0x1234,4096,0o600|0x200|0o1000);e=ctypes.get_errno();print("id",i,"errno",e)' 2>&1)"
S ipc sysv_proc "$([ -r /proc/sysvipc/shm ] && echo PASS || echo FAIL)" "$(head -1 /proc/sysvipc/shm 2>&1)"
S ipc posix_mq "$(python3 -c 'import ctypes;l=ctypes.CDLL(None,use_errno=True);q=l.mq_open(b"/huskq",0x42,0o600,None);print("q",q,"errno",ctypes.get_errno())' 2>&1)"
S ipc eventfd "$(python3 -c 'import os;fd=os.eventfd(0);os.close(fd);print("ok")' 2>&1)"
S ipc timerfd "$(python3 -c 'import ctypes;l=ctypes.CDLL(None);print("fd",l.timerfd_create(1,0))' 2>&1)"
S ipc epoll "$(python3 -c 'import select;e=select.epoll();e.close();print("ok")' 2>&1)"
S ipc pty "$(python3 -c 'import pty,os;m,s=pty.openpty();print("ok",os.ttyname(s))' 2>&1)"
S ipc fifo "$(mkfifo /tmp/.fifo 2>/dev/null && echo ok && rm -f /tmp/.fifo)"

# ---------------------------------------------------------------- network ----
S net resolv_conf FAIL "$(head -3 /etc/resolv.conf 2>&1 | tr '\n' ' ')"
g=$(getent hosts example.com 2>&1 | head -1); case "$g" in *example.com*) S net getent_dns PASS "$g";; *) S net getent_dns FAIL "${g:-no answer}";; esac
g=$(python3 -c 'import socket;print(socket.getaddrinfo("example.com",443)[0][4])' 2>&1 | tail -1); case "$g" in *.*) S net getaddrinfo PASS "$g";; *) S net getaddrinfo FAIL "$g";; esac
g=$(python3 -c 'import socket;socket.setdefaulttimeout(5);c=socket.create_connection(("1.1.1.1",443));print("connected",c.getpeername())' 2>&1 | tail -1); case "$g" in *connected*) S net tcp_outbound PASS "$g";; *) S net tcp_outbound FAIL "$g";; esac
S net tcp_listen_all "$(python3 -c 'import socket;s=socket.socket();s.bind(("0.0.0.0",0));s.listen(1);print("port",s.getsockname()[1])' 2>&1)"
S net udp "$(python3 -c 'import socket;s=socket.socket(socket.AF_INET,socket.SOCK_DGRAM);s.bind(("127.0.0.1",0));c=socket.socket(socket.AF_INET,socket.SOCK_DGRAM);c.sendto(b"x",s.getsockname());d,_=s.recvfrom(4);print("ok",d)' 2>&1)"
S net ipv6 "$(python3 -c 'import socket;s=socket.socket(socket.AF_INET6,socket.SOCK_STREAM);print("created")' 2>&1)"
S net hostname "$(hostname 2>&1)"
S net proc_net "$([ -r /proc/net/dev ] && echo PASS || echo FAIL)" "$(head -2 /proc/net/dev 2>&1 | tr '\n' ' ')"
S net sys_class_net "$([ -d /sys/class/net ] && ls /sys/class/net 2>&1 | tr '\n' ' ')"
S net ip_addr "$(ip -o addr 2>&1 | head -2 | tr '\n' ' ')"
S net ping "$(ping -c1 -W2 127.0.0.1 2>&1 | tail -2 | tr '\n' ' ')"

# ---------------------------------------------------------------- proc/sys ---
for f in /proc/stat /proc/loadavg /proc/uptime /proc/filesystems /proc/partitions /proc/locks /proc/self/maps /proc/self/environ /proc/self/fd; do
  [ -r "$f" ] && S proc "$f" PASS "$(head -c 40 "$f" 2>/dev/null | tr '\n' ' ')" || S proc "$f" FAIL unreadable
done
for f in /proc/sys/vm/max_map_count /proc/sys/vm/overcommit_memory /proc/sys/kernel/pid_max /proc/sys/kernel/hostname /proc/sys/net/ipv4/ip_local_port_range /proc/sys/net/core/somaxconn; do
  [ -r "$f" ] && S sysctl "$f" PASS "$(head -1 "$f" 2>/dev/null)" || S sysctl "$f" FAIL unreadable
done
S sysctl write_denied "$( (echo 1 > /proc/sys/kernel/pid_max) 2>&1 || echo 'denied as expected')"
S sys devnull_sysrq "$( (echo b > /proc/sysrq-trigger) 2>&1 || echo 'denied as expected')"
S sys fs_cgroup "$([ -d /sys/fs/cgroup ] && echo PASS || echo FAIL)" "$(ls /sys/fs/cgroup 2>&1 | head -5 | tr '\n' ' ')"
S sys dev_system_cpu "$([ -r /sys/devices/system/cpu/online ] && echo PASS || echo FAIL)" "$(cat /sys/devices/system/cpu/online 2>&1)"
S sys hwmon "$([ -d /sys/class/hwmon ] && ls /sys/class/hwmon 2>&1 | tr '\n' ' ' || echo missing)"
S sys pid1_hidden UNKNOWN "$(head -c 60 /proc/1/cmdline 2>&1 | tr '\0' ' ')"

# ---------------------------------------------------------------- /dev -------
for d in null zero full random urandom tty ptmx pts/0 shm mqueue log kmsg; do
  [ -e "/dev/$d" ] && S dev "/dev/$d" PASS "$(ls -ld "/dev/$d" 2>&1 | awk '{print $1, $5}')" || S dev "/dev/$d" FAIL missing
done
S dev shm_write "$( (printf x > /dev/shm/.p && echo ok) 2>&1 )"
S dev log_socket "$( (logger -t husk hi) 2>&1 || echo 'logger failed')"

# ---------------------------------------------------------------- services ---
S svc dbus_system "$([ -S /run/dbus/system_bus_socket ] && echo PASS || echo FAIL)" "$(ls -l /run/dbus 2>&1 | head -2 | tr '\n' ' ')"
S svc dbus_session "$(python3 -c '
import subprocess,os,time
d=os.environ.get("XDG_RUNTIME_DIR","/tmp")
p=subprocess.run(["dbus-daemon","--session","--fork","--print-address"],capture_output=True,text=True)
print(p.stdout.strip() or p.stderr.strip())' 2>&1 | tail -1)"
S svc systemd_binary "$(systemd --version 2>&1 | head -1)"
S svc su_binary "$(su -c true 2>&1 | head -1 || true)"

# ---------------------------------------------------------------- packages ---
S pkg dpkg "$(dpkg --version 2>&1 | head -1)"
S pkg apt "$(apt --version 2>&1 | head -1)"
S pkg dpkg_db "$([ -w /var/lib/dpkg ] && echo PASS || echo FAIL)" "$(ls /var/lib/dpkg 2>&1 | tr '\n' ' ')"
S pkg tzdata "$([ -f /usr/share/zoneinfo/Asia/Shanghai ] && echo PASS || echo FAIL)" "$(TZ=Asia/Shanghai date 2>&1)"
S pkg locale "$(locale 2>&1 | head -2 | tr '\n' ' ')"
S pkg apt_lists "$(ls /var/lib/apt/lists 2>/dev/null | wc -l) files"
echo "DONE"
