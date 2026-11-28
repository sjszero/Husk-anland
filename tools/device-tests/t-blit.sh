#!/bin/bash
export PATH=/usr/local/bin:/usr/bin:/bin
for sz in 348x156 512x256 352x160; do
  for v in blit blit-green blit-1to1; do
    echo "########## $v $sz"
    /root/gl-probe scale-wedge "$sz" "$v" 2>&1
  done
done
echo probe-done
