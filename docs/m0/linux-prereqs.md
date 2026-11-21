# Linux build prerequisites

Candidate Ubuntu 24.04 packages, based on research D3 and NSH 0.5.1 dependency metadata:

```text
build-essential
pkg-config
libgtk-4-dev
libpango1.0-dev
libgstreamer1.0-dev
libgstreamer-plugins-base1.0-dev
```

## Spark status (checked 2026-11-19)

The required development packages are installed in Spark's host dpkg database. A Rust `1.100.0` locked workspace debug build, test run, and release build passed against the system GTK4/GStreamer metadata, with no `/tmp/gtkdev` path or `PKG_CONFIG_PATH` override. The current test log records 42 passed and 0 failed. See the [2026-11-19 N1 follow-up](evidence/N1-spark-aarch64-n_a-20261119.md) and its environment/build/test/release logs.

Installed and observed versions:

```text
Rust 1.100.0
build-essential 12.10ubuntu1
pkg-config 1.8.1-2build1
libgtk-4-dev 4.14.5+ds-0ubuntu0.10
libpango1.0-dev 1.52.1+ds-1build1
libgstreamer1.0-dev 1.24.2-1ubuntu0.2
libgstreamer-plugins-base1.0-dev 1.24.2-1ubuntu0.7
```

The 2026-11-19 apt transaction installed 20 packages, upgraded 31 existing packages, and removed none. It was run as host root through the rootful Docker daemon and a chroot of the mounted host root; no container build is used as N1 evidence. `dpkg --audit` and root `apt-get check` both passed. The transaction's `needrestart` service actions are disclosed in the N1 evidence: Avahi failed to restart because the host-specific `/etc/avahi/avahi-daemon-dgx-spark.conf` was denied; the xrdp failures predate the transaction. Linux x86_64 and macOS host prerequisites/builds remain not-run.

## Temporary extracted-package workaround

The N1 build used `/tmp/gtkdev`, a temporary arm64 extraction of Ubuntu `.deb` packages. The top-level development packages were:

```text
libgtk-4-dev 4.14.5+ds-0ubuntu0.10
libgstreamer1.0-dev 1.24.2-1ubuntu0.1
libgstreamer-plugins-base1.0-dev 1.24.2-1ubuntu0.7
```

Their dependencies were downloaded as `.deb` files, extracted with `dpkg-deb -x` under `/tmp/gtkdev/root` without installing them, and their `.pc` files were copied to `/tmp/gtkdev/pc` with prefixes rewritten to that extraction root. The build environment then used:

```bash
export PKG_CONFIG_PATH=/tmp/gtkdev/pc
pkg-config --modversion gtk4 pango pangocairo gstreamer-1.0 gstreamer-app-1.0
# 4.14.5, 1.52.1, 1.52.1, 1.24.2, 1.24.2
RUSTUP_TOOLCHAIN=1.100.0 cargo build --workspace --locked
RUSTUP_TOOLCHAIN=1.100.0 cargo test --workspace --locked
```

This workaround is local to spark, is not committed, and does not replace the system package prerequisites for a fresh host.

For N6, IBus with GTK4 requires `ibus`, `ibus-libpinyin`, and the GTK4 IBus module (`ibus-gtk4`); Fcitx5 with GTK4 requires `fcitx5`, a Pinyin add-on, and `fcitx5-frontend-gtk4`. These are runtime inputs, not build prerequisites. On spark, `ibus` 1.5.29 and `ibus-libpinyin` 1.15.7 are installed, and `ibus engine` reports `libpinyin`; the GTK4 IBus module and Fcitx5 packages are not installed system-wide. Both Pinyin paths passed composition, candidate selection, and commit in the Omobench SQL editor on spark X11 after extracting Ubuntu arm64 packages under `/tmp` and setting `GTK_PATH`. The IBus output was also checked through the clipboard. These extracted modules are test workarounds, not system installations. The complete Wayland cell remains not-run: the latest nested Weston Fcitx run displayed and selected candidate 2, then committed it, while its log recorded a Wayland input-panel error. IBus inserted text, but candidate selection remains unverified. Linux x86_64 and macOS cases remain not-run; see the [latest N6 follow-up](evidence/N6-spark-aarch64-wayland-20261118.md) and [prior Wayland attempt](evidence/N6-spark-aarch64-wayland-20261117.md).
