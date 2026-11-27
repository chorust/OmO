//! Run resolution against Open-Meteo `meta.json` (research R-02).
//!
//! A run is completely published when its variable is listed and the last valid time
//! equals `reference_time + horizon(cycle)`. `latest` is tried first, then up to seven
//! earlier cycles at a 6 h step (eight candidates, 48 h).

use std::collections::BTreeMap;

use omo_protocol::series::CatalogEntry;
pub use omo_protocol::series::{EffectiveWindow, ForecastRun, LeadWindow};
use serde::Deserialize;
use thiserror::Error;

use super::{meta_url, object_url, open_meteo_https};
use crate::conn::BootstrappedConn;
use crate::series::quote_sql_literal;

const MAX_CANDIDATES: usize = 8;
const STEP_SECONDS: i64 = 6 * 3600;

#[derive(Clone, Debug, PartialEq)]
pub struct RunMeta {
    pub reference_time_utc: String,
    pub valid_times: Vec<String>,
    pub variables: Vec<String>,
    pub meta_etag: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunRef {
    LatestComplete,
    Explicit(String),
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RunError {
    #[error("remote catalog access is unavailable: {0}")]
    CapabilityUnavailable(String),
    #[error("latest.json could not be read: {0}")]
    LatestMissing(String),
    #[error("no completely published run in the last {MAX_CANDIDATES} cycles")]
    NoCompleteRun,
    #[error("run {reference_time_utc} is not completely published")]
    RunIncomplete { reference_time_utc: String },
    #[error("variable {variable} is missing from run {reference_time_utc}")]
    VariableMissing {
        variable: String,
        reference_time_utc: String,
    },
    #[error("run metadata is invalid: {0}")]
    Invalid(String),
    #[error("run metadata read failed: {0}")]
    Io(String),
}

pub trait MetaSource {
    fn latest(&self) -> Result<RunMeta, RunError>;
    fn meta(&self, reference_time_utc: &str) -> Result<RunMeta, RunError>;
}

pub struct ConnMetaSource<'a> {
    pub conn: &'a BootstrappedConn,
    pub entry: &'a CatalogEntry,
}

impl MetaSource for ConnMetaSource<'_> {
    fn latest(&self) -> Result<RunMeta, RunError> {
        self.ensure_remote()?;
        let url = format!(
            "{}/{}",
            self.entry.layout.base.trim_end_matches('/'),
            self.entry.layout.latest
        );
        read_meta_url(self.conn, &url)
    }

    fn meta(&self, reference_time_utc: &str) -> Result<RunMeta, RunError> {
        self.ensure_remote()?;
        read_meta_url(self.conn, &meta_url(self.entry, reference_time_utc))
    }
}

impl ConnMetaSource<'_> {
    fn ensure_remote(&self) -> Result<(), RunError> {
        self.conn
            .require_remote()
            .map_err(|error| RunError::CapabilityUnavailable(error.to_string()))
    }
}

pub fn resolve(
    entry: &CatalogEntry,
    source: &dyn MetaSource,
    run: RunRef,
) -> Result<ForecastRun, RunError> {
    match run {
        RunRef::Explicit(reference_time_utc) => {
            let meta = source.meta(&reference_time_utc)?;
            classify_explicit(entry, &meta)
        }
        RunRef::LatestComplete => {
            let latest = source.latest()?;
            let mut cursor = parse_utc(&latest.reference_time_utc)?;
            for index in 0..MAX_CANDIDATES {
                let reference = if index == 0 {
                    latest.reference_time_utc.clone()
                } else {
                    format_utc(cursor)
                };
                let meta = if index == 0 {
                    latest.clone()
                } else {
                    match source.meta(&reference) {
                        Ok(meta) => meta,
                        Err(
                            RunError::Io(_) | RunError::LatestMissing(_) | RunError::Invalid(_),
                        ) => {
                            cursor -= STEP_SECONDS;
                            continue;
                        }
                        Err(error) => return Err(error),
                    }
                };
                if let Some(resolved) = accept_if_complete(entry, &meta)? {
                    return Ok(resolved);
                }
                cursor -= STEP_SECONDS;
            }
            Err(RunError::NoCompleteRun)
        }
    }
}

pub fn effective_window(window: LeadWindow, run: &ForecastRun) -> EffectiveWindow {
    let end = window.end_h.min(run.horizon_h);
    let start = window.start_h.min(end);
    EffectiveWindow {
        requested: [window.start_h, window.end_h],
        effective: [start, end],
        truncated: start != window.start_h || end != window.end_h,
    }
}

/// `(time_index, valid_time)` pairs inside the effective lead window.
/// Indices are positions in the run's full `valid_times` vector.
pub fn time_index_table(
    run: &ForecastRun,
    window: &EffectiveWindow,
) -> Result<Vec<(usize, String)>, RunError> {
    let reference = parse_utc(&run.reference_time_utc)?;
    let start = i64::from(window.effective[0]) * 3600;
    let end = i64::from(window.effective[1]) * 3600;
    let mut table = Vec::new();
    for (index, valid_time) in run.valid_times.iter().enumerate() {
        let lead = parse_utc(valid_time)? - reference;
        if (start..=end).contains(&lead) {
            table.push((index, canonicalize_utc(valid_time)?));
        }
    }
    Ok(table)
}

fn classify_explicit(entry: &CatalogEntry, meta: &RunMeta) -> Result<ForecastRun, RunError> {
    if !meta
        .variables
        .iter()
        .any(|name| name == &entry.variable.name)
    {
        return Err(RunError::VariableMissing {
            variable: entry.variable.name.clone(),
            reference_time_utc: meta.reference_time_utc.clone(),
        });
    }
    accept_if_complete(entry, meta)?.ok_or_else(|| RunError::RunIncomplete {
        reference_time_utc: meta.reference_time_utc.clone(),
    })
}

fn accept_if_complete(
    entry: &CatalogEntry,
    meta: &RunMeta,
) -> Result<Option<ForecastRun>, RunError> {
    if !meta
        .variables
        .iter()
        .any(|name| name == &entry.variable.name)
    {
        return Ok(None);
    }
    let reference = parse_utc(&meta.reference_time_utc)?;
    let cycle = cycle_of(reference)?;
    let horizon =
        entry.cycles.get(&cycle).copied().ok_or_else(|| {
            RunError::Invalid(format!("catalog has no horizon for cycle {cycle}"))
        })?;
    let Some(last) = meta.valid_times.last() else {
        return Ok(None);
    };
    let expected = reference + i64::from(horizon) * 3600;
    if parse_utc(last)? != expected {
        return Ok(None);
    }
    Ok(Some(ForecastRun {
        reference_time_utc: canonicalize_utc(&meta.reference_time_utc)?,
        cycle,
        horizon_h: horizon,
        valid_times: meta
            .valid_times
            .iter()
            .map(|value| canonicalize_utc(value))
            .collect::<Result<Vec<_>, _>>()?,
        object_url: object_url(entry, &canonicalize_utc(&meta.reference_time_utc)?),
        meta_etag: meta.meta_etag.clone(),
    }))
}

#[derive(Deserialize)]
struct RawMeta {
    reference_time: String,
    valid_times: Vec<String>,
    #[serde(default)]
    variables: Vec<String>,
}

pub fn meta_from_json(text: &str, meta_etag: Option<String>) -> Result<RunMeta, RunError> {
    let raw: RawMeta =
        serde_json::from_str(text).map_err(|error| RunError::Invalid(error.to_string()))?;
    Ok(RunMeta {
        reference_time_utc: canonicalize_utc(&raw.reference_time)?,
        valid_times: raw
            .valid_times
            .into_iter()
            .map(|value| canonicalize_utc(&value))
            .collect::<Result<Vec<_>, _>>()?,
        variables: raw.variables,
        meta_etag,
    })
}

fn read_meta_url(conn: &BootstrappedConn, url: &str) -> Result<RunMeta, RunError> {
    let connection = conn
        .connection()
        .map_err(|error| RunError::Io(error.to_string()))?;
    let https = open_meteo_https(url);
    let sql = format!(
        "SELECT content FROM read_text({})",
        quote_sql_literal(&https)
    );
    let content = connection
        .query_row(&sql, [], |row| row.get::<_, String>(0))
        .map_err(|error| RunError::Io(error.to_string()))?;
    meta_from_json(&content, None)
}

#[derive(Clone, Debug)]
pub struct ReferenceParts {
    pub year: String,
    pub month: String,
    pub day: String,
    pub hour: String,
}

pub fn split_reference(reference_time_utc: &str) -> ReferenceParts {
    let digits: String = reference_time_utc
        .chars()
        .filter(|ch| ch.is_ascii_digit())
        .collect();
    let year = digits.get(0..4).unwrap_or("0000").to_owned();
    let month = digits.get(4..6).unwrap_or("00").to_owned();
    let day = digits.get(6..8).unwrap_or("00").to_owned();
    let hour = digits.get(8..10).unwrap_or("00").to_owned();
    ReferenceParts {
        year,
        month,
        day,
        hour,
    }
}

pub fn parse_utc(value: &str) -> Result<i64, RunError> {
    let digits: String = value.chars().filter(|ch| ch.is_ascii_digit()).collect();
    if digits.len() < 10 || !value.ends_with('Z') {
        return Err(RunError::Invalid(format!(
            "unsupported UTC timestamp `{value}`"
        )));
    }
    let year: i32 = digits[0..4]
        .parse()
        .map_err(|_| RunError::Invalid(value.into()))?;
    let month: u32 = digits[4..6]
        .parse()
        .map_err(|_| RunError::Invalid(value.into()))?;
    let day: u32 = digits[6..8]
        .parse()
        .map_err(|_| RunError::Invalid(value.into()))?;
    let hour: u32 = digits[8..10]
        .parse()
        .map_err(|_| RunError::Invalid(value.into()))?;
    let minute: u32 = if digits.len() >= 12 {
        digits[10..12]
            .parse()
            .map_err(|_| RunError::Invalid(value.into()))?
    } else {
        0
    };
    let second: u32 = if digits.len() >= 14 {
        digits[12..14]
            .parse()
            .map_err(|_| RunError::Invalid(value.into()))?
    } else {
        0
    };
    civil_to_unix(year, month, day, hour, minute, second)
}

pub fn canonicalize_utc(value: &str) -> Result<String, RunError> {
    Ok(format_utc(parse_utc(value)?))
}

pub fn format_utc(unix: i64) -> String {
    let (year, month, day, hour, minute, second) = unix_to_civil(unix);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

fn cycle_of(unix: i64) -> Result<String, RunError> {
    let (_, _, _, hour, minute, second) = unix_to_civil(unix);
    if minute != 0 || second != 0 || !matches!(hour, 0 | 6 | 12 | 18) {
        return Err(RunError::Invalid(format!(
            "reference time is not a 00/06/12/18Z cycle: {hour:02}:{minute:02}:{second:02}"
        )));
    }
    Ok(format!("{hour:02}"))
}

fn civil_to_unix(
    year: i32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
) -> Result<i64, RunError> {
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return Err(RunError::Invalid(format!(
            "invalid civil time {year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z"
        )));
    }
    let days =
        omo_protocol::utc::days_from_civil(i64::from(year), i64::from(month), i64::from(day));
    Ok(days * 86400 + i64::from(hour) * 3600 + i64::from(minute) * 60 + i64::from(second))
}

fn unix_to_civil(unix: i64) -> (i32, u32, u32, u32, u32, u32) {
    omo_protocol::utc::unix_to_civil(unix)
}

/// Build the non-uniform ECMWF valid-time list used by tests and the resolver.
pub fn synthetic_valid_times(
    reference_time_utc: &str,
    until_h: u32,
) -> Result<Vec<String>, RunError> {
    let reference = parse_utc(reference_time_utc)?;
    let mut leads = Vec::new();
    let mut hour = 0u32;
    while hour <= until_h.min(90) {
        leads.push(hour);
        hour += 1;
    }
    if until_h >= 93 {
        let mut hour = 93u32;
        while hour <= until_h.min(144) {
            leads.push(hour);
            hour += 3;
        }
    }
    if until_h >= 150 {
        let mut hour = 150u32;
        while hour <= until_h {
            leads.push(hour);
            hour += 6;
        }
    }
    Ok(leads
        .into_iter()
        .map(|lead| format_utc(reference + i64::from(lead) * 3600))
        .collect())
}

pub fn cycles_map() -> BTreeMap<String, u32> {
    BTreeMap::from([
        ("00".to_owned(), 360),
        ("06".to_owned(), 144),
        ("12".to_owned(), 360),
        ("18".to_owned(), 144),
    ])
}
