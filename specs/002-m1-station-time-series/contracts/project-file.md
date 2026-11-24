# Contract: project file `omo-project/1` and drafts

- **Extension**: `.omoproj`. UTF-8 JSON. Field definitions are in
  [data-model › ProjectDoc](../data-model.md#projectdoc-omo-projectdoc-file-schema-omo-project1).
- **Save** (atomic):
  1. Serialize.
  2. Write `.<name>.omoproj.tmp` in the target directory and fsync it.
  3. Rename it over the target. A cross-filesystem rename (EXDEV) is an error.
  4. fsync the directory.
  5. Set `saved_revision = revision` and delete the draft.
- **Open**:
  - an unknown or newer `schema` major version → refuse with a message and do not modify the
    file;
  - recompute the stations hash from the inline snapshot and refuse on mismatch
    (`CorruptSnapshot`);
  - if `stations.source.orig_path` exists with a different SHA-256, show an informational
    notice and keep the snapshot (spec US5-4).
- **Drafts**:
  - path: `<app_data>/drafts/<doc_id>.json`;
  - written every 30 s while dirty, 2 s after the last edit (debounce), and on close-request;
  - never written to a project path.
- **Recovery**: at startup, list drafts. Offer recovery for a draft when its
  `staged_at_utc` > the project file mtime, or when it belongs to an unsaved document.
  - Accept → the draft opens as an edited (dirty) document.
  - Discard → the draft is deleted.
- **Not stored**: results, readiness, and worker state.

Minimal example:

```json
{
  "schema": "omo-project/1",
  "doc_id": "6f1c…",
  "stations": {
    "content_sha256": "…",
    "mapping": {"id": "id", "lat": "lat", "lon": "lon", "name": "name"},
    "source": {"orig_path": "/data/stations.csv", "orig_sha256": "…", "imported_at_utc": "2026-11-23T07:00:00Z"},
    "rows": [{"station_id": "A1", "latitude": 39.9, "longitude": 116.4, "name": "北京"}]
  },
  "catalog_selection": {"entry_id": "ecmwf_ifs.temperature_2m", "catalog_version": "2026-11-23.1"},
  "window": {"start_h": 0, "end_h": 168},
  "max_distance_m": 7200,
  "sql_tabs": [{"title": "Daily max", "sql": "SELECT …"}],
  "view": {"selected_station_ids": ["A1"], "chart": {"show_legend": true, "y_auto": true}}
}
```
