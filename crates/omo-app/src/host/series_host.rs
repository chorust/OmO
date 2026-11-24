//! UI-free M1 host state: catalog loading, station CSV import session,
//! match Arrow decoding, and v2 export sidecar assembly.

use std::{
    env, fs,
    path::{Path, PathBuf},
};

use arrow::array::{Array, Float64Array, StringArray};
use arrow_ipc::reader::FileReader;
use omo_project::stations::{
    ColumnMapping, RowIssue, StationSnapshot,
    import::{
        ImportError, ParsedTable, confirm, import_csv, import_csv_with_delimiter, write_canonical,
    },
};
use omo_protocol::series::{CatalogDocument, CatalogEntry, SeriesResultBody};
use serde_json::{Value, json};

pub type CatalogFile = CatalogDocument;

pub fn load_catalog() -> Result<CatalogFile, String> {
    let mut candidates = Vec::new();
    if let Some(explicit) = env::var_os("OMOBENCH_CATALOG") {
        candidates.push(PathBuf::from(explicit));
    }
    candidates.push(PathBuf::from("catalog/products.json"));
    if let Ok(exe) = env::current_exe()
        && let Some(dir) = exe.parent()
    {
        candidates.push(dir.join("catalog/products.json"));
        candidates.push(dir.join("../catalog/products.json"));
        candidates.push(dir.join("../../catalog/products.json"));
    }
    candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../catalog/products.json"));

    for path in candidates {
        if path.is_file() {
            let text = fs::read_to_string(&path)
                .map_err(|error| format!("cannot read catalog {}: {error}", path.display()))?;
            return CatalogDocument::parse(&text)
                .map_err(|error| format!("invalid catalog {}: {error}", path.display()));
        }
    }
    Err("catalog/products.json not found".into())
}

#[derive(Clone, Debug, PartialEq)]
pub struct StationImportDraft {
    pub orig_path: String,
    pub orig_bytes: Vec<u8>,
    pub imported_at_utc: String,
    pub delimiter: Option<u8>,
    pub ambiguous_delimiter: bool,
    pub parsed: Option<ParsedTable>,
    pub mapping: ColumnMapping,
    pub issues: Vec<RowIssue>,
    pub error: Option<String>,
    pub large_file_confirmed: bool,
}

impl StationImportDraft {
    pub fn from_path(path: &Path, now_utc: &str) -> Self {
        let orig_path = path.to_string_lossy().into_owned();
        match fs::read(path) {
            Ok(bytes) => Self::from_bytes(orig_path, bytes, now_utc),
            Err(error) => Self {
                orig_path,
                orig_bytes: Vec::new(),
                imported_at_utc: now_utc.to_owned(),
                delimiter: None,
                ambiguous_delimiter: false,
                parsed: None,
                mapping: empty_mapping(),
                issues: Vec::new(),
                error: Some(format!("Cannot read file: {error}")),
                large_file_confirmed: false,
            },
        }
    }

    pub fn from_bytes(orig_path: String, orig_bytes: Vec<u8>, now_utc: &str) -> Self {
        let mut draft = Self {
            orig_path: orig_path.clone(),
            orig_bytes,
            imported_at_utc: now_utc.to_owned(),
            delimiter: None,
            ambiguous_delimiter: false,
            parsed: None,
            mapping: empty_mapping(),
            issues: Vec::new(),
            error: None,
            large_file_confirmed: false,
        };
        match import_csv(&draft.orig_bytes, &orig_path, now_utc) {
            Ok(parsed) => {
                draft.delimiter = Some(parsed.delimiter);
                draft.mapping = parsed.proposed_mapping.clone();
                draft.parsed = Some(parsed);
                draft.revalidate();
            }
            Err(ImportError::AmbiguousDelimiter) => {
                draft.ambiguous_delimiter = true;
                draft.error = Some(
                    "Both comma and semicolon delimiters appear consistently. Choose the delimiter below."
                        .into(),
                );
            }
            Err(error) => {
                draft.error = Some(format_import_error(&error));
            }
        }
        draft
    }

    pub fn set_delimiter(&mut self, delimiter: u8) {
        self.delimiter = Some(delimiter);
        self.ambiguous_delimiter = false;
        self.error = None;
        self.issues.clear();
        match import_csv_with_delimiter(
            &self.orig_bytes,
            &self.orig_path,
            &self.imported_at_utc,
            delimiter,
        ) {
            Ok(parsed) => {
                self.mapping = parsed.proposed_mapping.clone();
                self.parsed = Some(parsed);
                self.revalidate();
            }
            Err(error) => {
                self.parsed = None;
                self.error = Some(format_import_error(&error));
            }
        }
    }

    pub fn set_mapping(&mut self, mapping: ColumnMapping) {
        self.mapping = mapping;
        self.revalidate();
    }

    pub fn headers(&self) -> &[String] {
        self.parsed
            .as_ref()
            .map(|parsed| parsed.headers.as_slice())
            .unwrap_or(&[])
    }

    pub fn mapping_complete(&self) -> bool {
        !self.mapping.id.is_empty() && !self.mapping.lat.is_empty() && !self.mapping.lon.is_empty()
    }

    pub fn exceeds_large_warning(&self) -> bool {
        self.parsed
            .as_ref()
            .is_some_and(ParsedTable::exceeds_large_warning)
    }

    pub fn needs_large_confirmation(&self) -> bool {
        self.exceeds_large_warning() && !self.large_file_confirmed
    }

    pub fn large_warning_message(&self) -> Option<String> {
        let count = self.parsed.as_ref()?.data_rows.len();
        if count > omo_project::stations::import::LARGE_STATION_WARNING_THRESHOLD {
            Some(format!(
                "Warning: CSV contains {count} stations (> 1,000). Confirm large import to proceed."
            ))
        } else {
            None
        }
    }

    pub fn can_confirm(&self) -> bool {
        self.parsed.is_some()
            && self.mapping_complete()
            && self.issues.is_empty()
            && self.error.is_none()
            && !self.needs_large_confirmation()
    }

    pub fn confirm_and_write(&self, app_data: &Path) -> Result<(StationSnapshot, PathBuf), String> {
        let Some(parsed) = &self.parsed else {
            return Err(self
                .error
                .clone()
                .unwrap_or_else(|| "No parsed CSV is ready.".into()));
        };
        let snapshot = confirm(
            parsed,
            self.mapping.clone(),
            &self.orig_path,
            &self.orig_bytes,
            &self.imported_at_utc,
        )
        .map_err(|error| format_import_error(&error))?;
        let canonical_path = write_canonical(&snapshot, &app_data.join("stations"))
            .map_err(|error| format_import_error(&error))?;
        Ok((snapshot, canonical_path))
    }

    fn revalidate(&mut self) {
        self.issues.clear();
        let Some(parsed) = &self.parsed else {
            return;
        };
        if !self.mapping_complete() {
            self.error = Some("Map all required columns: id, latitude, longitude.".into());
            return;
        }
        self.error = None;
        match confirm(
            parsed,
            self.mapping.clone(),
            &self.orig_path,
            &self.orig_bytes,
            &self.imported_at_utc,
        ) {
            Ok(_) => {}
            Err(ImportError::Rows(issues)) => {
                self.issues = issues;
            }
            Err(error) => {
                self.error = Some(format_import_error(&error));
            }
        }
    }
}

fn empty_mapping() -> ColumnMapping {
    ColumnMapping {
        id: String::new(),
        lat: String::new(),
        lon: String::new(),
        name: None,
        elevation: None,
    }
}

fn format_import_error(error: &ImportError) -> String {
    match error {
        ImportError::Encoding { byte_offset } => {
            format!("File is not valid UTF-8 at byte offset {byte_offset}.")
        }
        ImportError::Empty => "CSV file has no data rows.".into(),
        ImportError::NoHeader => "CSV file has no header row.".into(),
        ImportError::AmbiguousDelimiter => {
            "Both comma and semicolon delimiters are plausible; choose one.".into()
        }
        ImportError::MappingIncomplete => {
            "Required mapping incomplete (need id, latitude, longitude).".into()
        }
        ImportError::Rows(issues) => format!("{} row validation issue(s) found.", issues.len()),
        ImportError::Io(message) => format!("I/O error: {message}"),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct StationMatchInfo {
    pub station_id: String,
    pub status: String,
    pub reason: String,
    pub grid_lat: Option<f64>,
    pub grid_lon: Option<f64>,
    pub distance_m: Option<f64>,
}

pub fn read_matches_arrow(path: &Path) -> Result<Vec<StationMatchInfo>, String> {
    let file = fs::File::open(path)
        .map_err(|error| format!("cannot open matches Arrow {}: {error}", path.display()))?;
    let reader = FileReader::try_new(file, None)
        .map_err(|error| format!("invalid matches Arrow {}: {error}", path.display()))?;
    let mut rows = Vec::new();
    for batch in reader {
        let batch = batch.map_err(|error| format!("Arrow batch error: {error}"))?;
        let station_id = string_col(&batch, "station_id")?;
        let status = string_col(&batch, "status")?;
        let reason = string_col(&batch, "reason")?;
        let grid_lat = f64_col(&batch, "grid_lat")?;
        let grid_lon = f64_col(&batch, "grid_lon")?;
        let distance_m = f64_col(&batch, "distance_m")?;

        for row in 0..batch.num_rows() {
            rows.push(StationMatchInfo {
                station_id: station_id.value(row).to_owned(),
                status: status.value(row).to_owned(),
                reason: if reason.is_null(row) {
                    String::new()
                } else {
                    reason.value(row).to_owned()
                },
                grid_lat: (!grid_lat.is_null(row)).then(|| grid_lat.value(row)),
                grid_lon: (!grid_lon.is_null(row)).then(|| grid_lon.value(row)),
                distance_m: (!distance_m.is_null(row)).then(|| distance_m.value(row)),
            });
        }
    }
    Ok(rows)
}

fn string_col<'a>(
    batch: &'a arrow::array::RecordBatch,
    name: &str,
) -> Result<&'a StringArray, String> {
    let col = batch
        .column_by_name(name)
        .ok_or_else(|| format!("missing `{name}` column"))?;
    col.as_any()
        .downcast_ref::<StringArray>()
        .ok_or_else(|| format!("column `{name}` is not Utf8"))
}

fn f64_col<'a>(
    batch: &'a arrow::array::RecordBatch,
    name: &str,
) -> Result<&'a Float64Array, String> {
    let col = batch
        .column_by_name(name)
        .ok_or_else(|| format!("missing `{name}` column"))?;
    col.as_any()
        .downcast_ref::<Float64Array>()
        .ok_or_else(|| format!("column `{name}` is not Float64"))
}

pub fn build_m1_sidecar_json(
    stations_sha256: Option<&str>,
    station_count: usize,
    entry: &CatalogEntry,
    catalog_version: &str,
    series: Option<&SeriesResultBody>,
    sql_query: Option<&str>,
) -> Value {
    let run_year = series.and_then(|body| {
        let prefix = body.run.reference_time_utc.get(..4)?;
        prefix.chars().all(|ch| ch.is_ascii_digit()).then_some(prefix)
    });
    let attribution = entry
        .attribution
        .iter()
        .map(|line| match run_year {
            Some(year) => line.replace("<run year>", year),
            None => line.clone(),
        })
        .collect::<Vec<_>>();
    json!({
        "stations": stations_sha256.map(|sha| json!({
            "sha256": sha,
            "count": station_count,
        })),
        "catalog": {
            "entry_id": entry.id,
            "version": catalog_version,
        },
        "run": series.map(|body| {
            let object_version = body
                .run
                .meta_etag
                .as_deref()
                .or_else(|| body.scan_metrics.get("version_token").and_then(Value::as_str));
            json!({
                "reference_time_utc": body.run.reference_time_utc,
                "cycle": body.run.cycle,
                "object_url": body.run.object_url,
                "object_version": object_version,
            })
        }),
        "window": series.map(|body| json!({
            "requested": body.effective_window.requested,
            "effective": body.effective_window.effective,
            "truncated": body.effective_window.truncated,
        })),
        "plan": series.map(|body| body.plan.as_str()).unwrap_or("G"),
        "query": sql_query,
        "attribution": attribution,
    })
}

pub fn now_utc_iso() -> String {
    omo_protocol::utc::now_utc_rfc3339()
}

/// `YYYY-MM-DDTHH:MM:SSZ` plus whole hours, in the same format.
pub fn add_hours_iso(iso: &str, hours: u32) -> Option<String> {
    let bytes = iso.as_bytes();
    if bytes.len() != 20 || bytes[10] != b'T' || bytes[19] != b'Z' {
        return None;
    }
    let number = |range: std::ops::Range<usize>| iso.get(range)?.parse::<i64>().ok();
    let (year, month, day) = (number(0..4)?, number(5..7)?, number(8..10)?);
    let (hour, minute, second) = (number(11..13)?, number(14..16)?, number(17..19)?);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let secs = omo_protocol::utc::days_from_civil(year, month, day) * 86_400
        + hour * 3600
        + minute * 60
        + second
        + i64::from(hours) * 3600;
    Some(omo_protocol::utc::format_utc_rfc3339(secs))
}

/// Clamp a lead-time view window to the received (effective) range.
/// Returns the clamped window and whether it differed from the request.
pub fn clamp_view_window(view: [u32; 2], effective: [u32; 2]) -> ([u32; 2], bool) {
    let start = view[0].clamp(effective[0], effective[1]);
    let end = view[1].clamp(start, effective[1]);
    ([start, end], [start, end] != view)
}

/// Most stations the chart pivot carries (the M0 plot cap).
pub const MAX_PIVOT_STATIONS: usize = 20;

fn sql_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn sql_ident(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

/// Column label per station: its name, or its id; duplicates fall back to `name (id)`.
pub fn station_labels(stations: &[(String, Option<String>)]) -> Vec<String> {
    let base = |(id, name): &(String, Option<String>)| {
        name.as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or(id)
            .to_owned()
    };
    let labels = stations.iter().map(base).collect::<Vec<_>>();
    stations
        .iter()
        .zip(&labels)
        .map(|((id, _), label)| {
            if labels.iter().filter(|other| *other == label).count() > 1 {
                format!("{label} ({id})")
            } else {
                label.clone()
            }
        })
        .collect()
}

/// One column per station over the run's valid times, restricted to the lead window.
/// Capped at [`MAX_PIVOT_STATIONS`] columns.
pub fn pivot_series_sql(
    stations: &[(String, Option<String>)],
    run_time: &str,
    window: [u32; 2],
) -> String {
    let stations = &stations[..stations.len().min(MAX_PIVOT_STATIONS)];
    let labels = station_labels(stations);
    let columns = stations
        .iter()
        .zip(&labels)
        .map(|((id, _), label)| {
            format!(
                "max(value) FILTER (WHERE station_id = {}) AS {}",
                sql_string(id),
                sql_ident(label)
            )
        })
        .collect::<Vec<_>>();
    let mut sql = String::from("SELECT valid_time");
    for column in columns {
        sql.push_str(", ");
        sql.push_str(&column);
    }
    sql.push_str(" FROM station_series");
    let mut predicates = Vec::new();
    if let (Some(from), Some(to)) = (
        add_hours_iso(run_time, window[0]),
        add_hours_iso(run_time, window[1]),
    ) {
        predicates.push(format!(
            "valid_time >= {} AND valid_time <= {}",
            sql_string(&from),
            sql_string(&to)
        ));
    }
    let ids = stations
        .iter()
        .map(|(id, _)| sql_string(id))
        .collect::<Vec<_>>();
    if !ids.is_empty() {
        predicates.push(format!("station_id IN ({})", ids.join(", ")));
    }
    if !predicates.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&predicates.join(" AND "));
    }
    sql.push_str(" GROUP BY valid_time ORDER BY valid_time");
    sql
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn sidecar_v2_populates_object_version_and_substitutes_run_year() {
        let catalog = load_catalog().unwrap();
        let entry = catalog.default_entry().unwrap();
        let body = SeriesResultBody {
            result_ref: json!({"id": "series-1"}),
            parquet_path: "/tmp/series-1.parquet".into(),
            matches_ref: json!({"id": "series-1-matches"}),
            plot_ref: None,
            run: omo_protocol::series::ForecastRun {
                reference_time_utc: "2026-11-23T06:00:00Z".into(),
                cycle: "06".into(),
                horizon_h: 144,
                valid_times: vec!["2026-11-23T06:00:00Z".into()],
                object_url: "s3://openmeteo/data_run/ecmwf_ifs/2026/11/23/0600Z/temperature_2m.om"
                    .into(),
                meta_etag: None,
            },
            effective_window: omo_protocol::series::EffectiveWindow {
                requested: [0, 168],
                effective: [0, 144],
                truncated: true,
            },
            plan: "G".into(),
            scan_metrics: json!({"version_token": "12345|2026-11-23T06:30:00Z"}),
        };
        let sidecar = build_m1_sidecar_json(
            Some("abc"),
            10,
            entry,
            &catalog.version,
            Some(&body),
            None,
        );
        assert_eq!(
            sidecar.pointer("/run/object_version").and_then(Value::as_str),
            Some("12345|2026-11-23T06:30:00Z")
        );
        let attribution = sidecar["attribution"].as_array().unwrap();
        assert!(attribution
            .iter()
            .all(|line| !line.as_str().unwrap().contains("<run year>")));
        assert!(attribution
            .iter()
            .any(|line| line.as_str().unwrap().contains("Copyright © 2026")));
    }

    #[test]
    fn large_station_csv_requires_explicit_confirmation() {
        let mut csv = String::from("id,lat,lon\n");
        for i in 0..1_001 {
            csv.push_str(&format!("S{i},35.0,120.0\n"));
        }
        let mut draft = StationImportDraft::from_bytes(
            "large.csv".into(),
            csv.into_bytes(),
            "2026-11-23T00:00:00Z",
        );
        assert!(draft.exceeds_large_warning());
        assert!(draft.needs_large_confirmation());
        assert!(!draft.can_confirm());
        draft.large_file_confirmed = true;
        assert!(draft.can_confirm());
    }

    #[test]
    fn add_hours_crosses_days_and_months() {
        assert_eq!(
            add_hours_iso("2026-11-23T06:00:00Z", 0).as_deref(),
            Some("2026-11-23T06:00:00Z")
        );
        assert_eq!(
            add_hours_iso("2026-11-23T06:00:00Z", 72).as_deref(),
            Some("2026-11-26T06:00:00Z")
        );
        assert_eq!(
            add_hours_iso("2026-02-27T18:00:00Z", 48).as_deref(),
            Some("2026-03-01T18:00:00Z")
        );
        assert_eq!(add_hours_iso("garbage", 1), None);
    }

    #[test]
    fn view_window_is_clamped_inside_the_received_range() {
        assert_eq!(clamp_view_window([0, 72], [0, 144]), ([0, 72], false));
        assert_eq!(clamp_view_window([0, 168], [0, 144]), ([0, 144], true));
        assert_eq!(clamp_view_window([200, 300], [0, 144]), ([144, 144], true));
    }

    #[test]
    fn pivot_sql_labels_station_columns_and_limits_window() {
        let stations = vec![
            ("A1".to_owned(), Some("Alpha".to_owned())),
            ("B2".to_owned(), None),
            ("C'3".to_owned(), Some("Alpha".to_owned())),
        ];
        let sql = pivot_series_sql(&stations, "2026-11-23T06:00:00Z", [0, 72]);
        assert!(sql.contains("AS \"Alpha (A1)\""));
        assert!(sql.contains("AS \"B2\""));
        assert!(sql.contains("station_id = 'C''3'"));
        assert!(sql.contains("valid_time >= '2026-11-23T06:00:00Z'"));
        assert!(sql.contains("valid_time <= '2026-11-26T06:00:00Z'"));
        assert!(sql.ends_with("GROUP BY valid_time ORDER BY valid_time"));
    }

    #[test]
    fn pivot_sql_caps_the_number_of_station_columns() {
        let stations = (0..30)
            .map(|index| (format!("S{index}"), None))
            .collect::<Vec<_>>();
        let sql = pivot_series_sql(&stations, "2026-11-23T06:00:00Z", [0, 6]);
        assert_eq!(sql.matches("FILTER (WHERE").count(), MAX_PIVOT_STATIONS);
    }

    #[test]
    fn catalog_loads_default_entry() {
        let catalog = load_catalog().unwrap();
        let def = catalog.default_entry().unwrap();
        assert_eq!(def.id, "ecmwf_ifs.temperature_2m");
    }

    #[test]
    fn station_import_draft_detects_issues_and_writes_canonical_on_valid_csv() {
        let dir = tempdir().unwrap();
        let bad = StationImportDraft::from_bytes(
            "bad.csv".into(),
            b"id,lat,lon\n,95.0,abc\n1,39.9,116.4\n1,40.0,116.5\n".to_vec(),
            "2026-11-23T08:00:00Z",
        );
        assert!(!bad.can_confirm());
        assert_eq!(bad.issues.len(), 4);

        let good = StationImportDraft::from_bytes(
            "good.csv".into(),
            b"id,lat,lon,name\nA1,39.9,116.4,\xE5\x8C\x97\xE4\xBA\xAC\n".to_vec(),
            "2026-11-23T08:00:00Z",
        );
        assert!(good.can_confirm());
        let (snap, path) = good.confirm_and_write(dir.path()).unwrap();
        assert_eq!(snap.rows.len(), 1);
        assert!(path.exists());
    }
}
