#!/bin/sh
# Add the accounts the rootfs' service configuration expects. Idempotent.
set -e
R=${1:?usage: provision-accounts.sh <rootfs>}
while IFS=: read -r name uid gid extra; do
    case "$name" in ''|'#'*) continue;; esac
    if [ -n "$gid" ]; then
        grep -q "^$name:" "$R/etc/group" || echo "$name:x:$gid:" >> "$R/etc/group"
    fi
    if [ -n "$uid" ]; then
        grep -q "^$name:" "$R/etc/passwd" || \
            echo "$name:x:$uid:$gid::$([ "$name" = pulse ] && echo /var/run/pulse || echo /nonexistent):/usr/sbin/nologin" >> "$R/etc/passwd"
        grep -q "^$name:" "$R/etc/shadow" || echo "$name:!:20000:0:99999:7:::" >> "$R/etc/shadow"
    fi
done < "$(dirname "$0")/guest-accounts.txt"
echo "accounts: ok"
