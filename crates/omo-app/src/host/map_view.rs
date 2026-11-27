//! UI-free map viewport math (`FR-001`, `FR-002`, `FR-009`): pan, anchored zoom, visible tile
//! cover and the worker bbox. Re-exported as `map_host::Viewport`.

use std::{cell::Cell, collections::HashMap, hash::Hash};

use omo_project::{
    map::{MapViewState, MapViewport, PALETTES},
    preview_key::MapPreviewKey,
};
use omo_protocol::map::{MapBbox, TileCoord};
use omo_protocol::mercator::{
    MAX_LATITUDE, MAX_ZOOM, MIN_ZOOM, StationFrame, TILE_SIZE, is_in_projection,
    lonlat_to_world_px, normalize_lon, world_px, world_px_to_lonlat,
};
use sha2::{Digest, Sha256};

/// A tile that intersects the viewport, with its top-left corner in viewport pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VisibleTile {
    /// Wrapped tile coordinate (`x` in `0..2^z`).
    pub coord: TileCoord,
    /// Tile column before wrapping; unique per on-screen copy of the tile.
    pub unwrapped_x: i64,
    pub left: f64,
    pub top: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Viewport {
    /// Center as a fraction of the world square (`x` wraps, `y` is clamped to `[0, 1]`).
    cx: f64,
    cy: f64,
    pub zoom: u8,
    pub width: f64,
    pub height: f64,
}

impl Viewport {
    pub fn new(center_lon: f64, center_lat: f64, zoom: u8, width: f64, height: f64) -> Self {
        let zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        let (x, y) = lonlat_to_world_px(zoom, normalize_lon(center_lon), center_lat);
        let size = world_px(zoom) as f64;
        Self {
            cx: x / size,
            cy: (y / size).clamp(0.0, 1.0),
            zoom,
            width: width.max(1.0),
            height: height.max(1.0),
        }
    }

    pub fn from_frame(frame: &StationFrame, width: f64, height: f64) -> Self {
        Self::new(
            frame.center_lon,
            frame.center_lat,
            frame.zoom,
            width,
            height,
        )
    }

    fn size(&self) -> f64 {
        world_px(self.zoom) as f64
    }

    /// World pixel of the viewport's top-left corner (x unwrapped).
    fn origin(&self) -> (f64, f64) {
        let size = self.size();
        (
            self.cx * size - self.width / 2.0,
            self.cy * size - self.height / 2.0,
        )
    }

    /// Viewport pixel of the top-left corner of tile column `unwrapped_x`, row `y` at the current
    /// zoom.
    pub fn tile_screen_pos(&self, unwrapped_x: i64, y: u32) -> (f64, f64) {
        let (ox, oy) = self.origin();
        let tile = f64::from(TILE_SIZE);
        (unwrapped_x as f64 * tile - ox, f64::from(y) * tile - oy)
    }

    pub fn center_lonlat(&self) -> (f64, f64) {
        let size = self.size();
        world_px_to_lonlat(self.zoom, self.cx * size, self.cy * size)
    }

    pub fn resize(&mut self, width: f64, height: f64) {
        self.width = width.max(1.0);
        self.height = height.max(1.0);
    }

    pub fn screen_to_lonlat(&self, sx: f64, sy: f64) -> (f64, f64) {
        let (ox, oy) = self.origin();
        let size = self.size();
        world_px_to_lonlat(self.zoom, ox + sx, (oy + sy).clamp(0.0, size))
    }

    /// Viewport pixel of `(lon, lat)` on the copy of the world nearest the center; `None` when
    /// `lat` is outside the projection.
    pub fn lonlat_to_screen(&self, lon: f64, lat: f64) -> Option<(f64, f64)> {
        if !lon.is_finite() || !is_in_projection(lat) {
            return None;
        }
        let size = self.size();
        let (wx, wy) = lonlat_to_world_px(self.zoom, normalize_lon(lon), lat);
        let mut dx = (wx - self.cx * size).rem_euclid(size);
        if dx >= size / 2.0 {
            dx -= size;
        }
        let dy = wy - self.cy * size;
        Some((self.width / 2.0 + dx, self.height / 2.0 + dy))
    }

    /// Viewport pixel of `(lon, lat)` with `lon` *unwrapped*: no copy of the world is chosen,
    /// so a segment whose ends straddle the antimeridian stays one piece. Pairs with
    /// [`Self::geo_window`].
    pub fn project_unwrapped(&self, lon: f64, lat: f64) -> (f64, f64) {
        let (ox, oy) = self.origin();
        let (wx, wy) = lonlat_to_world_px(self.zoom, lon, lat);
        (wx - ox, wy - oy)
    }

    /// The visible window as `[min_lon, min_lat, max_lon, max_lat]` with *unwrapped* longitudes
    /// (the western edge may be below `-180°` and the eastern one above `180°`); latitudes are
    /// limited to the projection.
    pub fn geo_window(&self) -> [f64; 4] {
        let (ox, oy) = self.origin();
        let size = self.size();
        let lon = |sx: f64| (ox + sx) / size * 360.0 - 180.0;
        let (_, lat_max) = world_px_to_lonlat(self.zoom, 0.0, (oy).clamp(0.0, size));
        let (_, lat_min) = world_px_to_lonlat(self.zoom, 0.0, (oy + self.height).clamp(0.0, size));
        [lon(0.0), lat_min, lon(self.width), lat_max]
    }

    /// Drag by `(dx, dy)` pixels: the map follows the pointer.
    pub fn pan_pixels(&mut self, dx: f64, dy: f64) {
        let size = self.size();
        self.cx = (self.cx - dx / size).rem_euclid(1.0);
        self.cy = (self.cy - dy / size).clamp(0.0, 1.0);
    }

    /// Change zoom by `delta` levels keeping the geography under `(ax, ay)` fixed.
    pub fn zoom_by(&mut self, delta: i32, ax: f64, ay: f64) {
        let target = (i32::from(self.zoom) + delta).clamp(i32::from(MIN_ZOOM), i32::from(MAX_ZOOM));
        let target = target as u8;
        if target == self.zoom {
            return;
        }
        let size = self.size();
        let (ox, oy) = self.origin();
        let fx = (ox + ax) / size;
        let fy = ((oy + ay) / size).clamp(0.0, 1.0);
        self.zoom = target;
        let new_size = self.size();
        self.cx = (fx * new_size - (ax - self.width / 2.0)) / new_size;
        self.cx = self.cx.rem_euclid(1.0);
        self.cy = ((fy * new_size - (ay - self.height / 2.0)) / new_size).clamp(0.0, 1.0);
    }

    pub fn set_zoom(&mut self, zoom: u8) {
        let (cx, cy) = (self.width / 2.0, self.height / 2.0);
        self.zoom_by(i32::from(zoom) - i32::from(self.zoom), cx, cy);
    }

    /// Tiles covering the viewport, in row-major order.
    pub fn visible_tiles(&self) -> Vec<VisibleTile> {
        let tile = f64::from(TILE_SIZE);
        let n = 1i64 << self.zoom;
        let (ox, oy) = self.origin();
        let tx0 = (ox / tile).floor() as i64;
        let tx1 = ((ox + self.width - 1e-9) / tile).floor() as i64;
        let ty0 = ((oy / tile).floor() as i64).max(0);
        let ty1 = (((oy + self.height - 1e-9) / tile).floor() as i64).min(n - 1);
        let mut out = Vec::new();
        for ty in ty0..=ty1 {
            for tx in tx0..=tx1 {
                out.push(VisibleTile {
                    coord: TileCoord {
                        z: self.zoom,
                        x: tx.rem_euclid(n) as u32,
                        y: ty as u32,
                    },
                    unwrapped_x: tx,
                    left: tx as f64 * tile - ox,
                    top: ty as f64 * tile - oy,
                });
            }
        }
        out
    }

    /// Lat/lon box of the viewport grown by `margin_px` on each side.
    pub fn bbox(&self, margin_px: f64) -> MapBbox {
        let size = self.size();
        let (ox, oy) = self.origin();
        let m = margin_px.max(0.0);
        let top = (oy - m).clamp(0.0, size);
        let bottom = (oy + self.height + m).clamp(0.0, size);
        let lat_max = world_px_to_lonlat(self.zoom, 0.0, top).1.min(MAX_LATITUDE);
        let lat_min = world_px_to_lonlat(self.zoom, 0.0, bottom)
            .1
            .max(-MAX_LATITUDE);
        let span = (self.width + 2.0 * m) / size * 360.0;
        let lon_spans = if span >= 360.0 {
            vec![[-180.0, 180.0]]
        } else {
            let west = normalize_lon((ox - m) / size * 360.0 - 180.0);
            let east = west + span;
            if east <= 180.0 {
                vec![[west, east]]
            } else {
                vec![[west, 180.0], [-180.0, east - 360.0]]
            }
        };
        MapBbox {
            lat_min,
            lat_max,
            lon_spans,
        }
    }

    /// [`Viewport::bbox`] clipped around the viewport center to at most `max_lat_deg` of latitude
    /// and `max_lon_deg` of longitude, so a wide view never asks for an unbounded field scan.
    pub fn capped_bbox(&self, margin_px: f64, max_lat_deg: f64, max_lon_deg: f64) -> MapBbox {
        let full = self.bbox(margin_px);
        let (center_lon, center_lat) = self.center_lonlat();
        let lat_min = full.lat_min.max(center_lat - max_lat_deg / 2.0);
        let lat_max = full.lat_max.min(center_lat + max_lat_deg / 2.0);
        let full_span: f64 = full.lon_spans.iter().map(|s| s[1] - s[0]).sum();
        if full_span <= max_lon_deg {
            return MapBbox {
                lat_min,
                lat_max,
                lon_spans: full.lon_spans,
            };
        }
        let west = normalize_lon(center_lon - max_lon_deg / 2.0);
        let east = west + max_lon_deg;
        let lon_spans = if east <= 180.0 {
            vec![[west, east]]
        } else {
            vec![[west, 180.0], [-180.0, east - 360.0]]
        };
        MapBbox {
            lat_min,
            lat_max,
            lon_spans,
        }
    }
}

// ---- persisted-state helpers (`FR-023`) ---------------------------------------------------

/// Whole forecast hours from the run's reference time to `valid`, when `valid` is not earlier.
pub fn lead_hours(reference: &str, valid: &str) -> Option<u32> {
    let reference = omo_protocol::utc::parse_utc_rfc3339(reference)?;
    let valid = omo_protocol::utc::parse_utc_rfc3339(valid)?;
    let delta = valid - reference;
    (delta >= 0).then(|| u32::try_from(delta / 3_600).unwrap_or(u32::MAX))
}

/// The valid time of `valid_times` that is `lead_h` hours after the reference time. Lead `0` means
/// the run's first valid time, so a project saved at "the start" follows any run.
pub fn valid_time_for_lead(reference: &str, valid_times: &[String], lead_h: u32) -> Option<String> {
    if lead_h == 0 {
        return valid_times.first().cloned();
    }
    valid_times
        .iter()
        .find(|valid| lead_hours(reference, valid) == Some(lead_h))
        .cloned()
}

/// Resolution of a persisted or requested `lead_h` against a newly resolved run's `valid_times`,
/// visibly flagging out-of-range or snapped timestamps (`spec.md` Edge Cases).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeadResolution {
    pub valid_time_utc: String,
    pub effective_lead_h: u32,
    pub clamp_notice: Option<String>,
}

pub fn resolve_lead_with_clamp(
    reference: &str,
    valid_times: &[String],
    lead_h: u32,
) -> Option<LeadResolution> {
    let first = valid_times.first()?;
    if lead_h == 0 {
        return Some(LeadResolution {
            valid_time_utc: first.clone(),
            effective_lead_h: 0,
            clamp_notice: None,
        });
    }
    if let Some(exact) = valid_time_for_lead(reference, valid_times, lead_h) {
        return Some(LeadResolution {
            valid_time_utc: exact,
            effective_lead_h: lead_h,
            clamp_notice: None,
        });
    }
    let mut best: Option<(String, u32, u32)> = None;
    let mut max_lead = 0u32;
    for candidate in valid_times {
        let Some(cand_lead) = lead_hours(reference, candidate) else {
            continue;
        };
        max_lead = max_lead.max(cand_lead);
        let diff = cand_lead.abs_diff(lead_h);
        if best.as_ref().is_none_or(|(_, b_lead, b_diff)| {
            diff < *b_diff || (diff == *b_diff && cand_lead < *b_lead)
        }) {
            best = Some((candidate.clone(), cand_lead, diff));
        }
    }
    let (chosen, effective_lead_h, _) = best.unwrap_or_else(|| (first.clone(), 0, lead_h));
    let reason = if lead_h > max_lead {
        format!(
            "Valid time +{lead_h} h exceeds this run's +{max_lead} h horizon; clamped to {chosen} (+{effective_lead_h} h)."
        )
    } else {
        format!(
            "Valid time +{lead_h} h is not a step of this run; snapped to {chosen} (+{effective_lead_h} h)."
        )
    };
    Some(LeadResolution {
        valid_time_utc: chosen,
        effective_lead_h,
        clamp_notice: Some(reason),
    })
}

/// Snapshot identity for `saved → <path>` on the Map view (Constitution Principle IV): any change
/// to `MapPreviewKey`, viewport framing, or station selection invalidates the saved badge.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct MapExportKey {
    pub preview_key: MapPreviewKey,
    pub center_lat_bits: u64,
    pub center_lon_bits: u64,
    pub zoom: u8,
    pub selected_stations_hash: String,
}

impl MapExportKey {
    pub fn new(preview_key: MapPreviewKey, vp: &Viewport, selected_ids: &[String]) -> Self {
        let fix = |v: f64| {
            if v == 0.0 {
                0.0f64.to_bits()
            } else {
                v.to_bits()
            }
        };
        let (lon, lat) = vp.center_lonlat();
        let mut sorted: Vec<&str> = selected_ids.iter().map(String::as_str).collect();
        sorted.sort_unstable();
        sorted.dedup();
        let mut hasher = Sha256::new();
        for id in sorted {
            hasher.update(id.as_bytes());
            hasher.update([0x1f]);
        }
        Self {
            preview_key,
            center_lat_bits: fix(lat),
            center_lon_bits: fix(lon),
            zoom: vp.zoom,
            selected_stations_hash: format!("{:x}", hasher.finalize()),
        }
    }
}

/// Bounded tile cache with true least-recently-used eviction (`FR-004`).
/// Recency stamps use interior mutability (`Cell<u64>`) so reactive UI reads (`&self`) can record
/// tile hits without requiring `&mut self`.
#[derive(Clone, Debug)]
pub struct TileLruCache<K, V> {
    clock: Cell<u64>,
    entries: HashMap<K, (V, Cell<u64>)>,
}

impl<K: Eq + Hash + Clone, V: PartialEq> PartialEq for TileLruCache<K, V> {
    fn eq(&self, other: &Self) -> bool {
        self.entries.len() == other.entries.len()
            && self
                .entries
                .iter()
                .all(|(k, (v, _))| other.entries.get(k).is_some_and(|(ov, _)| v == ov))
    }
}

impl<K: Eq + Hash, V> Default for TileLruCache<K, V> {
    fn default() -> Self {
        Self {
            clock: Cell::new(0),
            entries: HashMap::new(),
        }
    }
}

impl<K: Eq + Hash + Clone, V> TileLruCache<K, V> {
    pub fn new() -> Self {
        Self::default()
    }

    fn next_stamp(&self) -> u64 {
        let next = self.clock.get().wrapping_add(1);
        self.clock.set(next);
        next
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Look up `key` and mark it most-recently-used.
    pub fn get_touch(&self, key: &K) -> Option<&V> {
        let (value, stamp) = self.entries.get(key)?;
        stamp.set(self.next_stamp());
        Some(value)
    }

    /// Look up `key` without updating its recency stamp.
    pub fn get_peek(&self, key: &K) -> Option<&V> {
        self.entries.get(key).map(|(value, _)| value)
    }

    pub fn insert(&mut self, key: K, value: V) {
        let stamp = self.next_stamp();
        self.entries.insert(key, (value, Cell::new(stamp)));
    }

    pub fn retain(&mut self, mut predicate: impl FnMut(&K, &mut V) -> bool) {
        self.entries.retain(|k, (v, _)| predicate(k, v));
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Evict least-recently-used entries down to `target_len` when `len() > max_entries`,
    /// never evicting any key for which `keep(key)` is true.
    pub fn evict_lru(&mut self, max_entries: usize, target_len: usize, keep: impl Fn(&K) -> bool) {
        if self.entries.len() <= max_entries {
            return;
        }
        let target = target_len.min(max_entries);
        let needed = self.entries.len().saturating_sub(target);
        if needed == 0 {
            return;
        }
        let mut candidates: Vec<(u64, K)> = self
            .entries
            .iter()
            .filter(|(k, _)| !keep(k))
            .map(|(k, (_, stamp))| (stamp.get(), k.clone()))
            .collect();
        candidates.sort_by_key(|(stamp, _)| *stamp);
        for (_, key) in candidates.into_iter().take(needed) {
            self.entries.remove(&key);
        }
    }
}

/// Both lock fields as a usable colour range; anything partial, unordered or non-finite is not.
pub fn locked_range(min: &str, max: &str) -> Option<(f64, f64)> {
    let min = min.trim().parse::<f64>().ok()?;
    let max = max.trim().parse::<f64>().ok()?;
    (min.is_finite() && max.is_finite() && min < max).then_some((min, max))
}

/// Combine the layer stack with the live viewport, valid time and colour controls into the
/// canonical document state. Values that could not be saved (unknown palette, …) fall back to
/// their defaults rather than poisoning the file.
pub fn compose_state(
    layers: &MapViewState,
    viewport: Option<MapViewport>,
    lead_h: u32,
    palette: &str,
    locked_min: &str,
    locked_max: &str,
) -> MapViewState {
    let mut state = layers.clone();
    state.viewport = viewport;
    state.valid_time_lead_h = lead_h;
    state.color_scale.palette = if PALETTES.contains(&palette) {
        palette.to_owned()
    } else {
        omo_project::map::DEFAULT_PALETTE.to_owned()
    };
    let range = locked_range(locked_min, locked_max);
    state.color_scale.locked_min = range.map(|r| r.0);
    state.color_scale.locked_max = range.map(|r| r.1);
    state.clone().canonical().unwrap_or(state)
}
