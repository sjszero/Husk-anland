# One entry point per thing a contributor or a release needs.
#
#   make check   type-check the runtime for the target it actually runs on
#   make lint    formatting gate (what CI runs)
#   make test    unit tests; needs an arm64 Linux host (the shim is inline asm)
#   make loader  rebuild tools/loader/ld-embedded.so
#   make bench   three-runner benchmark (needs a device)
#   make gate-a  the 36-item acceptance gate (needs a device, desktop running)
#   make docs    markdown link check
#   make dist    release artifacts + SHA256SUMS

SHELL       := /bin/sh
TARGET      := aarch64-unknown-linux-gnu
MANIFEST    := runtime/Cargo.toml
PROFILE_DIR := runtime/target/$(TARGET)/release

.PHONY: all check lint test loader bench gate-a docs dist clean

all: lint check

check:
	cargo check --manifest-path $(MANIFEST) --target $(TARGET) \
		-p runtime-core -p nr-tools -p ldconfig -p path-shim

lint:
	cargo fmt --all --check --manifest-path $(MANIFEST)

test:
	cargo test --manifest-path $(MANIFEST) --target $(TARGET) \
		-p runtime-core -p nr-tools

loader:
	@test -n "$$NDK" || { echo "set NDK=<ndk>/bin (needs clang + llvm-objcopy)" >&2; exit 1; }
	sh tools/loader/build-embedded-loader.sh

bench:
	sh tools/bench/install.sh
	sh tools/bench/run-all.sh husk
	sh tools/bench/run-all.sh proot
	sh tools/bench/run-all.sh proot-distro

gate-a:
	DEV=$${DEV:?} PKG=$${PKG:-com.linuxhub.myth.husk} \
		ROOTFS=$${ROOTFS:-files/linux/distributions/ubuntu-24.04/rootfs} \
		LD=$${LD:-files/linux/husk/libld.so} \
		sh tools/device-tests/14-gate-a-acceptance.sh

docs:
	python3 tools/ci/check-links.py

dist: 
	@test -n "$$NDK" || { echo "set NDK=<ndk>/bin" >&2; exit 1; }
	sh tools/loader/build-embedded-loader.sh
	sh tools/guest-config/build-guest-tools.sh
	@mkdir -p dist
	cp tools/loader/ld-embedded.so dist/libld.so
	cp $(PROFILE_DIR)/husk-ldconfig $(PROFILE_DIR)/husk-su \
	   $(PROFILE_DIR)/husk-usermgr $(PROFILE_DIR)/husk-rd dist/
	cd dist && sha256sum * > SHA256SUMS && cat SHA256SUMS

clean:
	cargo clean --manifest-path $(MANIFEST)
