//! Forecast tile rasterization (`research.md` R-04, `SC-002`, `SC-006`).
//!
//! Every pixel is a pure function of its integer global pixel address `(z, g_x, g_y)`, so tiles
//! rendered separately are byte-identical to one single pass over the same window — across
//! interior seams, the prime meridian and the antimeridian. Longitude never goes through
//! floating-point wrapping: `g_x` is masked into the world and shifted to an east-longitude
//! index with integer arithmetic.
//!
//! O1280 fields use scanline-separable trapezoid-bilinear interpolation: linear along the two
//! bracketing Gaussian rows, then linear between the rows. Missing (`NaN`) cells make only the
//! pixels that give them non-zero weight transparent.

use omo_protocol::mercator::{TILE_SIZE, east_px, lat_of_global_y, normalize_lon, world_px};
use thiserror::Error;

use crate::grid::{MEAN_EARTH_RADIUS_M, reduced_gaussian::O1280, sphere_haversine_f64_v1};

/// Entries in the pre-baked color table.
pub const LUT_SIZE: usize = 4096;

#[derive(Debug, Error)]
pub enum RasterError {
    #[error("unknown palette `{0}` (expected turbo, viridis, coolwarm, magma or cividis)")]
    UnknownPalette(String),
}

/// `4096 x RGBA8` color table baked from a named palette. Colors are opaque; missing values
/// render as `[0, 0, 0, 0]`.
#[derive(Clone, Debug)]
pub struct ColorLut {
    table: Vec<[u8; 4]>,
}

impl ColorLut {
    pub fn new(palette: &str) -> Result<Self, RasterError> {
        use colorgrad::Gradient;
        let at = |t: f32| -> [u8; 4] {
            match palette {
                "turbo" => colorgrad::preset::turbo().at(t).to_rgba8(),
                "viridis" => colorgrad::preset::viridis().at(t).to_rgba8(),
                "magma" => colorgrad::preset::magma().at(t).to_rgba8(),
                "cividis" => colorgrad::preset::cividis().at(t).to_rgba8(),
                _ => coolwarm(t),
            }
        };
        if !matches!(
            palette,
            "turbo" | "viridis" | "magma" | "cividis" | "coolwarm"
        ) {
            return Err(RasterError::UnknownPalette(palette.to_owned()));
        }
        let table = (0..LUT_SIZE)
            .map(|index| {
                let mut color = at(index as f32 / (LUT_SIZE - 1) as f32);
                color[3] = 255;
                color
            })
            .collect();
        Ok(Self { table })
    }

    /// `count >= 2` evenly spaced `[r, g, b]` samples from the low to the high end of the palette.
    pub fn legend_stops(&self, count: usize) -> Vec<[u8; 3]> {
        let count = count.max(2);
        (0..count)
            .map(|i| {
                let c = self.table[i * (LUT_SIZE - 1) / (count - 1)];
                [c[0], c[1], c[2]]
            })
            .collect()
    }

    /// Color of `value` on the domain `[min, max]` (clamped). `NaN` is transparent. A
    /// degenerate or inverted domain maps every finite value to the palette midpoint.
    pub fn color(&self, value: f32, min: f64, max: f64) -> [u8; 4] {
        if !value.is_finite() {
            return [0, 0, 0, 0];
        }
        let t = if max > min && min.is_finite() && max.is_finite() {
            ((f64::from(value) - min) / (max - min)).clamp(0.0, 1.0)
        } else {
            0.5
        };
        self.table[(t * (LUT_SIZE - 1) as f64).round() as usize]
    }
}

/// Moreland-style diverging blue-white-red ramp (piecewise linear, five stops).
fn coolwarm(t: f32) -> [u8; 4] {
    const STOPS: [[f32; 3]; 5] = [
        [59.0, 76.0, 192.0],
        [141.0, 176.0, 254.0],
        [221.0, 221.0, 221.0],
        [245.0, 156.0, 125.0],
        [180.0, 4.0, 38.0],
    ];
    let scaled = t.clamp(0.0, 1.0) * 4.0;
    let index = (scaled.floor() as usize).min(3);
    let frac = scaled - index as f32;
    let channel =
        |c: usize| (STOPS[index][c] + (STOPS[index + 1][c] - STOPS[index][c]) * frac).round() as u8;
    [channel(0), channel(1), channel(2), 255]
}

/// Source of forecast values addressed by integer global pixel.
pub trait PixelSampler {
    /// Value at global pixel `(g_x, g_y)` of zoom `z`; `g_x` wraps modulo the world width and
    /// rows outside the world are `NaN`.
    fn sample(&self, z: u8, gx: u64, gy: u64) -> f32;

    /// Fill `out` with the values of pixels `gx0 .. gx0 + out.len()` in row `gy`.
    fn sample_row(&self, z: u8, gy: u64, gx0: u64, out: &mut [f32]) {
        for (offset, slot) in out.iter_mut().enumerate() {
            *slot = self.sample(z, gx0 + offset as u64, gy);
        }
    }
}

/// `v0 + (v1 - v0) t`, skipping a neighbour that has zero weight so a `NaN` there is harmless.
fn lerp(v0: f32, v1: f32, t: f64) -> f32 {
    if t <= 0.0 {
        v0
    } else if t >= 1.0 {
        v1
    } else if v0.is_nan() || v1.is_nan() {
        f32::NAN
    } else {
        (f64::from(v0) + (f64::from(v1) - f64::from(v0)) * t) as f32
    }
}

/// The two Gaussian rows bracketing `lat` and the weight of the southern one.
fn bracket_rows(lat: f64) -> (usize, usize, f64) {
    let last = O1280::rows() - 1;
    let top = f64::from(O1280::lat_of_row(0));
    let bottom = f64::from(O1280::lat_of_row(last));
    if lat >= top {
        return (0, 0, 0.0);
    }
    if lat <= bottom {
        return (last, last, 0.0);
    }
    let near = O1280::row_near_lat(lat);
    let y0 = if f64::from(O1280::lat_of_row(near)) >= lat {
        near
    } else {
        near - 1
    };
    let y1 = y0 + 1;
    let lat0 = f64::from(O1280::lat_of_row(y0));
    let lat1 = f64::from(O1280::lat_of_row(y1));
    (y0, y1, (lat0 - lat) / (lat0 - lat1))
}

fn sample_in_row(values: &[f32], y: usize, east: u64, world: u64) -> f32 {
    let nx = O1280::nx(y) as usize;
    let offset = O1280::row_offset(y) as usize;
    let position = (east as f64 + 0.5) * nx as f64 / world as f64;
    let c0 = (position.floor() as usize).min(nx - 1);
    let frac = position - c0 as f64;
    let c1 = (c0 + 1) % nx;
    lerp(values[offset + c0], values[offset + c1], frac)
}

fn sample_with_rows(values: &[f32], rows: (usize, usize, f64), z: u8, gx: u64) -> f32 {
    let world = world_px(z);
    let east = east_px(z, gx & (world - 1));
    let north = sample_in_row(values, rows.0, east, world);
    if rows.0 == rows.1 {
        return north;
    }
    lerp(north, sample_in_row(values, rows.1, east, world), rows.2)
}

/// One pixel of an O1280 field (`values` has `O1280::points()` entries).
pub fn sample_o1280(values: &[f32], z: u8, gx: u64, gy: u64) -> f32 {
    if gy >= world_px(z) {
        return f32::NAN;
    }
    sample_with_rows(values, bracket_rows(lat_of_global_y(z, gy)), z, gx)
}

/// [`PixelSampler`] over a dense O1280 value slice.
#[derive(Clone, Copy)]
pub struct O1280Sampler<'a> {
    values: &'a [f32],
}

impl<'a> O1280Sampler<'a> {
    pub fn new(values: &'a [f32]) -> Self {
        assert_eq!(
            values.len(),
            O1280::points() as usize,
            "dense O1280 values expected"
        );
        Self { values }
    }
}

impl PixelSampler for O1280Sampler<'_> {
    fn sample(&self, z: u8, gx: u64, gy: u64) -> f32 {
        sample_o1280(self.values, z, gx, gy)
    }

    fn sample_row(&self, z: u8, gy: u64, gx0: u64, out: &mut [f32]) {
        if gy >= world_px(z) {
            out.fill(f32::NAN);
            return;
        }
        let rows = bracket_rows(lat_of_global_y(z, gy));
        for (offset, slot) in out.iter_mut().enumerate() {
            *slot = sample_with_rows(self.values, rows, z, gx0 + offset as u64);
        }
    }
}

/// Regular lon/lat grid, row 0 at the north edge, bilinear sampling.
#[derive(Clone, Debug)]
pub struct RegularGrid {
    pub lat_top: f64,
    pub lon_west: f64,
    pub dlat: f64,
    pub dlon: f64,
    pub nx: usize,
    pub ny: usize,
    pub values: Vec<f32>,
}

impl RegularGrid {
    /// True when the longitude axis spans the full `360°` globe periodically.
    pub fn is_global_lon(&self) -> bool {
        self.nx >= 2 && self.dlon > 0.0 && ((self.nx as f64) * self.dlon - 360.0).abs() <= 1e-4
    }

    pub fn lat_of_row(&self, y: usize) -> f64 {
        self.lat_top - (y as f64) * self.dlat
    }

    pub fn lon_of_col(&self, x: usize) -> f64 {
        normalize_lon(self.lon_west + (x as f64) * self.dlon)
    }

    /// Bilinear value at `(lat, lon)`; `NaN` outside the grid or when a contributing corner is
    /// missing. Global `360°` grids wrap periodically from column `nx - 1` back to column `0`.
    pub fn sample(&self, lat: f64, lon: f64) -> f32 {
        if self.nx == 0
            || self.ny == 0
            || self.values.len() < self.nx.saturating_mul(self.ny)
            || self.dlat <= 0.0
            || self.dlon <= 0.0
            || !lat.is_finite()
            || !lon.is_finite()
        {
            return f32::NAN;
        }
        let fy = (self.lat_top - lat) / self.dlat;
        let max_y = (self.ny - 1) as f64;
        if !(0.0..=max_y).contains(&fy) {
            return f32::NAN;
        }
        let (y0, y1, ty) = if self.ny == 1 {
            (0, 0, 0.0)
        } else {
            let y0 = (fy.floor() as usize).min(self.ny - 2);
            (y0, y0 + 1, fy - y0 as f64)
        };
        let delta_lon = (lon - self.lon_west).rem_euclid(360.0);
        let fx = delta_lon / self.dlon;
        let (x0, x1, tx) = if self.is_global_lon() {
            let x0 = (fx.floor() as usize) % self.nx;
            let x1 = (x0 + 1) % self.nx;
            (x0, x1, fx - fx.floor())
        } else {
            let max_x = (self.nx - 1) as f64;
            if !(0.0..=max_x).contains(&fx) {
                return f32::NAN;
            }
            if self.nx == 1 {
                (0, 0, 0.0)
            } else {
                let x0 = (fx.floor() as usize).min(self.nx - 2);
                (x0, x0 + 1, fx - x0 as f64)
            }
        };
        let at = |y: usize, x: usize| self.values[y * self.nx + x];
        let north = lerp(at(y0, x0), at(y0, x1), tx);
        if y0 == y1 {
            return north;
        }
        let south = lerp(at(y1, x0), at(y1, x1), tx);
        lerp(north, south, ty)
    }

    /// Sample global pixel `(gx, gy)` at zoom `z` with integer-wrapped `gx` so tile seams
    /// (including `x = 2^z - 1 -> 0`) receive bit-identical coordinates (`SC-006`).
    pub fn sample_global_px(&self, z: u8, gx: u64, gy: u64) -> f32 {
        let world = world_px(z);
        if gy >= world {
            return f32::NAN;
        }
        let wrapped_gx = gx & (world - 1);
        let lon = ((wrapped_gx as f64 + 0.5) / world as f64) * 360.0 - 180.0;
        self.sample(lat_of_global_y(z, gy), lon)
    }

    /// Nearest grid cell `(y, x, grid_lat, grid_lon, distance_m)` by spherical haversine.
    pub fn nearest(&self, lat: f64, lon: f64) -> Option<(usize, usize, f64, f64, f64)> {
        if self.nx == 0
            || self.ny == 0
            || self.values.len() < self.nx.saturating_mul(self.ny)
            || self.dlat <= 0.0
            || self.dlon <= 0.0
            || !lat.is_finite()
            || !lon.is_finite()
        {
            return None;
        }
        let fy = ((self.lat_top - lat) / self.dlat).clamp(0.0, (self.ny - 1) as f64);
        let y_candidates = [fy.floor() as usize, (fy.ceil() as usize).min(self.ny - 1)];
        let delta_lon = (lon - self.lon_west).rem_euclid(360.0);
        let fx = delta_lon / self.dlon;
        let x_candidates = if self.is_global_lon() {
            [
                (fx.floor() as usize) % self.nx,
                (fx.ceil() as usize) % self.nx,
            ]
        } else {
            // Check shortest signed offset to either edge when outside a regional span.
            let max_x = (self.nx - 1) as f64;
            if fx <= max_x {
                [fx.floor() as usize, (fx.ceil() as usize).min(self.nx - 1)]
            } else {
                [0, self.nx - 1]
            }
        };
        let mut best: Option<(usize, usize, f64, f64, f64)> = None;
        for y in y_candidates {
            let glat = self.lat_of_row(y);
            for x in x_candidates {
                let glon = self.lon_of_col(x);
                let dist = sphere_haversine_f64_v1(lon, lat, glon, glat, MEAN_EARTH_RADIUS_M);
                let take = match best {
                    None => true,
                    Some((by, bx, _, _, bdist)) => {
                        dist < bdist || (dist == bdist && (y, x) < (by, bx))
                    }
                };
                if take {
                    best = Some((y, x, glat, glon, dist));
                }
            }
        }
        best
    }
}

/// [`PixelSampler`] over a [`RegularGrid`].
#[derive(Clone, Copy)]
pub struct RegularSampler<'a> {
    grid: &'a RegularGrid,
}

impl<'a> RegularSampler<'a> {
    pub fn new(grid: &'a RegularGrid) -> Self {
        Self { grid }
    }
}

impl PixelSampler for RegularSampler<'_> {
    fn sample(&self, z: u8, gx: u64, gy: u64) -> f32 {
        self.grid.sample_global_px(z, gx, gy)
    }

    fn sample_row(&self, z: u8, gy: u64, gx0: u64, out: &mut [f32]) {
        let world = world_px(z);
        if gy >= world {
            out.fill(f32::NAN);
            return;
        }
        let lat = lat_of_global_y(z, gy);
        for (offset, slot) in out.iter_mut().enumerate() {
            let wrapped_gx = (gx0 + offset as u64) & (world - 1);
            let lon = ((wrapped_gx as f64 + 0.5) / world as f64) * 360.0 - 180.0;
            *slot = self.grid.sample(lat, lon);
        }
    }
}

/// Grid-dispatched [`PixelSampler`] supporting both O1280 and regular lon/lat fields (`FR-009`).
#[derive(Clone, Copy)]
pub enum FieldSampler<'a> {
    O1280(O1280Sampler<'a>),
    Regular(RegularSampler<'a>),
}

impl PixelSampler for FieldSampler<'_> {
    fn sample(&self, z: u8, gx: u64, gy: u64) -> f32 {
        match self {
            Self::O1280(sampler) => sampler.sample(z, gx, gy),
            Self::Regular(sampler) => sampler.sample(z, gx, gy),
        }
    }

    fn sample_row(&self, z: u8, gy: u64, gx0: u64, out: &mut [f32]) {
        match self {
            Self::O1280(sampler) => sampler.sample_row(z, gy, gx0, out),
            Self::Regular(sampler) => sampler.sample_row(z, gy, gx0, out),
        }
    }
}

/// Render the window `[gx0, gx0 + width) x [gy0, gy0 + height)` as straight RGBA8.
#[allow(clippy::too_many_arguments)]
pub fn render_window(
    sampler: &dyn PixelSampler,
    lut: &ColorLut,
    min: f64,
    max: f64,
    z: u8,
    gx0: u64,
    gy0: u64,
    width: u32,
    height: u32,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(width as usize * height as usize * 4);
    let mut row = vec![0f32; width as usize];
    for v in 0..u64::from(height) {
        sampler.sample_row(z, gy0 + v, gx0, &mut row);
        for value in &row {
            out.extend_from_slice(&lut.color(*value, min, max));
        }
    }
    out
}

/// Render tile `(z, x, y)` as `256 x 256` straight RGBA8; the flag is true when every pixel
/// is transparent. `x` wraps modulo `2^z`.
pub fn render_tile(
    sampler: &dyn PixelSampler,
    lut: &ColorLut,
    min: f64,
    max: f64,
    z: u8,
    x: u32,
    y: u32,
) -> (Vec<u8>, bool) {
    let size = u64::from(TILE_SIZE);
    let gx0 = (u64::from(x) * size) & (world_px(z) - 1);
    let pixels = render_window(
        sampler,
        lut,
        min,
        max,
        z,
        gx0,
        u64::from(y) * size,
        TILE_SIZE,
        TILE_SIZE,
    );
    let empty = pixels.as_chunks::<4>().0.iter().all(|px| px[3] == 0);
    (pixels, empty)
}
