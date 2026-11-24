//! O1280 row table and nearest-search contract (research R-01).

use omo_engine::grid::reduced_gaussian::{Nearest, O1280};
use omo_engine::grid::{MEAN_EARTH_RADIUS_M, normalize_longitude, sphere_haversine_f64_v1};

#[test]
fn row_table_matches_the_float32_producer_rule() {
    assert_eq!(O1280::points(), 6_599_680);
    assert_eq!(O1280::nx(0), 20);
    assert_eq!(O1280::nx(1279), 5136);
    assert_eq!(O1280::nx(1280), 5136);
    assert_eq!(O1280::nx(2559), 20);

    let dy = 180.0f32 / ((2.0f32 * 1280.0f32) + 0.5);
    let lat0 = ((1280.0f32 - 0.0 - 1.0) * dy) + (dy / 2.0);
    assert_eq!(O1280::point(0).0, lat0);

    let mut flat = 0u32;
    for y in [0usize, 1, 1279, 1280, 2558, 2559] {
        let nx = O1280::nx(y);
        assert_eq!(O1280::row_offset(y), flat_before(y));
        for x in [0, nx as usize / 2, nx as usize - 1] {
            let index = O1280::flat_index(y, x);
            let (lat, lon) = O1280::point(index);
            let (lat_again, lon_again) = O1280::point(O1280::flat_index(y, x));
            assert_eq!((lat, lon), (lat_again, lon_again));
            assert!((-180.0..180.0).contains(&lon), "lon {lon} at y={y} x={x}");
            let (back_y, back_x) = inverse(index);
            assert_eq!((back_y, back_x), (y, x));
        }
        flat = flat.saturating_add(nx);
    }
    let _ = flat;

    let mut total = 0u32;
    for y in 0..2560 {
        total += O1280::nx(y);
        let (lat, lon) = O1280::point(O1280::flat_index(y, 0));
        assert!(lat.is_finite() && lon.is_finite());
        assert!((-180.0..180.0).contains(&lon));
    }
    assert_eq!(total, 6_599_680);
}

#[test]
fn nearest_search_matches_a_wider_brute_force_window() {
    let (lat, lon) = O1280::point(O1280::flat_index(400, 10));
    let exact = O1280::nearest(f64::from(lat), f64::from(lon));
    assert_eq!(exact.distance_m, 0.0);
    assert_eq!(exact.flat_index, O1280::flat_index(400, 10));

    assert_eq!(row_of(O1280::nearest(90.0, 10.0).flat_index), 0);
    assert_eq!(row_of(O1280::nearest(-90.0, 10.0).flat_index), 2559);
    assert_eq!(row_of(O1280::nearest(89.95, 0.0).flat_index), 0);
    assert_eq!(row_of(O1280::nearest(-89.95, 0.0).flat_index), 2559);

    let east = O1280::nearest(10.0, 180.0);
    let west = O1280::nearest(10.0, -180.0);
    assert_eq!(east.flat_index, west.flat_index);
    let wrapped = O1280::nearest(10.0, 359.99);
    let near_zero = O1280::nearest(10.0, -0.01);
    assert_eq!(wrapped.flat_index, near_zero.flat_index);

    let tie = equidistant_pair();
    assert!(
        tie.distance_m > 0.0,
        "tie fixture must not be an exact grid point"
    );
    assert_eq!(
        tie.flat_index,
        brute_force(tie_station().0, tie_station().1).flat_index
    );

    let mut max_distance = 0.0f64;
    let mut seed = 0x00C0_FFEE_u64;
    for _ in 0..10_000 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let lat = (seed % 180_000) as f64 / 1000.0 - 90.0;
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let lon = (seed % 360_000) as f64 / 1000.0 - 180.0;
        let found = O1280::nearest(lat, lon);
        let brute = brute_force(lat, lon);
        assert_eq!(found.flat_index, brute.flat_index, "lat={lat} lon={lon}");
        assert!((found.distance_m - brute.distance_m).abs() < 1e-6);
        max_distance = max_distance.max(found.distance_m);
    }
    let bound = O1280::max_nearest_distance_bound_m();
    assert!(bound <= 7_200.0, "half-cell bound {bound} exceeds 7200 m");
    assert!(
        bound > 7_000.0,
        "half-cell bound {bound} is unexpectedly small"
    );
    assert!(
        max_distance <= 7_200.0,
        "observed nearest {max_distance} exceeds 7200 m"
    );
    assert!(
        max_distance <= bound + 1.0,
        "observed nearest {max_distance} exceeds half-cell bound {bound}"
    );
    let _ = MEAN_EARTH_RADIUS_M;
}

fn flat_before(y: usize) -> u32 {
    (0..y).map(O1280::nx).sum()
}

fn inverse(flat: u32) -> (usize, usize) {
    let mut y = 0usize;
    let mut cursor = 0u32;
    loop {
        let nx = O1280::nx(y);
        if flat < cursor + nx {
            return (y, (flat - cursor) as usize);
        }
        cursor += nx;
        y += 1;
    }
}

fn row_of(flat: u32) -> usize {
    inverse(flat).0
}

fn tie_station() -> (f64, f64) {
    let y = 1280usize;
    let x = 100usize;
    let left = O1280::point(O1280::flat_index(y, x));
    let right = O1280::point(O1280::flat_index(y, x + 1));
    let lon = (f64::from(left.1) + f64::from(right.1)) / 2.0;
    (f64::from(left.0), lon)
}

fn equidistant_pair() -> Nearest {
    let (lat, lon) = tie_station();
    O1280::nearest(lat, lon)
}

fn brute_force(lat: f64, lon: f64) -> Nearest {
    // Rows ±3, every longitude that can beat a 200 km threshold. 200 km is far
    // outside the 7.2 km half-cell bound, so the retained points include the
    // true nearest. The scan wraps across the antimeridian.
    let center = O1280::row_near_lat(lat);
    let y0 = center.saturating_sub(3);
    let y1 = (center + 3).min(2559);
    let cos_lat = lat.to_radians().cos().abs().max(1e-3);
    let max_dlon_deg = (200_000.0 / (MEAN_EARTH_RADIUS_M * cos_lat)).to_degrees();
    let station_lon = normalize_longitude(lon);
    let mut best: Option<Nearest> = None;
    for y in y0..=y1 {
        let nx = O1280::nx(y) as usize;
        let unwrapped = if station_lon < 0.0 {
            station_lon + 360.0
        } else {
            station_lon
        };
        let center_x = unwrapped / 360.0 * nx as f64;
        let half = (max_dlon_deg / 360.0 * nx as f64).ceil() as i64 + 2;
        let anchor = center_x.floor() as i64;
        for delta in -half..=half {
            let x = (anchor + delta).rem_euclid(nx as i64) as usize;
            consider(lat, lon, y, x, &mut best);
        }
    }
    best.expect("brute force window is non-empty")
}

fn consider(lat: f64, lon: f64, y: usize, x: usize, best: &mut Option<Nearest>) {
    let flat = O1280::flat_index(y, x);
    let (grid_lat, grid_lon) = O1280::point(flat);
    let distance_m = sphere_haversine_f64_v1(
        lon,
        lat,
        f64::from(grid_lon),
        f64::from(grid_lat),
        MEAN_EARTH_RADIUS_M,
    );
    let candidate = Nearest {
        flat_index: flat,
        lat: grid_lat,
        lon: grid_lon,
        distance_m,
    };
    *best = Some(match best {
        None => candidate,
        Some(current) => {
            if candidate.distance_m < current.distance_m
                || (candidate.distance_m == current.distance_m
                    && candidate.flat_index < current.flat_index)
            {
                candidate
            } else {
                *current
            }
        }
    });
}
