#!/bin/sh
# Keep the repository landing page to title, mark, and badges.
#
# The project description starts at "## 概览".  A plain paragraph or heading in
# the HTML hero block makes the GitHub landing page drift into an introduction.
set -eu
cd "$(dirname "$0")/../.."

awk '
    BEGIN {
        in_comment = 0
        first = ""
        h1 = 0
        images = 0
        badges = 0
        overview = 0
        bad = 0
    }
    in_comment {
        if ($0 ~ /-->/) in_comment = 0
        next
    }
    /^[[:space:]]*<!--/ {
        if ($0 !~ /-->/) in_comment = 1
        next
    }
    /^##[[:space:]]+概览[[:space:]]*$/ {
        overview = 1
        exit
    }
    /^[[:space:]]*$/ { next }
    $0 == "<h1 align=\"center\">Husk</h1>" {
        h1++
        if (first == "") first = "h1"
        next
    }
    $0 == "<p align=\"center\">" { next }
    $0 == "</p>" { next }
    /^[[:space:]]*<img[[:space:]]/ {
        images++
        next
    }
    /^[[:space:]]*<a[[:space:]]/ {
        badges++
        next
    }
    /^[[:space:]]*<\/a>[[:space:]]*$/ { next }
    {
        print "README header: unexpected content before ## 概览: " $0
        bad = 1
    }
    END {
        if (!overview) {
            print "README header: missing ## 概览"
            bad = 1
        }
        if (h1 != 1) {
            print "README header: expected exactly one Husk h1, found " h1
            bad = 1
        }
        if (first != "h1") {
            print "README header: the first visible element must be the Husk h1"
            bad = 1
        }
        if (images < 1) {
            print "README header: missing Husk mark"
            bad = 1
        }
        if (badges < 1) {
            print "README header: missing badges"
            bad = 1
        }
        exit bad
    }
' README.md

echo "README header: PASS"
