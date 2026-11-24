//! PreviewKey covers run, window, and distance; superseded previews drop and stay stale.

use omo_project::preview_key::{PreviewKey, PreviewWindow};

#[test]
fn preview_key_equality_includes_all_m1_fields() {
    let base = PreviewKey::m1_defaults();
    let mut changed = base.clone();
    changed.run_ref = "2026-11-23T00:00:00Z".into();
    assert_ne!(base, changed);

    let mut changed = base.clone();
    changed.window = PreviewWindow {
        start_h: 0,
        end_h: 72,
    };
    assert_ne!(base, changed);

    let mut changed = base.clone();
    changed.max_distance_m = 5000;
    assert_ne!(base, changed);

    let mut changed = base.clone();
    changed.stations_hash = Some("abc".into());
    assert_ne!(base, changed);

    let mut changed = base.clone();
    changed.catalog_version = "2026-11-23.2".into();
    assert_ne!(base, changed);
}
