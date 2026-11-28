#!/bin/bash
export PATH=/usr/bin:/bin:$PATH
/usr/bin/gl_probe readback-suite 2>&1 | head -5; echo "rc=$?"
echo ---
/usr/bin/gl_probe readback 64x64 2>&1 | head -5; echo "rc=$?"
