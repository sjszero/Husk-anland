#!/bin/sh
# The embedded X11 server exposes touch/key/pointer ABI, but it is useless if
# Android input callbacks still send every event only to the terminal/socket
# path. Keep the routing and multi-pointer FFI wired into the app build.
set -eu
cd "$(dirname "$0")/../.."

require() {
    pattern=$1
    file=$2
    if ! grep -F "$pattern" "$file" >/dev/null 2>&1; then
        echo "desktop input routing: FAIL missing '$pattern' in $file" >&2
        exit 1
    fi
}

require 'if desktop::active()' app-rs/src/lib.rs
require 'desktop::key(code as i32, down);' app-rs/src/lib.rs
require 'forward_desktop_motion(ev, raw_action);' app-rs/src/lib.rs
require 'forward_desktop_mouse(ev, raw_action);' app-rs/src/lib.rs
require 'desktop::mouse(dx, dy, 0, false, true);' app-rs/src/lib.rs
require 'AMotionEvent_getPointerId' app-rs/src/sys.rs
require 'AMOTION_EVENT_ACTION_POINTER_DOWN' app-rs/src/sys.rs
require 'AINPUT_SOURCE_MOUSE_RELATIVE' app-rs/src/sys.rs

echo "desktop input routing: PASS (touch + mouse + multi-pointer + key edges)"
