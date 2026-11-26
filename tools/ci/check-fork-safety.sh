#!/bin/sh
# The App is multithreaded when it forks. Between fork() and execve()/exit only
# async-signal-safe calls are legal; even a Rust Vec allocation can deadlock if
# another thread held the allocator lock at fork time. Keep this audit on every
# child path, not only the product PTY.
set -eu
cd "$(dirname "$0")/../.."

check_block() {
    file=$1
    start=$2
    end=$3
    label=$4
    block=$(awk -v start="$start" -v end="$end" '
        /if .*== 0 \{/ && $0 ~ start { inside = 1 }
        inside {
            line = $0
            sub(/\/\/.*/, "", line)
            print line
        }
        inside && $0 ~ end { exit }
    ' "$file")
    [ -n "$block" ] || { echo "fork-safety: child block not found: $label" >&2; exit 1; }
    case "$block" in
        *execve\(*|*_exit\(*) ;;
        *) echo "fork-safety: child block has no execve/_exit: $label" >&2; exit 1 ;;
    esac
    for pattern in 'Vec::' 'vec!' 'format!' 'CString::new' '.collect(' '.to_string(' '.to_owned(' 'Box::' 'String::' 'println!' 'eprintln!'; do
        if printf '%s\n' "$block" | grep -F "$pattern" >/dev/null; then
            echo "fork-safety: allocating call '$pattern' after fork in $label" >&2
            exit 1
        fi
    done
}

check_block app-rs/src/term.rs 'guard_pid == 0 \\{' 'execve\\(guard_c' 'session guard'
check_block app-rs/src/term.rs 'if pid == 0 \\{' 'execve\\(loader_c' 'guest PTY leader'
check_block app-rs/src/lib.rs 'if pid == 0 \\{' '_exit\\(' 'SIGSYS semantics probe'
check_block app-rs/src/trace.rs 'if pid == 0 \\{' 'execve\\(loader_c' 'trace child'

echo "fork safety: all child exec/exit paths have no allocations"
