#!/bin/bash
export PATH=/usr/bin:/bin:$PATH
GP=/root/gl-probe
ref="llvmpipe reference: mip level 1 mean=222 near_black=81504/648000"
echo "== $ref"
for v in PANVK_ENABLE_AFBC=0 PANVK_AFBC=0 PANVK_NO_SMC=1 PANVK_OVERLAP=1 PANVK_NOATT_CLAMP=1 PANVK_SKIP_FS=0; do
  k=${v%%=*}; val=${v#*=}
  out=$(env $k=$val $GP scale-wedge 348x156 dumpmips 2>&1 | grep -E "mip level 1 ")
  sc=$(env $k=$val $GP scale-wedge 348x156 mip 2>&1 | grep -E "dark_cells")
  echo "-- $k=$val | $out | $sc"
done
