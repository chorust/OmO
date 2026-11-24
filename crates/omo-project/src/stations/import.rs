//! CSV sniffing, mapping proposal, validation, and canonical hashing (research R-04).

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};
use thiserror::Error;

use super::{ColumnMapping, RowIssue, RowIssueKind, StationRow, StationSnapshot, StationSource};

pub const CANONICAL_HEADER: &str = "station_id,latitude,longitude,name,elevation_m";
pub const LARGE_STATION_WARNING_THRESHOLD: usize = 1_000;

#[derive(Clone, Debug, PartialEq)]
pub struct ParsedTable {
    pub headers: Vec<String>,
    pub sample_rows: Vec<Vec<String>>,
    pub proposed_mapping: ColumnMapping,
    pub delimiter: u8,
    pub data_rows: Vec<(u64, Vec<String>)>,
}

impl ParsedTable {
    pub fn exceeds_large_warning(&self) -> bool {
        self.data_rows.len() > LARGE_STATION_WARNING_THRESHOLD
    }
}

#[derive(Debug, Error, PartialEq)]
pub enum ImportError {
    #[error("station CSV is empty")]
    Empty,
    #[error("station CSV has no header")]
    NoHeader,
    #[error("station CSV is not UTF-8 at byte {byte_offset}")]
    Encoding { byte_offset: usize },
    #[error("station CSV delimiter is ambiguous")]
    AmbiguousDelimiter,
    #[error("station CSV mapping is incomplete")]
    MappingIncomplete,
    #[error("station CSV rows failed validation")]
    Rows(Vec<RowIssue>),
    #[error("station CSV could not be read: {0}")]
    Io(String),
}

pub fn sniff_delimiter(header_line: &str) -> Result<u8, ImportError> {
    let comma = header_line.contains(',');
    let semicolon = header_line.contains(';');
    match (comma, semicolon) {
        (true, false) => Ok(b','),
        (false, true) => Ok(b';'),
        (true, true) => Err(ImportError::AmbiguousDelimiter),
        (false, false) => Err(ImportError::MappingIncomplete),
    }
}

pub fn propose_mapping(headers: &[String]) -> ColumnMapping {
    ColumnMapping {
        id: find_alias(headers, &["station_id", "id", "code", "name_id"]).unwrap_or_default(),
        lat: find_alias(headers, &["lat", "latitude", "y"]).unwrap_or_default(),
        lon: find_alias(headers, &["lon", "lng", "longitude", "x"]).unwrap_or_default(),
        name: find_alias(headers, &["name", "station_name"]),
        elevation: find_alias(headers, &["elevation", "elev", "alt", "altitude", "height"]),
    }
}

pub fn import_csv(
    bytes: &[u8],
    orig_path: &str,
    imported_at_utc: &str,
) -> Result<ParsedTable, ImportError> {
    if bytes.is_empty() || bytes.iter().all(|byte| byte.is_ascii_whitespace()) {
        return Err(ImportError::Empty);
    }
    let text = std::str::from_utf8(bytes).map_err(|error| ImportError::Encoding {
        byte_offset: error.valid_up_to(),
    })?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    if text.trim().is_empty() {
        return Err(ImportError::Empty);
    }
    let header_line = text.lines().next().unwrap_or("");
    let delimiter = sniff_delimiter(header_line)?;
    import_csv_with_delimiter(bytes, orig_path, imported_at_utc, delimiter)
}

pub fn import_csv_with_delimiter(
    bytes: &[u8],
    orig_path: &str,
    imported_at_utc: &str,
    delimiter: u8,
) -> Result<ParsedTable, ImportError> {
    if bytes.is_empty() || bytes.iter().all(|byte| byte.is_ascii_whitespace()) {
        return Err(ImportError::Empty);
    }
    let text = std::str::from_utf8(bytes).map_err(|error| ImportError::Encoding {
        byte_offset: error.valid_up_to(),
    })?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    if text.trim().is_empty() {
        return Err(ImportError::Empty);
    }
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .has_headers(true)
        .flexible(true)
        .from_reader(text.as_bytes());
    let headers = reader
        .headers()
        .map_err(|error| ImportError::Io(error.to_string()))?
        .iter()
        .map(str::trim)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if headers.is_empty() {
        return Err(ImportError::NoHeader);
    }
    let proposed_mapping = propose_mapping(&headers);
    let mut sample_rows = Vec::new();
    let mut data_rows = Vec::new();
    for (index, record) in reader.records().enumerate() {
        let record = record.map_err(|error| ImportError::Io(error.to_string()))?;
        let fields = record
            .iter()
            .map(str::trim)
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let line = (index as u64) + 2;
        if sample_rows.len() < 20 {
            sample_rows.push(fields.clone());
        }
        data_rows.push((line, fields));
    }
    let _ = (orig_path, imported_at_utc);
    Ok(ParsedTable {
        headers,
        sample_rows,
        proposed_mapping,
        delimiter,
        data_rows,
    })
}

pub fn confirm(
    table: &ParsedTable,
    mapping: ColumnMapping,
    orig_path: &str,
    orig_bytes: &[u8],
    imported_at_utc: &str,
) -> Result<StationSnapshot, ImportError> {
    if mapping.id.is_empty() || mapping.lat.is_empty() || mapping.lon.is_empty() {
        return Err(ImportError::MappingIncomplete);
    }
    let id_index = column_index(&table.headers, &mapping.id)?;
    let lat_index = column_index(&table.headers, &mapping.lat)?;
    let lon_index = column_index(&table.headers, &mapping.lon)?;
    let name_index = mapping
        .name
        .as_deref()
        .map(|name| column_index(&table.headers, name))
        .transpose()?;
    let elevation_index = mapping
        .elevation
        .as_deref()
        .map(|name| column_index(&table.headers, name))
        .transpose()?;
    let mut issues = Vec::new();
    let mut rows = Vec::new();
    let mut seen: Vec<(String, u64)> = Vec::new();
    if table.data_rows.is_empty() {
        return Err(ImportError::Empty);
    }
    for (line, fields) in &table.data_rows {
        let station_id = field(fields, id_index).trim().to_owned();
        if station_id.is_empty() {
            issues.push(RowIssue {
                line: *line,
                field: "station_id".into(),
                kind: RowIssueKind::Empty,
            });
        } else if let Some((_, first_line)) = seen.iter().find(|(id, _)| id == &station_id) {
            issues.push(RowIssue {
                line: *line,
                field: "station_id".into(),
                kind: RowIssueKind::Duplicate {
                    first_line: *first_line,
                },
            });
        } else {
            seen.push((station_id.clone(), *line));
        }
        let latitude = parse_number(
            fields,
            lat_index,
            *line,
            "latitude",
            -90.0,
            90.0,
            &mut issues,
        );
        let longitude = parse_number(
            fields,
            lon_index,
            *line,
            "longitude",
            -180.0,
            360.0,
            &mut issues,
        );
        let name = name_index.and_then(|index| {
            let value = field(fields, index).trim();
            if value.is_empty() {
                None
            } else {
                Some(value.to_owned())
            }
        });
        let elevation_m = elevation_index.and_then(|index| {
            let value = field(fields, index).trim();
            if value.is_empty() {
                None
            } else {
                match value.parse::<f64>() {
                    Ok(number) if number.is_finite() => Some(number),
                    _ => {
                        issues.push(RowIssue {
                            line: *line,
                            field: "elevation_m".into(),
                            kind: RowIssueKind::NotNumber,
                        });
                        None
                    }
                }
            }
        });
        if let (Some(latitude), Some(longitude)) = (latitude, longitude) {
            rows.push(StationRow {
                station_id,
                latitude,
                longitude: normalize_longitude(longitude),
                name,
                elevation_m,
            });
        }
    }
    if !issues.is_empty() {
        return Err(ImportError::Rows(issues));
    }
    let canonical = canonical_bytes(&rows);
    let content_sha256 = hex_sha256(&canonical);
    let _ = orig_bytes;
    Ok(StationSnapshot {
        rows,
        content_sha256,
        source: StationSource {
            orig_path: orig_path.to_owned(),
            orig_sha256: hex_sha256(orig_bytes),
            imported_at_utc: imported_at_utc.to_owned(),
        },
        mapping,
    })
}

pub fn canonical_bytes(rows: &[StationRow]) -> Vec<u8> {
    let mut buffer = Vec::new();
    buffer.extend_from_slice(CANONICAL_HEADER.as_bytes());
    buffer.push(b'\n');
    let mut writer = csv::WriterBuilder::new()
        .has_headers(false)
        .terminator(csv::Terminator::Any(b'\n'))
        .from_writer(Vec::new());
    for row in rows {
        writer
            .write_record([
                row.station_id.as_str(),
                &format_float(row.latitude),
                &format_float(row.longitude),
                row.name.as_deref().unwrap_or(""),
                row.elevation_m.map(format_float).as_deref().unwrap_or(""),
            ])
            .expect("canonical CSV write to memory");
    }
    buffer.extend_from_slice(&writer.into_inner().expect("canonical CSV buffer"));
    buffer
}

pub fn write_canonical(snapshot: &StationSnapshot, dir: &Path) -> Result<PathBuf, ImportError> {
    fs::create_dir_all(dir).map_err(|error| ImportError::Io(error.to_string()))?;
    let path = dir.join(format!("stations-{}.csv", snapshot.content_sha256));
    let mut file = fs::File::create(&path).map_err(|error| ImportError::Io(error.to_string()))?;
    file.write_all(&canonical_bytes(&snapshot.rows))
        .map_err(|error| ImportError::Io(error.to_string()))?;
    Ok(path)
}

fn find_alias(headers: &[String], aliases: &[&str]) -> Option<String> {
    headers
        .iter()
        .find(|header| {
            aliases
                .iter()
                .any(|alias| header.eq_ignore_ascii_case(alias))
        })
        .cloned()
}

fn column_index(headers: &[String], name: &str) -> Result<usize, ImportError> {
    headers
        .iter()
        .position(|header| header.eq_ignore_ascii_case(name))
        .ok_or(ImportError::MappingIncomplete)
}

fn field(fields: &[String], index: usize) -> &str {
    fields.get(index).map(String::as_str).unwrap_or("")
}

fn parse_number(
    fields: &[String],
    index: usize,
    line: u64,
    name: &str,
    min: f64,
    max: f64,
    issues: &mut Vec<RowIssue>,
) -> Option<f64> {
    let text = field(fields, index).trim();
    if text.is_empty() {
        issues.push(RowIssue {
            line,
            field: name.to_owned(),
            kind: RowIssueKind::NotNumber,
        });
        return None;
    }
    match text.parse::<f64>() {
        Ok(number) if number.is_finite() && (min..=max).contains(&number) => Some(number),
        Ok(number) if number.is_finite() => {
            issues.push(RowIssue {
                line,
                field: name.to_owned(),
                kind: RowIssueKind::OutOfRange,
            });
            None
        }
        _ => {
            issues.push(RowIssue {
                line,
                field: name.to_owned(),
                kind: RowIssueKind::NotNumber,
            });
            None
        }
    }
}

pub fn normalize_longitude(longitude: f64) -> f64 {
    let mut normalized = longitude % 360.0;
    if normalized < -180.0 {
        normalized += 360.0;
    } else if normalized >= 180.0 {
        normalized -= 360.0;
    }
    if normalized == 0.0 { 0.0 } else { normalized }
}

fn format_float(value: f64) -> String {
    let mut buffer = ryu::Buffer::new();
    buffer.format_finite(value).to_owned()
}

fn hex_sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
