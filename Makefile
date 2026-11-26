# One entry point per thing a contributor or a release needs.
#
#   make check   type-check the runtime for the target it actually runs on
#   make lint    formatting gate (what CI runs)
#   make test    unit tests; needs an arm64 Linux host (the shim is inline asm)
#   make loader  rebuild the embedded loader into the APK's jniLibs
#   make apk     loader + ArkTS-free Rust app .so + Gradle assembleDebug
#   make bench   three-runner benchmark (needs a device)
#   make gate-a  the 38-item acceptance gate + path/V4L2/DRM probes (needs a device)
#   make docs    markdown link check
#   make dist    release artifacts + SHA256SUMS

SHELL       := /bin/sh
TARGET      := aarch64-unknown-linux-gnu
MANIFEST    := runtime/Cargo.toml
PROFILE_DIR := runtime/target/$(TARGET)/release
HUSK_TOOLS  := runtime/target/debug/husk-tools

.PHONY: all check lint test tools nr-tools loader apk bench gate-a virt-race docs dist clean version-check version-bump \
        libc-patch libc-verify

all: lint check version-check

# What README promises `make check` does: formatting, type check for the target
# the runtime actually runs on, lints, shell syntax, doc links and the version
# triple. Before, `check` only ran the cargo check and the other gates lived in
# separate targets, so "make check passed" said less than it looked like.
check:
	cargo fmt --all --check --manifest-path $(MANIFEST)
	cargo check --manifest-path $(MANIFEST) --target $(TARGET) \
		-p runtime-core -p nr-tools -p ldconfig -p path-shim -p gpu-env -p session-env -p v4l2-preload -p v4l2-ref-probe -p fd-identity-probe -p drm-preload
	cargo test --manifest-path $(MANIFEST) -p gpu-env -p session-env
# No --all-targets: path-shim is no_std and its test harness collides with the
# crate's own panic handler (duplicate lang item). Warnings there are tracked
# separately; this gate fails on errors.
	cargo clippy --manifest-path $(MANIFEST) --target $(TARGET) \
		-p runtime-core -p nr-tools -p ldconfig -p path-shim -p gpu-env -p session-env -p v4l2-preload -p v4l2-ref-probe -p fd-identity-probe -p drm-preload
	cargo check --manifest-path app-rs/Cargo.toml --target aarch64-linux-android --lib --bin husk-session-guard
	sh tools/ci/test-locale-defaults.sh
	sh tools/ci/check-fork-safety.sh
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

nr-tools:
	cargo build --manifest-path $(MANIFEST) -p nr-tools

# The guest libc is not stock (slot page + clone3 -> ENOSYS, LIMITATIONS 4), and
# `apt upgrade` of libc6 puts the distro's copy back. This re-applies both
# transforms to whatever libc the tree has now, with the assertions the old
# shell script could not make (nr-tools libc-patch; evidence 170).
DEV        ?= UKHMMB8PYTGECMNV
PKG        ?= com.linuxhub.myth.husk
DEV_ROOTFS ?= files/linux/distributions/ubuntu-24.04/rootfs
DEV_LIBC   := $(DEV_ROOTFS)/usr/lib/aarch64-linux-gnu/libc.so.6
LIBCTMP    := runtime/target/libc-patch

libc-patch: nr-tools
	@mkdir -p $(LIBCTMP)
	adb -s $(DEV) shell "run-as $(PKG) sh -c 'cat $(DEV_LIBC)'" > $(LIBCTMP)/libc.so.6.stock
	runtime/target/debug/nr-tools libc-patch $(LIBCTMP)/libc.so.6.stock $(LIBCTMP)/libc.so.6 \
	    --table tools/loader/libc-interpose.txt
	adb -s $(DEV) shell "run-as $(PKG) sh -c 'cat > $(DEV_LIBC).husk'" < $(LIBCTMP)/libc.so.6
	adb -s $(DEV) shell "run-as $(PKG) sh -c 'R=$$(dirname $(DEV_LIBC)); \
	    [ -f $$R/libc.so.6.stock ] || cp $$R/libc.so.6 $$R/libc.so.6.stock; \
	    mv $(DEV_LIBC).husk $(DEV_LIBC); chmod 755 $(DEV_LIBC); sha256sum $(DEV_LIBC)'"
	cp $(LIBCTMP)/libc.so.6 tools/loader/libc-nop.so.6
	@echo "tools/loader/libc-nop.so.6 refreshed from this run (generated artifact; consumed by make dist)"

# What the release artifact has to satisfy, checked without a device. Pass
# STOCK=<a stock glibc 2.39 libc.so.6> to also prove the whole transform is
# byte-stable against tools/loader/libc-nop.so.6.
libc-verify: nr-tools
	@mkdir -p $(LIBCTMP)
	runtime/target/debug/nr-tools libc-patch tools/loader/libc-nop.so.6 x \
	    --table tools/loader/libc-interpose.txt --assert-only
	@if [ -n "$$STOCK" ]; then \
	    runtime/target/debug/nr-tools libc-patch "$$STOCK" $(LIBCTMP)/verify.so.6 \
	        --table tools/loader/libc-interpose.txt && \
	    cmp $(LIBCTMP)/verify.so.6 tools/loader/libc-nop.so.6 && \
	    echo "byte-identical to tools/loader/libc-nop.so.6"; \
	else \
	    echo "(set STOCK=<stock glibc 2.39 libc.so.6> to also check byte identity)"; \
	fi

test:
	cargo test --manifest-path $(MANIFEST) --target $(TARGET) \
		-p runtime-core -p nr-tools -p gpu-env

loader:
	@test -n "$$NDK" || { echo "set NDK=<ndk>/bin (needs clang + llvm-objcopy)" >&2; exit 1; }
	sh tools/loader/build-embedded-loader.sh

# The APK is two native pieces and one Gradle skeleton: the embedded loader
# (jniLibs/libld.so) and the App's own .so (jniLibs/libhuskapp.so). Both are
# build outputs and neither is tracked, so a clean clone builds them here.
JNI := app/app/src/main/jniLibs/arm64-v8a
NDK_BIN := $(if $(filter %/bin,$(NDK)),$(NDK),$(NDK)/bin)
CC_ANDROID := $(NDK_BIN)/aarch64-linux-android24-clang

apk:
	@test -n "$$NDK" || { echo "set NDK=<ndk>/bin (needs clang, llvm-objcopy, llvm-ar)" >&2; exit 1; }
	@$(MAKE) --no-print-directory loader
	mkdir -p $(JNI)
	CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=$(CC_ANDROID) \
	CC_aarch64_linux_android=$(CC_ANDROID) AR_aarch64_linux_android=$(NDK_BIN)/llvm-ar \
	  cargo build --release --manifest-path app-rs/Cargo.toml --target aarch64-linux-android \
	    --lib --bin husk-session-guard
	cp app-rs/target/aarch64-linux-android/release/libhuskapp.so $(JNI)/
	# Android only extracts/executes native libraries below jniLibs. Keep the
	# guard's .so suffix so the package manager extracts it, even though it is a
	# PIE executable rather than a shared library.
	cp app-rs/target/aarch64-linux-android/release/husk-session-guard $(JNI)/libhuskguard.so
	chmod 755 $(JNI)/libhuskguard.so
	cd app && ./gradlew :app:assembleDebug
	@ls -l $(JNI) app/app/build/outputs/apk/debug/*.apk

bench:
	sh tools/bench/install.sh
	sh tools/bench/run-interleaved.sh $${REPS:-5}
	@$(MAKE) --no-print-directory tools
	$(HUSK_TOOLS) bench-compare docs/design/evidence/bench-*-il.log

# 门禁 A = 38 项验收 + 覆盖度探针 + 补充组/session locale/身份初始化/组表竞态/session guard/rootfd/最终 symlink 真机回归。
# 探针单列一步，因为 locale 看 env -i 后的真实值，竞态看宿主哨兵是否被改动。
gate-a:
	DEV=$${DEV:?} PKG=$${PKG:-com.linuxhub.myth.husk} \
		ROOTFS=$${ROOTFS:-files/linux/distributions/ubuntu-24.04/rootfs} \
		LD=$${LD:-files/linux/husk/libld.so} \
		sh tools/device-tests/14-gate-a-acceptance.sh
	sh tools/probe/run-coverage-probe.sh "$${DEV}" "$${ROOTFS:-}" "$${LD:-}"
	sh tools/probe/coverage-sweep.sh "$${DEV}"
	sh tools/device-tests/run-dpkg-conffile-mode-probe.sh "$${DEV}"
	sh tools/device-tests/run-v4l2-ref-probe.sh "$${DEV}"
	sh tools/device-tests/run-drm-fd-probe.sh "$${DEV}"
	sh tools/device-tests/run-fd-identity-probe.sh "$${DEV}"
	DEV=$${DEV} PKG=$${PKG:-com.linuxhub.myth.husk} \
		ROOTFS=$${ROOTFS:-files/linux/distributions/ubuntu-24.04/rootfs} LD=$${LD:-files/linux/husk/libld.so} \
		sh tools/device-tests/run-guest.sh audit-supplementary-groups.sh
	DEV=$${DEV} PKG=$${PKG:-com.linuxhub.myth.husk} \
		ROOTFS=$${ROOTFS:-files/linux/distributions/ubuntu-24.04/rootfs} LD=$${LD:-files/linux/husk/libld.so} \
		sh tools/device-tests/run-session-locale-probe.sh
	DEV=$${DEV} PKG=$${PKG:-com.linuxhub.myth.husk} \
		ROOTFS=$${ROOTFS:-files/linux/distributions/ubuntu-24.04/rootfs} LD=$${LD:-files/linux/husk/libld.so} \
		sh tools/device-tests/run-identity-init-race-probe.sh
	DEV=$${DEV} PKG=$${PKG:-com.linuxhub.myth.husk} \
		ROOTFS=$${ROOTFS:-files/linux/distributions/ubuntu-24.04/rootfs} LD=$${LD:-files/linux/husk/libld.so} \
		sh tools/device-tests/run-groups-race-probe.sh
	DEV=$${DEV} sh tools/device-tests/run-session-guard-probe.sh
	DEV=$${DEV} PKG=$${PKG:-com.linuxhub.myth.husk} \
		ROOTFS=$${ROOTFS:-files/linux/distributions/ubuntu-24.04/rootfs} LD=$${LD:-files/linux/husk/libld.so} \
		sh tools/device-tests/run-rootfd-close-range-probe.sh
	DEV=$${DEV} PKG=$${PKG:-com.linuxhub.myth.husk} \
		ROOTFS=$${ROOTFS:-files/linux/distributions/ubuntu-24.04/rootfs} LD=$${LD:-files/linux/husk/libld.so} \
		sh tools/device-tests/run-path-race-probe.sh

rootfd-race:
	DEV=$${DEV:?} PKG=$${PKG:-com.linuxhub.myth.husk} \
		ROOTFS=$${ROOTFS:-files/linux/distributions/ubuntu-24.04/rootfs} LD=$${LD:-files/linux/husk/libld.so} \
		sh tools/device-tests/run-rootfd-close-range-probe.sh

virt-race:
	@test -n "$${DEV:-}" || { echo "set DEV=<serial> and the device paths" >&2; exit 1; }
	sh tools/device-tests/run-virt-concurrency-probe.sh \
		"$${DEV}" "$${PKG:-com.linuxhub.myth.husk}" \
		"$${ROOTFS:?set ROOTFS}" "$${LD:?set LD}" "$${ROUNDS:-4000}"

docs: tools
	$(HUSK_TOOLS) check-links

# One version string, three places: VERSION (what `./husk --version` prints),
# the newest CHANGELOG heading, and the release-tag link that heading points at.
version-check:
	@v=$$(cat VERSION); \
	printf '%s\n' "$$v" | awk -F. 'NF != 3 || $$1 !~ /^[0-9]+$$/ || $$2 !~ /^[0-9]+$$/ || $$3 !~ /^[0-9]+$$/ || $$1 > 99 || $$2 > 99 || $$3 > 99 { exit 1 }' || { \
	    echo "invalid version (expected X.Y.Z with components 0..99): $$v" >&2; exit 1; \
	}; \
	c=$$(sed -n 's/^## \[\([^]]*\)\].*/\1/p' CHANGELOG.md | grep -v -e '^Unreleased$$' -e '^未发布$$' | head -1); \
	if [ "$$v" != "$$c" ]; then \
	    echo "version mismatch: VERSION=$$v CHANGELOG=$$c" >&2; exit 1; \
	fi; \
	grep -q "releases/tag/v$$v" CHANGELOG.md || { \
	    echo "CHANGELOG has no releases/tag/v$$v link" >&2; exit 1; }; \
	echo "version ok: $$v"
version-bump:
	@sh tools/ci/version-bump.sh

# Two artifacts: the launcher bundle (loader + installer + session script) and,
# built on a device from a verified tree, the rootfs tarball itself. Both get a
# SHA256SUMS; the release notes quote those hashes.
dist:
	@test -n "$$NDK" || { echo "set NDK=<ndk>/bin" >&2; exit 1; }
	@if [ ! -f tools/loader/libc-nop.so.6 ] && [ -z "$(LIBC)" ]; then \
	    echo "missing guest libc artifact; set LIBC=<pristine glibc 2.39 libc.so.6> or run make libc-patch on a device" >&2; \
	    exit 1; \
	fi
	LIBC="$(LIBC)" sh tools/loader/build-embedded-loader.sh
	@test -f tools/loader/libc-nop.so.6 || { echo "guest libc artifact was not produced" >&2; exit 1; }
	sh tools/guest-config/build-guest-tools.sh
	@mkdir -p dist
	cp tools/loader/ld-embedded.so dist/libld.so
	@chmod 755 dist/libld.so   # tar keeps the mode; a 644 loader is "not executable"
	cp $(PROFILE_DIR)/husk-ldconfig $(PROFILE_DIR)/husk-usermgr \
	   $(PROFILE_DIR)/husk-su $(PROFILE_DIR)/husk-symlinks dist/
	cp tools/loader/libc-nop.so.6 dist/libc.so.6   # the artifact `make libc-patch` refreshed
	cp tools/guest/install-husk.sh tools/guest/husk-session.sh \
	   tools/guest/husk-desktop.sh tools/guest/husk-ding-fallback.sh \
	   tools/guest/husk-wallpaper-fit.sh dist/
	@mkdir -p dist/guest dist/guest-config
	cp tools/guest/husk-wallpaper-scale.js dist/guest/
	cp tools/guest-config/husk-gpu-defaults.sh tools/guest-config/husk-look.override \
	   tools/guest-config/husk-locale.sh tools/guest-config/fcitx5-profile \
	   tools/guest-config/pulse-client-no-shm.conf dist/guest-config/
	cp VERSION dist/VERSION
	@if [ -f tools/langpack/husk-lang-zh.tar.gz ]; then \
	    cp tools/langpack/husk-lang-zh.tar.gz dist/; \
	    tar tzf dist/husk-lang-zh.tar.gz | grep -qx 'usr/share/locale-langpack/zh_CN/LC_MESSAGES/gnome-shell.mo' || \
	        { echo "dist: lang pack is missing gnome-shell.mo" >&2; exit 1; }; \
	else \
	    echo "dist: no tools/langpack/husk-lang-zh.tar.gz; UI text will stay English" >&2; \
	fi
	tar czf dist/husk-arm64.tar.gz -C dist \
	    libld.so libc.so.6 husk-ldconfig husk-usermgr husk-su husk-symlinks \
	    guest guest-config install-husk.sh husk-session.sh husk-desktop.sh \
	    husk-ding-fallback.sh husk-wallpaper-fit.sh husk-lang-zh.tar.gz VERSION
# The installer writes /etc/profile.d/husk-gpu.sh from guest-config/, and skips it
# silently when the file is not next to it: a bundle without it installs a tree
# with no GPU defaults and no error. Asserted on the tarball, not on dist/.
	@tar tzf dist/husk-arm64.tar.gz | grep -qx 'guest-config/husk-gpu-defaults.sh' || \
	    { echo "dist: bundle is missing guest-config/husk-gpu-defaults.sh" >&2; exit 1; }
# 少了 locale 默认值，装出来的树不会报错，只是桌面回到英文界面、中文按日文字形渲染。
	@tar tzf dist/husk-arm64.tar.gz | grep -qx 'guest-config/husk-locale.sh' || \
	    { echo "dist: bundle is missing guest-config/husk-locale.sh" >&2; exit 1; }
	@tar tzf dist/husk-arm64.tar.gz | grep -qx 'guest-config/fcitx5-profile' || \
	    { echo "dist: bundle is missing guest-config/fcitx5-profile" >&2; exit 1; }
# Same failure mode for the look defaults: a bundle without them installs a tree where
# the shell is Yaru and every app window is Adwaita, and nothing says why.
	@tar tzf dist/husk-arm64.tar.gz | grep -qx 'guest-config/husk-look.override' || \
	    { echo "dist: bundle is missing guest-config/husk-look.override" >&2; exit 1; }
# 0.0.6 补的三条断言：这几个文件少了不会报错，只会让装出来的树缺一件东西
# （husk-session.sh 的 `[ -x ... ]` 判假、pulse 客户端静默丢掉 enable-shm=no）。
	@for f in husk-desktop.sh husk-ding-fallback.sh husk-wallpaper-fit.sh \
	          guest/husk-wallpaper-scale.js guest-config/pulse-client-no-shm.conf; do \
	    tar tzf dist/husk-arm64.tar.gz | grep -qx "$$f" || \
	        { echo "dist: bundle is missing $$f" >&2; exit 1; }; \
	done
	@rm -f dist/libld.so dist/libc.so.6 dist/husk-ldconfig dist/husk-usermgr \
	    dist/husk-su dist/husk-symlinks dist/install-husk.sh dist/husk-session.sh \
	    dist/husk-desktop.sh dist/husk-ding-fallback.sh dist/husk-wallpaper-fit.sh \
	    dist/husk-lang-zh.tar.gz dist/VERSION
	@rm -rf dist/guest dist/guest-config
	@cd dist && sha256sum husk-arm64.tar.gz > SHA256SUMS && cat SHA256SUMS

clean:
	cargo clean --manifest-path $(MANIFEST)
