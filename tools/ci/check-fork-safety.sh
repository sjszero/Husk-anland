#!/bin/sh
# The App is multithreaded when it forks the guest leader. Between fork() and
# execve() only async-signal-safe calls are legal; even a Rust Vec allocation
# can deadlock if another thread held the allocator lock at fork time.
set -eu
cd "$(dirname "$0")/../.."

block=$(awk '
    /if pid == 0 \{/ { inside = 1 }
    inside {
        line = $0
        sub(/\/\/.*/, "", line)
        print line
    }
    inside && /execve\(loader_c/ { exit }
' app-rs/src/term.rs)

[ -n "$block" ] || { echo "fork-safety: child exec block not found" >&2; exit 1; }

case "$block" in
    *execve\(*) ;;
    *) echo "fork-safety: child block has no execve" >&2; exit 1 ;;
esac

for pattern in 'Vec::' 'vec!' 'format!' 'CString::new' '.collect(' '.to_string(' '.to_owned('; do
    if printf '%s\n' "$block" | grep -F "$pattern" >/dev/null; then
        echo "fork-safety: allocating call '$pattern' after fork" >&2
        exit 1
    fi
done

echo "fork safety: child exec path has no allocations"
