#!/bin/sh
# Xlorie being loaded and fork() succeeding is not the same as GNOME being
# ready. Keep an explicit starting -> ready/failed contract between the guest
# launcher, the Rust monitor and the capability matrix.
set -eu
cd "$(dirname "$0")/../.."

require() {
    pattern=$1
    file=$2
    if ! grep -F "$pattern" "$file" >/dev/null 2>&1; then
        echo "desktop session contract: FAIL missing '$pattern' in $file" >&2
        exit 1
    fi
}

require 'status_write "starting"' tools/guest/husk-desktop.sh
require 'status_write "ready"' tools/guest/husk-desktop.sh
require 'status_write "failed"' tools/guest/husk-desktop.sh
require 'monitor_readiness(status, child_pid);' app-rs/src/desktop.rs
require 'desktop: launcher failed' app-rs/src/desktop.rs
require '"x11=ready"' app-rs/src/caps.rs
require '"x11=starting"' app-rs/src/caps.rs
require 'mit_shm=disabled' app-rs/src/desktop.rs
require '"-extension"' app-rs/src/desktop.rs
require '"MIT-SHM"' app-rs/src/desktop.rs

echo "desktop session contract: PASS (starting -> ready/failed -> caps -> MIT-SHM off)"
