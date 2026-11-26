#!/bin/sh
# Exercise the Android-side Rust session guard's cleanup truthfulness.
set -eu
DEV=${DEV:-${1:-}}
[ -n "$DEV" ] || { echo "usage: $0 <device-serial>" >&2; exit 1; }
GUARD=${GUARD:-app/app/src/main/jniLibs/arm64-v8a/libhuskguard.so}
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
case "$GUARD" in
    /*) ;;
    *) GUARD="$ROOT/$GUARD" ;;
esac
TMP=/data/local/tmp
SESSION="guard-probe-$$-$(date +%s)"
VALID_LOG="$TMP/husk-guard-valid.log"
MISSING_LOG="$TMP/husk-guard-missing.log"
MISSING_PROC="$TMP/husk-guard-proc-missing"
FD_READY="$TMP/husk-guard-fd-ready"
FD_LEASE="$TMP/husk-guard-fd-lease"
FD_LOG="$TMP/husk-guard-fd.log"

cleanup() {
    adb -s "$DEV" shell "rm -f '$TMP/husk-session-guard-probe' '$VALID_LOG' '$MISSING_LOG' '$FD_READY' '$FD_LEASE' '$FD_LOG'" >/dev/null 2>&1 || true
}
trap cleanup EXIT HUP INT TERM

adb -s "$DEV" push "$GUARD" "$TMP/husk-session-guard-probe" >/dev/null
adb -s "$DEV" shell "chmod 755 '$TMP/husk-session-guard-probe'; rm -rf '$MISSING_PROC' '$VALID_LOG' '$MISSING_LOG' '$FD_READY' '$FD_LEASE' '$FD_LOG'"

VALID=$(adb -s "$DEV" shell "setsid env NR_SESSION_ID=$SESSION sleep 60 >/dev/null 2>&1 & p=\$!; sleep 1; '$TMP/husk-session-guard-probe' --lease-fd 3 --session-id '$SESSION' --proc-root /proc --test-lease-eof --log '$VALID_LOG'; rc=\$?; sleep 1; if kill -0 \$p 2>/dev/null; then echo GUARD_MARKER=alive; kill -9 \$p 2>/dev/null || true; else echo GUARD_MARKER=gone; fi; echo GUARD_VALID_RC=\$rc; cat '$VALID_LOG'")
printf '%s\n' "$VALID"
printf '%s\n' "$VALID" | grep -q 'GUARD_VALID_RC=0'
printf '%s\n' "$VALID" | grep -q 'GUARD_MARKER=gone'
printf '%s\n' "$VALID" | grep -q 'clean=true'

MISSING=$(adb -s "$DEV" shell "'$TMP/husk-session-guard-probe' --lease-fd 3 --session-id '$SESSION-missing' --proc-root '$MISSING_PROC' --test-lease-eof --log '$MISSING_LOG'; rc=\$?; echo GUARD_MISSING_RC=\$rc; cat '$MISSING_LOG'")
printf '%s\n' "$MISSING"
printf '%s\n' "$MISSING" | grep -q 'GUARD_MISSING_RC=70'
printf '%s\n' "$MISSING" | grep -q 'marker scan failed errno=2'
printf '%s\n' "$MISSING" | grep -q 'remaining=UNKNOWN'
if printf '%s\n' "$MISSING" | grep -q 'remaining=\[\]'; then
    echo '--- session-guard-probe: FAIL (scan failure reported as empty)' >&2
    exit 1
fi
# The App remaps the guard pipes onto 3/4. Exercise the inverse layout too:
# ready=3 and lease=4, with both descriptors already at the destination numbers.
FD34=$(adb -s "$DEV" shell "cd '$TMP'; : > '$FD_READY'; : > '$FD_LEASE'; ./husk-session-guard-probe --lease-fd 4 --ready-fd 3 --session-id '$SESSION-fd34' --proc-root '$MISSING_PROC' --test-lease-eof --log '$FD_LOG' 3>'$FD_READY' 4<'$FD_LEASE'; rc=\$?; printf 'GUARD_FD34_RC=%s\n' \$rc; r=\$(cat '$FD_READY'); printf 'GUARD_FD34_READY=%s\n' \$r; cat '$FD_LOG'")
printf '%s\n' "$FD34"
printf '%s\n' "$FD34" | grep -q 'GUARD_FD34_RC=70'
printf '%s\n' "$FD34" | grep -q 'GUARD_FD34_READY=R'

echo '--- session-guard-probe: PASS'
