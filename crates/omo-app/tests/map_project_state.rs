//! How the live map controls map onto the persisted `MapViewState` (`FR-023`, `SC-008`).

use omo_app::host::map_view::{
    MapExportKey, TileLruCache, Viewport, compose_state, lead_hours, locked_range,
    resolve_lead_with_clamp, valid_time_for_lead,
};
use omo_project::{
    map::{MapLayerKind, MapViewState, MapViewport},
    preview_key::MapPreviewKey,
};

fn times() -> Vec<String> {
    ["00", "03", "06", "09"]
        .iter()
        .map(|h| format!("2026-11-25T{h}:00:00Z"))
        .collect()
}

#[test]
fn lead_hours_counts_from_the_run_reference_time() {
    let reference = "2026-11-25T00:00:00Z";
    assert_eq!(lead_hours(reference, "2026-11-25T06:00:00Z"), Some(6));
    assert_eq!(lead_hours(reference, "2026-11-25T00:00Z"), Some(0));
    assert_eq!(lead_hours(reference, "garbage"), None);
    // A valid time before the run is not a forecast lead.
    assert_eq!(lead_hours(reference, "2026-11-24T18:00:00Z"), None);
}

#[test]
fn a_lead_resolves_to_the_matching_valid_time_or_none() {
    let times = times();
    let reference = "2026-11-25T00:00:00Z";
    assert_eq!(
        valid_time_for_lead(reference, &times, 6).as_deref(),
        Some("2026-11-25T06:00:00Z")
    );
    // Lead 0 means "the run's first valid time", whatever it is.
    assert_eq!(
        valid_time_for_lead(reference, &times, 0).as_deref(),
        Some("2026-11-25T00:00:00Z")
    );
    assert_eq!(valid_time_for_lead(reference, &times, 7), None);
}

#[test]
fn locked_range_needs_both_finite_ordered_bounds() {
    assert_eq!(locked_range(" -5 ", "30.5"), Some((-5.0, 30.5)));
    assert_eq!(locked_range("", "30"), None);
    assert_eq!(locked_range("30", "5"), None);
    assert_eq!(locked_range("5", "5"), None);
    assert_eq!(locked_range("abc", "5"), None);
    assert_eq!(locked_range("NaN", "5"), None);
}

#[test]
fn composed_state_is_canonical_and_keeps_the_layer_stack() {
    let mut layers = MapViewState::default();
    let id = layers
        .add_local_pmtiles("/data/basemap.pmtiles".into())
        .unwrap();
    let state = compose_state(
        &layers,
        Some(MapViewport {
            center_lat: -0.0,
            center_lon: 190.0,
            zoom: 40,
        }),
        6,
        "viridis",
        "-0",
        "10",
    );
    let viewport = state.viewport.unwrap();
    assert_eq!(viewport.center_lat.to_bits(), 0.0f64.to_bits());
    assert_eq!(viewport.center_lon, -170.0);
    assert_eq!(viewport.zoom, 12);
    assert_eq!(state.valid_time_lead_h, 6);
    assert_eq!(state.color_scale.palette, "viridis");
    assert_eq!(
        state.color_scale.locked_min.unwrap().to_bits(),
        0.0f64.to_bits()
    );
    assert_eq!(state.color_scale.locked_max, Some(10.0));
    assert!(
        state
            .layer(&id)
            .is_some_and(|l| matches!(l.kind, MapLayerKind::LocalPmtiles { .. }))
    );
}

#[test]
fn untouched_controls_compose_to_the_default_state() {
    let state = compose_state(&MapViewState::default(), None, 0, "turbo", "", "");
    assert!(state.is_default());
}

#[test]
fn an_unknown_palette_never_reaches_the_file() {
    let state = compose_state(&MapViewState::default(), None, 0, "rainbow", "", "");
    assert_eq!(state.color_scale.palette, "turbo");
}

#[test]
fn out_of_range_lead_clamps_to_run_horizon_with_visible_notice() {
    let times = times(); // 00, 03, 06, 09 (+9 h max)
    let reference = "2026-11-25T00:00:00Z";
    let exact = resolve_lead_with_clamp(reference, &times, 6).unwrap();
    assert_eq!(exact.valid_time_utc, "2026-11-25T06:00:00Z");
    assert_eq!(exact.effective_lead_h, 6);
    assert!(exact.clamp_notice.is_none());

    let clamped = resolve_lead_with_clamp(reference, &times, 240).unwrap();
    assert_eq!(clamped.valid_time_utc, "2026-11-25T09:00:00Z");
    assert_eq!(clamped.effective_lead_h, 9);
    assert!(
        clamped
            .clamp_notice
            .as_deref()
            .is_some_and(|msg| msg.contains("+240 h") && msg.contains("+9 h")),
        "{clamped:?}"
    );
}

#[test]
fn tile_lru_cache_evicts_oldest_non_visible_entries_and_honors_reads() {
    let mut cache: TileLruCache<u32, &'static str> = TileLruCache::new();
    cache.insert(1, "one");
    cache.insert(2, "two");
    cache.insert(3, "three");
    cache.insert(4, "four");
    // Touch `1` via shared `&self` so `2` becomes the oldest non-visible entry.
    assert_eq!(cache.get_touch(&1), Some(&"one"));
    // Keep `2` as visible (`keep(2) == true`), so `3` must be evicted first when shrinking to 3!
    cache.evict_lru(3, 3, |k| *k == 2);
    assert_eq!(cache.len(), 3);
    assert!(cache.get_peek(&1).is_some());
    assert!(cache.get_peek(&2).is_some());
    assert!(
        cache.get_peek(&3).is_none(),
        "3 was LRU among non-visible tiles"
    );
    assert!(cache.get_peek(&4).is_some());
}

#[test]
fn map_export_key_changes_on_viewport_selection_or_preview_edits() {
    let pkey = MapPreviewKey {
        doc_id: "d1".into(),
        revision: 1,
        stations_hash: Some("s1".into()),
        catalog_entry_id: "ecmwf_ifs.temperature_2m".into(),
        catalog_version: "1".into(),
        run_ref: "2026-11-25T00:00:00Z".into(),
        valid_time_utc: "2026-11-25T06:00:00Z".into(),
        color_palette: "turbo".into(),
        color_locked_range: None,
        layers_hash: "lh".into(),
        worker_generation: 1,
    };
    let vp1 = Viewport::new(120.0, 35.0, 5, 800.0, 600.0);
    let vp2 = Viewport::new(121.0, 35.0, 5, 800.0, 600.0);
    let base = MapExportKey::new(pkey.clone(), &vp1, &["S1".into(), "S2".into()]);
    // Order/duplication of selected IDs does not change key:
    assert_eq!(
        base,
        MapExportKey::new(pkey.clone(), &vp1, &["S2".into(), "S1".into(), "S1".into()])
    );
    // Panning invalidates saved export:
    assert_ne!(
        base,
        MapExportKey::new(pkey.clone(), &vp2, &["S1".into(), "S2".into()])
    );
    // Changing station selection invalidates saved export:
    assert_ne!(base, MapExportKey::new(pkey.clone(), &vp1, &["S1".into()]));
    // Changing preview key (e.g. palette) invalidates saved export:
    let mut pkey2 = pkey;
    pkey2.color_palette = "viridis".into();
    assert_ne!(
        base,
        MapExportKey::new(pkey2, &vp1, &["S1".into(), "S2".into()])
    );
}
