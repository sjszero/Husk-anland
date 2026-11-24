#!/bin/sh
# Runner: Husk. In-process loader + path shim, no ptrace, no namespace.
# Usage (on device): husk-run.sh '<command run inside the guest root>'
B=/data/user/0/com.linuxhub.myth.husk/files/linux
exec "$B/xe.sh" bin/sh -c "cd /root 2>/dev/null || cd /; exec $*"
