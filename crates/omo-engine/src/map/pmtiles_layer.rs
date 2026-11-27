//! Local `.pmtiles` v3 layer reader and rasterizer (`FR-015`, `FR-016`, contract
//! `layer-stack-and-pmtiles.md` §4).
//!
//! A synchronous, memory-mapped reader written against the PMTiles v3 spec: 127-byte header,
//! varint directories (gzip / brotli / uncompressed), Hilbert tile ids. Tile payloads decode in
//! pure Rust (`flate2`, `brotli`, `ruzstd`). Raster archives (`png`, `jpeg`, `webp`) become
//! straight RGBA8 `256 x 256` tiles; vector archives (`mvt`) are rasterized by a deterministic
//! built-in geometry theme with `tiny-skia`. Point labels, style JSON, fonts and sprites are
//! intentionally ignored in M2.
//!
//! Failures are per archive or per tile ([`PmtilesError`]); the caller isolates them as layer
//! errors so healthy layers keep rendering.

use std::{fs::File, io::Read, path::Path};

use memmap2::Mmap;
use omo_protocol::{
    map::{PmtilesLayerInfo, PmtilesTileKind, ValueProvenance},
    mercator::{MAX_LATITUDE, TILE_SIZE},
};
use sha2::{Digest, Sha256};
use thiserror::Error;

const HEADER_LEN: usize = 127;
/// Directory / tile decompression ceiling, so a hostile archive cannot exhaust memory.
const MAX_DECODED_BYTES: u64 = 128 * 1024 * 1024;
const MAX_DIRECTORY_DEPTH: usize = 4;
/// Archives larger than this report `file_size` only (no whole-file SHA-256).
const FINGERPRINT_LIMIT: u64 = 256 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum PmtilesError {
    #[error("cannot read the archive: {0}")]
    Io(#[from] std::io::Error),
    #[error("not a PMTiles archive")]
    NotPmtiles,
    #[error("unsupported PMTiles version {0} (only v3 is supported)")]
    UnsupportedVersion(u8),
    #[error("unsupported archive feature: {0}")]
    Unsupported(String),
    #[error("corrupt archive: {0}")]
    Corrupt(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Compression {
    None,
    Gzip,
    Brotli,
    Zstd,
}

impl Compression {
    fn from_byte(byte: u8) -> Result<Self, PmtilesError> {
        match byte {
            // 0 = unknown: treated as uncompressed, as most writers use it for "none".
            0 | 1 => Ok(Self::None),
            2 => Ok(Self::Gzip),
            3 => Ok(Self::Brotli),
            4 => Ok(Self::Zstd),
            other => Err(PmtilesError::Unsupported(format!(
                "compression code {other}"
            ))),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Entry {
    tile_id: u64,
    offset: u64,
    length: u64,
    run_length: u64,
}

#[derive(Debug)]
struct Header {
    root_offset: u64,
    root_length: u64,
    metadata_offset: u64,
    metadata_length: u64,
    leaf_offset: u64,
    leaf_length: u64,
    data_offset: u64,
    data_length: u64,
    internal_compression: Compression,
    tile_compression: Compression,
    tile_kind: PmtilesTileKind,
    min_zoom: u8,
    max_zoom: u8,
    /// `[min_lon, min_lat, max_lon, max_lat]` in degrees.
    bounds: [f64; 4],
}

/// An opened archive.
#[derive(Debug)]
pub struct PmtilesLayer {
    path: String,
    mmap: Mmap,
    header: Header,
    root: Vec<Entry>,
    metadata: serde_json::Value,
}

/// PMTiles v3 Hilbert tile id of `z/x/y`.
pub fn tile_id(z: u8, x: u32, y: u32) -> u64 {
    let base: u64 = (0..z).map(|t| 1u64 << (2 * u32::from(t))).sum();
    let n = 1u64 << z;
    let (mut tx, mut ty) = (u64::from(x), u64::from(y));
    let mut d = 0u64;
    let mut s = n / 2;
    while s > 0 {
        let rx = u64::from(tx & s > 0);
        let ry = u64::from(ty & s > 0);
        d += s * s * ((3 * rx) ^ ry);
        // Rotate the quadrant (Hilbert curve, `n` is the full world size).
        if ry == 0 {
            if rx == 1 {
                tx = n - 1 - tx;
                ty = n - 1 - ty;
            }
            std::mem::swap(&mut tx, &mut ty);
        }
        s /= 2;
    }
    base + d
}

fn decompress(kind: Compression, data: &[u8]) -> Result<Vec<u8>, PmtilesError> {
    let mut out = Vec::new();
    let corrupt = |what: &str, e: &dyn std::fmt::Display| {
        PmtilesError::Corrupt(format!("{what} decompression failed: {e}"))
    };
    match kind {
        Compression::None => return Ok(data.to_vec()),
        Compression::Gzip => {
            flate2::read::GzDecoder::new(data)
                .take(MAX_DECODED_BYTES)
                .read_to_end(&mut out)
                .map_err(|e| corrupt("gzip", &e))?;
        }
        Compression::Brotli => {
            brotli::Decompressor::new(data, 4096)
                .take(MAX_DECODED_BYTES)
                .read_to_end(&mut out)
                .map_err(|e| corrupt("brotli", &e))?;
        }
        Compression::Zstd => {
            let decoder =
                ruzstd::decoding::StreamingDecoder::new(data).map_err(|e| corrupt("zstd", &e))?;
            decoder
                .take(MAX_DECODED_BYTES)
                .read_to_end(&mut out)
                .map_err(|e| corrupt("zstd", &e))?;
        }
    }
    Ok(out)
}

struct Cursor<'a> {
    data: &'a [u8],
    at: usize,
}

impl Cursor<'_> {
    fn varint(&mut self) -> Result<u64, PmtilesError> {
        let mut value = 0u64;
        for shift in (0..64).step_by(7) {
            let byte = *self
                .data
                .get(self.at)
                .ok_or_else(|| PmtilesError::Corrupt("directory ended inside a varint".into()))?;
            self.at += 1;
            value |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err(PmtilesError::Corrupt("varint is too long".into()))
    }
}

fn parse_directory(data: &[u8]) -> Result<Vec<Entry>, PmtilesError> {
    let mut cur = Cursor { data, at: 0 };
    let count = cur.varint()? as usize;
    if count > data.len() {
        return Err(PmtilesError::Corrupt(
            "directory entry count is absurd".into(),
        ));
    }
    let mut entries = vec![
        Entry {
            tile_id: 0,
            offset: 0,
            length: 0,
            run_length: 0
        };
        count
    ];
    let mut last = 0u64;
    for entry in &mut entries {
        last = last
            .checked_add(cur.varint()?)
            .ok_or_else(|| PmtilesError::Corrupt("tile id overflow".into()))?;
        entry.tile_id = last;
    }
    for entry in &mut entries {
        entry.run_length = cur.varint()?;
    }
    for entry in &mut entries {
        entry.length = cur.varint()?;
    }
    for i in 0..count {
        let raw = cur.varint()?;
        entries[i].offset = if raw == 0 && i > 0 {
            entries[i - 1]
                .offset
                .checked_add(entries[i - 1].length)
                .ok_or_else(|| PmtilesError::Corrupt("tile offset overflow".into()))?
        } else {
            raw.saturating_sub(1)
        };
    }
    Ok(entries)
}

fn find_entry(entries: &[Entry], id: u64) -> Option<Entry> {
    let index = entries.partition_point(|e| e.tile_id <= id);
    let candidate = *entries.get(index.checked_sub(1)?)?;
    if candidate.run_length == 0 {
        // Leaf directory pointer: covers every id from its tile id onward.
        return Some(candidate);
    }
    (id < candidate.tile_id + candidate.run_length).then_some(candidate)
}

fn u64_at(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap())
}

fn i32_at(bytes: &[u8], at: usize) -> i32 {
    i32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

fn parse_header(bytes: &[u8]) -> Result<Header, PmtilesError> {
    if bytes.len() < 7 || &bytes[..7] != b"PMTiles" {
        return Err(PmtilesError::NotPmtiles);
    }
    if bytes.len() < HEADER_LEN {
        return Err(PmtilesError::Corrupt("header is truncated".into()));
    }
    if bytes[7] != 3 {
        return Err(PmtilesError::UnsupportedVersion(bytes[7]));
    }
    let tile_kind = match bytes[99] {
        1 => PmtilesTileKind::VectorMvt,
        2 => PmtilesTileKind::RasterPng,
        3 => PmtilesTileKind::RasterJpeg,
        4 => PmtilesTileKind::RasterWebp,
        other => {
            return Err(PmtilesError::Unsupported(format!(
                "tile type code {other} (supported: mvt, png, jpeg, webp)"
            )));
        }
    };
    let internal_compression = Compression::from_byte(bytes[97])?;
    if internal_compression == Compression::Zstd {
        return Err(PmtilesError::Unsupported(
            "Zstd internal directory compression is not supported".into(),
        ));
    }
    let e7 = |at| f64::from(i32_at(bytes, at)) / 1e7;
    Ok(Header {
        root_offset: u64_at(bytes, 8),
        root_length: u64_at(bytes, 16),
        metadata_offset: u64_at(bytes, 24),
        metadata_length: u64_at(bytes, 32),
        leaf_offset: u64_at(bytes, 40),
        leaf_length: u64_at(bytes, 48),
        data_offset: u64_at(bytes, 56),
        data_length: u64_at(bytes, 64),
        internal_compression,
        tile_compression: Compression::from_byte(bytes[98])?,
        tile_kind,
        min_zoom: bytes[100],
        max_zoom: bytes[101],
        bounds: [e7(102), e7(106), e7(110), e7(114)],
    })
}

fn slice<'a>(
    mmap: &'a [u8],
    offset: u64,
    length: u64,
    what: &str,
) -> Result<&'a [u8], PmtilesError> {
    let end = offset
        .checked_add(length)
        .ok_or_else(|| PmtilesError::Corrupt(format!("{what} range overflows")))?;
    if end > mmap.len() as u64 {
        return Err(PmtilesError::Corrupt(format!(
            "{what} extends beyond the end of the file"
        )));
    }
    Ok(&mmap[offset as usize..end as usize])
}

impl PmtilesLayer {
    pub fn open(path: &Path) -> Result<Self, PmtilesError> {
        let file = File::open(path)?;
        // SAFETY: the map is read-only; a concurrently truncated file can only fault the
        // render of this layer, which the worker already treats as a recoverable error path
        // (the worker owns the process and restarts on SIGBUS).
        let mmap = unsafe { Mmap::map(&file)? };
        let header = parse_header(&mmap)?;
        let root_raw = slice(
            &mmap,
            header.root_offset,
            header.root_length,
            "root directory",
        )?;
        let root = parse_directory(&decompress(header.internal_compression, root_raw)?)?;
        let metadata = if header.metadata_length == 0 {
            serde_json::Value::Null
        } else {
            let raw = slice(
                &mmap,
                header.metadata_offset,
                header.metadata_length,
                "metadata",
            )?;
            serde_json::from_slice(&decompress(header.internal_compression, raw)?)
                .unwrap_or(serde_json::Value::Null)
        };
        Ok(Self {
            path: path.display().to_string(),
            mmap,
            header,
            root,
            metadata,
        })
    }

    /// Layer description for `map_layer_probe`. `with_fingerprint` hashes the whole file when
    /// it is at most 256 MiB.
    pub fn info(&self, with_fingerprint: bool) -> PmtilesLayerInfo {
        let file_size = self.mmap.len() as u64;
        let text = |key: &str| {
            self.metadata
                .get(key)
                .and_then(|v| v.as_str())
                .map(str::to_owned)
        };
        PmtilesLayerInfo {
            path: self.path.clone(),
            file_size,
            fingerprint_sha256: (with_fingerprint && file_size <= FINGERPRINT_LIMIT)
                .then(|| hex::encode(Sha256::digest(&self.mmap[..]))),
            tile_kind: self.header.tile_kind,
            min_zoom: self.header.min_zoom,
            max_zoom: self.header.max_zoom,
            bounds: self.header.bounds,
            attribution: text("attribution"),
            name: text("name"),
            value_provenance: ValueProvenance::DisplayOnly,
        }
    }

    /// Decode the first addressable tile as a cheap integrity probe (used by
    /// `map_layer_probe`, so an archive whose payloads cannot decode is rejected up front).
    pub fn smoke_check(&self) -> Result<(), PmtilesError> {
        let Some(entry) = self.root.iter().find(|e| e.run_length > 0) else {
            return Ok(()); // Only leaf pointers or no tiles: nothing cheap to decode.
        };
        let Some(payload) = self.lookup(entry.tile_id)? else {
            return Ok(());
        };
        let body = decompress(self.header.tile_compression, payload)?;
        let z = self.header.max_zoom;
        match self.header.tile_kind {
            PmtilesTileKind::VectorMvt => render_mvt(&body, z, 0, 0, 0).map(|_| ()),
            kind => decode_raster(&body, kind).map(|_| ()),
        }
        .map_err(|m| PmtilesError::Corrupt(format!("first tile does not decode: {m}")))
    }

    pub fn tile_kind(&self) -> PmtilesTileKind {
        self.header.tile_kind
    }

    fn lookup(&self, id: u64) -> Result<Option<&[u8]>, PmtilesError> {
        let mut entries = std::borrow::Cow::Borrowed(&self.root[..]);
        for _ in 0..MAX_DIRECTORY_DEPTH {
            let Some(entry) = find_entry(&entries, id) else {
                return Ok(None);
            };
            if entry.run_length > 0 {
                let raw = slice(
                    &self.mmap,
                    self.header
                        .data_offset
                        .checked_add(entry.offset)
                        .ok_or_else(|| PmtilesError::Corrupt("tile offset overflows".into()))?,
                    entry.length,
                    "tile data",
                )?;
                if entry.offset.saturating_add(entry.length) > self.header.data_length {
                    return Err(PmtilesError::Corrupt(
                        "tile data extends beyond the data section".into(),
                    ));
                }
                return Ok(Some(raw));
            }
            if entry.offset.saturating_add(entry.length) > self.header.leaf_length {
                return Err(PmtilesError::Corrupt(
                    "leaf directory extends beyond the leaf section".into(),
                ));
            }
            let raw = slice(
                &self.mmap,
                self.header.leaf_offset + entry.offset,
                entry.length,
                "leaf directory",
            )?;
            entries = std::borrow::Cow::Owned(parse_directory(&decompress(
                self.header.internal_compression,
                raw,
            )?)?);
        }
        Err(PmtilesError::Corrupt(
            "directory nesting is too deep".into(),
        ))
    }

    /// Straight RGBA8 `256 x 256` pixels of `z/x/y`, or `None` when the tile is transparent
    /// (below `minzoom`, outside the archive bounds, or absent). Zooms above `maxzoom`
    /// overzoom the `maxzoom` ancestor.
    pub fn tile_rgba(&self, z: u8, x: u32, y: u32) -> Result<Option<Vec<u8>>, PmtilesError> {
        let h = &self.header;
        if z < h.min_zoom || !tile_intersects_bounds(z, x, y, h.bounds) {
            return Ok(None);
        }
        let (src_z, dz) = if z > h.max_zoom {
            (h.max_zoom, z - h.max_zoom)
        } else {
            (z, 0)
        };
        let (sx, sy) = (x >> dz, y >> dz);
        let Some(payload) = self.lookup(tile_id(src_z, sx, sy))? else {
            return Ok(None);
        };
        let body = decompress(h.tile_compression, payload).map_err(|e| match e {
            PmtilesError::Corrupt(m) => {
                PmtilesError::Corrupt(format!("tile {src_z}/{sx}/{sy}: {m}"))
            }
            other => other,
        })?;
        let (rel_x, rel_y) = (
            u64::from(x & ((1u32 << dz) - 1)),
            u64::from(y & ((1u32 << dz) - 1)),
        );
        let context = |m: String| PmtilesError::Corrupt(format!("tile {src_z}/{sx}/{sy}: {m}"));
        match h.tile_kind {
            PmtilesTileKind::VectorMvt => render_mvt(&body, src_z, dz, rel_x, rel_y)
                .map(Some)
                .map_err(context),
            kind => {
                let tile = decode_raster(&body, kind).map_err(context)?;
                Ok(Some(if dz == 0 {
                    tile
                } else {
                    overzoom_raster(&tile, dz, rel_x, rel_y)
                }))
            }
        }
    }
}

fn tile_intersects_bounds(z: u8, x: u32, y: u32, bounds: [f64; 4]) -> bool {
    let n = f64::from(1u32 << z);
    let lon_w = f64::from(x) / n * 360.0 - 180.0;
    let lon_e = f64::from(x + 1) / n * 360.0 - 180.0;
    let lat_of = |ty: f64| {
        (std::f64::consts::PI * (1.0 - 2.0 * ty / n))
            .sinh()
            .atan()
            .to_degrees()
    };
    let (lat_n, lat_s) = (lat_of(f64::from(y)), lat_of(f64::from(y + 1)));
    let eps = 1e-9;
    lon_w < bounds[2] + eps
        && lon_e > bounds[0] - eps
        && lat_s < bounds[3] + eps
        && lat_n > bounds[1] - eps
        && lat_n.abs() <= MAX_LATITUDE + 1.0
}

fn decode_raster(body: &[u8], kind: PmtilesTileKind) -> Result<Vec<u8>, String> {
    let format = match kind {
        PmtilesTileKind::RasterPng => image::ImageFormat::Png,
        PmtilesTileKind::RasterJpeg => image::ImageFormat::Jpeg,
        PmtilesTileKind::RasterWebp => image::ImageFormat::WebP,
        PmtilesTileKind::VectorMvt => return Err("not a raster archive".into()),
    };
    let decoded = image::load_from_memory_with_format(body, format)
        .map_err(|e| format!("cannot decode the image: {e}"))?
        .to_rgba8();
    let image = if decoded.width() == TILE_SIZE && decoded.height() == TILE_SIZE {
        decoded
    } else {
        if decoded.width() == 0 || decoded.height() == 0 {
            return Err("empty image".into());
        }
        image::imageops::resize(
            &decoded,
            TILE_SIZE,
            TILE_SIZE,
            image::imageops::FilterType::Triangle,
        )
    };
    Ok(image.into_raw())
}

/// Nearest-neighbour crop of the `(rel_x, rel_y)` sub-tile at `dz` levels deeper.
fn overzoom_raster(ancestor: &[u8], dz: u8, rel_x: u64, rel_y: u64) -> Vec<u8> {
    let size = u64::from(TILE_SIZE);
    let mut out = vec![0u8; (TILE_SIZE * TILE_SIZE * 4) as usize];
    for v in 0..size {
        let sv = ((rel_y * size + v) >> dz) as usize;
        for u in 0..size {
            let su = ((rel_x * size + u) >> dz) as usize;
            let from = (sv * size as usize + su) * 4;
            let to = ((v * size + u) * 4) as usize;
            out[to..to + 4].copy_from_slice(&ancestor[from..from + 4]);
        }
    }
    out
}

// ---- deterministic MVT theme -------------------------------------------------------------

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum Stratum {
    Earth,
    Landcover,
    Water,
    Building,
    Waterway,
    Road,
    Boundary,
}

fn stratum(layer: &str) -> Option<Stratum> {
    Some(match layer {
        "earth" | "land" => Stratum::Earth,
        "landcover" | "landuse" | "park" | "natural" => Stratum::Landcover,
        "water" | "ocean" => Stratum::Water,
        "building" | "buildings" => Stratum::Building,
        "waterway" => Stratum::Waterway,
        "transportation" | "road" | "roads" => Stratum::Road,
        "boundary" | "admin" | "boundaries" => Stratum::Boundary,
        _ => return None,
    })
}

fn paint(rgba: [u8; 4]) -> tiny_skia::Paint<'static> {
    let mut paint = tiny_skia::Paint::default();
    paint.set_color_rgba8(rgba[0], rgba[1], rgba[2], rgba[3]);
    paint.anti_alias = true;
    paint
}

fn admin_level(
    properties: &Option<std::collections::HashMap<String, mvt_reader::feature::Value>>,
) -> u64 {
    use mvt_reader::feature::Value;
    let Some(map) = properties else { return 2 };
    for key in ["admin_level", "kind_detail", "level"] {
        match map.get(key) {
            Some(Value::UInt(v)) => return *v,
            Some(Value::Int(v) | Value::SInt(v)) => return (*v).max(0) as u64,
            Some(Value::Double(v)) => return v.max(0.0) as u64,
            Some(Value::Float(v)) => return v.max(0.0) as u64,
            Some(Value::String(s)) => {
                if let Ok(v) = s.parse::<u64>() {
                    return v;
                }
            }
            _ => {}
        }
    }
    2
}

fn render_mvt(body: &[u8], z: u8, dz: u8, rel_x: u64, rel_y: u64) -> Result<Vec<u8>, String> {
    use geo_types::Geometry;
    use tiny_skia::{FillRule, PathBuilder, Pixmap, Stroke, StrokeDash, Transform};

    let reader = mvt_reader::Reader::new(body.to_vec()).map_err(|e| format!("invalid MVT: {e}"))?;
    let layers = reader
        .get_layer_metadata()
        .map_err(|e| format!("invalid MVT layers: {e}"))?;
    let mut work: Vec<(Stratum, usize, u32)> = layers
        .iter()
        .filter_map(|l| stratum(&l.name).map(|s| (s, l.layer_index, l.extent.max(1))))
        .collect();
    work.sort_by_key(|(s, index, _)| (*s, *index));
    let mut pixmap = Pixmap::new(TILE_SIZE, TILE_SIZE).ok_or("cannot allocate the tile")?;
    let scale = f64::from(1u32 << dz);
    for (stratum, index, extent) in work {
        if stratum == Stratum::Building && u32::from(z) + u32::from(dz) < 11 {
            continue;
        }
        let features = reader
            .get_features(index)
            .map_err(|e| format!("invalid MVT features: {e}"))?;
        // Tile-extent units -> output pixels (with overzoom shift).
        let k = (f64::from(TILE_SIZE) * scale / f64::from(extent)) as f32;
        let ox = (rel_x as f64 * f64::from(TILE_SIZE)) as f32;
        let oy = (rel_y as f64 * f64::from(TILE_SIZE)) as f32;
        let map = |c: geo_types::Coord<f32>| (c.x * k - ox, c.y * k - oy);
        for feature in &features {
            let is_polygon = matches!(
                feature.geometry,
                Geometry::Polygon(_) | Geometry::MultiPolygon(_)
            );
            let mut builder = PathBuilder::new();
            let mut add_ring = |ring: &geo_types::LineString<f32>, close: bool| {
                let mut points = ring.0.iter().map(|c| map(*c));
                let Some((x0, y0)) = points.next() else {
                    return;
                };
                builder.move_to(x0, y0);
                for (x, y) in points {
                    builder.line_to(x, y);
                }
                if close {
                    builder.close();
                }
            };
            match &feature.geometry {
                Geometry::Polygon(p) => {
                    add_ring(p.exterior(), true);
                    p.interiors().iter().for_each(|r| add_ring(r, true));
                }
                Geometry::MultiPolygon(mp) => {
                    for p in &mp.0 {
                        add_ring(p.exterior(), true);
                        p.interiors().iter().for_each(|r| add_ring(r, true));
                    }
                }
                Geometry::LineString(l) => add_ring(l, false),
                Geometry::MultiLineString(ml) => ml.0.iter().for_each(|l| add_ring(l, false)),
                // Points (labels, POIs) are not drawn in M2.
                _ => continue,
            }
            let Some(path) = builder.finish() else {
                continue;
            };
            let level = f64::from(z) + f64::from(dz);
            match (stratum, is_polygon) {
                (Stratum::Earth, true) => pixmap.fill_path(
                    &path,
                    &paint([236, 232, 222, 255]),
                    FillRule::EvenOdd,
                    Transform::identity(),
                    None,
                ),
                (Stratum::Landcover, true) => pixmap.fill_path(
                    &path,
                    &paint([208, 224, 196, 255]),
                    FillRule::EvenOdd,
                    Transform::identity(),
                    None,
                ),
                (Stratum::Water, true) => pixmap.fill_path(
                    &path,
                    &paint([150, 172, 198, 255]),
                    FillRule::EvenOdd,
                    Transform::identity(),
                    None,
                ),
                (Stratum::Building, true) => pixmap.fill_path(
                    &path,
                    &paint([212, 206, 198, 255]),
                    FillRule::EvenOdd,
                    Transform::identity(),
                    None,
                ),
                (Stratum::Waterway, false) => {
                    let stroke = Stroke {
                        width: 0.8 + 0.1 * level as f32,
                        ..Stroke::default()
                    };
                    pixmap.stroke_path(
                        &path,
                        &paint([132, 158, 190, 255]),
                        &stroke,
                        Transform::identity(),
                        None,
                    );
                }
                (Stratum::Road, false) => {
                    let stroke = Stroke {
                        width: 0.4 + 0.15 * level as f32,
                        ..Stroke::default()
                    };
                    pixmap.stroke_path(
                        &path,
                        &paint([168, 160, 150, 255]),
                        &stroke,
                        Transform::identity(),
                        None,
                    );
                }
                (Stratum::Boundary, false) => {
                    let level_admin = admin_level(&feature.properties);
                    let mut stroke = Stroke {
                        width: if level_admin <= 2 { 1.6 } else { 1.0 },
                        ..Stroke::default()
                    };
                    if level_admin > 2 {
                        stroke.dash = StrokeDash::new(vec![4.0, 3.0], 0.0);
                    }
                    pixmap.stroke_path(
                        &path,
                        &paint([112, 92, 120, 255]),
                        &stroke,
                        Transform::identity(),
                        None,
                    );
                }
                // Polygon outlines of line strata or fills of line strata: not styled.
                _ => {}
            }
        }
    }
    // tiny-skia stores premultiplied pixels; the layer contract is straight RGBA8.
    let mut out = Vec::with_capacity((TILE_SIZE * TILE_SIZE * 4) as usize);
    for pixel in pixmap.pixels() {
        let c = pixel.demultiply();
        out.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }
    Ok(out)
}
