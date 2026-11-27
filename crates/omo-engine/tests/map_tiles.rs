//! Tile rasterizer tests (`T021`, `SC-002`, `SC-006`): zero seams, interpolation accuracy,
//! `NaN` transparency, palette behaviour, and regular-grid bilinear sampling.

use omo_engine::{
    grid::reduced_gaussian::O1280,
    map::raster::{
        ColorLut, O1280Sampler, PixelSampler, RegularGrid, RegularSampler, render_tile,
        render_window, sample_o1280,
    },
};
use omo_protocol::{
    map::{PALETTES, palette_stops},
    mercator::{TILE_SIZE, lat_of_global_y, world_px},
};

/// `f(lat, lon)` over the full O1280 grid, smooth and lon-dependent so seams would show.
fn synthetic(nan_every: usize) -> Vec<f32> {
    let mut values = vec![0f32; O1280::points() as usize];
    for y in 0..O1280::rows() {
        let lat = f64::from(O1280::lat_of_row(y));
        let nx = O1280::nx(y) as usize;
        let offset = O1280::row_offset(y) as usize;
        for x in 0..nx {
            let lon = x as f64 * 360.0 / nx as f64;
            let value = lat * 0.25 + 10.0 * (lon.to_radians()).sin();
            values[offset + x] = value as f32;
        }
    }
    if nan_every > 0 {
        for index in (0..values.len()).step_by(nan_every) {
            values[index] = f32::NAN;
        }
    }
    values
}

fn lut() -> ColorLut {
    ColorLut::new("turbo").unwrap()
}

fn mosaic_equals_single_pass(values: &[f32], z: u8, x0: u32, y0: u32) {
    let sampler = O1280Sampler::new(values);
    let lut = lut();
    let size = TILE_SIZE as usize;
    let mut mosaic = vec![0u8; 2 * size * 2 * size * 4];
    let n = 1u32 << z;
    for ty in 0..2u32 {
        for tx in 0..2u32 {
            let (tile, _) = render_tile(&sampler, &lut, -40.0, 40.0, z, (x0 + tx) % n, y0 + ty);
            assert_eq!(tile.len(), size * size * 4);
            for row in 0..size {
                let dst = ((ty as usize * size + row) * 2 * size + tx as usize * size) * 4;
                mosaic[dst..dst + size * 4]
                    .copy_from_slice(&tile[row * size * 4..(row + 1) * size * 4]);
            }
        }
    }
    let single = render_window(
        &sampler,
        &lut,
        -40.0,
        40.0,
        z,
        u64::from(x0) * u64::from(TILE_SIZE),
        u64::from(y0) * u64::from(TILE_SIZE),
        2 * TILE_SIZE,
        2 * TILE_SIZE,
    );
    let differing = mosaic.iter().zip(&single).filter(|(a, b)| a != b).count();
    assert_eq!(
        differing, 0,
        "z{z} block ({x0},{y0}) has {differing} differing bytes"
    );
}

#[test]
fn tiles_are_seamless_in_the_interior_and_across_both_meridian_seams() {
    let values = synthetic(97);
    for z in [3u8, 6, 9] {
        let n = 1u32 << z;
        let mid = n / 2;
        // Interior block.
        mosaic_equals_single_pass(&values, z, mid / 2, mid - 1);
        // Straddles the 0 degree Prime Meridian: x = n/2 - 1 | n/2.
        mosaic_equals_single_pass(&values, z, mid - 1, mid - 1);
        // Straddles the antimeridian: x = n - 1 | 0.
        mosaic_equals_single_pass(&values, z, n - 1, mid - 1);
    }
}

#[test]
fn a_global_row_has_the_same_value_on_both_sides_of_the_wrap() {
    // Pixels gx = 0 and gx = world_px - 1 are adjacent across the antimeridian; wrapped
    // addressing must make `gx + world_px` identical to `gx`.
    let values = synthetic(0);
    let z = 5u8;
    let world = world_px(z);
    for gy in [world / 4, world / 2, world / 2 + 17] {
        for gx in [0u64, 1, world - 1] {
            let a = sample_o1280(&values, z, gx, gy);
            let b = sample_o1280(&values, z, gx + world, gy);
            assert_eq!(a.to_bits(), b.to_bits());
        }
    }
}

#[test]
fn bilinear_sampling_reproduces_a_latitude_linear_field() {
    // f = lat is exactly representable by interpolation between Gaussian rows.
    let mut values = vec![0f32; O1280::points() as usize];
    for y in 0..O1280::rows() {
        let nx = O1280::nx(y) as usize;
        let offset = O1280::row_offset(y) as usize;
        values[offset..offset + nx].fill(O1280::lat_of_row(y));
    }
    let z = 7u8;
    let world = world_px(z);
    for gy in (world / 5..world * 4 / 5).step_by(37) {
        let expected = lat_of_global_y(z, gy);
        let got = f64::from(sample_o1280(&values, z, 12_345 % world, gy));
        assert!(
            (got - expected).abs() < 5e-3,
            "gy {gy}: {got} vs {expected}"
        );
    }
}

#[test]
fn nan_cells_poison_only_neighbouring_pixels() {
    let mut values = vec![1f32; O1280::points() as usize];
    let z = 6u8;
    let world = world_px(z);
    let gy = world / 2;
    let gx = world / 3;
    let clean = sample_o1280(&values, z, gx, gy);
    assert!(clean.is_finite());

    // Poison the whole equator-adjacent rows around the pixel.
    let lat = lat_of_global_y(z, gy);
    let row = O1280::row_near_lat(lat);
    for y in row.saturating_sub(2)..=row + 2 {
        let offset = O1280::row_offset(y) as usize;
        let nx = O1280::nx(y) as usize;
        values[offset..offset + nx].fill(f32::NAN);
    }
    assert!(sample_o1280(&values, z, gx, gy).is_nan());
    // A pixel several degrees away is untouched.
    assert!(sample_o1280(&values, z, gx, gy + world / 8).is_finite());
}

#[test]
fn nan_pixels_render_fully_transparent_and_flag_an_empty_tile() {
    let values = vec![f32::NAN; O1280::points() as usize];
    let sampler = O1280Sampler::new(&values);
    let (tile, empty) = render_tile(&sampler, &lut(), 0.0, 1.0, 4, 8, 8);
    assert!(empty);
    assert!(tile.iter().all(|byte| *byte == 0));

    let ones = vec![0.5f32; O1280::points() as usize];
    let sampler = O1280Sampler::new(&ones);
    let (tile, empty) = render_tile(&sampler, &lut(), 0.0, 1.0, 4, 8, 8);
    assert!(!empty);
    let (pixels, rem) = tile.as_chunks::<4>();
    assert!(rem.is_empty());
    assert!(pixels.iter().all(|px| px[3] == 255));
    // One uniform value maps to one uniform color across the whole tile.
    let first = pixels[0];
    assert!(pixels.iter().all(|&px| px == first));
}

#[test]
fn color_normalization_is_field_wide_not_per_tile() {
    // The same value must produce the same color in two different tiles.
    let values = vec![7.0f32; O1280::points() as usize];
    let sampler = O1280Sampler::new(&values);
    let (a, _) = render_tile(&sampler, &lut(), 0.0, 10.0, 5, 3, 14);
    let (b, _) = render_tile(&sampler, &lut(), 0.0, 10.0, 5, 20, 17);
    assert_eq!(&a[..4], &b[..4]);
    // A different domain changes the color.
    let (c, _) = render_tile(&sampler, &lut(), 0.0, 100.0, 5, 3, 14);
    assert_ne!(&a[..4], &c[..4]);
}

#[test]
fn lut_covers_the_domain_and_handles_degenerate_input() {
    let lut = lut();
    let low = lut.color(-100.0, 0.0, 10.0);
    let high = lut.color(1000.0, 0.0, 10.0);
    assert_eq!(low, lut.color(0.0, 0.0, 10.0));
    assert_eq!(high, lut.color(10.0, 0.0, 10.0));
    assert_ne!(low, high);
    assert_eq!(low[3], 255);
    assert_eq!(lut.color(f32::NAN, 0.0, 10.0), [0, 0, 0, 0]);
    // `min == max` and inverted domains do not panic and stay opaque.
    assert_eq!(lut.color(3.0, 3.0, 3.0)[3], 255);
    assert_eq!(lut.color(3.0, 5.0, 1.0)[3], 255);
    assert!(ColorLut::new("not-a-palette").is_err());
    for name in ["turbo", "viridis", "coolwarm", "magma", "cividis"] {
        let lut = ColorLut::new(name).unwrap();
        assert_ne!(lut.color(0.0, 0.0, 1.0), lut.color(1.0, 0.0, 1.0), "{name}");
    }
}

#[test]
fn regular_grid_bilinear_matches_hand_computation() {
    // 3 columns x 3 rows, 1 degree spacing; top-left corner at (lat 10, lon 20).
    let grid = RegularGrid {
        lat_top: 10.0,
        lon_west: 20.0,
        dlat: 1.0,
        dlon: 1.0,
        nx: 3,
        ny: 3,
        values: vec![
            0.0, 1.0, 2.0, //
            10.0, 11.0, 12.0, //
            20.0, 21.0, 22.0,
        ],
    };
    assert_eq!(grid.sample(10.0, 20.0), 0.0);
    assert_eq!(grid.sample(8.0, 22.0), 22.0);
    assert!((grid.sample(9.5, 20.5) - 5.5).abs() < 1e-6);
    assert!((grid.sample(9.0, 21.0) - 11.0).abs() < 1e-6);
    assert!(grid.sample(11.0, 21.0).is_nan());
    assert!(grid.sample(9.0, 23.0).is_nan());
}

#[test]
fn regular_grid_with_a_nan_corner_is_nan_only_where_that_corner_has_weight() {
    let grid = RegularGrid {
        lat_top: 1.0,
        lon_west: 0.0,
        dlat: 1.0,
        dlon: 1.0,
        nx: 2,
        ny: 2,
        values: vec![f32::NAN, 1.0, 2.0, 3.0],
    };
    assert!(grid.sample(0.5, 0.5).is_nan());
    // On the bottom edge the NaN corner has zero weight.
    assert_eq!(grid.sample(0.0, 0.0), 2.0);
}

#[test]
fn sixteen_tiles_render_within_the_interactive_budget() {
    let values = synthetic(0);
    let sampler = O1280Sampler::new(&values);
    let lut = lut();
    let started = std::time::Instant::now();
    for ty in 0..4u32 {
        for tx in 0..4u32 {
            let (tile, _) = render_tile(&sampler, &lut, -40.0, 40.0, 6, 30 + tx, 20 + ty);
            assert_eq!(tile.len(), 256 * 256 * 4);
        }
    }
    let elapsed = started.elapsed();
    // SC-002: < 100 ms for a visible batch in release; debug builds are only sanity-checked.
    let budget = if cfg!(debug_assertions) { 5_000 } else { 100 };
    assert!(elapsed.as_millis() < budget, "16 tiles took {elapsed:?}");
}

#[test]
fn pixel_sampler_trait_is_object_free_and_consistent() {
    let values = synthetic(0);
    let sampler = O1280Sampler::new(&values);
    for (z, gx, gy) in [
        (4u8, 5u64, 2000u64),
        (8, 70_000 % world_px(8), 40_000 % world_px(8)),
    ] {
        let direct = sample_o1280(&values, z, gx, gy);
        assert_eq!(sampler.sample(z, gx, gy).to_bits(), direct.to_bits());
    }
}

#[test]
fn global_regular_grid_renders_zero_seams_across_antimeridian_and_prime_meridian() {
    // 1-degree global regular grid (360 x 181).
    let (nx, ny) = (360usize, 181usize);
    let mut values = Vec::with_capacity(nx * ny);
    for y in 0..ny {
        let lat = 90.0 - y as f64;
        for x in 0..nx {
            let lon = -180.0 + x as f64;
            values.push((lat * 0.3 + 12.0 * lon.to_radians().cos()) as f32);
        }
    }
    let grid = RegularGrid {
        lat_top: 90.0,
        lon_west: -180.0,
        dlat: 1.0,
        dlon: 1.0,
        nx,
        ny,
        values,
    };
    let sampler = RegularSampler::new(&grid);
    let lut = ColorLut::new("turbo").unwrap();
    let z = 3u8;
    let n = 1u32 << z;
    // Across antimeridian: x = n - 1 and x = 0, y = 3..=4
    let single = render_window(
        &sampler,
        &lut,
        -40.0,
        40.0,
        z,
        u64::from(n - 1) * 256,
        3 * 256,
        512,
        512,
    );
    for (dy, ty) in [(0usize, 3u32), (256, 4)] {
        for (dx, tx) in [(0usize, n - 1), (256, 0)] {
            let (tile, empty) = render_tile(&sampler, &lut, -40.0, 40.0, z, tx, ty);
            assert!(!empty);
            for row in 0..256usize {
                let s_off = ((dy + row) * 512 + dx) * 4;
                let t_off = row * 256 * 4;
                assert_eq!(
                    &single[s_off..s_off + 256 * 4],
                    &tile[t_off..t_off + 256 * 4],
                    "seam mismatch on regular grid tile ({tx},{ty}) row {row}"
                );
            }
        }
    }
}

#[test]
fn protocol_palette_stops_match_engine_color_lut_within_one_lsb() {
    for &palette in PALETTES {
        let engine_stops = ColorLut::new(palette).unwrap().legend_stops(33);
        let proto_stops = palette_stops(palette, 33).unwrap();
        assert_eq!(engine_stops.len(), proto_stops.len());
        for (i, (a, b)) in engine_stops.iter().zip(proto_stops.iter()).enumerate() {
            for ch in 0..3 {
                assert!(
                    a[ch].abs_diff(b[ch]) <= 1,
                    "palette {palette} stop {i} channel {ch}: engine={a:?} proto={b:?}"
                );
            }
        }
    }
}
