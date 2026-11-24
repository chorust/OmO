# Contract: station CSV import

| Aspect | Rule |
|---|---|
| Encoding | UTF-8, BOM allowed. Anything else → `Encoding{byte_offset}` error |
| Delimiter | sniffed from the header line: `,` or `;`. Ambiguous → ask the user to choose |
| Quoting | RFC 4180 double quotes |
| Header | required. Mapping proposed from aliases (research R-04); the user confirms |
| Required columns | id, latitude, longitude (decimal degrees, WGS 84) |
| Optional columns | name, elevation (m) |
| Validation | all issues reported at once, with line numbers: empty input, duplicate id, non-numeric, out of range |
| Normalization | longitude in 0..360 → [−180, 180) |
| Size | no hard limit. Above 1,000 rows → warning and confirmation (provisional) |
| Snapshot hash | SHA-256 of canonical CSV bytes: header `station_id,latitude,longitude,name,elevation_m`, LF, shortest round-trip float formatting, empty field for missing optional values |
| Worker handoff | canonical CSV written to `<session_tmp>/stations-<sha256>.csv`, then passed as `stations_ref` |

Error identifiers (`ImportError`): `Empty`, `NoHeader`, `Encoding`, `AmbiguousDelimiter`,
`MappingIncomplete`, and `Rows(Vec<RowIssue>)`.
