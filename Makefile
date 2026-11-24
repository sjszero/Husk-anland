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

.PHONY: all check lint test loader bench gate-a docs dist clean version-check

all: lint check version-check

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
	sh tools/bench/run-interleaved.sh $${REPS:-5}
	python3 tools/bench/compare.py docs/design/evidence/bench-*-il.log

gate-a:
	DEV=$${DEV:?} PKG=$${PKG:-com.linuxhub.myth.husk} \
		ROOTFS=$${ROOTFS:-files/linux/distributions/ubuntu-24.04/rootfs} \
		LD=$${LD:-files/linux/husk/libld.so} \
		sh tools/device-tests/14-gate-a-acceptance.sh

docs:
	python3 tools/ci/check-links.py

# One version string, three places: VERSION (what `./husk --version` prints),
# the newest CHANGELOG heading, and the release-tag link that heading points at.
version-check:
	@v=$$(cat VERSION); \
	c=$$(sed -n 's/^## \[\([^]]*\)\].*/\1/p' CHANGELOG.md | grep -v -e '^Unreleased$$' -e '^未发布$$' | head -1); \
	if [ "$$v" != "$$c" ]; then \
	    echo "version mismatch: VERSION=$$v CHANGELOG=$$c" >&2; exit 1; \
	fi; \
	grep -q "releases/tag/v$$v" CHANGELOG.md || { \
	    echo "CHANGELOG has no releases/tag/v$$v link" >&2; exit 1; }; \
	echo "version ok: $$v"

# Two artifacts: the launcher bundle (loader + installer + session script) and,
# built on a device from a verified tree, the rootfs tarball itself. Both get a
# SHA256SUMS; the release notes quote those hashes.
dist:
	@test -n "$$NDK" || { echo "set NDK=<ndk>/bin" >&2; exit 1; }
	sh tools/loader/build-embedded-loader.sh
	sh tools/guest-config/build-guest-tools.sh
	@mkdir -p dist
	cp tools/loader/ld-embedded.so dist/libld.so
	cp $(PROFILE_DIR)/husk-ldconfig $(PROFILE_DIR)/husk-usermgr \
	   $(PROFILE_DIR)/husk-su dist/
	cp binaries/libc-nop.so.6 dist/libc.so.6
	cp tools/guest/install-husk.sh tools/guest/husk-session.sh dist/
	cp VERSION dist/VERSION
	tar czf dist/husk-arm64.tar.gz -C dist \
	    libld.so libc.so.6 husk-ldconfig husk-usermgr husk-su \
	    install-husk.sh husk-session.sh VERSION
	@rm -f dist/libld.so dist/libc.so.6 dist/husk-ldconfig dist/husk-usermgr \
	    dist/husk-su dist/install-husk.sh dist/husk-session.sh dist/VERSION
	@cd dist && sha256sum husk-arm64.tar.gz > SHA256SUMS && cat SHA256SUMS

clean:
	cargo clean --manifest-path $(MANIFEST)
