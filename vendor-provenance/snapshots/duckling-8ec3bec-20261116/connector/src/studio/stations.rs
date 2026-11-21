use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::dialect::duckdb::session::NativeSession;
use crate::studio::error::{details, StudioError, StudioErrorCode};
use crate::studio::model::{canonical_sha256, hex_sha256};
use crate::studio::profile::load_profile;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LongitudeConvention {
  Signed180,
  Unsigned360,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WhitespacePolicy {
  Preserve,
  Trim,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StationPreview {
  pub schema_version: u32,
  pub source_id: String,
  pub content_sha256: String,
  pub column_names: Vec<String>,
  pub row_count: u64,
  pub sample_rows: Vec<BTreeMap<String, String>>,
  pub sample_offset: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RowIssue {
  pub row_ordinal: u64,
  pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StationSnapshot {
  pub schema_version: u32,
  pub snapshot_id: String,
  pub source_id: String,
  pub content_sha256: String,
  pub mapping_revision: u64,
  pub rows: Vec<SnapshotRow>,
  pub issues: Vec<RowIssue>,
  pub snapshot_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotRow {
  pub station_id: String,
  pub row_ordinal: u64,
  pub latitude: f64,
  pub longitude: f64,
  pub business: BTreeMap<String, String>,
}

struct Table {
  columns: Vec<String>,
  rows: Vec<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StationMapping {
  pub id_column: String,
  pub latitude_column: String,
  pub longitude_column: String,
  pub convention: LongitudeConvention,
  pub whitespace: WhitespacePolicy,
  pub selected: Vec<u64>,
}

struct CsvSnapshot(PathBuf);

impl Drop for CsvSnapshot {
  fn drop(&mut self) {
    let _ = fs::remove_dir_all(&self.0);
  }
}

async fn read_csv_snapshot(
  path: &Path,
  session: Arc<NativeSession>,
) -> Result<(String, Table), StudioError> {
  let profile = load_profile()?;
  let metadata = fs::symlink_metadata(path).map_err(|_| {
    StudioError::new(
      StudioErrorCode::InvalidRequest,
      details(&[("reason", json!("station file is not readable"))]),
    )
  })?;
  if metadata.file_type().is_symlink() || !metadata.is_file() {
    return Err(StudioError::new(
      StudioErrorCode::InvalidRequest,
      details(&[("reason", json!("station source must be a regular file"))]),
    ));
  }
  if metadata.len() > profile.stations.max_file_bytes {
    return Err(StudioError::new(
      StudioErrorCode::InvalidRequest,
      details(&[("reason", json!("station file exceeds 16 MiB"))]),
    ));
  }
  let io_error = |_| {
    StudioError::new(
      StudioErrorCode::InvalidRequest,
      json!({"reason": "station snapshot could not be read or created"}),
    )
  };
  let mut bytes = Vec::new();
  fs::File::open(path)
    .map_err(io_error)?
    .take(profile.stations.max_file_bytes + 1)
    .read_to_end(&mut bytes)
    .map_err(io_error)?;
  if bytes.len() as u64 > profile.stations.max_file_bytes {
    return Err(StudioError::new(
      StudioErrorCode::StationValidation,
      json!({"reason": "station file exceeds 16 MiB"}),
    ));
  }
  let digest = hex_sha256(&bytes);
  let root = std::env::temp_dir().join(format!("duckling-station-{}", nanoid::nanoid!()));
  fs::create_dir(&root).map_err(io_error)?;
  let snapshot = CsvSnapshot(root);
  #[cfg(unix)]
  {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&snapshot.0, fs::Permissions::from_mode(0o700)).map_err(io_error)?;
  }
  let frozen = snapshot.0.join("stations.csv");
  let mut file = fs::OpenOptions::new()
    .write(true)
    .create_new(true)
    .open(&frozen)
    .map_err(io_error)?;
  file.write_all(&bytes).map_err(io_error)?;
  drop(file);
  let literal = sql_path_literal(&frozen)?;
  let sql =
    format!("SELECT * FROM read_csv({literal}, header=true, all_varchar=true, strict_mode=true)");
  let table = session
    .run(None, move |connection, token| {
      token.check()?;
      read_table(&connection.inner, &sql).map_err(anyhow::Error::from)
    })
    .await
    .map_err(|error| match error.downcast::<StudioError>() {
      Ok(error) => error,
      Err(error) => StudioError::new(
        StudioErrorCode::Internal,
        details(&[("reason", json!(error.to_string()))]),
      ),
    })?;
  if table.columns.len() as u64 > profile.stations.max_columns {
    return Err(StudioError::new(
      StudioErrorCode::StationValidation,
      details(&[("reason", json!("too many columns"))]),
    ));
  }
  if table.columns.len() != table.columns.iter().collect::<BTreeSet<_>>().len() {
    return Err(StudioError::new(
      StudioErrorCode::StationValidation,
      details(&[("reason", json!("column names are ambiguous"))]),
    ));
  }
  if table.rows.len() as u64 > profile.stations.max_imported_rows {
    return Err(StudioError::new(
      StudioErrorCode::StationValidation,
      details(&[("reason", json!("too many imported rows"))]),
    ));
  }
  for row in &table.rows {
    for cell in row {
      if cell.len() as u64 > profile.stations.max_cell_utf8_bytes {
        return Err(StudioError::new(
          StudioErrorCode::StationValidation,
          details(&[("reason", json!("a cell exceeds 64 KiB"))]),
        ));
      }
    }
  }
  Ok((digest, table))
}

pub async fn inspect_csv(
  path: &Path,
  session: Arc<NativeSession>,
) -> Result<StationPreview, StudioError> {
  inspect_csv_page(path, session, 0).await
}

pub async fn inspect_csv_page(
  path: &Path,
  session: Arc<NativeSession>,
  offset: u64,
) -> Result<StationPreview, StudioError> {
  if offset > 10_000 {
    return Err(StudioError::new(
      StudioErrorCode::InvalidRequest,
      json!({"reason": "invalid sample offset"}),
    ));
  }
  let (digest, table) = read_csv_snapshot(path, session).await?;
  let sample_rows = table
    .rows
    .iter()
    .skip(offset as usize)
    .take(20)
    .map(|row| {
      table
        .columns
        .iter()
        .cloned()
        .zip(row.iter().cloned())
        .collect::<BTreeMap<_, _>>()
    })
    .collect();
  Ok(StationPreview {
    schema_version: 1,
    source_id: format!("file-{digest}"),
    content_sha256: digest,
    column_names: table.columns,
    row_count: table.rows.len() as u64,
    sample_rows,
    sample_offset: offset,
  })
}

pub async fn confirm_csv(
  path: &Path,
  session: Arc<NativeSession>,
  preview_hash: &str,
  mapping: &StationMapping,
) -> Result<StationSnapshot, StudioError> {
  let (digest, table) = read_csv_snapshot(path, session).await?;
  let rows = table
    .rows
    .iter()
    .map(|row| {
      table
        .columns
        .iter()
        .cloned()
        .zip(row.iter().cloned())
        .collect()
    })
    .collect::<Vec<_>>();
  confirm_rows(
    preview_hash,
    &digest,
    &table.columns,
    &rows,
    &mapping.id_column,
    &mapping.latitude_column,
    &mapping.longitude_column,
    mapping.convention,
    mapping.whitespace,
    &BTreeSet::new(),
    &mapping.selected,
  )
}

pub fn confirm_rows(
  preview_hash: &str,
  actual_hash: &str,
  columns: &[String],
  rows: &[BTreeMap<String, String>],
  id_column: &str,
  latitude_column: &str,
  longitude_column: &str,
  convention: LongitudeConvention,
  whitespace: WhitespacePolicy,
  excluded: &BTreeSet<u64>,
  selected: &[u64],
) -> Result<StationSnapshot, StudioError> {
  if preview_hash != actual_hash {
    return Err(StudioError::new(
      StudioErrorCode::StationSourceChanged,
      details(&[("reason", json!("station file hash changed"))]),
    ));
  }
  let profile = load_profile()?;
  if selected.len() as u64 > profile.task.max_stations {
    return Err(StudioError::new(
      StudioErrorCode::StationValidation,
      details(&[("reason", json!("too many selected stations"))]),
    ));
  }
  let mapping_columns = [id_column, latitude_column, longitude_column];
  if mapping_columns
    .iter()
    .any(|name| !columns.iter().any(|column| column == name))
    || mapping_columns.into_iter().collect::<BTreeSet<_>>().len() != 3
    || selected.iter().any(|ordinal| *ordinal >= rows.len() as u64)
    || selected.iter().collect::<BTreeSet<_>>().len() != selected.len()
    || excluded
      .iter()
      .any(|ordinal| *ordinal >= rows.len() as u64 || selected.contains(ordinal))
    || actual_hash.len() != 64
    || !actual_hash.bytes().all(|byte| byte.is_ascii_hexdigit())
  {
    return Err(StudioError::new(
      StudioErrorCode::StationValidation,
      json!({"reason": "invalid columns, selection, or source hash"}),
    ));
  }
  let mut issues = Vec::new();
  let mut seen = BTreeMap::<String, u64>::new();
  let mut snapshot_rows = Vec::new();
  for (ordinal, row) in rows.iter().enumerate() {
    let ordinal = ordinal as u64;
    if !selected.contains(&ordinal) {
      continue;
    }
    if excluded.contains(&ordinal) {
      issues.push(RowIssue {
        row_ordinal: ordinal,
        reason: "excluded".into(),
      });
      continue;
    }
    let raw_id = row.get(id_column).cloned().unwrap_or_default();
    let station_id = match whitespace {
      WhitespacePolicy::Preserve => raw_id,
      WhitespacePolicy::Trim => raw_id.trim().to_string(),
    };
    if station_id.trim().is_empty() {
      issues.push(RowIssue {
        row_ordinal: ordinal,
        reason: "missing-id".into(),
      });
      continue;
    }
    if let Some(previous) = seen.insert(station_id.clone(), ordinal) {
      issues.push(RowIssue {
        row_ordinal: ordinal,
        reason: format!("duplicate-id:{previous}"),
      });
      continue;
    }
    let lat = parse_finite(row.get(latitude_column).map(String::as_str).unwrap_or(""));
    let lon = parse_finite(row.get(longitude_column).map(String::as_str).unwrap_or(""));
    let (Some(lat), Some(lon)) = (lat, lon) else {
      issues.push(RowIssue {
        row_ordinal: ordinal,
        reason: "non-finite-coordinate".into(),
      });
      continue;
    };
    if !(-90.0..=90.0).contains(&lat) {
      issues.push(RowIssue {
        row_ordinal: ordinal,
        reason: "latitude-out-of-range".into(),
      });
      continue;
    }
    let longitude = match convention {
      LongitudeConvention::Signed180 if (-180.0..180.0).contains(&lon) => lon,
      LongitudeConvention::Unsigned360 if (0.0..360.0).contains(&lon) => normalize_longitude(lon),
      _ => {
        issues.push(RowIssue {
          row_ordinal: ordinal,
          reason: "longitude-convention".into(),
        });
        continue;
      }
    };
    let mut business = BTreeMap::new();
    for column in columns {
      if let Some(value) = row.get(column) {
        business.insert(column.clone(), value.clone());
      }
    }
    snapshot_rows.push(SnapshotRow {
      station_id,
      row_ordinal: ordinal,
      latitude: lat,
      longitude,
      business,
    });
  }
  if snapshot_rows.is_empty() || !issues.is_empty() {
    return Err(StudioError::new(
      StudioErrorCode::StationValidation,
      details(&[
        (
          "issues",
          serde_json::to_value(&issues).unwrap_or(Value::Null),
        ),
        (
          "reason",
          json!("station mapping has errors or no selected rows"),
        ),
      ]),
    ));
  }
  let mut snapshot = StationSnapshot {
    schema_version: 1,
    snapshot_id: format!("snap-{}", &actual_hash[..12]),
    source_id: format!("file-{actual_hash}"),
    content_sha256: actual_hash.to_string(),
    mapping_revision: 1,
    rows: snapshot_rows,
    issues,
    snapshot_hash: String::new(),
  };
  snapshot.snapshot_hash =
    canonical_sha256(&serde_json::to_value(&snapshot).unwrap_or(Value::Null));
  Ok(snapshot)
}

fn parse_finite(value: &str) -> Option<f64> {
  let parsed: f64 = value.trim().parse().ok()?;
  parsed.is_finite().then_some(parsed)
}

fn normalize_longitude(value: f64) -> f64 {
  let mut lon = value;
  while lon >= 180.0 {
    lon -= 360.0;
  }
  while lon < -180.0 {
    lon += 360.0;
  }
  lon
}

fn sql_path_literal(path: &Path) -> Result<String, StudioError> {
  let text = path.to_str().ok_or_else(|| {
    StudioError::new(
      StudioErrorCode::InvalidRequest,
      details(&[("reason", json!("station path is not UTF-8"))]),
    )
  })?;
  if text.contains('\0') || text.contains('\n') || text.contains('\r') {
    return Err(StudioError::new(
      StudioErrorCode::InvalidRequest,
      details(&[("reason", json!("station path contains a control character"))]),
    ));
  }
  Ok(format!("'{}'", text.replace('\'', "''")))
}

fn describe_columns(
  connection: &duckdb::Connection,
  sql: &str,
) -> Result<Vec<String>, StudioError> {
  let describe = format!("DESCRIBE {sql}");
  let mut statement = connection.prepare(&describe).map_err(|error| {
    StudioError::new(
      StudioErrorCode::Internal,
      details(&[("reason", json!(error.to_string()))]),
    )
  })?;
  let mut rows = statement.query([]).map_err(|error| {
    StudioError::new(
      StudioErrorCode::Internal,
      details(&[("reason", json!(error.to_string()))]),
    )
  })?;
  let mut columns = Vec::new();
  while let Some(row) = rows.next().map_err(|error| {
    StudioError::new(
      StudioErrorCode::Internal,
      details(&[("reason", json!(error.to_string()))]),
    )
  })? {
    let name: String = row.get(0).map_err(|error| {
      StudioError::new(
        StudioErrorCode::Internal,
        details(&[("reason", json!(error.to_string()))]),
      )
    })?;
    columns.push(name);
  }
  Ok(columns)
}

fn read_table(connection: &duckdb::Connection, sql: &str) -> Result<Table, StudioError> {
  let columns = describe_columns(connection, sql)?;
  let mut statement = connection.prepare(sql).map_err(|error| {
    StudioError::new(
      StudioErrorCode::Internal,
      details(&[("reason", json!(error.to_string()))]),
    )
  })?;
  let mut rows_iter = statement.query([]).map_err(|error| {
    StudioError::new(
      StudioErrorCode::Internal,
      details(&[("reason", json!(error.to_string()))]),
    )
  })?;
  let mut rows = Vec::new();
  loop {
    let Some(row) = rows_iter.next().map_err(|error| {
      StudioError::new(
        StudioErrorCode::Internal,
        details(&[("reason", json!(error.to_string()))]),
      )
    })?
    else {
      break;
    };
    let mut values = Vec::new();
    for index in 0..columns.len() {
      let value: Option<String> = row.get(index).map_err(|error| {
        StudioError::new(
          StudioErrorCode::Internal,
          details(&[("reason", json!(error.to_string()))]),
        )
      })?;
      values.push(value.unwrap_or_default());
    }
    rows.push(values);
    if rows.len() > 10_000 {
      break;
    }
  }
  Ok(Table { columns, rows })
}
