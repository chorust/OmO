//! Map viewport math used by the interactive Map surface (`FR-001`, `FR-002`, `FR-009`).

use omo_app::host::map_host::Viewport;
use omo_protocol::mercator::{frame_stations, world_px};

fn close(a: f64, b: f64, eps: f64) -> bool {
    (a - b).abs() < eps
}

#[test]
fn center_round_trips_through_screen_space() {
    let vp = Viewport::new(116.4, 39.9, 5, 1000.0, 600.0);
    let (lon, lat) = vp.screen_to_lonlat(500.0, 300.0);
    assert!(
        close(lon, 116.4, 1e-9) && close(lat, 39.9, 1e-9),
        "{lon} {lat}"
    );
    let (sx, sy) = vp.lonlat_to_screen(116.4, 39.9).unwrap();
    assert!(close(sx, 500.0, 1e-6) && close(sy, 300.0, 1e-6));
    // A point to the east appears to the right.
    let (east_x, _) = vp.lonlat_to_screen(120.0, 39.9).unwrap();
    assert!(east_x > 500.0);
}

#[test]
fn dragging_moves_the_map_with_the_pointer() {
    let mut vp = Viewport::new(0.0, 0.0, 4, 800.0, 600.0);
    let before = vp.screen_to_lonlat(400.0, 300.0);
    vp.pan_pixels(100.0, 0.0);
    // The geography that was at the center is now 100 px to the right.
    let (sx, _) = vp.lonlat_to_screen(before.0, before.1).unwrap();
    assert!(close(sx, 500.0, 1e-6), "{sx}");
}

#[test]
fn panning_across_the_antimeridian_wraps_longitude() {
    let mut vp = Viewport::new(179.0, 10.0, 4, 800.0, 600.0);
    vp.pan_pixels(-(world_px(4) as f64) / 2.0 + 50.0, 0.0); // drag far left = move east
    let (lon, _) = vp.center_lonlat();
    assert!((-180.0..180.0).contains(&lon), "{lon}");
}

#[test]
fn latitude_is_clamped_to_the_projection() {
    let mut vp = Viewport::new(0.0, 80.0, 3, 800.0, 600.0);
    vp.pan_pixels(0.0, 1.0e9); // drag down forever = move north
    let (_, lat) = vp.center_lonlat();
    assert!(lat <= 85.05112878 + 1e-9 && lat > 80.0, "{lat}");
}

#[test]
fn zoom_keeps_the_geography_under_the_cursor_fixed_and_clamps() {
    let mut vp = Viewport::new(100.0, 30.0, 5, 1000.0, 700.0);
    let anchor = (800.0, 200.0);
    let geo = vp.screen_to_lonlat(anchor.0, anchor.1);
    vp.zoom_by(2, anchor.0, anchor.1);
    assert_eq!(vp.zoom, 7);
    let (sx, sy) = vp.lonlat_to_screen(geo.0, geo.1).unwrap();
    assert!(
        close(sx, anchor.0, 1e-6) && close(sy, anchor.1, 1e-6),
        "{sx} {sy}"
    );
    vp.zoom_by(100, 0.0, 0.0);
    assert_eq!(vp.zoom, 12);
    vp.zoom_by(-100, 0.0, 0.0);
    assert_eq!(vp.zoom, 1);
}

#[test]
fn visible_tiles_cover_the_viewport_with_wrapped_coordinates() {
    let vp = Viewport::new(179.5, 0.0, 3, 1000.0, 600.0);
    let tiles = vp.visible_tiles();
    assert!(!tiles.is_empty());
    let n = 1u32 << 3;
    assert!(tiles.iter().all(|t| t.coord.is_valid() && t.coord.x < n));
    // Both sides of the antimeridian are present: x = n-1 and x = 0.
    assert!(tiles.iter().any(|t| t.coord.x == n - 1));
    assert!(tiles.iter().any(|t| t.coord.x == 0));
    // Every pixel of the viewport lies inside some tile rectangle.
    for (sx, sy) in [(0.0, 0.0), (999.0, 599.0), (500.0, 300.0), (999.0, 0.0)] {
        assert!(
            tiles
                .iter()
                .any(|t| sx >= t.left && sx < t.left + 256.0 && sy >= t.top && sy < t.top + 256.0),
            "({sx},{sy}) is not covered"
        );
    }
    // Unwrapped keys are unique even when two tiles map to the same wrapped coordinate.
    let mut keys: Vec<_> = tiles.iter().map(|t| (t.unwrapped_x, t.coord.y)).collect();
    keys.sort_unstable();
    keys.dedup();
    assert_eq!(keys.len(), tiles.len());
}

#[test]
fn tiles_outside_the_world_vertically_are_skipped() {
    let vp = Viewport::new(0.0, 84.0, 2, 800.0, 800.0);
    assert!(vp.visible_tiles().iter().all(|t| t.coord.y < 4));
}

#[test]
fn from_frame_uses_the_station_framing() {
    let stations = [(116.4, 39.9), (121.5, 31.2), (114.1, 22.5)];
    let frame = frame_stations(&stations, 1000.0, 600.0).unwrap();
    let vp = Viewport::from_frame(&frame, 1000.0, 600.0);
    assert_eq!(vp.zoom, frame.zoom);
    for (lon, lat) in stations {
        let (sx, sy) = vp.lonlat_to_screen(lon, lat).unwrap();
        assert!(
            (0.0..1000.0).contains(&sx) && (0.0..600.0).contains(&sy),
            "({sx},{sy})"
        );
    }
}

#[test]
fn bbox_covers_the_viewport_and_splits_at_the_antimeridian() {
    let vp = Viewport::new(116.0, 35.0, 5, 800.0, 600.0);
    let bbox = vp.bbox(0.0);
    assert_eq!(bbox.lon_spans.len(), 1);
    assert!(bbox.lat_min < 35.0 && bbox.lat_max > 35.0);
    assert!(bbox.lon_spans[0][0] < 116.0 && bbox.lon_spans[0][1] > 116.0);

    let wrap = Viewport::new(179.9, 0.0, 4, 800.0, 600.0).bbox(0.0);
    assert_eq!(wrap.lon_spans.len(), 2, "{wrap:?}");
    assert_eq!(wrap.lon_spans[0][1], 180.0);
    assert_eq!(wrap.lon_spans[1][0], -180.0);
}

#[test]
fn resize_keeps_the_center() {
    let mut vp = Viewport::new(10.0, 20.0, 6, 800.0, 600.0);
    let c = vp.center_lonlat();
    vp.resize(1200.0, 900.0);
    let c2 = vp.center_lonlat();
    assert!(close(c.0, c2.0, 1e-9) && close(c.1, c2.1, 1e-9));
    assert_eq!((vp.width, vp.height), (1200.0, 900.0));
}

#[test]
fn capped_bbox_bounds_a_wide_view_around_the_center() {
    let vp = Viewport::new(105.0, 30.0, 3, 1600.0, 900.0);
    let full = vp.bbox(256.0);
    assert!(full.lon_spans[0][1] - full.lon_spans[0][0] > 80.0);
    let capped = vp.capped_bbox(256.0, 60.0, 80.0);
    assert!(
        (capped.lat_max - capped.lat_min) <= 60.0 + 1e-9,
        "{capped:?}"
    );
    let span: f64 = capped.lon_spans.iter().map(|s| s[1] - s[0]).sum();
    assert!((span - 80.0).abs() < 1e-9, "{capped:?}");
    assert!(capped.lon_spans[0][0] < 105.0 && capped.lon_spans[0][1] > 105.0);

    // A view already smaller than the cap is unchanged.
    let small = Viewport::new(105.0, 30.0, 6, 800.0, 600.0);
    assert_eq!(small.capped_bbox(256.0, 60.0, 80.0), small.bbox(256.0));

    // The cap wraps across the antimeridian.
    let wrap = Viewport::new(179.0, 0.0, 2, 1600.0, 900.0).capped_bbox(0.0, 60.0, 80.0);
    assert_eq!(wrap.lon_spans.len(), 2, "{wrap:?}");
}

#[test]
fn unwrapped_projection_agrees_with_the_wrapped_one_and_never_splits_at_the_antimeridian() {
    let vp = Viewport::new(116.4, 39.9, 5, 1000.0, 600.0);
    let (x, y) = vp.project_unwrapped(118.0, 40.5);
    let (sx, sy) = vp.lonlat_to_screen(118.0, 40.5).unwrap();
    assert!(close(x, sx, 1e-6) && close(y, sy, 1e-6));

    let across = Viewport::new(179.5, 0.0, 6, 800.0, 400.0);
    let west = across.project_unwrapped(179.0, 0.0);
    let east = across.project_unwrapped(181.0, 0.0);
    assert!(east.0 > west.0, "{east:?} {west:?}");
    // The unwrapped window reaches past 180°.
    let window = across.geo_window();
    assert!(window[2] > 180.0 && window[0] < 180.0, "{window:?}");
    assert!(window[1] < 0.0 && window[3] > 0.0);
}

#[test]
fn the_window_corners_project_onto_the_viewport_corners() {
    let vp = Viewport::new(10.0, 50.0, 4, 640.0, 480.0);
    let w = vp.geo_window();
    let (x0, y_top) = vp.project_unwrapped(w[0], w[3]);
    let (x1, y_bottom) = vp.project_unwrapped(w[2], w[1]);
    assert!(
        close(x0, 0.0, 1e-6) && close(y_top, 0.0, 1e-6),
        "{x0} {y_top}"
    );
    assert!(
        close(x1, 640.0, 1e-6) && close(y_bottom, 480.0, 1e-6),
        "{x1} {y_bottom}"
    );
}
