#!/bin/sh
# Repair a usr-merged rootfs whose /bin, /sbin or /lib lost their symlinks.
#
# Ubuntu 24.04 is usr-merged: /bin -> usr/bin, /sbin -> usr/sbin, /lib -> usr/lib.
# An extractor that does not follow directory symlinks (libarchive/bsdtar, toybox
# tar) turns them into real directories the moment a package ships bin/foo, and
# the result looks fine until something runs /bin/sh - which is how apt runs
# every maintainer script. nr-tools' own extractor no longer does this; this
# script repairs images produced before that fix, without deleting anything:
# entries move into usr/ unless the same name is already there, and a directory
# only becomes a symlink once it is empty.
#
#   run-as <pkg> sh repair-usrmerge.sh <rootfs-dir>
#   DRY=1 ...   # print what would move
#
# Nothing is deleted: entries move into usr/ unless the same name is already
# there, and a directory only becomes a symlink after it is empty.
cd "$1" || exit 1
DRY=${DRY:-0}

merge_tree() { # $1 src dir, $2 dst dir
  for f in "$1"/* "$1"/.[!.]*; do
    [ -e "$f" ] || [ -L "$f" ] || continue
    n=$(basename "$f")
    if [ -d "$f" ] && [ ! -L "$f" ] && [ -d "$2/$n" ] && [ ! -L "$2/$n" ]; then
      merge_tree "$f" "$2/$n"
      [ "$DRY" = 1 ] || rmdir "$f" 2>/dev/null
    elif [ -e "$2/$n" ] || [ -L "$2/$n" ]; then
      echo "  keep   $2/$n   (same name already there, leaving $f)"
    else
      if [ "$DRY" = 1 ]; then echo "  move   $f -> $2/$n"; else mv "$f" "$2/$n" || echo "  FAILED $f"; fi
    fi
  done
}

for pair in bin:usr/bin sbin:usr/sbin lib:usr/lib; do
  d=${pair%%:*}; t=${pair#*:}
  if [ -L "$d" ]; then echo "$d -> $(readlink $d) (already a symlink)"; continue; fi
  if [ ! -d "$d" ]; then echo "$d missing, creating symlink"; [ "$DRY" = 1 ] || ln -s "$t" "$d"; continue; fi
  echo "merging $d into $t"
  merge_tree "$d" "$t"
  left=$(ls -A "$d" 2>/dev/null | wc -l)
  if [ "$left" -gt 0 ]; then
    echo "  !! $d still has $left entries, not replacing with a symlink:"; ls -A "$d" | head -10
  else
    [ "$DRY" = 1 ] || { rmdir "$d" && ln -s "$t" "$d"; }
    echo "  $d -> $t"
  fi
done
echo "== result =="
ls -ld bin sbin lib
ls -l bin/sh sbin/init 2>&1
