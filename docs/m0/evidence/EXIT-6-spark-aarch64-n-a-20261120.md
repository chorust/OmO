# EXIT-6 export recovery follow-up on spark-aarch64/n-a — 2026-11-20

- Host / OS / kernel: lizhan-spark, Ubuntu 24.04, Linux aarch64, kernel `6.17.0-1029-nvidia`.
- Toolchain: Rust `1.100.0`; current workspace source; `omo-harness` built with `--locked -p omo-app --no-default-features --features in-process-engine --bin omo-harness` after the system GTK/GStreamer install recorded in [N1](N1-spark-aarch64-n_a-20261119.md).
- Commands: `omo-harness export-kill --at step1`, `--at step2`, and `--at step3`.
- Result: **pass on Spark.** Step 1 recovered as incomplete with `saved=false` and staging preserved; step 2 also recovered as incomplete with `saved=false` and staging preserved; step 3 recovered as finished with `saved=true` and staging removed.
- Outputs: [step 1 JSON](EXIT-6-spark-aarch64-n-a-20261120-step1.json), [step 2 JSON](EXIT-6-spark-aarch64-n-a-20261120-step2.json), [step 3 JSON](EXIT-6-spark-aarch64-n-a-20261120-step3.json). Export scanner and EXDEV tests are included in the [N1 workspace test log](N1-spark-aarch64-n_a-20261119-test.log).
- Scope: Linux x86_64 and macOS arm64 remain not-run. This Spark rerun does not close T061.
