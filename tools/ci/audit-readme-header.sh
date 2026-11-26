#!/bin/sh
# Enforce the repository landing-page hero contract.
#
# Exact order before "## 概览":
#   1. the single centered <h1>Husk</h1>;
#   2. one centered paragraph containing exactly one icon <img>;
#   3. one centered paragraph containing badges only;
#   4. "## 概览", where project prose begins.
#
# This keeps a project introduction, theme line, or version text out of the
# title page. HTML comments and blank lines are ignored.
set -eu
cd "$(dirname "$0")/../.."

awk '
    function reject(message) {
        print "README header: " message > "/dev/stderr"
        bad = 1
        exit
    }
    BEGIN {
        in_comment = 0
        stage = 0
        icon_images = 0
        badges = 0
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
    /^[[:space:]]*$/ { next }

    stage == 0 {
        if ($0 != "<h1 align=\"center\">Husk</h1>")
            reject("first visible element must be the Husk h1")
        stage = 1
        next
    }
    stage == 1 {
        if ($0 != "<p align=\"center\">")
            reject("the icon must be in the first centered paragraph below the h1")
        stage = 2
        next
    }
    stage == 2 {
        if ($0 !~ /^[[:space:]]*<img[[:space:]]/)
            reject("the icon paragraph must contain exactly one image")
        icon_images++
        stage = 3
        next
    }
    stage == 3 {
        if ($0 != "</p>")
            reject("the icon paragraph must contain exactly one image")
        stage = 4
        next
    }
    stage == 4 {
        if ($0 != "<p align=\"center\">")
            reject("badges must be in their own centered paragraph below the icon")
        stage = 5
        next
    }
    stage == 5 {
        if ($0 == "</p>") {
            if (badges == 0)
                reject("the badge paragraph is empty")
            stage = 6
            next
        }
        if ($0 ~ /^[[:space:]]*<a[[:space:]].*<\/a>[[:space:]]*$/ ||
            $0 ~ /^[[:space:]]*<img[[:space:]].*>?[[:space:]]*$/) {
            if ($0 ~ /<img[[:space:]]/) badges++
            next
        }
        reject("the badge paragraph may contain only badge links and images")
    }
    stage == 6 {
        if ($0 != "## 概览")
            reject("project prose must start at ## 概览")
        stage = 7
        exit
    }
    END {
        if (!bad && stage != 7)
            reject("hero ended before ## 概览")
        exit bad
    }
' README.md

echo "README header: PASS (title -> icon -> badges -> 概览)"
