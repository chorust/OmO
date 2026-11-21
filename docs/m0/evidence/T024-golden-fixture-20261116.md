# T024 golden fixture and independent result reference — 2026-11-16

## Retained fixture

- Path: `fixtures/om/chmi-aladin-cz-hsurf.om`
- Open-Meteo object: `s3://openmeteo/data/chmi_aladin_cz_1km/static/HSURF.om`
- HTTPS URL: <https://openmeteo.s3.us-west-2.amazonaws.com/data/chmi_aladin_cz_1km/static/HSURF.om>
- Distribution license: CC BY 4.0 per the [AWS Open Data Registry](https://registry.opendata.aws/open-meteo/) and the [Open-Meteo open-data README](https://github.com/open-meteo/open-data/blob/main/README.md). Attribution: Open-Meteo.
- HTTP metadata on 2026-11-16: length 141,504 bytes; `Last-Modified: 2026-06-23T07:41:27Z`; ETag `65cc6a165f56868639a75657aaa007c9-1`; no S3 VersionId.
- Direct HTTPS GET and retained file were byte-identical. Both SHA-256 values are `91188834f14615910bb01bb952d87e3d8da20835bbfd4aede6fd82bb6597d6fe`.
- The retained bytes were copied from Duckomo's existing public sample cache. Duckomo was not modified.

## Golden query and independent hash

`golden-v1` is `SELECT value FROM read_om(<fixture>) ORDER BY value ASC NULLS FIRST LIMIT 64`. The hash encoding is `0x00` for NULL and `0x01 || IEEE-754 float32 big-endian` for each value. The local helper `scripts/hash-golden-oracle.py` applies the same ordering and encoding to the official OM reader's CSV output.

- Oracle command: `/home/blizhan/repo/github/duckomo/build/release-vcpkg/test/tools/duckomo_fixture_tool --oracle fixtures/om/chmi-aladin-cz-hsurf.om --csv docs/m0/evidence/T024-golden-v1-oracle.csv`.
- Oracle tool SHA-256: `382588fa219cc154a7a372a8a523faa9d301676a5c7782e307147d17a92df926`.
- CSV: 145,290 rows, 1,488,439 bytes, SHA-256 `02e9deea7f4fa28d569854224558339dd49c7b59385e3800149ccf9460d59de0`; retained at `docs/m0/evidence/T024-golden-v1-oracle.csv`.
- Command: `python3 scripts/hash-golden-oracle.py docs/m0/evidence/T024-golden-v1-oracle.csv`.
- Independent expected hash: `bc7cf6db243073d9baee64cdda6900cae9547a33bde00f61b8da10a7ce30254f` over 64 rows.
- A measurement-only unsigned bootstrap on Linux arm64, DuckDB `v2.0.0`, and Duckomo `v0.4.0` computed the same hash. That report had `expected: null` and `ok: false`; it is recorded as a hash comparison, not as a passing locked bootstrap. T029 records the separate locked pass.

The per-platform/channel lock remains the authority for expected hashes. Only the Linux arm64 unsigned entry is eligible for pinning after its locked repeat run passes; other entries remain placeholders until their platform/channel runs are verified.
