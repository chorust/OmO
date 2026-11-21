# N4 on spark-aarch64/wayland — 2026-11-17

- Host / OS: lizhan-spark, Ubuntu 24.04, Linux aarch64, kernel `6.17.0-1029-nvidia`.
- Session: nested Weston 13.0.0 Wayland compositor on Xvfb `:102` (1600×1000), Wayland socket `wayland-test2`. The host had no native Wayland desktop. The Omobench process environment recorded `GDK_BACKEND=wayland`, `WAYLAND_DISPLAY=wayland-test2`, and `XDG_RUNTIME_DIR=/tmp/omo-wayland/runtime2`; its process maps included `libwayland-client.so.0`. Xvfb `:102` supplied input and screenshots to the Weston X11 backend; Omobench itself connected to the Wayland socket.
- Versions: Rust `1.100.0`, GTK4 `4.14.5`, NSH `0.5.1`, Weston `13.0.0`, `wl-clipboard` `2.2.1-1build1` (extracted under `/tmp/omo-wayland`, not installed system-wide).
- Application: `target/release/omo-app`, SHA-256 `508062f63348612804140e924a25253fa92f8ece93cde7fc05d115698934df8e`.
- Result: **pass for N4 editor interactions in this nested Weston Wayland session.** This is not a native desktop run and does not close the spark Wayland N2–N8 matrix.

## Verified behavior

The editor accepted multiline SQL and highlighted the `SELECT` keyword, numeric literals, and SQL comment in the [multiline screenshot](n4-spark-aarch64-wayland-20261117/editor-multiline-highlight.png). The query buffer was:

```sql
SELECT
  1 AS id, -- identifier
  2 AS value
```

A separate incremental quote-entry check showed the opening quote pair and caret in the [quote-open screenshot](n4-spark-aarch64-wayland-20261117/quote-open.png). Typing `alpha`, closing the pair, and entering the suffix produced the exact clipboard buffer `SELECT 'alpha' AS name`; the [final screenshot](n4-spark-aarch64-wayland-20261117/quote-final.png) shows string highlighting. The buffer was read through the Wayland clipboard with `wl-paste --no-newline`.

Copy and paste were verified in both directions. `Ctrl+A`, `Ctrl+C`, followed by `wl-paste --no-newline`, returned the exact three-line query above. Separately, `wl-copy 'LIMIT 7'` was read back as exactly `LIMIT 7`, then `Ctrl+V` inserted it after the query. The [before-undo screenshot](n4-spark-aarch64-wayland-20261117/editor-before-undo.png) shows the pasted line.

The app uses separate editor history entries for Return and pasted text. Two `Ctrl+Z` presses produced these exact buffers, read back through the clipboard:

1. `SELECT\n  1 AS id, -- identifier\n  2 AS value\n  ` — the pasted `LIMIT 7` is removed; the blank line retains two auto-indent spaces. See [undo step 1](n4-spark-aarch64-wayland-20261117/undo-step-1.png).
2. `SELECT\n  1 AS id, -- identifier\n  2 AS value` — the blank line is removed. See [after undo](n4-spark-aarch64-wayland-20261117/editor-after-undo.png).

Two `Ctrl+Shift+Z` presses restored those states in reverse order. [Redo step 1](n4-spark-aarch64-wayland-20261117/redo-step-1.png) matches the first buffer, and [after redo](n4-spark-aarch64-wayland-20261117/editor-after-redo.png) returned the exact pre-undo buffer `SELECT\n  1 AS id, -- identifier\n  2 AS value\n  LIMIT 7`.

For application-to-application copy/paste, the complete four-line buffer was copied with `Ctrl+A`, `Ctrl+C` and read exactly through `wl-paste`. After collapsing the selection with `Right`, `Return` and `Ctrl+V` pasted the copied buffer into the editor. The resulting exact buffer was `SELECT\n  1 AS id, -- identifier\n  2 AS value\n  LIMIT 7\n  SELECT\n  1 AS id, -- identifier\n  2 AS value\n  LIMIT 7`; the two leading spaces before the second `SELECT` are the editor’s auto-indent. The [copy/paste screenshot](n4-spark-aarch64-wayland-20261117/editor-copy-paste.png) shows the duplicate.

No query execution result is claimed for this editor interaction check.

## Artifact hashes

| Artifact | SHA-256 |
|---|---|
| `editor-multiline-highlight.png` | `73fa94f1ec6a9b5ee3c3d29eec96d367628712ba8625041746daed145afcb77b` |
| `editor-before-undo.png` | `73154937cb3f8a159de9b3cb7007b00d8dc6d7d3070b4ca66f83c6c5bdbce82a` |
| `editor-after-undo.png` | `b4f41976d92579e91c241c1edc897bcf2cea47e259766441fdfa890c0fb9be25` |
| `editor-after-redo.png` | `79238b0ea7b858bd0432d55cdeca407d9386f17bb75b0d2ce02249b6e47ccba6` |
| `editor-copy-paste.png` | `0066dec9a984b81aed2046ee37bb77ef2a74289ee78a9dd98a32aa2e33612001` |
| `quote-open.png` | `19a9bfd90090638d88cdc25a714401d88a0bc8ccb221b52acf4d5351f5df87fa` |
| `quote-final.png` | `3f4ad82307773881b06938ac9bf9181d8ecfc20529cc9044583696089868dac3` |
| `undo-step-1.png` | `4dd221af65b18f96dd6684ce0e7b0195de914a34c18a0fabe822899b7c995fa5` |
| `redo-step-1.png` | `d3cdf3a2781a2bf98f85f78ea2b42e7f9e43781f9a3b269ea92144a09f860d8f` |
