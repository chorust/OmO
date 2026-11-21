# N5 file-dialog fallback on nested Weston Wayland — 2026-11-20

The error-returning test service is retained as [mock-portal.c](n5-spark-aarch64-wayland-20261120/mock-portal.c); its `error` mode returns the recorded D-Bus error from both chooser methods. The no-service run used the retained [private bus configuration](n5-spark-aarch64-wayland-20261120/no-portal-session.conf), which has no service directories. Screenshot and D-Bus trace SHA-256 digests are listed below.

- Host: `lizhan-spark`, Ubuntu 24.04, Linux aarch64, kernel `6.17.0-1029-nvidia`.
- Session: Weston 13.0.0 nested on Xvfb `:102`, Wayland socket `wayland-test2`. The app ran with `GDK_BACKEND=wayland`; captures are of the nested Weston output, not a native Wayland desktop.
- App / worker: `target/release/omo-app` SHA-256 `7bf424781804ec8894a488f6d41227f5b0af12ae56d2e9231bab830cb71eb0d7`; `target/release/omo-worker` SHA-256 `bcd1e8deaf89f82e721695fdb105d93939e5345b805298798bfce2f1d1ea0b24`.
- Bootstrap: current `community-signed/linux_arm64` artifact and probe, with `no_network=true`; the engine reached `ENGINE READY`.
- GTK4 `4.14.5`, NSH `0.5.1`. Both test sessions set `GTK_USE_PORTAL=1` and `GIO_USE_PORTALS=1`.

## Portal returns an error

A private session bus hosted a test `org.freedesktop.portal.Desktop` service. Its `FileChooser.OpenFile` and `SaveFile` methods returned the D-Bus error `org.freedesktop.portal.Error.Failed` with message `mock desktop portal rejected the file chooser request`. The attached bus traces record the method calls and error replies.

- **Open:** Browse for CSV called `OpenFile`; GTK displayed its native open chooser inside nested Weston after the error. Escape closed the chooser and the workbench still reported `No station CSV selected`.
- **Manual path:** After the failed portal request, entering `/home/blizhan/repo/github/OmO/fixtures/stations/m0.csv` and selecting **Use path** produced `Path saved in this window only; CSV ingestion is not connected yet.`
- **Save:** `SELECT 1 AS value` completed with one row. Export called `SaveFile`; GTK displayed its native save chooser inside nested Weston after the error. Escape returned to the workbench with the preview intact and `No file action yet`.

Screenshots and D-Bus traces: [open chooser](n5-spark-aarch64-wayland-20261120/open-dialog-portal-error.png), [open cancellation](n5-spark-aarch64-wayland-20261120/open-cancelled.png), [manual path accepted after error](n5-spark-aarch64-wayland-20261120/manual-path-after-portal-error.png), [save chooser](n5-spark-aarch64-wayland-20261120/save-dialog-portal-error.png), [save cancellation](n5-spark-aarch64-wayland-20261120/save-dialog-cancelled.png), [OpenFile error trace](n5-spark-aarch64-wayland-20261120/dbus-open-error.log), [SaveFile error trace](n5-spark-aarch64-wayland-20261120/dbus-save-error.log), and [mock service log](n5-spark-aarch64-wayland-20261120/mock-portal.log).

## Portal service unavailable

A second run used a private D-Bus configuration with no service directories and no portal service owner. GTK sent `OpenFile` and `SaveFile` requests, each receiving `org.freedesktop.DBus.Error.ServiceUnknown` for `org.freedesktop.portal.Desktop`. GTK displayed its native open and save choosers inside nested Weston. Cancelling Open left `No station CSV selected`; cancelling Save preserved the completed one-row preview and `No file action yet`.

Evidence: [open chooser](n5-spark-aarch64-wayland-20261120/no-portal-open-dialog.png), [open cancellation](n5-spark-aarch64-wayland-20261120/no-portal-open-cancelled.png), [save chooser](n5-spark-aarch64-wayland-20261120/no-portal-save-dialog.png), [save cancellation](n5-spark-aarch64-wayland-20261120/no-portal-save-cancelled.png), [OpenFile trace](n5-spark-aarch64-wayland-20261120/dbus-no-portal-open.log), and [SaveFile trace](n5-spark-aarch64-wayland-20261120/dbus-no-portal-save.log).

| Evidence file | SHA-256 |
|---|---|
| `open-dialog-portal-error.png` | `2e7095b80a222beeb3fd125d9b9a1d280cd9e758bcfed5dba6f3cbaca6940af4` |
| `open-cancelled.png` | `010cf0771d972bcd4b5c7c656076f69034741a561f60c2ec3aed85dfa55a4b00` |
| `manual-path-after-portal-error.png` | `0a7e1c692375f26bccc3c359490d8e60342db5e8c8d1146636a432e0560d2417` |
| `save-dialog-portal-error.png` | `91e24b7643bd5c42d24f0d14b02ce09839a952e1e52d2185ef847e180e750ace` |
| `save-dialog-cancelled.png` | `69247472260120bab886528d3eae5ddb15a4a589d4eef25ed5400436bea88855` |
| `no-portal-open-dialog.png` | `edd8aed53b70cf8f891a4a45e23a7eebf9f318a1fc485303555227d6880162f6` |
| `no-portal-open-cancelled.png` | `44f28dbfef203bcd521d4ef42d700e1ec3a0d41531ddbe92b6e191b0676dc21d` |
| `no-portal-save-dialog.png` | `e8007d798939f84d236d474498fc8934928e0a290d3fd344b5c72b02f713dc78` |
| `no-portal-save-cancelled.png` | `3bfbb26963b25f28a4ed2d688a3324bbe7654255eb7547deb348f40115e381ac` |
| `dbus-open-error.log` | `4e6487d3005e716462a7249a1e644d107a91f64fb8d5164bdffd40c5e0b0a944` |
| `dbus-save-error.log` | `6261004c565c53582653e2ef089cc9738679dc55eb91a1a74a2f141ca2b1e599` |
| `dbus-no-portal-open.log` | `8f14a13ff32ab2d38b5f2942ca584e2816985959df0c0ec1f9ba0542c576eb31` |
| `dbus-no-portal-save.log` | `eda0a328c4919372b424aa6d06aec8c48568e7b56c32afb49e390f2973c8b958` |
| `mock-portal.log` | `e9047840459af87b3562f66cba3122ff345dd478336348f1fd19bbdce0d0cb12` |
| `mock-portal.c` | `d3823195aa2d59ca26e46eb4568475b41cec645d194aa247900fb397cbfa626e` |
| `no-portal-session.conf` | `792ffb7a5cb0ee3347a534f401cbd388b8b2f5c85584ef9e21f65f98d9ce20e6` |

## Scope

This closes the nested Weston Wayland-client cases for portal D-Bus errors, no-service fallback, manual path entry, and open/save cancellation. The successful portal path and its cancellation were exercised in the [2026-11-18 rerun](N5-spark-aarch64-wayland-20261118.md); that desktop portal UI rendered on host X11 outside Weston. Native Wayland desktop, Linux x86_64, and macOS dialog sessions remain unverified. The Linux x86_64 and macOS N5 cells remain `not-run`.
