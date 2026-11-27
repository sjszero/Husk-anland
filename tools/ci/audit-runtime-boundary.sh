#!/bin/sh
# Assert that the distributable Husk bundle is a runtime asset, not a host UI.
#
# The host App owns its X server, window, input and Surface. Husk ships the
# loader/path-shim, patched libc, guest tools, installer and session support.
# An Xorg/Xvfb/Xwayland/libXlorie binary in this bundle means the host/runtime
# boundary has regressed, even if the archive otherwise installs correctly.
set -eu
cd "$(dirname "$0")/../.."

bundle=${1:?usage: audit-runtime-boundary.sh <husk-arm64.tar.gz>}
[ -f "$bundle" ] || { echo "runtime boundary: missing bundle: $bundle" >&2; exit 1; }

tmp=$(mktemp "${TMPDIR:-/tmp}/husk-runtime-boundary.XXXXXX")
trap 'rm -f "$tmp"' EXIT HUP INT TERM
tar tzf "$bundle" | sed 's#^\./##' > "$tmp"

if awk '
    {
        p = $0
        sub(/^\.\//, "", p)
        if (p ~ /(^|\/)(libXlorie\.so|Xorg|Xorg\.bin|Xvfb|Xwayland)$/ ||
            p ~ /(^|\/)xlorie(\/|$)/ ||
            p ~ /\.apk$/) {
            print "  forbidden: " p > "/dev/stderr"
            bad = 1
        }
    }
    END { exit bad ? 1 : 0 }
' "$tmp"; then
    :
else
    echo "runtime boundary: host UI/X server leaked into $bundle" >&2
    exit 1
fi

for required in \
    libld.so \
    libc.so.6 \
    husk-ldconfig \
    husk-usermgr \
    husk-su \
    husk-symlinks \
    husk-logd \
    install-husk.sh \
    husk-session.sh \
    guest-config/husk-gpu-defaults.sh \
    guest-config/mksys-net.sh; do
    if ! grep -Fqx "$required" "$tmp"; then
        echo "runtime boundary: required runtime asset missing: $required" >&2
        exit 1
    fi
done

echo "runtime boundary: PASS ($bundle)"
