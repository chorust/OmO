//! `EPSG:3857` tile math and station framing (contracts/layer-stack-and-pmtiles.md §1).

use omo_protocol::mercator::{
    MAX_LATITUDE, east_px, frame_stations, is_in_projection, lonlat_to_world_px, tile_pixel_global,
    world_px, world_px_to_lonlat,
};

#[test]
fn world_size_is_256_shifted_by_zoom() {
    assert_eq!(world_px(1), 512);
    assert_eq!(world_px(12), 256 << 12);
}

#[test]
fn pixel_wgs84_round_trip_for_every_zoom() {
    let samples = [
        (0.0, 0.0),
        (116.4074, 39.9042),
        (-179.999, -85.0),
        (179.9999, 85.0),
        (-0.0001, 0.0001),
        (121.4737, 31.2304),
        (-73.9857, 40.7484),
    ];
    for z in 1..=12u8 {
        for (lon, lat) in samples {
            let (px, py) = lonlat_to_world_px(z, lon, lat);
            let (lon2, lat2) = world_px_to_lonlat(z, px, py);
            assert!((lon - lon2).abs() < 1e-9, "z{z} lon {lon} -> {lon2}");
            assert!((lat - lat2).abs() < 1e-9, "z{z} lat {lat} -> {lat2}");
        }
    }
    // Corners of the pyramid.
    let (px, py) = lonlat_to_world_px(3, -180.0, MAX_LATITUDE);
    assert!(px.abs() < 1e-9 && py.abs() < 1e-6);
    let (px, py) = lonlat_to_world_px(3, 0.0, 0.0);
    assert_eq!((px, py), (1024.0, 1024.0));
}

#[test]
fn polar_clamp_classification() {
    assert_eq!(MAX_LATITUDE, 85.05112878);
    assert!(is_in_projection(0.0));
    assert!(is_in_projection(85.0511));
    assert!(is_in_projection(-85.0511));
    assert!(is_in_projection(MAX_LATITUDE));
    assert!(!is_in_projection(85.06));
    assert!(!is_in_projection(-85.06));
    assert!(!is_in_projection(90.0));
    assert!(!is_in_projection(f64::NAN));
    // Out-of-range latitudes clamp instead of producing non-finite pixels.
    let (_, py) = lonlat_to_world_px(4, 0.0, 89.9);
    assert!(py.is_finite() && py.abs() < 1e-6);
}

#[test]
fn east_px_wraps_exactly_at_prime_meridian_and_antimeridian() {
    for z in 1..=12u8 {
        let world = world_px(z);
        let half_tiles = 1u32 << (z - 1);
        // x = 2^(z-1), u = 0 is exactly 0 deg east.
        let (gx, _) = tile_pixel_global(z, half_tiles, 0, 0, 0);
        assert_eq!(gx, world / 2);
        assert_eq!(east_px(z, gx), 0);
        // The pixel left of 0 deg is the last pixel before 360 deg east.
        let (gx_left, _) = tile_pixel_global(z, half_tiles - 1, 0, 255, 0);
        assert_eq!(east_px(z, gx_left), world - 1);
        // -180 deg (x = 0, u = 0) is 180 deg east.
        let (gx, _) = tile_pixel_global(z, 0, 0, 0, 0);
        assert_eq!(gx, 0);
        assert_eq!(east_px(z, gx), world / 2);
        // +180 deg wraps from the last pixel of x = 2^z - 1 to x = 2^z == 0.
        let last = (1u32 << z) - 1;
        let (gx_last, _) = tile_pixel_global(z, last, 0, 255, 0);
        assert_eq!(east_px(z, gx_last), world / 2 - 1);
        let (gx_wrap, _) = tile_pixel_global(z, last + 1, 0, 0, 0);
        assert_eq!(gx_wrap, 0, "x = 2^z wraps to 0");
        assert_eq!(east_px(z, gx_wrap), world / 2);
    }
}

#[test]
fn tile_pixel_global_y_is_not_wrapped() {
    let (_, gy) = tile_pixel_global(5, 3, 7, 10, 20);
    assert_eq!(gy, 7 * 256 + 20);
}

#[test]
fn single_station_gets_the_minimum_two_degree_half_span() {
    let frame = frame_stations(&[(116.4, 39.9)], 1200.0, 800.0).unwrap();
    assert_eq!(frame.out_of_projection, 0);
    assert!((frame.lon_span_deg - 4.0).abs() < 1e-12);
    assert!((frame.lat_max - frame.lat_min - 4.0).abs() < 1e-12);
    assert!((frame.center_lon - 116.4).abs() < 1e-9);
    assert!((frame.center_lat - 39.9).abs() < 1e-9);
    let spans = frame.lon_spans();
    assert_eq!(spans.len(), 1);
    assert!((spans[0][0] - 114.4).abs() < 1e-9 && (spans[0][1] - 118.4).abs() < 1e-9);
    assert!((1..=12).contains(&frame.zoom));
}

#[test]
fn shortest_arc_across_the_antimeridian() {
    let frame = frame_stations(&[(178.0, 10.0), (-178.0, 12.0)], 1200.0, 800.0).unwrap();
    assert!((frame.lon_span_deg - 4.0).abs() < 1e-12, "{frame:?}");
    assert!((frame.lon_west - 178.0).abs() < 1e-12);
    assert!(
        (frame.center_lon - (-180.0)).abs() < 1e-9,
        "{}",
        frame.center_lon
    );
    assert_eq!(frame.lon_spans(), vec![[178.0, 180.0], [-180.0, -178.0]]);
    // The 2 deg latitude band is widened to the 4 deg minimum around its center.
    assert_eq!((frame.lat_min, frame.lat_max), (9.0, 13.0));
}

#[test]
fn order_of_stations_does_not_change_the_frame() {
    let a = frame_stations(
        &[(100.0, 12.0), (140.0, 42.0), (120.0, 30.0)],
        1000.0,
        700.0,
    )
    .unwrap();
    let b = frame_stations(
        &[(140.0, 42.0), (120.0, 30.0), (100.0, 12.0)],
        1000.0,
        700.0,
    )
    .unwrap();
    assert_eq!(a, b);
    assert!((a.lon_span_deg - 40.0).abs() < 1e-12);
    assert_eq!(a.lon_spans(), vec![[100.0, 140.0]]);
}

#[test]
fn regional_frame_fits_the_viewport_with_padding() {
    let stations: Vec<(f64, f64)> = (0..100)
        .map(|i| {
            (
                100.0 + 40.0 * f64::from(i % 10) / 9.0,
                12.0 + 30.0 * f64::from(i / 10) / 9.0,
            )
        })
        .collect();
    let (w, h) = (1200.0, 800.0);
    let frame = frame_stations(&stations, w, h).unwrap();
    assert!((3..=5).contains(&frame.zoom), "zoom {}", frame.zoom);
    let (x0, y0) = lonlat_to_world_px(frame.zoom, frame.lon_west, frame.lat_max);
    let (x1, y1) = lonlat_to_world_px(
        frame.zoom,
        frame.lon_west + frame.lon_span_deg,
        frame.lat_min,
    );
    assert!(
        x1 - x0 < w && y1 - y0 < h,
        "bbox must fit inside the viewport"
    );
    // One zoom level higher would not fit (padding included): the frame is the largest fit.
    if frame.zoom < 12 {
        let (x0, y0) = lonlat_to_world_px(frame.zoom + 1, frame.lon_west, frame.lat_max);
        let (x1, y1) = lonlat_to_world_px(
            frame.zoom + 1,
            frame.lon_west + frame.lon_span_deg,
            frame.lat_min,
        );
        assert!((x1 - x0) * 1.2 > w || (y1 - y0) * 1.2 > h);
    }
}

#[test]
fn polar_stations_are_counted_not_framed() {
    let frame =
        frame_stations(&[(0.0, 89.0), (116.0, 40.0), (117.0, -88.0)], 1000.0, 700.0).unwrap();
    assert_eq!(frame.out_of_projection, 2);
    assert!((frame.center_lat - 40.0).abs() < 1e-9);
    assert!(frame_stations(&[(0.0, 89.0)], 1000.0, 700.0).is_none());
    assert!(frame_stations(&[], 1000.0, 700.0).is_none());
}
