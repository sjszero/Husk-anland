## What this changes


## Why


## How it was verified

- [ ] `make lint` (cargo fmt --all --check)
- [ ] `make check` (cargo check --target aarch64-unknown-linux-gnu)
- [ ] On device: which gate / which command, with the output pasted or the
      evidence file referenced (`docs/design/evidence/NNN-*.md`)

## Checklist

- [ ] Conventional Commits subject (`feat|fix|docs|perf|refactor|test|build|ci|chore`)
- [ ] No third-party code, no keys, no personal paths, no private repository names
- [ ] Numbers in this PR come from a measurement, and the raw output is committed
      under `docs/design/evidence/`
- [ ] If behaviour changed, `docs/STATUS.md` or `docs/BENCHMARKS.md` was updated
