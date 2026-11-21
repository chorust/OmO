# Community local-repository network isolation — spark-aarch64 — 2026-11-20

The signed-channel bootstrap was run in a bubblewrap network namespace (`bwrap --unshare-net`) with the repository read-only and a fresh writable `/tmp` data/extension cache. The only Duckomo artifact input was the locked compressed community file. The probe also used `--no-network`, so its HTTPFS install path was intentionally skipped.

```bash
root=/tmp/omo-probe-community-hardening-smoke-20261120
mkdir -p "$root/data" "$root/extensions"
bwrap --unshare-net \
  --ro-bind / / \
  --dev-bind /dev /dev \
  --proc /proc \
  --bind "$root" "$root" \
  --chdir /home/blizhan/repo/github/OmO \
  ./target/debug/omo-probe \
    --channel community-signed --repeat 1 --no-network --verbose \
    --artifact /tmp/omo-probe-community-route-20261120/artifact/duckomo.duckdb_extension.gz \
    --lock artifacts/duckomo.lock.json \
    --data-dir "$root/data" \
    --ext-cache-dir "$root/extensions"
```

- Current probe SHA-256: `0348de869c4c3bd52b130f9ac29b94383f66ea87d9a1393bb55f1e189069936d`.
- Bubblewrap accepted `--unshare-net`, so the process had no network namespace access.
- Result: 1/1 passed; Duckomo `v0.4.0`; 64 rows; golden SHA-256 `bc7cf6db243073d9baee64cdda6900cae9547a33bde00f61b8da10a7ce30254f`.
- Remote task result: `rejected_with_capability_unavailable_remote`.
- Final policy: unsigned loading false, built-in community loading false, and custom repository access forbidden after the load.
- The extension cache root, repository metadata directory, staged version/platform directories, and installed repository directories checked in the smoke run were mode `0700`; artifact and lock files were mode `0600`. Some intermediate staging container directories retain their umask mode, but remain inaccessible through the owner-only cache root. A separate [conflicting-cache run](SPIKE-3-community-conflicting-cache-spark-aarch64-20261120.md) verifies that boundary while retaining a pre-existing canonical file.
- Fresh cache report: [JSON](EXIT-1-spark-aarch64-community-hardening-smoke-20261120.json), [probe log](EXIT-1-spark-aarch64-community-hardening-smoke-20261120.log).

This isolates the signed local artifact route from network access. HTTPFS provisioning is outside this check and was intentionally skipped.
