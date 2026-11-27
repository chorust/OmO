//! Offline tests for the map field fetcher and cache (`T018`). Live reads run in
//! `map_fetch_live.rs` (`--ignored`).

use omo_engine::{
    grid::reduced_gaussian::O1280,
    map::{
        CachedField, FieldCache, GridValues,
        fetch::{
            BandRow, BandTask, FetchError, MAX_READERS, band_sql, classify_duckdb_error, clip_bbox,
            fill_o1280, is_object_not_found, lon_extent, needs_clip, plan_bands, rows_in_lat_range,
        },
        field::compute_stats,
    },
};
use omo_protocol::map::{FieldStats, MapBbox, SourceLayout};

fn bbox(lat_min: f64, lat_max: f64, spans: &[[f64; 2]]) -> MapBbox {
    MapBbox {
        lat_min,
        lat_max,
        lon_spans: spans.to_vec(),
    }
}

fn lat(y: usize) -> f64 {
    f64::from(O1280::lat_of_row(y))
}

#[test]
fn rows_in_range_are_exactly_the_rows_inside() {
    let range = rows_in_lat_range(20.0, 50.0).unwrap();
    let expected: Vec<usize> = (0..O1280::rows())
        .filter(|&y| (20.0..=50.0).contains(&lat(y)))
        .collect();
    assert_eq!(*range.start(), expected[0]);
    assert_eq!(*range.end(), *expected.last().unwrap());
    assert_eq!(range.count(), expected.len());
    assert!(rows_in_lat_range(50.0, 20.0).is_none());
    // A band narrower than one row (~0.07 deg) may contain no row.
    let between = (lat(1000) + lat(1001)) / 2.0;
    assert!(rows_in_lat_range(between - 1e-4, between + 1e-4).is_none());
}

#[test]
fn bands_partition_every_row_exactly_once() {
    let region = bbox(20.0, 50.0, &[[100.0, 140.0]]);
    for readers in [1usize, 3, 12, 16, 40] {
        let tasks = plan_bands(&region, readers);
        assert_eq!(tasks.len(), readers.clamp(1, MAX_READERS));
        let expected = rows_in_lat_range(20.0, 50.0).unwrap();
        let mut owners = vec![0u32; O1280::rows()];
        for task in &tasks {
            for (y, count) in owners.iter_mut().enumerate() {
                let l = lat(y);
                if l >= task.lat_lo && l <= task.lat_hi {
                    *count += 1;
                }
            }
        }
        for (y, &owner) in owners.iter().enumerate() {
            let want = u32::from(expected.contains(&y));
            assert_eq!(owner, want, "readers={readers} row {y} lat {}", lat(y));
        }
        assert!(tasks.iter().all(|t| t.lon_lo == 100.0 && t.lon_hi == 140.0));
    }
}

#[test]
fn antimeridian_bbox_plans_one_task_set_per_longitude_segment() {
    let region = bbox(10.0, 14.0, &[[178.0, 180.0], [-180.0, -178.0]]);
    let tasks = plan_bands(&region, 4);
    assert_eq!(tasks.len(), 8);
    assert!(tasks[..4].iter().all(|t| t.lon_lo == 178.0));
    assert!(tasks[4..].iter().all(|t| t.lon_hi == -178.0));
    assert!(plan_bands(&bbox(10.0, 10.0, &[[0.0, 1.0]]), 4).len() <= 4);
}

#[test]
fn spatial_sql_names_the_variable_and_data_run_filters_valid_time() {
    let task = BandTask {
        lat_lo: 39.0,
        lat_hi: 41.0,
        lon_lo: 115.0,
        lon_hi: 117.0,
    };
    let spatial = band_sql(
        SourceLayout::DataSpatial,
        "https://openmeteo.s3.amazonaws.com/data_spatial/ecmwf_ifs/2026/11/25/0000Z/2026-11-25T0600.om",
        "ecmwf_ifs",
        "temperature_2m",
        "2026-11-25T06:00:00Z",
        &task,
    );
    assert!(
        spatial.contains("variable := 'temperature_2m'"),
        "{spatial}"
    );
    assert!(!spatial.contains("valid_time"), "{spatial}");
    assert!(spatial.contains("\"temperature_2m\"::FLOAT"), "{spatial}");
    assert!(spatial.contains("lat BETWEEN 39.00000000 AND 41.00000000"));
    assert!(spatial.contains("lon BETWEEN 115.00000000 AND 117.00000000"));

    let run = band_sql(
        SourceLayout::DataRun,
        "https://openmeteo.s3.amazonaws.com/data_run/ecmwf_ifs/2026/11/25/0000Z/temperature_2m.om",
        "ecmwf_ifs",
        "temperature_2m",
        "2026-11-25T06:00:00Z",
        &task,
    );
    assert!(!run.contains("variable :="), "{run}");
    assert!(run.contains("value::FLOAT"), "{run}");
    assert!(
        run.contains("valid_time = '2026-11-25 06:00:00'::TIMESTAMP"),
        "{run}"
    );

    let quoted = band_sql(SourceLayout::DataSpatial, "x'y", "d", "v", "t", &task);
    assert!(quoted.contains("'x''y'"));
}

#[test]
fn object_not_found_requires_both_markers() {
    let msg = "remote OM open failed for 'https://x/y.om' [object_not_found] (HTTP 404); check the exact object key";
    assert!(is_object_not_found(msg));
    assert!(!is_object_not_found("[object_not_found] without status"));
    assert!(!is_object_not_found("(HTTP 404) only"));
    assert!(!is_object_not_found("HTTP 503 network down"));
    let head = "HTTP Error: HTTP HEAD error on 'https://x/y.om' (HTTP 404 Not Found)";
    assert!(omo_engine::map::fetch::is_head_not_found(head));
    assert!(!omo_engine::map::fetch::is_head_not_found(
        "HTTP HEAD error (HTTP 503)"
    ));
    assert!(matches!(
        classify_duckdb_error("s", msg.to_owned()),
        FetchError::ObjectNotFound(_)
    ));
    assert!(matches!(
        classify_duckdb_error("s", "syntax error".to_owned()),
        FetchError::DuckDb { .. }
    ));
}

#[test]
fn wide_areas_are_clipped_and_regional_ones_are_not() {
    let regional = bbox(20.0, 50.0, &[[100.0, 140.0]]);
    assert!(!needs_clip(&regional));
    assert_eq!(clip_bbox(&regional, None, 0.0), regional);

    let wide = bbox(-80.0, 56.0, &[[-170.0, 137.0]]);
    assert!(needs_clip(&wide));
    let clipped = clip_bbox(&wide, None, 0.0);
    assert!(!needs_clip(&clipped), "{clipped:?}");
    assert!(clipped.lat_min >= -80.0 && clipped.lat_max <= 56.0);

    let viewport = bbox(30.0, 45.0, &[[110.0, 130.0]]);
    let around_view = clip_bbox(&wide, Some(&viewport), 1.0);
    assert_eq!(around_view.lat_min, 29.0);
    assert_eq!(around_view.lat_max, 46.0);
    assert_eq!(around_view.lon_spans, vec![[109.0, 131.0]]);
    assert!((lon_extent(&around_view) - 22.0).abs() < 1e-12);
}

fn sample_rows(flats: &[u32]) -> Vec<BandRow> {
    flats
        .iter()
        .map(|&flat| {
            let (lat, lon) = O1280::point(flat);
            BandRow {
                lat,
                lon,
                value: flat as f32 * 0.5,
            }
        })
        .collect()
}

#[test]
fn dense_fill_places_values_at_the_matching_flat_index() {
    let flats = [0u32, 19, 20, 1_234_567, 3_299_839, 3_299_840, 6_599_679];
    let values = fill_o1280(&sample_rows(&flats)).unwrap();
    let slice = values.as_slice();
    assert_eq!(slice.len(), 6_599_680);
    for &flat in &flats {
        assert_eq!(slice[flat as usize], flat as f32 * 0.5, "flat {flat}");
    }
    assert_eq!(slice.iter().filter(|v| v.is_finite()).count(), flats.len());
}

#[test]
fn dense_fill_rejects_coordinates_that_are_not_on_the_grid() {
    let mut rows = sample_rows(&[100_000]);
    rows[0].lat += 0.0123;
    assert!(matches!(
        fill_o1280(&rows),
        Err(FetchError::GridMismatch(_))
    ));
    let mut rows = sample_rows(&[100_000]);
    rows[0].lon += 0.0071;
    assert!(matches!(
        fill_o1280(&rows),
        Err(FetchError::GridMismatch(_))
    ));
}

#[test]
fn null_values_stay_nan_and_do_not_count_as_valid() {
    let mut rows = sample_rows(&[10, 11, 12]);
    rows[1].value = f32::NAN;
    let values = fill_o1280(&rows).unwrap();
    let stats = compute_stats(values.as_slice(), "°C");
    assert_eq!(stats.valid_count, 2);
    assert_eq!((stats.min, stats.max), (5.0, 6.0));
    assert_eq!(stats.unit, "°C");
    assert_eq!(compute_stats(&[f32::NAN], "x").valid_count, 0);
}

fn tiny_field(id: &str) -> CachedField {
    CachedField {
        field_id: id.to_owned(),
        entry_id: "ecmwf_ifs.temperature_2m".into(),
        reference_time_utc: "2026-11-25T00:00:00Z".into(),
        valid_time_utc: "2026-11-25T06:00:00Z".into(),
        variable: "temperature_2m".into(),
        unit: "°C".into(),
        source_layout: SourceLayout::DataSpatial,
        source_object: "s3://openmeteo/x.om".into(),
        object_version: None,
        coverage: bbox(0.0, 1.0, &[[0.0, 1.0]]),
        values: GridValues::O1280Dense(vec![f32::NAN; 4]),
        stats: FieldStats {
            valid_count: 0,
            min: 0.0,
            max: 0.0,
            unit: "°C".into(),
        },
    }
}

#[test]
fn cache_keeps_two_fields_and_evicts_least_recently_used() {
    let mut cache = FieldCache::new();
    assert!(cache.insert(tiny_field("a")).is_empty());
    assert!(cache.insert(tiny_field("b")).is_empty());
    assert!(cache.get("a").is_some(), "touch a so b becomes oldest");
    assert_eq!(cache.insert(tiny_field("c")), vec!["b".to_owned()]);
    assert_eq!(cache.len(), 2);
    assert!(cache.peek("b").is_none());
    assert!(cache.peek("a").is_some() && cache.peek("c").is_some());
    // Re-inserting an id replaces it without evicting the other entry.
    assert!(cache.insert(tiny_field("a")).is_empty());
    assert_eq!(cache.len(), 2);
    cache.clear();
    assert!(cache.is_empty());
}

#[test]
fn a_full_dense_field_is_about_26_4_mb() {
    let mut field = tiny_field("big");
    field.values = GridValues::o1280_empty();
    assert_eq!(field.resident_bytes(), 6_599_680 * 4);
}

#[test]
fn regular_field_spec_supports_both_axis_signs_and_fills_bands_cleanly() {
    use omo_engine::map::fetch::{RegularFieldSpec, fill_regular, plan_regular_bands};
    use serde_json::json;

    // South-to-north dlat > 0 (lat0 = -90, dlat = +1.0, ny = 181, nx = 360).
    let def = json!({
        "parameters": {
            "nx": 360,
            "ny": 181,
            "lat0": -90.0,
            "lon0": 0.0,
            "dlat": 1.0,
            "dlon": 1.0
        }
    });
    let spec = RegularFieldSpec::from_om_descriptor("regular_latlon", &def, 360 * 181).unwrap();
    assert_eq!(spec.lat_top, 90.0);
    assert_eq!(spec.dlat, 1.0);
    assert!(spec.is_global_lon());

    let tasks = plan_regular_bands(&bbox(20.0, 40.0, &[[100.0, 120.0]]), &spec, 4);
    assert_eq!(tasks.len(), 4);

    let filled = fill_regular(
        &spec,
        &[
            BandRow {
                lat: 35.0,
                lon: 120.0,
                value: 18.5,
            },
            BandRow {
                lat: 35.0,
                lon: -60.0,
                value: 9.25,
            },
        ],
    )
    .unwrap();
    match filled {
        GridValues::Regular(grid) => {
            assert_eq!(grid.sample(35.0, 120.0), 18.5);
            assert_eq!(grid.sample(35.0, -60.0), 9.25);
            assert!(grid.sample(10.0, 10.0).is_nan());
        }
        other => panic!("expected Regular, got {other:?}"),
    }
}
