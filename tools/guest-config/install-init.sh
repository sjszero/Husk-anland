#!/bin/sh
# Guest-side installer for husk-init, the service supervisor.
#
# Run inside the guest after the binary has been copied to /usr/bin/husk-init
# (build it with tools/guest-config/build-init.sh). It only creates the places
# the supervisor needs and a commented template config; an existing
# /etc/husk/services.conf is never touched.
set -e
BIN=/usr/bin/husk-init
[ -x "$BIN" ] || { echo "install-init: $BIN missing or not executable"; exit 1; }
mkdir -p /etc/husk /var/log/husk
chmod 755 /var/log/husk
if [ ! -f /etc/husk/services.conf ]; then
    cat > /etc/husk/services.conf <<'CONF'
# Husk services. One per line: name policy program [args...]
#   policy: always | on-failure | once
# Arguments are split on whitespace; there is no quoting, so put anything
# complicated in a script. Program paths are guest absolute paths.
#
# Example:
# ticker   always      /usr/bin/bash /usr/local/bin/ticker.sh
CONF
    echo "install-init: wrote a template /etc/husk/services.conf"
fi
echo "install-init: ok ($BIN, $(wc -c < "$BIN") bytes)"
