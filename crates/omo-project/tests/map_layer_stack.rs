//! Layer-stack editing of `MapViewState` (`FR-013`..`FR-016`, `T037`/`T038`).

use std::path::PathBuf;

use omo_project::map::{LayerMove, LayerStackError, MapLayerKind, MapViewState};

fn ids(state: &MapViewState) -> Vec<&str> {
    state.layers.iter().map(|l| l.id.as_str()).collect()
}

#[test]
fn the_default_stack_is_coastlines_forecast_stations_bottom_to_top() {
    assert_eq!(
        ids(&MapViewState::default()),
        ["coastlines", "forecast", "stations"]
    );
}

#[test]
fn mounting_an_archive_inserts_it_below_the_pinned_station_overlay() {
    let mut state = MapViewState::default();
    let id = state
        .add_local_pmtiles(PathBuf::from("/data/base.pmtiles"))
        .unwrap();
    assert_eq!(id, "pmtiles-1");
    assert_eq!(
        ids(&state),
        ["coastlines", "forecast", "pmtiles-1", "stations"]
    );
    let second = state
        .add_local_pmtiles(PathBuf::from("/data/other.PMTILES"))
        .unwrap();
    assert_eq!(second, "pmtiles-2");
    assert!(matches!(
        state.layer(&second).unwrap().kind,
        MapLayerKind::LocalPmtiles { .. }
    ));
}

#[test]
fn only_local_pmtiles_paths_are_accepted_once() {
    let mut state = MapViewState::default();
    for bad in [
        "https://x/y.pmtiles",
        "s3://b/k.pmtiles",
        "/data/a.mbtiles",
        "/data/a.png",
        "",
    ] {
        assert!(
            matches!(
                state.add_local_pmtiles(PathBuf::from(bad)),
                Err(LayerStackError::NotLocalPmtiles(_))
            ),
            "{bad}"
        );
    }
    state
        .add_local_pmtiles(PathBuf::from("/data/a.pmtiles"))
        .unwrap();
    assert_eq!(
        state.add_local_pmtiles(PathBuf::from("/data/a.pmtiles")),
        Err(LayerStackError::AlreadyMounted("pmtiles-1".into()))
    );
    assert_eq!(state.layers.len(), 4);
}

#[test]
fn removed_ids_are_reused_and_builtins_cannot_be_removed() {
    let mut state = MapViewState::default();
    state
        .add_local_pmtiles(PathBuf::from("/a.pmtiles"))
        .unwrap();
    state
        .add_local_pmtiles(PathBuf::from("/b.pmtiles"))
        .unwrap();
    state.remove_layer("pmtiles-1").unwrap();
    assert_eq!(
        ids(&state),
        ["coastlines", "forecast", "pmtiles-2", "stations"]
    );
    assert_eq!(
        state
            .add_local_pmtiles(PathBuf::from("/c.pmtiles"))
            .unwrap(),
        "pmtiles-1"
    );
    for builtin in ["coastlines", "forecast", "stations"] {
        assert_eq!(
            state.remove_layer(builtin),
            Err(LayerStackError::BuiltinLayer(builtin.into()))
        );
    }
    assert_eq!(
        state.remove_layer("nope"),
        Err(LayerStackError::UnknownLayer("nope".into()))
    );
}

#[test]
fn layers_move_but_never_above_the_station_overlay() {
    let mut state = MapViewState::default();
    // Coastlines above the forecast (the user's choice) and back below it.
    assert!(state.move_layer("coastlines", LayerMove::Up).unwrap());
    assert_eq!(ids(&state), ["forecast", "coastlines", "stations"]);
    assert!(!state.move_layer("coastlines", LayerMove::Up).unwrap());
    assert!(state.move_layer("coastlines", LayerMove::Down).unwrap());
    assert_eq!(ids(&state), ["coastlines", "forecast", "stations"]);
    // The bottom has no neighbour; the pinned overlay never moves.
    assert!(!state.move_layer("coastlines", LayerMove::Down).unwrap());
    assert!(!state.move_layer("stations", LayerMove::Down).unwrap());
    assert!(!state.move_layer("stations", LayerMove::Up).unwrap());
    assert!(state.move_layer("forecast", LayerMove::Up).is_ok());
    assert_eq!(ids(&state).last(), Some(&"stations"));
}

#[test]
fn visibility_and_opacity_edit_one_layer_and_clamp() {
    let mut state = MapViewState::default();
    state.set_layer_visible("forecast", false).unwrap();
    state.set_layer_opacity("forecast", 1.7).unwrap();
    assert!(!state.layer("forecast").unwrap().visible);
    assert_eq!(state.layer("forecast").unwrap().opacity, 1.0);
    state.set_layer_opacity("forecast", -3.0).unwrap();
    assert_eq!(state.layer("forecast").unwrap().opacity, 0.0);
    state.set_layer_opacity("forecast", f32::NAN).unwrap();
    assert_eq!(state.layer("forecast").unwrap().opacity, 0.0);
    assert!(state.layer("coastlines").unwrap().visible);
    assert!(state.set_layer_visible("x", true).is_err());
    // An edited stack is still canonical.
    state.canonical().unwrap();
}
