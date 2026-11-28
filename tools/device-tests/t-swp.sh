#!/bin/bash
export PATH=/usr/local/bin:/usr/bin:/bin
for s in 1200x540 696x312 348x156 174x78; do
  echo "########## draw(nomip) $s"
  /root/gl-probe scale-wedge "$s" nomip 2>&1 | grep -E "variant=|dark_cells=|near_black="
done
echo sweep-done
