# One entry point per thing a contributor or a release needs.
#
#   make check   type-check the runtime for the target it actually runs on
#   make lint    formatting gate (what CI runs)
#   make test    unit tests; needs an arm64 Linux host (the shim is inline asm)
#   make loader  rebuild the embedded loader into the APK's jniLibs
#   make apk     loader + ArkTS-free Rust app .so + Gradle assembleDebug
#   make bench   three-runner benchmark (needs a device)
#   make gate-a  the 36-item acceptance gate (needs a device, desktop running)
#   make docs    markdown link check
#   make dist    release artifacts + SHA256SUMS

SHELL       := /bin/sh
TARGET      := aarch64-unknown-linux-gnu
MANIFEST    := runtime/Cargo.toml
PROFILE_DIR := runtime/target/$(TARGET)/release
HUSK_TOOLS  := runtime/target/debug/husk-tools

.PHONY: all check lint test tools loader apk bench gate-a docs dist clean version-check

all: lint check version-check

# What README promises `make check` does: formatting, type check for the target
# the runtime actually runs on, lints, shell syntax, doc links and the version
# triple. Before, `check` only ran the cargo check and the other gates lived in
# separate targets, so "make check passed" said less than it looked like.
check:
	cargo fmt --all --check --manifest-path $(MANIFEST)
	cargo check --manifest-path $(MANIFEST) --target $(TARGET) \
		-p runtime-core -p nr-tools -p ldconfig -p path-shim
# No --all-targets: path-shim is no_std and its test harness collides with the
# crate's own panic handler (duplicate lang item). Warnings there are tracked
# separately; this gate fails on errors.
	cargo clippy --manifest-path $(MANIFEST) --target $(TARGET) \
		-p runtime-core -p nr-tools -p ldconfig -p path-shim
	sh tools/ci/check-shell.sh
	@$(MAKE) --no-print-directory tools
	$(HUSK_TOOLS) check-links
	@$(MAKE) --no-print-directory version-check

lint:
	cargo fmt --all --check --manifest-path $(MANIFEST)

# Repository tooling is Rust too (it used to be three Python scripts): build it
# for the host and call it. Never built for $(TARGET) -- it runs on the dev box.
tools:
	cargo build --manifest-path $(MANIFEST) -p husk-tools

test:
	cargo test --manifest-path $(MANIFEST) --target $(TARGET) \
		-p runtime-core -p nr-tools

loader:
	@test -n "$$NDK" || { echo "set NDK=<ndk>/bin (needs clang + llvm-objcopy)" >&2; exit 1; }
	sh tools/loader/build-embedded-loader.sh

# The APK is two native pieces and one Gradle skeleton: the embedded loader
# (jniLibs/libld.so) and the App's own .so (jniLibs/libhuskapp.so). Both are
# build outputs and neither is tracked, so a clean clone builds them here.
JNI := app/app/src/main/jniLibs/arm64-v8a
CC_ANDROID := $(NDK)/aarch64-linux-android24-clang

apk:
	@test -n "$$NDK" || { echo "set NDK=<ndk>/bin (needs clang, llvm-objcopy, llvm-ar)" >&2; exit 1; }
	@$(MAKE) --no-print-directory loader
	mkdir -p $(JNI)
	CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=$(CC_ANDROID) \
	CC_aarch64_linux_android=$(CC_ANDROID) AR_aarch64_linux_android=$(NDK)/llvm-ar \
	  cargo build --release --manifest-path app-rs/Cargo.toml --target aarch64-linux-android
	cp app-rs/target/aarch64-linux-android/release/libhuskapp.so $(JNI)/
	cd app && ./gradlew :app:assembleDebug
	@ls -l $(JNI) app/app/build/outputs/apk/debug/*.apk

bench:
	sh tools/bench/install.sh
	sh tools/bench/run-interleaved.sh $${REPS:-5}
	@$(MAKE) --no-print-directory tools
	$(HUSK_TOOLS) bench-compare docs/design/evidence/bench-*-il.log

gate-a:
	DEV=$${DEV:?} PKG=$${PKG:-com.linuxhub.myth.husk} \
		ROOTFS=$${ROOTFS:-files/linux/distributions/ubuntu-24.04/rootfs} \
		LD=$${LD:-files/linux/husk/libld.so} \
		sh tools/device-tests/14-gate-a-acceptance.sh

docs: tools
	$(HUSK_TOOLS) check-links

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
	@chmod 755 dist/libld.so   # tar keeps the mode; a 644 loader is "not executable"
	cp $(PROFILE_DIR)/husk-ldconfig $(PROFILE_DIR)/husk-usermgr \
	   $(PROFILE_DIR)/husk-su $(PROFILE_DIR)/husk-symlinks dist/
	cp tools/loader/libc-nop.so.6 dist/libc.so.6   # the artifact install-libc-patch.sh just refreshed
	cp tools/guest/install-husk.sh tools/guest/husk-session.sh dist/
	cp VERSION dist/VERSION
	tar czf dist/husk-arm64.tar.gz -C dist \
	    libld.so libc.so.6 husk-ldconfig husk-usermgr husk-su husk-symlinks \
	    install-husk.sh husk-session.sh VERSION
	@rm -f dist/libld.so dist/libc.so.6 dist/husk-ldconfig dist/husk-usermgr \
	    dist/husk-su dist/husk-symlinks dist/install-husk.sh dist/husk-session.sh dist/VERSION
	@cd dist && sha256sum husk-arm64.tar.gz > SHA256SUMS && cat SHA256SUMS

clean:
	cargo clean --manifest-path $(MANIFEST)
