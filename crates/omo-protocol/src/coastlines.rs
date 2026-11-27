//! `OCB1` offline coastline / admin-0 boundary asset reader (`FR-014`, contract
//! `layer-stack-and-pmtiles.md` §3).
//!
//! The reader lives in `omo-protocol` next to [`crate::mercator`] because both the UI host
//! (interactive `canvas` painter) and the engine (composite export) draw from the same bytes
//! and the app cannot link the engine. It is pure, allocation-light and `#![forbid(unsafe)]`.
//!
//! Layout (little-endian): 16-byte header (`OCB1`, `version u16`, `flags u16`,
//! `line_count u32`, `vertex_count u32`), `line_count x 32 B` index records, then
//! `(vertex_count - line_count)` `(i16, i16)` deltas in `1e-4` degrees.

use std::sync::OnceLock;

use thiserror::Error;

pub const MAGIC: &[u8; 4] = b"OCB1";
const HEADER_LEN: usize = 16;
const RECORD_LEN: usize = 32;
const E4: f64 = 1e-4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FeatureKind {
    Coastline,
    Admin0Border,
}

impl FeatureKind {
    fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0 => Some(Self::Coastline),
            1 => Some(Self::Admin0Border),
            _ => None,
        }
    }
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum Ocb1Error {
    #[error("not an OCB1 asset (bad magic)")]
    BadMagic,
    #[error("unsupported OCB1 version {0}")]
    UnsupportedVersion(u16),
    #[error("OCB1 asset is truncated or inconsistent: {0}")]
    Corrupt(&'static str),
}

/// A parsed index record.
#[derive(Clone, Copy, Debug)]
struct LineRecord {
    /// `[min_lon, min_lat, max_lon, max_lat]` in `1e-4` degrees.
    bbox_e4: [i32; 4],
    first_e4: [i32; 2],
    delta_offset: u32,
    vertex_len: u16,
    kind: FeatureKind,
}

/// One drawable segment in degrees: `[lon0, lat0, lon1, lat1]`. Longitudes may be shifted by
/// multiples of 360 to follow the requested (unwrapped) view window.
pub type Segment = [f64; 4];

/// Parsed `OCB1` asset borrowing nothing: the index is decoded, the deltas stay packed.
#[derive(Clone, Debug)]
pub struct Ocb1 {
    lines: Vec<LineRecord>,
    deltas: Vec<u8>,
    vertex_count: u32,
}

impl Ocb1 {
    pub fn parse(bytes: &[u8]) -> Result<Self, Ocb1Error> {
        if bytes.len() < HEADER_LEN || &bytes[..4] != MAGIC {
            return Err(Ocb1Error::BadMagic);
        }
        let version = u16::from_le_bytes([bytes[4], bytes[5]]);
        if version != 1 {
            return Err(Ocb1Error::UnsupportedVersion(version));
        }
        let line_count = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
        let vertex_count = u32::from_le_bytes(bytes[12..16].try_into().unwrap());
        let index_end = HEADER_LEN
            .checked_add(
                line_count
                    .checked_mul(RECORD_LEN)
                    .ok_or(Ocb1Error::Corrupt("line count overflow"))?,
            )
            .ok_or(Ocb1Error::Corrupt("line count overflow"))?;
        let delta_entries = (vertex_count as usize)
            .checked_sub(line_count)
            .ok_or(Ocb1Error::Corrupt("fewer vertices than lines"))?;
        if bytes.len() != index_end + delta_entries * 4 {
            return Err(Ocb1Error::Corrupt("size does not match header"));
        }
        let mut lines = Vec::with_capacity(line_count);
        let mut vertices_seen = 0u64;
        for record in bytes[HEADER_LEN..index_end].as_chunks::<RECORD_LEN>().0 {
            let i32_at = |at: usize| i32::from_le_bytes(record[at..at + 4].try_into().unwrap());
            let delta_offset = u32::from_le_bytes(record[24..28].try_into().unwrap());
            let vertex_len = u16::from_le_bytes([record[28], record[29]]);
            let kind = FeatureKind::from_byte(record[30])
                .ok_or(Ocb1Error::Corrupt("unknown feature kind"))?;
            if vertex_len < 2
                || u64::from(delta_offset) + u64::from(vertex_len) - 1 > delta_entries as u64
            {
                return Err(Ocb1Error::Corrupt("line outside the delta table"));
            }
            vertices_seen += u64::from(vertex_len);
            lines.push(LineRecord {
                bbox_e4: [i32_at(0), i32_at(4), i32_at(8), i32_at(12)],
                first_e4: [i32_at(16), i32_at(20)],
                delta_offset,
                vertex_len,
                kind,
            });
        }
        if vertices_seen != u64::from(vertex_count) {
            return Err(Ocb1Error::Corrupt("vertex count mismatch"));
        }
        Ok(Self {
            lines,
            deltas: bytes[index_end..].to_vec(),
            vertex_count,
        })
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    pub fn vertex_count(&self) -> u32 {
        self.vertex_count
    }

    /// Decode the vertices of line `index` as `(lon, lat)` degrees.
    pub fn vertices(&self, index: usize) -> Vec<(f64, f64)> {
        let line = &self.lines[index];
        let mut lon = line.first_e4[0];
        let mut lat = line.first_e4[1];
        let mut out = Vec::with_capacity(line.vertex_len as usize);
        out.push((f64::from(lon) * E4, f64::from(lat) * E4));
        let base = line.delta_offset as usize * 4;
        for step in 0..(line.vertex_len as usize - 1) {
            let at = base + step * 4;
            lon += i32::from(i16::from_le_bytes([self.deltas[at], self.deltas[at + 1]]));
            lat += i32::from(i16::from_le_bytes([
                self.deltas[at + 2],
                self.deltas[at + 3],
            ]));
            out.push((f64::from(lon) * E4, f64::from(lat) * E4));
        }
        out
    }

    pub fn kind(&self, index: usize) -> FeatureKind {
        self.lines[index].kind
    }

    /// `[min_lon, min_lat, max_lon, max_lat]` of line `index`, in degrees.
    pub fn line_bbox(&self, index: usize) -> [f64; 4] {
        let b = self.lines[index].bbox_e4;
        [
            f64::from(b[0]) * E4,
            f64::from(b[1]) * E4,
            f64::from(b[2]) * E4,
            f64::from(b[3]) * E4,
        ]
    }

    /// Indices of lines whose bounding box intersects `view` (`[min_lon, min_lat, max_lon,
    /// max_lat]`, degrees, no longitude wrapping applied).
    pub fn lines_in_bbox(&self, view: [f64; 4]) -> Vec<usize> {
        (0..self.lines.len())
            .filter(|&i| {
                let b = self.line_bbox(i);
                b[0] <= view[2] && b[2] >= view[0] && b[1] <= view[3] && b[3] >= view[1]
            })
            .collect()
    }

    /// Visit every segment of the lines in `view`, clipped to it. `view` is an *unwrapped*
    /// window (longitudes may leave `[-180, 180]`); the asset is replicated at `k * 360`
    /// offsets so views across the antimeridian or wider than the world stay filled. Segments
    /// are reported in the window's coordinate space.
    pub fn visit_segments(&self, view: [f64; 4], mut visit: impl FnMut(FeatureKind, Segment)) {
        let first = ((view[0] + 180.0) / 360.0).floor() as i64;
        let last = ((view[2] + 180.0) / 360.0).floor() as i64;
        for k in first..=last {
            let shift = k as f64 * 360.0;
            let local = [view[0] - shift, view[1], view[2] - shift, view[3]];
            for index in self.lines_in_bbox(local) {
                let kind = self.lines[index].kind;
                let vertices = self.vertices(index);
                for pair in vertices.windows(2) {
                    let (x0, y0) = pair[0];
                    let (x1, y1) = pair[1];
                    if let Some([a, b, c, d]) = clip_segment([x0, y0, x1, y1], local) {
                        visit(kind, [a + shift, b, c + shift, d]);
                    }
                }
            }
        }
    }
}

const INSIDE: u8 = 0;
const LEFT: u8 = 1;
const RIGHT: u8 = 2;
const BOTTOM: u8 = 4;
const TOP: u8 = 8;

fn outcode(x: f64, y: f64, rect: [f64; 4]) -> u8 {
    let mut code = INSIDE;
    if x < rect[0] {
        code |= LEFT;
    } else if x > rect[2] {
        code |= RIGHT;
    }
    if y < rect[1] {
        code |= BOTTOM;
    } else if y > rect[3] {
        code |= TOP;
    }
    code
}

/// Cohen–Sutherland clip of `[x0, y0, x1, y1]` to `rect = [min_x, min_y, max_x, max_y]`.
/// Returns `None` when the segment lies fully outside.
pub fn clip_segment(segment: Segment, rect: [f64; 4]) -> Option<Segment> {
    let [mut x0, mut y0, mut x1, mut y1] = segment;
    let mut code0 = outcode(x0, y0, rect);
    let mut code1 = outcode(x1, y1, rect);
    loop {
        if code0 | code1 == INSIDE {
            return Some([x0, y0, x1, y1]);
        }
        if code0 & code1 != 0 {
            return None;
        }
        let code = if code0 != INSIDE { code0 } else { code1 };
        let (x, y);
        if code & TOP != 0 {
            x = x0 + (x1 - x0) * (rect[3] - y0) / (y1 - y0);
            y = rect[3];
        } else if code & BOTTOM != 0 {
            x = x0 + (x1 - x0) * (rect[1] - y0) / (y1 - y0);
            y = rect[1];
        } else if code & RIGHT != 0 {
            y = y0 + (y1 - y0) * (rect[2] - x0) / (x1 - x0);
            x = rect[2];
        } else {
            y = y0 + (y1 - y0) * (rect[0] - x0) / (x1 - x0);
            x = rect[0];
        }
        if code == code0 {
            x0 = x;
            y0 = y;
            code0 = outcode(x0, y0, rect);
        } else {
            x1 = x;
            y1 = y;
            code1 = outcode(x1, y1, rect);
        }
    }
}

/// Polylines in some pixel space: `(coastlines, borders)`.
pub type Polylines = (Vec<Vec<(f32, f32)>>, Vec<Vec<(f32, f32)>>);

/// Polylines of `asset` inside the unwrapped lon/lat window `view`, projected by `project`
/// (unwrapped longitude and latitude to pixels). A new polyline starts wherever one segment does
/// not continue the previous one; points closer than one pixel to the previous one are dropped,
/// so a world view stays cheap. Shared by the Map view and the composite PNG export so both draw
/// the same lines.
#[derive(Clone, Copy, Default)]
struct PolylineState {
    last_end: Option<(f64, f64)>,
    cursor: Option<(f64, f64)>,
}

pub fn polylines(
    asset: &Ocb1,
    view: [f64; 4],
    project: impl Fn(f64, f64) -> (f64, f64),
) -> Polylines {
    let mut coast: Vec<Vec<(f32, f32)>> = Vec::new();
    let mut border: Vec<Vec<(f32, f32)>> = Vec::new();
    let mut coast_state = PolylineState::default();
    let mut border_state = PolylineState::default();
    asset.visit_segments(view, |kind, seg| {
        let start = (seg[0], seg[1]);
        let end = (seg[2], seg[3]);
        let (target, state) = match kind {
            FeatureKind::Coastline => (&mut coast, &mut coast_state),
            FeatureKind::Admin0Border => (&mut border, &mut border_state),
        };
        if state.last_end != Some(start) || target.is_empty() {
            let (x, y) = project(start.0, start.1);
            target.push(vec![(x as f32, y as f32)]);
            state.cursor = Some((x, y));
        }
        let (x, y) = project(end.0, end.1);
        if let (Some((cx, cy)), Some(line)) = (state.cursor, target.last_mut())
            && ((x - cx).abs() >= 1.0 || (y - cy).abs() >= 1.0)
        {
            line.push((x as f32, y as f32));
            state.cursor = Some((x, y));
        }
        state.last_end = Some(end);
    });
    coast.retain(|line| line.len() >= 2);
    border.retain(|line| line.len() >= 2);
    (coast, border)
}

/// The built-in Natural Earth `1:50m` coastline + admin-0 asset, embedded at compile time so
/// the map works offline and independent of the working directory.
pub fn builtin() -> &'static Ocb1 {
    static ASSET: OnceLock<Ocb1> = OnceLock::new();
    ASSET.get_or_init(|| {
        Ocb1::parse(include_bytes!(
            "../../../fixtures/geodata/ne_50m_coast_borders.ocb1"
        ))
        .expect("the embedded OCB1 asset is valid")
    })
}
