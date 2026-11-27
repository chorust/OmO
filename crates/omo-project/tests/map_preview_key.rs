//! `MapPreviewKey`: equality, `-0.0 -> 0.0` range normalisation, staleness invalidation.

use omo_project::map::{MapLayerConfig, MapLayerKind};
use omo_project::preview_key::MapPreviewKey;

fn layers() -> Vec<MapLayerConfig> {
    vec![
        MapLayerConfig::new("coastlines", MapLayerKind::BuiltinCoastlines, true, 1.0),
        MapLayerConfig::new("forecast", MapLayerKind::ForecastField, true, 0.78),
        MapLayerConfig::new("stations", MapLayerKind::StationOverlay, true, 1.0),
    ]
}

fn key() -> MapPreviewKey {
    MapPreviewKey {
        doc_id: "doc".into(),
        revision: 3,
        stations_hash: Some("abc".into()),
        catalog_entry_id: "ecmwf_ifs.temperature_2m".into(),
        catalog_version: "2026-11-25.1".into(),
        run_ref: "2026-11-25T00:00:00Z".into(),
        valid_time_utc: "2026-11-25T06:00:00Z".into(),
        color_palette: "turbo".into(),
        color_locked_range: None,
        layers_hash: MapPreviewKey::layers_digest(&layers()),
        worker_generation: 1,
    }
}

#[test]
fn equality_covers_every_field() {
    let base = key();
    assert_eq!(base, key());
    assert!(!base.is_stale_against(&key()));

    let mut variants: Vec<MapPreviewKey> = Vec::new();
    macro_rules! vary {
        ($field:ident, $value:expr) => {{
            let mut changed = base.clone();
            changed.$field = $value;
            variants.push(changed);
        }};
    }
    vary!(doc_id, "other".into());
    vary!(revision, 4);
    vary!(stations_hash, None);
    vary!(catalog_entry_id, "other.entry".into());
    vary!(catalog_version, "2026-11-25.2".into());
    vary!(run_ref, "latest_complete".into());
    vary!(valid_time_utc, "2026-11-25T12:00:00Z".into());
    vary!(color_palette, "viridis".into());
    vary!(
        color_locked_range,
        MapPreviewKey::normalized_range(Some(0.0), Some(1.0))
    );
    vary!(layers_hash, "different".into());
    vary!(worker_generation, 2);
    for changed in variants {
        assert_ne!(base, changed);
        assert!(base.is_stale_against(&changed));
    }
}

#[test]
fn negative_zero_range_normalises_to_positive_zero() {
    let a = MapPreviewKey::normalized_range(Some(-0.0), Some(5.0)).unwrap();
    let b = MapPreviewKey::normalized_range(Some(0.0), Some(5.0)).unwrap();
    assert_eq!(a, b);
    assert_eq!(a.0, 0.0f64.to_bits());
    let c = MapPreviewKey::normalized_range(Some(-5.0), Some(-0.0)).unwrap();
    assert_eq!(c.1, 0.0f64.to_bits());
}

#[test]
fn range_requires_both_finite_ordered_bounds() {
    assert_eq!(MapPreviewKey::normalized_range(None, None), None);
    assert_eq!(MapPreviewKey::normalized_range(Some(1.0), None), None);
    assert_eq!(MapPreviewKey::normalized_range(Some(5.0), Some(5.0)), None);
    assert_eq!(MapPreviewKey::normalized_range(Some(6.0), Some(5.0)), None);
    assert_eq!(
        MapPreviewKey::normalized_range(Some(f64::NAN), Some(5.0)),
        None
    );
    assert_eq!(
        MapPreviewKey::normalized_range(Some(0.0), Some(f64::INFINITY)),
        None
    );
}

#[test]
fn layers_digest_tracks_visible_layers_opacity_and_order_only() {
    let base = layers();
    let digest = MapPreviewKey::layers_digest(&base);
    assert_eq!(digest, MapPreviewKey::layers_digest(&base.clone()));
    assert_eq!(digest.len(), 64);

    let mut opacity = base.clone();
    opacity[1].opacity = 0.5;
    assert_ne!(digest, MapPreviewKey::layers_digest(&opacity));

    let mut reordered = base.clone();
    reordered.swap(0, 1);
    assert_ne!(digest, MapPreviewKey::layers_digest(&reordered));

    let mut hidden = base.clone();
    hidden[0].visible = false;
    assert_ne!(digest, MapPreviewKey::layers_digest(&hidden));

    // A hidden layer's own opacity does not change the displayed map.
    let mut hidden_opacity = hidden.clone();
    hidden_opacity[0].opacity = 0.25;
    assert_eq!(
        MapPreviewKey::layers_digest(&hidden),
        MapPreviewKey::layers_digest(&hidden_opacity)
    );

    // -0.0 opacity is canonicalised.
    let mut zero = base.clone();
    zero[1].opacity = 0.0;
    let mut neg_zero = base.clone();
    neg_zero[1].opacity = -0.0;
    assert_eq!(
        MapPreviewKey::layers_digest(&zero),
        MapPreviewKey::layers_digest(&neg_zero)
    );
}
