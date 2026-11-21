# SPIKE-3 signed local install/load — spark-aarch64 — 2026-11-16

- Host: Ubuntu 24.04.4 LTS, aarch64, kernel `6.17.0-1029-nvidia`.
- Engine: DuckDB `v2.0.0` from `duckdb =1.20000.1`; platform probe reports `linux_arm64`.
- Lock entry: channel `community-signed`, extension release `v0.4.0`, DuckDB `v2.0.0`, Linux arm64. Compressed SHA-256 `a5429c62ec03142f4190d16959dd15584387da240bdf7dbe34e1b3d99007c9ac` (8,846,401 bytes); decompressed SHA-256 `03afe7177e1ebd4bf288e056f2a3013627db1084c2968911bc8c5d135fab6542` (29,347,014 bytes). Both hashes match the current downloads and the lock.
- The first direct `LOAD '<verified absolute path>'` attempt used a digest-named cache file and failed signature verification. After correcting the cache basename to `duckomo.duckdb_extension`, direct local `LOAD` still failed with `signature is either missing or invalid` while `allow_unsigned_extensions=false`.
- Controlled local install candidate used a fresh empty extension cache, a separate process, `--no-network`, `autoinstall_known_extensions=false`, `autoload_known_extensions=false`, and `allow_unsigned_extensions=false`. The engine ran `INSTALL '<verified absolute path>';` and would then run `LOAD duckomo;` only if installation succeeded.
- Observed setting snapshot before install: `allow_community_extensions=true`, `allow_extension_repositories=undecided`, `allow_extensions_metadata_mismatch=false`, `allow_unsigned_extensions=false`, `autoinstall_extension_repository=''`, `autoinstall_known_extensions=false`, `autoload_known_extensions=false`, `custom_extension_repository=''`, `extension_directories=[]`, and `extension_repository_directory=''`. No separate signature-key setting was exposed in this snapshot.

## Result

- `INSTALL`: failed with `Attempting to install an extension file that doesn't have a valid signature` and DuckDB's signed-extension security documentation URL.
- `LOAD duckomo`: not attempted after the failed `INSTALL`. The separate direct local `LOAD` attempt also failed signature verification as recorded above.
- No unsigned retry occurred. There is no `installed_from` metadata or installed digest because installation did not complete.
- Conclusion is limited to this locked community artifact, DuckDB `v2.0.0`, and the tested Linux arm64 local routes. The GitHub release unsigned artifact is a separate lock entry and process.

This is a measured channel failure, not a passing compatibility result. The per-platform channel decision is recorded separately; other target platforms remain untested.
