# Support

## Read first

| Question | Document |
|---|---|
| Does my device work? | [`docs/COMPATIBILITY.md`](docs/COMPATIBILITY.md) — one verified device, everything else UNKNOWN |
| How do I run it? | [`docs/RUNNING.md`](docs/RUNNING.md) |
| Why is it fast? | [`docs/BENCHMARKS.md`](docs/BENCHMARKS.md) |
| What does it not do? | [`docs/LIMITATIONS.md`](docs/LIMITATIONS.md) |
| How does the loader work? | [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) |

## Asking

Open an issue with the bug template. A useful report contains the device
properties, the exact command, the exact output, and `assets.version`. Reports
that say only "the desktop is broken" cannot be acted on: the desktop has a dozen
independent stages and each one has its own log.

## Not supported

- Devices other than the one in `docs/COMPATIBILITY.md`: the runtime is not
  claimed to work there, and a report that it does not is not a bug.
- `root`, bootloader unlocking, or running without the Android app sandbox.
- Distributions other than the Ubuntu 24.04 tree the loader was built against
  (glibc version is checked at build time, `docs/LIMITATIONS.md`).
