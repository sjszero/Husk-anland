#!/bin/sh
# Semantic checks for the shell-side locale default. `sh -n` cannot catch a
# gettext regression where LANG=en_US is followed by LANGUAGE=zh_CN:zh.
set -eu
cd "$(dirname "$0")/../.."
LOCALE_SH=$PWD/tools/guest-config/husk-locale.sh

run_case() {
    label=$1
    expected=$2
    shift 2
    got=$(env -i "$@" /bin/sh -c '. "$1"; printf "%s|%s" "$LANG" "${LANGUAGE-}"' sh "$LOCALE_SH")
    if [ "$got" != "$expected" ]; then
        echo "FAIL $label: got=$got expected=$expected" >&2
        exit 1
    fi
    echo "PASS $label: $got"
}

run_case "unset defaults to Chinese" "zh_CN.UTF-8|zh_CN:zh"
run_case "C.UTF-8 defaults to Chinese" "zh_CN.UTF-8|zh_CN:zh" LANG=C.UTF-8
run_case "explicit English LANG stays English" "en_US.UTF-8|" LANG=en_US.UTF-8
run_case "explicit LANGUAGE is preserved" "en_US.UTF-8|en_US:en" \
    LANG=en_US.UTF-8 LANGUAGE=en_US:en
