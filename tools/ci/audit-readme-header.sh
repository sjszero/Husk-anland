#!/bin/sh
# Enforce the repository landing-page hero contract.
#
# Visible order before project prose:
#   1. one centered <h1>Husk</h1>;
#   2. one centered icon paragraph;
#   3. one centered summary paragraph;
#   4. one centered technology-badge paragraph;
#   5. one centered navigation paragraph;
#   6. optional horizontal rule, then `## 目录`;
#   7. `## 跑起来的前提` immediately after the table of contents;
#   8. `## 概览`, where long-form project prose begins.
#
# HTML comments and blank lines are ignored.
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
        icons = 0
        badges = 0
        links = 0
        intro = 0
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
            reject("the icon must be in a centered paragraph below the h1")
        stage = 2
        next
    }
    stage == 2 {
        if ($0 !~ /^[[:space:]]*<img[[:space:]]/)
            reject("the icon paragraph must contain exactly one image")
        icons++
        stage = 3
        next
    }
    stage == 3 {
        if ($0 != "</p>" || icons != 1)
            reject("the icon paragraph must contain exactly one image")
        stage = 4
        next
    }
    stage == 4 {
        if ($0 != "<p align=\"center\">")
            reject("the summary must be in its own centered paragraph")
        stage = 5
        next
    }
    stage == 5 {
        if ($0 == "</p>") {
            if (intro == 0) reject("the summary paragraph is empty")
            stage = 6
            next
        }
        intro++
        next
    }
    stage == 6 {
        if ($0 != "<p align=\"center\">")
            reject("technology badges must be in their own centered paragraph")
        stage = 7
        next
    }
    stage == 7 {
        if ($0 == "</p>") {
            if (badges == 0) reject("the badge paragraph is empty")
            stage = 8
            next
        }
        if ($0 ~ /^[[:space:]]*<a[[:space:]].*<\/a>[[:space:]]*$/ ||
            $0 ~ /^[[:space:]]*<img[[:space:]].*>?[[:space:]]*$/) {
            if ($0 ~ /<img[[:space:]]/) badges++
            next
        }
        reject("the badge paragraph may contain only badge links and images")
    }
    stage == 8 {
        if ($0 != "<p align=\"center\">")
            reject("site navigation must be in its own centered paragraph")
        stage = 9
        next
    }
    stage == 9 {
        if ($0 == "</p>") {
            if (links == 0) reject("the navigation paragraph has no links")
            stage = 10
            next
        }
        if ($0 !~ /<a[[:space:]][^>]*>.*<\/a>/) {
            reject("the navigation paragraph may contain only links and separators")
        }
        links++
        next
    }
    stage == 10 {
        if ($0 == "---") { stage = 11; next }
        if ($0 != "## 目录")
            reject("the table of contents must follow the hero")
        stage = 12
        next
    }
    stage == 11 {
        if ($0 != "## 目录")
            reject("the table of contents must follow the hero")
        stage = 12
        next
    }
    stage == 12 {
        if ($0 ~ /^- /) next
        if ($0 != "## 跑起来的前提")
            reject("requirements must follow the table of contents")
        stage = 13
        next
    }
    stage == 13 {
        if ($0 == "## 概览") { stage = 14; exit }
        next
    }
    END {
        if (!bad && stage != 14)
            reject("hero ended before ## 概览")
        exit bad
    }
' README.md

echo "README header: PASS (hero -> summary -> badges -> navigation -> 目录 -> 跑起来的前提 -> 概览)"
