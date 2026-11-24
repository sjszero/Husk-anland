#!/usr/bin/env python3
"""Fail if a relative link or image in a tracked markdown file points at nothing.

    tools/ci/check-links.py

Only relative targets are checked: external URLs are not fetched (a flaky
network must not fail a build), and anchors are compared against the headings of
the target file when the target is markdown.
"""
import os
import re
import subprocess
import sys

LINK = re.compile(r"!?\[[^\]]*\]\(([^)\s]+)(?:\s+\"[^\"]*\")?\)")
FENCE = re.compile(r"^```.*?^```", re.M | re.S)
HEADING = re.compile(r"^#{1,6}\s+(.*)$", re.M)


def slug(text):
    text = text.strip().lower()
    text = re.sub(r"`|\*|_|\[|\]|\(|\)|\.|,|:|\"|'|!|\?|/", "", text)
    text = re.sub(r"\s+", "-", text)
    return text


def anchors(path):
    try:
        with open(path, encoding="utf-8") as fh:
            body = fh.read()
    except OSError:
        return set()
    return {slug(m.group(1)) for m in HEADING.finditer(body)}


def main():
    files = subprocess.run(
        ["git", "ls-files", "*.md"], capture_output=True, text=True, check=True
    ).stdout.split()
    root = os.getcwd()
    bad = 0
    for path in files:
        with open(path, encoding="utf-8") as fh:
            body = fh.read()
        # Fenced code is quoted output: a "link" inside it is not a link.
        body = FENCE.sub("", body)
        for m in LINK.finditer(body):
            target = m.group(1)
            if re.match(r"^[a-z][a-z0-9+.-]*:", target) or target.startswith("#"):
                continue
            file_part, _, anchor = target.partition("#")
            resolved = os.path.normpath(os.path.join(os.path.dirname(path), file_part)) if file_part else path
            if not os.path.exists(os.path.join(root, resolved)):
                print(f"{path}: missing target {target}")
                bad += 1
                continue
            if anchor and resolved.endswith(".md"):
                if anchor.lower() not in anchors(os.path.join(root, resolved)):
                    print(f"{path}: missing anchor {target}")
                    bad += 1
    print(f"checked {len(files)} markdown files, {bad} broken link(s)")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
