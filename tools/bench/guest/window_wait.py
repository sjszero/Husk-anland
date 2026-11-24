#!/usr/bin/env python3
"""Time from spawning a GTK app to the first *new* viewable top-level window.

Used for dimension 5 (desktop). Same script for every runner: it only talks to
the X server on $DISPLAY, so the number is "app start -> window really on
screen", not "app start -> process alive".
"""
import os, subprocess, sys, time

try:
    from Xlib import display, X
except Exception as exc:  # python3-xlib missing
    print("window_wait\tskip\t%s" % exc)
    sys.exit(0)

app = sys.argv[1] if len(sys.argv) > 1 else "gnome-control-center"
d = display.Display(os.environ.get("DISPLAY", ":0"))
root = d.screen().root


def viewable_windows():
    out = set()
    for w in root.query_tree().children:
        try:
            a = w.get_attributes()
        except Exception:
            continue
        if a.map_state == X.IsViewable:
            out.add(w.id)
    return out


before = viewable_windows()
t0 = time.time()
p = subprocess.Popen([app], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
deadline = t0 + 60
seen = None
while time.time() < deadline:
    new = viewable_windows() - before
    if new:
        seen = time.time() - t0
        break
    time.sleep(0.02)
print("window_wait\t%s" % (int(seen * 1000) if seen is not None else -1))
try:
    p.terminate()
except Exception:
    pass
