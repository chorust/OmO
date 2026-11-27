//! Required map attributions (`FR-018`), shared by the Map view footer and the composite export
//! so both show the same text (`contracts/export-sidecar-v3.md`, `contracts/catalog-m2.md`).

use crate::utc::{now_utc_rfc3339, parse_utc_rfc3339, unix_to_civil};

pub const OPEN_METEO: &str =
    "Weather data by Open-Meteo.com (CC BY 4.0, https://open-meteo.com/en/licence)";
pub const ECMWF_TEMPLATE: &str = "Copyright © <run year> European Centre for Medium-Range Weather Forecasts (ECMWF). Source: www.ecmwf.int. Licence: CC BY 4.0. ECMWF does not accept any liability whatsoever for any error or omission in the data, their availability, or for any loss or damage arising from their use.";
pub const NATURAL_EARTH: &str = "Basemap boundaries: Natural Earth 1:50m (Public Domain)";
pub const MODIFIED: &str = "Modified: regional tile rasterisation and station overlay by omobench";

/// Extract the UTC calendar year from an RFC 3339 forecast reference time.
pub fn run_year_from_utc(reference_time_utc: &str) -> Option<i32> {
    let secs = parse_utc_rfc3339(reference_time_utc)?;
    Some(unix_to_civil(secs).0)
}

/// Substitute `<run year>` using `reference_time_utc` (or the current UTC year before a run is
/// resolved).
pub fn substitute_run_year(template: &str, reference_time_utc: Option<&str>) -> String {
    if !template.contains("<run year>") {
        return template.to_owned();
    }
    let year = reference_time_utc
        .and_then(run_year_from_utc)
        .or_else(|| run_year_from_utc(&now_utc_rfc3339()))
        .unwrap_or(1970);
    template.replace("<run year>", &year.to_string())
}

/// What is on the map, as far as attribution is concerned.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AttributionInput<'a> {
    pub forecast_visible: bool,
    /// Resolved forecast `reference_time_utc`, used to fill `<run year>`.
    pub forecast_run_utc: Option<&'a str>,
    /// Catalog entry attribution templates; falls back to Open-Meteo + ECMWF when empty.
    pub forecast_attribution: &'a [String],
    pub coastlines_visible: bool,
    /// Embedded attribution metadata of every visible mounted `.pmtiles` archive, bottom to top.
    pub archives: Vec<ArchiveAttribution<'a>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArchiveAttribution<'a> {
    pub file_name: &'a str,
    pub attribution: Option<&'a str>,
}

/// The attribution lines for what is visible, in a stable order without duplicates.
pub fn attribution_lines(input: &AttributionInput<'_>) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut push = |line: String| {
        if !line.trim().is_empty() && !lines.contains(&line) {
            lines.push(line);
        }
    };
    if input.forecast_visible {
        if input.forecast_attribution.is_empty() {
            push(OPEN_METEO.to_owned());
            push(substitute_run_year(ECMWF_TEMPLATE, input.forecast_run_utc));
        } else {
            for line in input.forecast_attribution {
                if !line.starts_with("Modified:") {
                    push(substitute_run_year(line, input.forecast_run_utc));
                }
            }
        }
    }
    if input.coastlines_visible {
        push(NATURAL_EARTH.to_owned());
    }
    for archive in &input.archives {
        if let Some(text) = archive.attribution.map(str::trim).filter(|a| !a.is_empty()) {
            push(text.to_owned());
        }
    }
    push(MODIFIED.to_owned());
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forecast_and_coastlines_substitute_run_year_in_contracted_order() {
        let lines = attribution_lines(&AttributionInput {
            forecast_visible: true,
            forecast_run_utc: Some("2024-07-15T12:00:00Z"),
            forecast_attribution: &[],
            coastlines_visible: true,
            archives: vec![],
        });
        assert_eq!(
            lines,
            [
                OPEN_METEO.to_owned(),
                ECMWF_TEMPLATE.replace("<run year>", "2024"),
                NATURAL_EARTH.to_owned(),
                MODIFIED.to_owned(),
            ]
        );
    }

    #[test]
    fn catalog_attribution_templates_substitute_run_year_and_keep_map_modified_footer() {
        let catalog_lines = vec![
            OPEN_METEO.to_owned(),
            "Copyright © <run year> Custom Provider".to_owned(),
            "Modified: nearest-point sampling and regional tile rasterisation by omobench"
                .to_owned(),
        ];
        let lines = attribution_lines(&AttributionInput {
            forecast_visible: true,
            forecast_run_utc: Some("2025-01-01T00:00:00Z"),
            forecast_attribution: &catalog_lines,
            coastlines_visible: false,
            archives: vec![],
        });
        assert_eq!(
            lines,
            [
                OPEN_METEO.to_owned(),
                "Copyright © 2025 Custom Provider".to_owned(),
                MODIFIED.to_owned(),
            ]
        );
    }

    #[test]
    fn archive_attribution_is_surfaced_deduplicated_and_absent_metadata_is_omitted() {
        let lines = attribution_lines(&AttributionInput {
            forecast_visible: false,
            forecast_run_utc: None,
            forecast_attribution: &[],
            coastlines_visible: false,
            archives: vec![
                ArchiveAttribution {
                    file_name: "a.pmtiles",
                    attribution: Some("© Somebody"),
                },
                ArchiveAttribution {
                    file_name: "b.pmtiles",
                    attribution: Some("© Somebody"),
                },
                ArchiveAttribution {
                    file_name: "c.pmtiles",
                    attribution: Some("  "),
                },
                ArchiveAttribution {
                    file_name: "d.pmtiles",
                    attribution: None,
                },
            ],
        });
        assert_eq!(lines, ["© Somebody", MODIFIED]);
    }

    #[test]
    fn a_hidden_forecast_drops_the_data_attribution() {
        let lines = attribution_lines(&AttributionInput::default());
        assert_eq!(lines, [MODIFIED]);
    }
}
