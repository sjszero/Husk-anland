# Security policy

## Scope

Husk runs untrusted-by-the-OS code: a Linux userland inside an Android app
sandbox (`untrusted_app`, no root). The security boundary that matters is the
Android one — the runtime must never widen it. Reports that matter most:

- a way to make the guest read or write paths outside its rootfs;
- a way to make the loader or shim grant privileges the Android sandbox denies
  (capabilities, `setuid` to another uid, mount, ptrace);
- a way to make a guest process escape into the host Android namespace by
  re-executing a binary that the shim does not cover;
- anything that lets one app's container read another app's data.

Bugs in the *guest* distro (an Ubuntu package, GNOME, …) are upstream's; report
them there. A crash that is only reachable by the guest damaging itself is a bug
report, not a vulnerability.

## Reporting

Open a private report through GitHub Security Advisories on this repository
("Security" → "Report a vulnerability"). Please do not open a public issue for a
suspected sandbox escape.

Include: the device and Android version, `assets.version`, the exact command, and
what you observed. If you have a reproducer that survives a reboot, say so.

## What to expect

- Acknowledgement within 7 days.
- An assessment: in scope (sandbox escape, path escape, privilege widening) or
  out of scope (guest-internal crash, performance, missing feature).
- Fixes ship in a release note under "Security"; the advisory is published once a
  release containing the fix is out.

## Known and accepted

`docs/LIMITATIONS.md` lists behaviour that looks like a security property but is
a documented design limit — static binaries bypassing the shim, `/proc` being
synthesised, `setuid` being emulated rather than granted. Those are not
vulnerabilities; they are the reason Husk needs no root.
