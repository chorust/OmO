//! Station snapshots confirmed by the user (research R-04, data-model StationSnapshot).

pub mod import;

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub use import::{
    ImportError, ParsedTable, import_csv, propose_mapping, sniff_delimiter, write_canonical,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StationSnapshot {
    pub rows: Vec<StationRow>,
    pub content_sha256: String,
    pub source: StationSource,
    pub mapping: ColumnMapping,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StationRow {
    pub station_id: String,
    pub latitude: f64,
    pub longitude: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elevation_m: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StationSource {
    pub orig_path: String,
    pub orig_sha256: String,
    pub imported_at_utc: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ColumnMapping {
    pub id: String,
    pub lat: String,
    pub lon: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elevation: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowIssue {
    pub line: u64,
    pub field: String,
    pub kind: RowIssueKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowIssueKind {
    Empty,
    Duplicate { first_line: u64 },
    OutOfRange,
    NotNumber,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SnapshotError {
    #[error("station snapshot failed validation")]
    Invalid(Vec<RowIssue>),
}

impl StationSnapshot {
    pub fn validate(&self) -> Result<(), Vec<RowIssue>> {
        let mut issues = Vec::new();
        let mut seen: Vec<(String, u64)> = Vec::new();
        for (index, row) in self.rows.iter().enumerate() {
            let line = (index as u64) + 2;
            if row.station_id.is_empty() {
                issues.push(RowIssue {
                    line,
                    field: "station_id".into(),
                    kind: RowIssueKind::Empty,
                });
            } else if let Some((_, first_line)) = seen.iter().find(|(id, _)| id == &row.station_id)
            {
                issues.push(RowIssue {
                    line,
                    field: "station_id".into(),
                    kind: RowIssueKind::Duplicate {
                        first_line: *first_line,
                    },
                });
            } else {
                seen.push((row.station_id.clone(), line));
            }
            if !row.latitude.is_finite() || !(-90.0..=90.0).contains(&row.latitude) {
                issues.push(RowIssue {
                    line,
                    field: "latitude".into(),
                    kind: if row.latitude.is_finite() {
                        RowIssueKind::OutOfRange
                    } else {
                        RowIssueKind::NotNumber
                    },
                });
            }
            if !row.longitude.is_finite() || !(-180.0..=360.0).contains(&row.longitude) {
                issues.push(RowIssue {
                    line,
                    field: "longitude".into(),
                    kind: if row.longitude.is_finite() {
                        RowIssueKind::OutOfRange
                    } else {
                        RowIssueKind::NotNumber
                    },
                });
            }
        }
        if issues.is_empty() {
            Ok(())
        } else {
            Err(issues)
        }
    }
}
