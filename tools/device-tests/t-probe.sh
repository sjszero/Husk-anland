#!/bin/bash
export PATH=/usr/local/bin:/usr/bin:/bin
echo "self-test-start"
/root/gl-probe scale-wedge 348x156 blit-green 2>&1 | grep -E "renderer=|dark_cells|colors "
/root/gl-probe scale-wedge 696x312 blit 2>&1 | grep -E "dark_cells|near_black"
/root/gl-probe scale-wedge 348x156 nomip 2>&1 | grep -E "dark_cells|near_black"
/root/gl-probe scale-wedge 348x156 mip 2>&1 | grep -E "dark_cells|near_black"
echo probe-done
