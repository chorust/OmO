# EXIT-3 on spark-aarch64/n/a — 2026-11-23 (final NSH acceptance)

- Decision owner: the user, under FR-022 (only the user may classify N-results as blocking).
- Decision (2026-11-23): **accept NSH 0.5.1 for M0. No §7.3 GPUI Kit fallback.** The remaining
  Spark item, the N6 Wayland IBus candidate-popup display, is recorded as a known
  **non-blocking limitation**.
- Basis:
  - None of the three user-approved D8 blocking examples was observed:
    - the Spark build passes;
    - the million-row N2 table is usable on X11 and in nested Weston Wayland;
    - N6 IME passes on X11 for both IBus/libpinyin and Fcitx5. In nested Weston Wayland, Fcitx
      selected and committed a candidate, and IBus committed a candidate.
  - The IBus candidate popup did not map in nested Weston 13 because the protocols do not
    overlap: Weston 13 implements only `zwp_text_input_v1`, and GTK4 4.14.5 implements only
    `zwp_text_input_v3`. This is a limitation of the nested test compositor; no native Wayland
    GTK failure was demonstrated. See
    [N6 nested Weston IBus verification](N6-spark-aarch64-wayland-20261121.md).
  - Non-reference platforms are closed by
    [GitHub CI/release evidence](CI-CROSS-PLATFORM-20261123.md) under Constitution 1.3.0.
- Result: **pass (user FR-022 decision)**. SPIKE-1 for spark is closed by the same decision.
- Follow-up: M1 includes a Chinese IME regression pass (design §7.4). If a native Wayland
  desktop with `zwp_text_input_v3` IBus support becomes available, re-run N6 there. A blocking
  result would reopen this decision.
- Supersedes the 2026-11-17 `not-run` record in [EXIT-3 (2026-11-17)](EXIT-3-spark-aarch64-n-a-20261117.md).
