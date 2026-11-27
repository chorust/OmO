//! Interactive Map controller (`FR-001`..`FR-023`): [`MapRequestScheduler`] coordination,
//! [`MapPreviewKey`] staleness binding, snapshot-keyed export state, LRU tile caching, and
//! valid-time horizon clamping.
//!
//! NSH layout/canvas construction lives in [`panel`] so controller logic and view composition
//! evolve independently.

mod panel;

pub(super) use panel::panel;

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};

use nsh_core::{element::ImageSrc, prelude::*};
use omo_project::{
    map::{LayerMove, LayerStackError, MapLayerKind, MapViewState, MapViewport, PALETTES},
    preview_key::MapPreviewKey,
};
use omo_protocol::{
    attribution::{ArchiveAttribution, AttributionInput, attribution_lines},
    map::{
        ColorScale, DEFAULT_PALETTE, EXPORT_MAX_EDGE, EXPORT_MIN_EDGE, ExportColorScale,
        ExportForecast, ExportLayer, ExportLayerKind, ExportStation, ExportViewport,
        ForecastSampleState, InspectRequest, InspectResult, LayerProbeRequest, LayerProbeResult,
        LoadFieldRequest, LoadFieldResult, MapBbox, MapExportRequest, MapRunSelector,
        PmtilesLayerInfo, RenderLayerKind, RenderLayerRequest, RenderTilesRequest,
        RenderTilesResult, RenderedTile, SourceLayout, TileCoord,
    },
    mercator::{TILE_SIZE, frame_stations, is_in_projection},
    messages::parse_fail_kind,
    state::FailKind,
};
use serde_json::Value;

use super::series_panel::SeriesController;
use crate::host::{
    map_host::{
        ExportIntent, FieldIntent, InspectIntent, MapAction, MapCompletion, MapOutcome,
        MapRequestKind, MapRequestScheduler, ProbeIntent, TilesIntent,
    },
    map_view::{
        MapExportKey, TileLruCache, Viewport, compose_state, lead_hours, locked_range,
        resolve_lead_with_clamp,
    },
    runtime::EngineHandle,
};

const JOB_ID: u64 = 1;
/// Map request ids live in their own space so they never collide with `request_count` ids.
const FIRST_MAP_REQUEST_ID: u64 = 1 << 40;
pub(super) const FORECAST_LAYER: &str = "forecast";
const MAX_CACHED_TILES: usize = 640;
const MAX_TILES_PER_BATCH: usize = 96;
/// One tile of gutter around the viewport, and a bound on the scanned area (`contracts/`
/// `worker-protocol-m2.md`: wide views load a bounded window around the center).
const FIELD_GUTTER_PX: f64 = TILE_SIZE as f64;
const FIELD_MAX_LAT_DEG: f64 = 60.0;
const FIELD_MAX_LON_DEG: f64 = 80.0;
const COVERAGE_DEBOUNCE: Duration = Duration::from_millis(450);
const HOVER_MAX_DISTANCE_M: u32 = 25_000;

pub(super) type TileKey = (u8, u32, u32);

/// What the pixels of a rendered forecast tile depend on.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct RenderKey {
    pub field_revision: u64,
    pub palette: String,
    pub min_bits: u64,
    pub max_bits: u64,
}

#[derive(Clone, PartialEq)]
pub(super) struct StoredTile {
    pub id: u64,
    pub key: RenderKey,
    /// `None` for an entirely transparent tile.
    pub image: Option<ImageSrc>,
}

/// A rendered tile of a mounted archive: `key` is the archive path it was rendered from.
#[derive(Clone, PartialEq)]
pub(super) struct StoredLayerTile {
    pub id: u64,
    pub key: String,
    pub image: Option<ImageSrc>,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct LoadedField {
    pub revision: u64,
    pub entry_id: String,
    pub catalog_version: String,
    pub stations_hash: Option<String>,
    pub doc_id: String,
    pub worker_generation: u64,
    pub result: LoadFieldResult,
}

/// What the worker told us about a mounted archive.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct LayerMeta {
    pub info: Option<PmtilesLayerInfo>,
    pub error: Option<String>,
}

#[derive(Clone)]
enum Pending {
    Field {
        entry_id: String,
        catalog_version: String,
        stations_hash: Option<String>,
        doc_id: String,
        worker_generation: u64,
    },
    Probe {
        layer_id: String,
    },
    Tiles {
        forecast: Option<RenderKey>,
        /// Mounted archive layer id to the archive path it was rendered from.
        archives: HashMap<String, String>,
    },
    Inspect,
    Export {
        path: PathBuf,
        key: MapExportKey,
    },
}

#[derive(Clone)]
pub(super) struct MapController {
    engine: EngineHandle,
    pub(super) series: SeriesController,
    worker_ready: Signal<bool>,
    worker_generation: Signal<u64>,
    doc_id: Signal<String>,
    input_revision: Signal<u64>,
    sched: Rc<RefCell<MapRequestScheduler>>,
    pending: Rc<RefCell<HashMap<u64, Pending>>>,
    field_revision: Rc<Cell<u64>>,
    render_counter: Rc<Cell<u64>>,
    tile_ids: Rc<Cell<u64>>,
    coverage_dirty: Rc<Cell<Option<Instant>>>,
    busy_retry: Rc<Cell<Option<Instant>>>,
    field_retry: Rc<Cell<Option<Instant>>>,
    field_retries: Rc<Cell<u32>>,
    autoloaded_for: Rc<RefCell<Option<(String, String)>>>,
    fit_pending: Rc<Cell<bool>>,
    pub(super) drag_last: Rc<Cell<Option<(f32, f32)>>>,
    wheel_accum: Rc<Cell<f32>>,
    pub viewport: Signal<Viewport>,
    laid_out: Signal<bool>,
    pub(super) tiles: Signal<TileLruCache<TileKey, StoredTile>>,
    pub field: Signal<Option<LoadedField>>,
    pub field_stale: Signal<Option<String>>,
    pub settled_key: Signal<Option<MapPreviewKey>>,
    pub requested_valid_time: Signal<Option<String>>,
    pub clamp_notice: Signal<Option<String>>,
    pub loading: Signal<bool>,
    pub status: Signal<String>,
    pub notice: Signal<String>,
    pub palette: Signal<String>,
    pub locked_min: Signal<String>,
    pub locked_max: Signal<String>,
    pub hover: Signal<Option<(f64, f64)>>,
    pub readout: Signal<Option<InspectResult>>,
    /// The layer stack and persisted map settings (`MapViewState`).
    pub state: Signal<MapViewState>,
    pub layer_meta: Signal<HashMap<String, LayerMeta>>,
    pub(super) layer_tiles: Signal<TileLruCache<(String, TileKey), StoredLayerTile>>,
    pub layers_open: Signal<bool>,
    pub path_input: Signal<String>,
    pub layer_note: Signal<String>,
    /// User-chosen viewport (`None` while using automatic station framing).
    nav: Signal<Option<MapViewport>>,
    /// Valid-time lead in hours after the run's reference time (`0` = first step).
    lead_h: Signal<u32>,
    /// Saved lead to apply once a run is resolved.
    lead_target: Rc<Cell<Option<u32>>>,
    /// Snapshot-keyed `(MapExportKey, path)` of the last committed composite PNG.
    pub export_saved: Signal<Option<(MapExportKey, String)>>,
    pub exporting: Signal<bool>,
    /// Manual export path entry when the native save chooser is unavailable.
    pub export_manual_open: Signal<bool>,
    pub export_path_input: Signal<String>,
}

impl MapController {
    pub fn new(
        engine: EngineHandle,
        series: SeriesController,
        worker_ready: Signal<bool>,
        worker_generation: Signal<u64>,
    ) -> Self {
        let controller = Self {
            engine,
            series,
            worker_ready,
            worker_generation,
            doc_id: signal(String::from("default")),
            input_revision: signal(1),
            sched: Rc::new(RefCell::new(MapRequestScheduler::new(FIRST_MAP_REQUEST_ID))),
            pending: Rc::new(RefCell::new(HashMap::new())),
            field_revision: Rc::new(Cell::new(0)),
            render_counter: Rc::new(Cell::new(0)),
            tile_ids: Rc::new(Cell::new(0)),
            coverage_dirty: Rc::new(Cell::new(None)),
            busy_retry: Rc::new(Cell::new(None)),
            field_retry: Rc::new(Cell::new(None)),
            field_retries: Rc::new(Cell::new(0)),
            autoloaded_for: Rc::new(RefCell::new(None)),
            fit_pending: Rc::new(Cell::new(true)),
            drag_last: Rc::new(Cell::new(None)),
            wheel_accum: Rc::new(Cell::new(0.0)),
            viewport: signal(Viewport::new(0.0, 20.0, 2, 900.0, 600.0)),
            laid_out: signal(false),
            tiles: signal(TileLruCache::new()),
            field: signal(None),
            field_stale: signal(None),
            settled_key: signal(None),
            requested_valid_time: signal(None),
            clamp_notice: signal(None),
            loading: signal(false),
            status: signal(String::from("No forecast field loaded.")),
            notice: signal(String::new()),
            palette: signal(DEFAULT_PALETTE.to_owned()),
            locked_min: signal(String::new()),
            locked_max: signal(String::new()),
            hover: signal(None),
            readout: signal(None),
            state: signal(MapViewState::default()),
            layer_meta: signal(HashMap::new()),
            layer_tiles: signal(TileLruCache::new()),
            layers_open: signal(false),
            path_input: signal(String::new()),
            layer_note: signal(String::new()),
            nav: signal(None),
            lead_h: signal(0),
            lead_target: Rc::new(Cell::new(None)),
            export_saved: signal(None),
            exporting: signal(false),
            export_manual_open: signal(false),
            export_path_input: signal(String::new()),
        };

        // Auto-fit and auto-load whenever `(stations_hash, selected_entry)` changes while ready.
        let this = controller.clone();
        effect(move || {
            let ready = this.worker_ready.get();
            let catalog_ready = this.series.catalog.get().is_some();
            let laid_out = this.laid_out.get();
            let entry_id = this.series.selected_entry.get();
            let hash = this
                .series
                .snapshot
                .get()
                .map(|snapshot| snapshot.content_sha256);
            if !(ready && catalog_ready && laid_out && !entry_id.is_empty()) {
                return;
            }
            if let Some(hash) = hash {
                let pair = (hash, entry_id);
                if this.autoloaded_for.borrow().as_ref() == Some(&pair) {
                    return;
                }
                let station_changed = this
                    .autoloaded_for
                    .borrow()
                    .as_ref()
                    .is_none_or(|(prev_hash, _)| prev_hash != &pair.0);
                *this.autoloaded_for.borrow_mut() = Some(pair);
                this.bump_input_revision();
                if station_changed {
                    this.initial_view();
                }
                this.request_field(None);
            } else if this
                .field
                .get_untracked()
                .as_ref()
                .is_some_and(|f| f.entry_id != entry_id)
            {
                // Zero-station catalog switch on an already loaded map: mark stale and reload.
                this.bump_input_revision();
                this.request_field(None);
            }
        });

        // Debounced follow-ups: field coverage after pans, one retry after worker `busy`.
        let this = controller.clone();
        set_interval(Duration::from_millis(150), move || {
            if let Some(since) = this.coverage_dirty.get()
                && since.elapsed() >= COVERAGE_DEBOUNCE
            {
                this.coverage_dirty.set(None);
                this.ensure_coverage();
            }
            if let Some(at) = this.busy_retry.get()
                && Instant::now() >= at
            {
                this.busy_retry.set(None);
                this.reprobe_unprobed();
                this.refresh_tiles();
            }
            if let Some(at) = this.field_retry.get()
                && Instant::now() >= at
            {
                this.field_retry.set(None);
                this.request_field(None);
            }
        });
        controller
    }

    pub fn set_doc_id(&self, doc_id: String) {
        if self.doc_id.get_untracked() != doc_id {
            self.doc_id.set(doc_id);
            self.bump_input_revision();
        }
    }

    pub(super) fn bump_input_revision(&self) {
        self.input_revision
            .update(|rev| *rev = rev.saturating_add(1));
    }

    // ---- preview & export keys (`FR-020`, Principle IV) ---------------------------------

    fn read_sig<T: Clone + 'static>(signal: Signal<T>, tracked: bool) -> T {
        if tracked {
            signal.get()
        } else {
            signal.get_untracked()
        }
    }

    pub fn current_preview_key(&self, tracked: bool) -> MapPreviewKey {
        let doc_id = Self::read_sig(self.doc_id, tracked);
        let revision = Self::read_sig(self.input_revision, tracked);
        let stations_hash = Self::read_sig(self.series.snapshot, tracked).map(|s| s.content_sha256);
        let catalog_entry_id = Self::read_sig(self.series.selected_entry, tracked);
        let catalog_version = Self::read_sig(self.series.catalog, tracked)
            .map(|c| c.version)
            .unwrap_or_default();
        let field = Self::read_sig(self.field, tracked);
        let matching_field = field.as_ref().filter(|f| f.entry_id == catalog_entry_id);
        let run_ref = matching_field
            .map(|f| f.result.run.reference_time_utc.clone())
            .unwrap_or_else(|| "latest_complete".into());
        let valid_time_utc = Self::read_sig(self.requested_valid_time, tracked)
            .or_else(|| matching_field.map(|f| f.result.valid_time_utc.clone()))
            .unwrap_or_default();
        let color_palette = Self::read_sig(self.palette, tracked);
        let min_txt = Self::read_sig(self.locked_min, tracked);
        let max_txt = Self::read_sig(self.locked_max, tracked);
        let color_locked_range = MapPreviewKey::normalized_range(
            min_txt.trim().parse::<f64>().ok(),
            max_txt.trim().parse::<f64>().ok(),
        );
        let layers_hash = MapPreviewKey::layers_digest(&Self::read_sig(self.state, tracked).layers);
        let worker_generation = Self::read_sig(self.worker_generation, tracked);
        MapPreviewKey {
            doc_id,
            revision,
            stations_hash,
            catalog_entry_id,
            catalog_version,
            run_ref,
            valid_time_utc,
            color_palette,
            color_locked_range,
            layers_hash,
            worker_generation,
        }
    }

    pub fn current_export_key(&self, tracked: bool) -> MapExportKey {
        let key = self.current_preview_key(tracked);
        let vp = Self::read_sig(self.viewport, tracked);
        let filter = Self::read_sig(self.series.station_filter, tracked);
        let selected: Vec<String> = filter
            .split(',')
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(str::to_owned)
            .collect();
        MapExportKey::new(key, &vp, &selected)
    }

    /// True when a displayed forecast map is out of date against current inputs or last reload failed.
    pub fn is_map_stale(&self, tracked: bool) -> bool {
        let Some(loaded) = Self::read_sig(self.field, tracked) else {
            return false;
        };
        if Self::read_sig(self.field_stale, tracked).is_some() {
            return true;
        }
        let current = self.current_preview_key(tracked);
        if loaded.entry_id != current.catalog_entry_id
            || loaded.catalog_version != current.catalog_version
            || loaded.stations_hash != current.stations_hash
            || loaded.doc_id != current.doc_id
            || loaded.worker_generation != current.worker_generation
            || (!current.valid_time_utc.is_empty()
                && loaded.result.valid_time_utc != current.valid_time_utc)
        {
            return true;
        }
        match Self::read_sig(self.settled_key, tracked) {
            Some(settled) => settled.is_stale_against(&current),
            None => true,
        }
    }

    /// Path shown in `saved → <path>`, valid only while the exact exported snapshot remains live.
    pub fn active_saved_path(&self) -> Option<String> {
        let (saved_key, path) = self.export_saved.get()?;
        if self.loading.get() || self.is_map_stale(true) {
            return None;
        }
        (self.current_export_key(true) == saved_key).then_some(path)
    }

    fn maybe_mark_settled(&self) {
        if self.field_stale.get_untracked().is_some() {
            return;
        }
        let current = self.current_preview_key(false);
        if let Some(loaded) = self.field.get_untracked()
            && (loaded.entry_id != current.catalog_entry_id
                || loaded.catalog_version != current.catalog_version
                || loaded.stations_hash != current.stations_hash
                || loaded.doc_id != current.doc_id
                || loaded.worker_generation != current.worker_generation
                || (!current.valid_time_utc.is_empty()
                    && loaded.result.valid_time_utc != current.valid_time_utc))
        {
            return;
        }
        self.settled_key.set(Some(current));
    }

    // ---- derived state ------------------------------------------------------------------

    fn stations(&self) -> Vec<(String, f64, f64)> {
        self.series
            .snapshot
            .get_untracked()
            .map(|snapshot| {
                snapshot
                    .rows
                    .iter()
                    .map(|row| (row.station_id.clone(), row.longitude, row.latitude))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Reactive check: true when at least one confirmed station lies inside Web Mercator bounds.
    pub fn has_in_projection_stations(&self) -> bool {
        self.series.snapshot.get().is_some_and(|snapshot| {
            snapshot
                .rows
                .iter()
                .any(|row| is_in_projection(row.latitude) && row.longitude.is_finite())
        })
    }

    /// Toggle one station ID in the shared workbench selection filter (`FR-011`).
    pub fn toggle_station_selection(&self, station_id: &str) {
        let mut ids = self.series.selected_ids();
        if let Some(pos) = ids.iter().position(|id| id == station_id) {
            ids.remove(pos);
        } else {
            ids.push(station_id.to_owned());
        }
        self.series.station_filter.set(ids.join(", "));
    }

    fn color_domain(&self, field: &LoadedField) -> (f64, f64) {
        match locked_range(
            &self.locked_min.get_untracked(),
            &self.locked_max.get_untracked(),
        ) {
            Some((min, max)) => (min, max),
            None => (field.result.stats.min, field.result.stats.max),
        }
    }

    pub(super) fn render_key(&self, field: &LoadedField) -> RenderKey {
        let (min, max) = self.color_domain(field);
        let norm = |v: f64| if v == 0.0 { 0.0f64 } else { v };
        RenderKey {
            field_revision: field.revision,
            palette: self.palette.get_untracked(),
            min_bits: norm(min).to_bits(),
            max_bits: norm(max).to_bits(),
        }
    }

    pub fn color_domain_now(&self) -> Option<(f64, f64)> {
        self.field
            .get_untracked()
            .map(|field| self.color_domain(&field))
    }

    // ---- viewport & gestures ------------------------------------------------------------

    pub fn on_layout(&self, width: f64, height: f64) {
        if width < 16.0 || height < 16.0 {
            return;
        }
        let first = !self.laid_out.get_untracked();
        self.viewport.update(|vp| vp.resize(width, height));
        if first {
            if self.fit_pending.replace(false) {
                self.initial_view();
            }
            self.laid_out.set(true);
        }
        self.viewport_changed();
    }

    pub fn fit_to_stations(&self) {
        if !self
            .stations()
            .iter()
            .any(|(_, lon, lat)| is_in_projection(*lat) && lon.is_finite())
        {
            return;
        }
        self.nav.set(None);
        self.frame_stations_now();
    }

    fn initial_view(&self) {
        match self.nav.get_untracked() {
            Some(saved) => self.restore_viewport(saved),
            None => self.frame_stations_now(),
        }
    }

    fn restore_viewport(&self, saved: MapViewport) {
        let (w, h) = {
            let vp = self.viewport.get_untracked();
            (vp.width, vp.height)
        };
        self.viewport.set(Viewport::new(
            saved.center_lon,
            saved.center_lat,
            saved.zoom,
            w,
            h,
        ));
        self.viewport_changed();
    }

    pub(super) fn user_moved(&self) {
        let vp = self.viewport.get_untracked();
        let (center_lon, center_lat) = vp.center_lonlat();
        self.nav.set(Some(MapViewport {
            center_lat,
            center_lon,
            zoom: vp.zoom,
        }));
    }

    fn frame_stations_now(&self) {
        let stations = self.stations();
        let points: Vec<(f64, f64)> = stations.iter().map(|(_, lon, lat)| (*lon, *lat)).collect();
        let (w, h) = {
            let vp = self.viewport.get_untracked();
            (vp.width, vp.height)
        };
        match frame_stations(&points, w, h) {
            Some(frame) => {
                self.viewport.set(Viewport::from_frame(&frame, w, h));
                self.update_projection_notice(frame.out_of_projection, false);
            }
            None => {
                self.update_projection_notice(0, !points.is_empty());
            }
        }
        self.viewport_changed();
    }

    fn update_projection_notice(&self, polar_count: usize, all_outside: bool) {
        let mut notes = Vec::new();
        if all_outside {
            notes.push(String::from(
                "No station lies inside the map projection (|lat| ≤ 85.05°).",
            ));
        } else if polar_count > 0 {
            notes.push(format!(
                "{polar_count} polar station(s) outside Web Mercator bounds (|lat| > 85.05°)."
            ));
        }
        if let Some(clamp) = self.clamp_notice.get_untracked() {
            notes.push(clamp);
        }
        self.notice.set(notes.join("  ·  "));
    }

    fn refresh_notices(&self) {
        let stations = self.stations();
        let total = stations.len();
        let in_proj = stations
            .iter()
            .filter(|(_, lon, lat)| is_in_projection(*lat) && lon.is_finite())
            .count();
        self.update_projection_notice(total.saturating_sub(in_proj), total > 0 && in_proj == 0);
    }

    pub fn viewport_changed(&self) {
        self.refresh_tiles();
        self.coverage_dirty.set(Some(Instant::now()));
    }

    pub fn zoom_by(&self, delta: i32, anchor: Option<(f64, f64)>) {
        self.viewport.update(|vp| {
            let (ax, ay) = anchor.unwrap_or((vp.width / 2.0, vp.height / 2.0));
            vp.zoom_by(delta, ax, ay);
        });
        self.user_moved();
        self.viewport_changed();
    }

    pub(super) fn on_pan_grant(&self, event: &ResponderEvent) {
        let (dx, dy) = (
            f64::from(event.gesture.delta.x.0),
            f64::from(event.gesture.delta.y.0),
        );
        if dx != 0.0 || dy != 0.0 {
            self.viewport.update(|vp| vp.pan_pixels(dx, dy));
            self.user_moved();
            self.viewport_changed();
        }
        self.drag_last.set(Some((event.page.x.0, event.page.y.0)));
    }

    pub(super) fn on_pan_move(&self, event: &ResponderEvent) {
        let (px, py) = (event.page.x.0, event.page.y.0);
        if let Some((lx, ly)) = self.drag_last.get() {
            let (dx, dy) = (f64::from(px - lx), f64::from(py - ly));
            if dx != 0.0 || dy != 0.0 {
                self.viewport.update(|vp| vp.pan_pixels(dx, dy));
                self.user_moved();
                self.viewport_changed();
            }
        }
        self.drag_last.set(Some((px, py)));
    }

    pub(super) fn on_pan_end(&self) {
        self.drag_last.set(None);
    }

    pub(super) fn on_mouse_move(&self, event: &MouseEvent) {
        if event.pressed_button == Some(MouseButton::Left) || self.drag_last.get().is_some() {
            return;
        }
        let (x, y) = (event.position.x.0, event.position.y.0);
        let (lon, lat) = self
            .viewport
            .get_untracked()
            .screen_to_lonlat(f64::from(x), f64::from(y));
        self.hover.set(Some((lon, lat)));
        if self.forecast_visible() {
            self.request_inspect(lat, lon);
        } else {
            self.readout.set(Some(InspectResult {
                latitude: lat,
                longitude: lon,
                sample: ForecastSampleState::LayerHidden,
            }));
        }
    }

    pub(super) fn on_mouse_leave(&self) {
        // Keep `drag_last` intact so responder-captured drags continue outside map bounds (`FR-003`).
        self.hover.set(None);
    }

    pub(super) fn on_wheel(&self, event: &WheelEvent) {
        let delta = event.delta.points(px(40.0)).y.0;
        let mut accum = self.wheel_accum.get() + delta;
        let mut steps = 0;
        while accum.abs() >= 60.0 {
            steps += if accum > 0.0 { -1 } else { 1 };
            accum -= 60.0f32.copysign(accum);
        }
        self.wheel_accum.set(accum);
        if steps != 0 {
            self.zoom_by(
                steps,
                Some((f64::from(event.position.x.0), f64::from(event.position.y.0))),
            );
        }
    }

    pub(super) fn on_press(&self, event: &PressEvent) {
        if event.clicks >= 2 {
            self.zoom_by(
                1,
                Some((f64::from(event.position.x.0), f64::from(event.position.y.0))),
            );
        }
    }

    // ---- requests & cancellation --------------------------------------------------------

    pub fn cancel_active(&self) -> bool {
        let active = self.sched.borrow_mut().cancel_running();
        let Some((request_id, kind)) = active else {
            return false;
        };
        match self.engine.cancel(JOB_ID, request_id) {
            Ok(()) => {
                self.status
                    .set(format!("Cancelling {} ({request_id})…", kind.wire_kind()));
                true
            }
            Err(error) => {
                self.status.set(format!(
                    "Could not cancel map request {request_id}: {error}"
                ));
                false
            }
        }
    }

    fn pending_for_action(&self, kind: MapRequestKind, body: &Value) -> Option<Pending> {
        match kind {
            MapRequestKind::LoadField => {
                let entry_id = body
                    .pointer("/catalog_entry/id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let catalog_version = body
                    .get("catalog_version")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                Some(Pending::Field {
                    entry_id,
                    catalog_version,
                    stations_hash: self
                        .series
                        .snapshot
                        .get_untracked()
                        .map(|s| s.content_sha256),
                    doc_id: self.doc_id.get_untracked(),
                    worker_generation: self.worker_generation.get_untracked(),
                })
            }
            MapRequestKind::Inspect => Some(Pending::Inspect),
            MapRequestKind::LayerProbe => {
                let layer_id = body.get("layer_id").and_then(Value::as_str)?.to_owned();
                Some(Pending::Probe { layer_id })
            }
            MapRequestKind::Export => {
                let path = body
                    .get("target")
                    .and_then(Value::as_str)
                    .map(PathBuf::from)?;
                Some(Pending::Export {
                    path,
                    key: self.current_export_key(false),
                })
            }
            MapRequestKind::RenderTiles => {
                let req: RenderTilesRequest = serde_json::from_value(body.clone()).ok()?;
                let forecast = if req
                    .layers
                    .iter()
                    .any(|l| l.kind == RenderLayerKind::ForecastField)
                {
                    self.field.get_untracked().map(|f| self.render_key(&f))
                } else {
                    None
                };
                let archives = req
                    .layers
                    .into_iter()
                    .filter_map(|l| match (l.kind, l.path) {
                        (RenderLayerKind::LocalPmtiles, Some(p)) => Some((l.layer_id, p)),
                        _ => None,
                    })
                    .collect();
                Some(Pending::Tiles { forecast, archives })
            }
        }
    }

    fn dispatch(&self, actions: Vec<MapAction>, pendings: &mut Vec<(MapRequestKind, Pending)>) {
        for action in actions {
            match action {
                MapAction::Send {
                    request_id,
                    kind,
                    body,
                } => {
                    let pending = pendings
                        .iter()
                        .position(|(k, _)| *k == kind)
                        .map(|index| pendings.remove(index).1)
                        .or_else(|| self.pending_for_action(kind, &body));
                    if let Some(pending) = pending {
                        self.pending.borrow_mut().insert(request_id, pending);
                    }
                    if let Err(error) =
                        self.engine
                            .send_request(JOB_ID, request_id, kind.wire_kind(), body)
                    {
                        self.pending.borrow_mut().remove(&request_id);
                        self.status
                            .set(format!("Could not reach the engine: {error}"));
                        let (_, next) = self.sched.borrow_mut().complete(
                            request_id,
                            MapOutcome::Failure {
                                message: error,
                                cancelled: false,
                            },
                        );
                        self.dispatch(next, &mut Vec::new());
                    }
                }
                MapAction::Interrupt { request_id } => {
                    let _ = self.engine.cancel(JOB_ID, request_id);
                }
            }
        }
    }

    fn selected_entry(&self) -> Option<(omo_protocol::series::CatalogEntry, String)> {
        let entry = self.series.selected_catalog_entry()?;
        let version = self.series.catalog.get_untracked()?.version;
        Some((entry, version))
    }

    /// Load (or reload) the numeric field for the viewport plus margin.
    pub fn request_field(&self, valid_time: Option<String>) {
        if !self.worker_ready.get_untracked() {
            self.status.set("The engine is not ready yet.".into());
            return;
        }
        let Some((entry, catalog_version)) = self.selected_entry() else {
            self.status
                .set("Select a ready catalog entry first.".into());
            return;
        };
        let current = self
            .field
            .get_untracked()
            .filter(|f| f.entry_id == entry.id);
        let (run, valid) = match (&current, valid_time) {
            (Some(field), Some(valid)) => (
                MapRunSelector::Explicit(field.result.run.reference_time_utc.clone()),
                Some(valid),
            ),
            (Some(field), None) => (
                MapRunSelector::Explicit(field.result.run.reference_time_utc.clone()),
                Some(field.result.valid_time_utc.clone()),
            ),
            (None, explicit_valid) => {
                if current.is_none() && self.lead_h.get_untracked() > 0 && explicit_valid.is_none()
                {
                    self.lead_target.set(Some(self.lead_h.get_untracked()));
                }
                (MapRunSelector::LatestComplete, explicit_valid)
            }
        };
        self.requested_valid_time.set(valid.clone());
        self.bump_input_revision();
        let revision = self.field_revision.get() + 1;
        self.field_revision.set(revision);
        let vp = self.viewport.get_untracked();
        let entry_id = entry.id.clone();
        let stations_hash = self
            .series
            .snapshot
            .get_untracked()
            .map(|s| s.content_sha256);
        let doc_id = self.doc_id.get_untracked();
        let worker_generation = self.worker_generation.get_untracked();
        let request = LoadFieldRequest {
            field_id: format!("map-{revision}"),
            catalog_entry: entry,
            catalog_version: catalog_version.clone(),
            run,
            valid_time_utc: valid,
            bbox: vp.capped_bbox(FIELD_GUTTER_PX, FIELD_MAX_LAT_DEG, FIELD_MAX_LON_DEG),
            force_data_run: false,
        };
        let Ok(body) = serde_json::to_value(&request) else {
            return;
        };
        self.loading.set(true);
        self.status.set("Loading forecast field…".into());
        let actions = self
            .sched
            .borrow_mut()
            .submit_field(FieldIntent { revision, body });
        self.dispatch(
            actions,
            &mut vec![(
                MapRequestKind::LoadField,
                Pending::Field {
                    entry_id,
                    catalog_version,
                    stations_hash,
                    doc_id,
                    worker_generation,
                },
            )],
        );
    }

    pub fn on_color_input_changed(&self) {
        self.bump_input_revision();
        self.refresh_tiles();
    }

    pub fn refresh_tiles(&self) {
        if !self.worker_ready.get_untracked() {
            return;
        }
        let forecast = self.field.get_untracked().filter(|field| {
            self.forecast_visible()
                && self.sched.borrow().active_field_revision() == Some(field.revision)
        });
        let archives = self.archive_layers();
        if forecast.is_none() && archives.is_empty() {
            self.maybe_mark_settled();
            return;
        }
        let forecast_key = forecast.as_ref().map(|field| self.render_key(field));
        let (min, max) = forecast
            .as_ref()
            .map_or((0.0, 1.0), |field| self.color_domain(field));
        let palette = self.palette.get_untracked();
        let vp = self.viewport.get_untracked();
        let mut seen = HashSet::new();
        let coords: Vec<TileCoord> = vp
            .visible_tiles()
            .into_iter()
            .map(|tile| tile.coord)
            .filter(|coord| seen.insert((coord.z, coord.x, coord.y)))
            .collect();
        let mut wanted: Vec<TileCoord> = Vec::new();
        let mut need_forecast = false;
        let mut need_archives: HashSet<String> = HashSet::new();
        for coord in coords {
            let tile_key = (coord.z, coord.x, coord.y);
            let mut missing = false;
            if let Some(key) = &forecast_key
                && self.tiles.with(|store| {
                    store
                        .get_peek(&tile_key)
                        .is_none_or(|stored| stored.key != *key)
                })
            {
                need_forecast = true;
                missing = true;
            }
            for (id, path) in &archives {
                if self.layer_tiles.with(|store| {
                    store
                        .get_peek(&(id.clone(), tile_key))
                        .is_none_or(|stored| stored.key != *path)
                }) {
                    need_archives.insert(id.clone());
                    missing = true;
                }
            }
            if missing {
                wanted.push(coord);
                if wanted.len() >= MAX_TILES_PER_BATCH {
                    break;
                }
            }
        }
        if wanted.is_empty() {
            self.maybe_mark_settled();
            return;
        }
        let mut layers = Vec::new();
        let mut rendered_archives = HashMap::new();
        for (id, path) in &archives {
            if need_archives.contains(id) {
                layers.push(RenderLayerRequest {
                    layer_id: id.clone(),
                    kind: RenderLayerKind::LocalPmtiles,
                    path: Some(path.clone()),
                });
                rendered_archives.insert(id.clone(), path.clone());
            }
        }
        if need_forecast {
            layers.push(RenderLayerRequest {
                layer_id: FORECAST_LAYER.into(),
                kind: RenderLayerKind::ForecastField,
                path: None,
            });
        }
        let render_revision = self.render_counter.get() + 1;
        self.render_counter.set(render_revision);
        let request = RenderTilesRequest {
            result_id: format!("tiles-{render_revision}"),
            field_id: forecast.as_ref().map(|field| field.result.field_id.clone()),
            revision: render_revision,
            color_scale: ColorScale { palette, min, max },
            layers,
            tiles: wanted,
        };
        let Ok(body) = serde_json::to_value(&request) else {
            return;
        };
        let actions = self.sched.borrow_mut().submit_tiles(TilesIntent {
            field_revision: forecast.as_ref().map_or(0, |field| field.revision),
            body,
        });
        self.dispatch(
            actions,
            &mut vec![(
                MapRequestKind::RenderTiles,
                Pending::Tiles {
                    forecast: need_forecast.then_some(forecast_key).flatten(),
                    archives: rendered_archives,
                },
            )],
        );
    }

    // ---- layer stack ---------------------------------------------------------------------

    pub fn forecast_visible(&self) -> bool {
        self.state
            .with_untracked(|state| state.layer(FORECAST_LAYER).is_some_and(|l| l.visible))
    }

    fn archive_layers(&self) -> Vec<(String, String)> {
        let meta = self.layer_meta.get_untracked();
        self.state.with_untracked(|state| {
            state
                .layers
                .iter()
                .filter(|layer| layer.visible)
                .filter_map(|layer| match &layer.kind {
                    MapLayerKind::LocalPmtiles { path }
                        if meta
                            .get(&layer.id)
                            .is_some_and(|m| m.info.is_some() && m.error.is_none()) =>
                    {
                        Some((layer.id.clone(), path.to_string_lossy().into_owned()))
                    }
                    _ => None,
                })
                .collect()
        })
    }

    /// Display labels of active `.pmtiles` layers for the cursor readout (`US2.6`, `FR-017`).
    pub fn active_display_only_layers(&self) -> Vec<String> {
        let meta = self.layer_meta.get();
        self.state.with(|state| {
            state
                .layers
                .iter()
                .filter(|l| l.visible)
                .filter_map(|layer| match &layer.kind {
                    MapLayerKind::LocalPmtiles { path }
                        if meta.get(&layer.id).is_some_and(|m| m.error.is_none()) =>
                    {
                        let name = meta
                            .get(&layer.id)
                            .and_then(|m| m.info.as_ref())
                            .and_then(|info| info.name.clone())
                            .or_else(|| path.file_name().map(|n| n.to_string_lossy().into_owned()))
                            .unwrap_or_else(|| layer.id.clone());
                        Some(name)
                    }
                    _ => None,
                })
                .collect()
        })
    }

    /// Required attribution lines for currently visible layers (`FR-018`).
    pub fn attributions(&self) -> Vec<String> {
        let meta = self.layer_meta.get();
        let field = self.field.get();
        let entry_attr = self
            .series
            .catalog
            .get()
            .and_then(|cat| {
                let id = self.series.selected_entry.get();
                cat.entry_by_id(&id).map(|e| e.attribution.clone())
            })
            .unwrap_or_default();
        self.state.with(|state| {
            let mut archives = Vec::new();
            for layer in state.layers.iter().filter(|l| l.visible) {
                if let MapLayerKind::LocalPmtiles { path } = &layer.kind
                    && meta.get(&layer.id).is_some_and(|m| m.error.is_none())
                {
                    archives.push((
                        path.file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default(),
                        meta.get(&layer.id)
                            .and_then(|m| m.info.as_ref())
                            .and_then(|info| info.attribution.clone()),
                    ));
                }
            }
            attribution_lines(&AttributionInput {
                forecast_visible: state
                    .layer(FORECAST_LAYER)
                    .is_some_and(|l| l.visible && field.is_some()),
                forecast_run_utc: field
                    .as_ref()
                    .map(|f| f.result.run.reference_time_utc.as_str()),
                forecast_attribution: &entry_attr,
                coastlines_visible: state
                    .layers
                    .iter()
                    .any(|l| l.visible && matches!(l.kind, MapLayerKind::BuiltinCoastlines)),
                archives: archives
                    .iter()
                    .map(|(file_name, attribution)| ArchiveAttribution {
                        file_name,
                        attribution: attribution.as_deref(),
                    })
                    .collect(),
            })
        })
    }

    fn layer_error(&self, error: LayerStackError) {
        self.layer_note.set(error.to_string());
    }

    pub fn add_archive(&self, path: PathBuf) {
        let mut added = None;
        self.state
            .update(|state| match state.add_local_pmtiles(path) {
                Ok(id) => added = Some(id),
                Err(error) => self.layer_error(error),
            });
        if let Some(id) = added {
            self.bump_input_revision();
            self.layer_note.set(String::new());
            self.path_input.set(String::new());
            self.probe_layer(&id);
        }
    }

    pub fn add_archive_from_input(&self) {
        let text = self.path_input.get_untracked();
        let text = text.trim();
        if text.is_empty() {
            self.layer_note
                .set("Enter the path of a .pmtiles file.".into());
            return;
        }
        self.add_archive(PathBuf::from(text));
    }

    pub fn remove_layer(&self, id: &str) {
        let mut result = Ok(());
        self.state.update(|state| result = state.remove_layer(id));
        match result {
            Ok(()) => {
                self.bump_input_revision();
                self.layer_meta.update(|meta| {
                    meta.remove(id);
                });
                self.layer_tiles
                    .update(|store| store.retain(|(layer, _), _| layer != id));
                self.layer_note.set(String::new());
                self.maybe_mark_settled();
            }
            Err(error) => self.layer_error(error),
        }
    }

    pub fn move_layer(&self, id: &str, direction: LayerMove) {
        let mut result = Ok(false);
        self.state
            .update(|state| result = state.move_layer(id, direction));
        match result {
            Ok(true) => {
                self.bump_input_revision();
                self.maybe_mark_settled();
            }
            Ok(false) => {}
            Err(error) => self.layer_error(error),
        }
    }

    pub fn set_layer_visible(&self, id: &str, visible: bool) {
        let mut result = Ok(());
        self.state
            .update(|state| result = state.set_layer_visible(id, visible));
        if let Err(error) = result {
            self.layer_error(error);
            return;
        }
        self.bump_input_revision();
        if id == FORECAST_LAYER && !visible {
            if let Some((lon, lat)) = self.hover.get_untracked() {
                self.readout.set(Some(InspectResult {
                    latitude: lat,
                    longitude: lon,
                    sample: ForecastSampleState::LayerHidden,
                }));
            }
        } else if id == FORECAST_LAYER {
            self.readout.set(None);
        }
        self.refresh_tiles();
    }

    pub fn set_layer_opacity(&self, id: &str, opacity: f32) {
        let mut result = Ok(());
        self.state
            .update(|state| result = state.set_layer_opacity(id, opacity));
        match result {
            Ok(()) => {
                self.bump_input_revision();
                self.maybe_mark_settled();
            }
            Err(error) => self.layer_error(error),
        }
    }

    pub fn probe_layer(&self, id: &str) {
        let Some(path) = self.state.with_untracked(|state| {
            state.layer(id).and_then(|layer| match &layer.kind {
                MapLayerKind::LocalPmtiles { path } => Some(path.to_string_lossy().into_owned()),
                _ => None,
            })
        }) else {
            return;
        };
        if !self.worker_ready.get_untracked() {
            return;
        }
        self.layer_meta.update(|meta| {
            meta.entry(id.to_owned()).or_default().error = None;
        });
        let request = LayerProbeRequest {
            layer_id: id.to_owned(),
            kind: RenderLayerKind::LocalPmtiles,
            path,
        };
        let Ok(body) = serde_json::to_value(&request) else {
            return;
        };
        let actions = self.sched.borrow_mut().submit_probe(ProbeIntent {
            layer_id: id.to_owned(),
            body,
        });
        self.dispatch(
            actions,
            &mut vec![(
                MapRequestKind::LayerProbe,
                Pending::Probe {
                    layer_id: id.to_owned(),
                },
            )],
        );
    }

    pub fn retry_layer(&self, id: &str) {
        self.probe_layer(id);
    }

    pub fn reprobe_unprobed(&self) {
        let ids: Vec<String> = {
            let meta = self.layer_meta.get_untracked();
            self.state.with_untracked(|state| {
                state
                    .layers
                    .iter()
                    .filter(|l| matches!(l.kind, MapLayerKind::LocalPmtiles { .. }))
                    .filter(|l| {
                        meta.get(&l.id)
                            .is_none_or(|m| m.info.is_none() && m.error.is_none())
                    })
                    .map(|l| l.id.clone())
                    .collect()
            })
        };
        for id in ids {
            self.probe_layer(&id);
        }
    }

    fn request_inspect(&self, lat: f64, lon: f64) {
        let Some(field) = self.field.get_untracked() else {
            return;
        };
        if self.sched.borrow().active_field_revision() != Some(field.revision) {
            return;
        }
        let max_distance_m = self
            .selected_entry()
            .map(|(entry, _)| entry.max_distance_m)
            .unwrap_or(0)
            .max(HOVER_MAX_DISTANCE_M);
        let request = InspectRequest {
            field_id: field.result.field_id.clone(),
            latitude: lat,
            longitude: lon,
            max_distance_m,
        };
        let Ok(body) = serde_json::to_value(&request) else {
            return;
        };
        let actions = self.sched.borrow_mut().submit_inspect(InspectIntent {
            field_revision: field.revision,
            body,
        });
        self.dispatch(
            actions,
            &mut vec![(MapRequestKind::Inspect, Pending::Inspect)],
        );
    }

    fn ensure_coverage(&self) {
        let Some(field) = self.field.get_untracked() else {
            return;
        };
        if self.sched.borrow().field_state_is_loading() {
            return;
        }
        let view =
            self.viewport
                .get_untracked()
                .capped_bbox(0.0, FIELD_MAX_LAT_DEG, FIELD_MAX_LON_DEG);
        if !bbox_contains(&field.result.coverage_bbox, &view) {
            self.status
                .set("Extending the field to the new extent…".into());
            self.request_field(None);
        }
    }

    pub fn step_valid_time(&self, delta: i32) {
        let Some(field) = self.field.get_untracked() else {
            return;
        };
        let times = &field.result.run.valid_times;
        let Some(index) = times.iter().position(|t| *t == field.result.valid_time_utc) else {
            return;
        };
        let next = (index as i64 + i64::from(delta)).clamp(0, times.len() as i64 - 1) as usize;
        if next != index {
            self.clamp_notice.set(None);
            self.refresh_notices();
            let target = times[next].clone();
            if let Some(lead) = lead_hours(&field.result.run.reference_time_utc, &target) {
                self.lead_h.set(if next == 0 { 0 } else { lead });
            }
            self.request_field(Some(target));
        }
    }

    // ---- composite PNG export (`FR-021`) -----------------------------------------------

    pub fn export_blocker(&self) -> Option<String> {
        if !self.worker_ready.get_untracked() {
            return Some("The engine is not ready yet.".into());
        }
        if self.exporting.get_untracked() {
            return Some("An export is already running.".into());
        }
        if self.loading.get_untracked() || self.sched.borrow().field_state_is_loading() {
            return Some("Wait for the forecast field to finish loading, then export.".into());
        }
        if self.is_map_stale(false) {
            return Some(
                "The displayed map is stale; wait for it to settle or reload before exporting."
                    .into(),
            );
        }
        if let Some(field) = self.field.get_untracked()
            && self.sched.borrow().active_field_revision() != Some(field.revision)
        {
            return Some("The forecast field is not settled yet.".into());
        }
        let meta = self.layer_meta.get_untracked();
        let unprobed = self.state.with_untracked(|state| {
            state.layers.iter().any(|layer| {
                layer.visible
                    && matches!(layer.kind, MapLayerKind::LocalPmtiles { .. })
                    && meta
                        .get(&layer.id)
                        .is_none_or(|m| m.info.is_none() && m.error.is_none())
            })
        });
        if unprobed {
            return Some("A map archive is still being checked; try again in a moment.".into());
        }
        None
    }

    pub fn export_snapshot(&self) -> Result<MapExportRequest, String> {
        if let Some(blocker) = self.export_blocker() {
            return Err(blocker);
        }
        let (entry, catalog_version) = self
            .selected_entry()
            .ok_or("Select a ready catalog entry first.")?;
        let vp = self.viewport.get_untracked();
        let (center_lon, center_lat) = vp.center_lonlat();
        let edge = |value: f64| (value.round() as u32).clamp(EXPORT_MIN_EDGE, EXPORT_MAX_EDGE);
        let viewport = ExportViewport {
            center_lat,
            center_lon,
            zoom: vp.zoom,
            width: edge(vp.width),
            height: edge(vp.height),
        };
        let field = self.field.get_untracked();
        let meta = self.layer_meta.get_untracked();
        let state = self.state.get_untracked();
        let mut layers = Vec::new();
        for layer in state.layers.iter().filter(|layer| layer.visible) {
            let (kind, path) = match &layer.kind {
                MapLayerKind::BuiltinCoastlines => (ExportLayerKind::BuiltinCoastlines, None),
                MapLayerKind::ForecastField => {
                    if field.is_none() {
                        continue;
                    }
                    (ExportLayerKind::ForecastField, None)
                }
                MapLayerKind::StationOverlay => (ExportLayerKind::StationOverlay, None),
                MapLayerKind::LocalPmtiles { path } => {
                    if meta.get(&layer.id).is_none_or(|m| m.error.is_some()) {
                        continue;
                    }
                    (
                        ExportLayerKind::LocalPmtiles,
                        Some(path.to_string_lossy().into_owned()),
                    )
                }
            };
            layers.push(ExportLayer {
                id: layer.id.clone(),
                kind,
                path,
                opacity: layer.opacity.clamp(0.0, 1.0),
            });
        }
        if layers.is_empty() {
            return Err("Show at least one layer before exporting.".into());
        }
        let (forecast, color_scale, subtitle) = match field.as_ref() {
            Some(field)
                if layers
                    .iter()
                    .any(|layer| layer.kind == ExportLayerKind::ForecastField) =>
            {
                let (min, max) = self.color_domain(field);
                let result = &field.result;
                let locked = locked_range(
                    &self.locked_min.get_untracked(),
                    &self.locked_max.get_untracked(),
                )
                .is_some();
                (
                    Some(ExportForecast {
                        reference_time_utc: result.run.reference_time_utc.clone(),
                        cycle: result.run.cycle.clone(),
                        valid_time_utc: result.valid_time_utc.clone(),
                        source_layout: result.source_layout,
                        source_object: result.source_object.clone(),
                        object_version: result.object_version.clone(),
                    }),
                    Some(ExportColorScale {
                        palette: self.palette.get_untracked(),
                        min,
                        max,
                        unit: result.stats.unit.clone(),
                        locked,
                    }),
                    format!(
                        "Run {} · valid {}",
                        result.run.reference_time_utc, result.valid_time_utc
                    ),
                )
            }
            _ => (None, None, String::new()),
        };
        let selected_ids: HashSet<String> = self.series.selected_ids().into_iter().collect();
        let stations = self
            .stations()
            .into_iter()
            .map(|(id, lon, lat)| {
                let selected = selected_ids.contains(&id);
                ExportStation {
                    id,
                    lat,
                    lon,
                    selected,
                }
            })
            .collect();
        let stations_sha256 = self
            .series
            .snapshot
            .get_untracked()
            .map(|snapshot| snapshot.content_sha256);
        let request = MapExportRequest {
            revision: self.field_revision.get(),
            field_id: field.as_ref().map(|field| field.result.field_id.clone()),
            viewport,
            stations,
            stations_sha256,
            catalog_entry: entry.id.clone(),
            catalog_version,
            forecast,
            color_scale,
            layers,
            attribution: self.attributions(),
            title: format!("Omobench · {}", entry.id),
            subtitle,
        };
        request.validate()?;
        Ok(request)
    }

    pub fn export_png(&self, path: PathBuf, overwrite_confirmed: bool) {
        if !path.is_absolute() {
            self.status.set("Enter an absolute export path.".into());
            return;
        }
        if path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_none_or(|ext| !ext.eq_ignore_ascii_case("png"))
        {
            self.status
                .set("Choose a destination ending in .png.".into());
            return;
        }
        let request = match self.export_snapshot() {
            Ok(request) => request,
            Err(message) => {
                self.status.set(format!("Cannot export the map: {message}"));
                return;
            }
        };
        let export_key = self.current_export_key(false);
        let field_revision = self
            .field
            .get_untracked()
            .filter(|_| request.field_id.is_some())
            .map_or(0, |field| field.revision);
        let Ok(snapshot) = serde_json::to_value(&request) else {
            return;
        };
        let body = serde_json::json!({
            "target": path.to_string_lossy(),
            "format": "png",
            "overwrite_confirmed": overwrite_confirmed,
            "map_export": snapshot,
        });
        self.exporting.set(true);
        self.status
            .set(format!("Exporting the map to {}…", path.display()));
        let actions = self.sched.borrow_mut().submit_export(ExportIntent {
            field_revision,
            body,
        });
        if actions.is_empty() && !self.sched.borrow().has_running() {
            self.exporting.set(false);
            self.status
                .set("The forecast field changed; export again once it settles.".into());
            return;
        }
        self.dispatch(
            actions,
            &mut vec![(
                MapRequestKind::Export,
                Pending::Export {
                    path,
                    key: export_key,
                },
            )],
        );
    }

    pub fn choose_export(&self) {
        if let Some(blocker) = self.export_blocker() {
            self.status.set(format!("Cannot export the map: {blocker}"));
            return;
        }
        if self.export_path_input.get_untracked().is_empty() {
            let default_path = std::env::var("OMO_DEFAULT_EXPORT_PATH")
                .unwrap_or_else(|_| "/tmp/omo-m2/export/gui-map.png".into());
            self.export_path_input.set(default_path);
        }
        if std::env::var("OMO_EXPORT_MANUAL").is_ok() {
            self.export_manual_open.set(true);
            self.status
                .set("Enter an absolute .png path to export.".into());
            return;
        }
        let this = self.clone();
        let fallback = self.clone();
        super::dialogs::choose_map_export_path(
            move |path, confirmed| this.export_png(path, confirmed),
            move |error| {
                fallback.export_manual_open.set(true);
                fallback.status.set(format!(
                    "Save chooser unavailable ({error}); enter an absolute .png path."
                ));
            },
        );
    }

    pub fn export_to_typed_path(&self) {
        let text = self.export_path_input.get_untracked();
        let this = self.clone();
        let fallback = self.clone();
        super::dialogs::confirm_export_path(
            PathBuf::from(text.trim()),
            move |path, confirmed| this.export_png(path, confirmed),
            move |error| fallback.status.set(error),
        );
    }

    // ---- persisted state (`FR-023`) ----------------------------------------------------

    fn read_state(&self, tracked: bool) -> MapViewState {
        let layers = Self::read_sig(self.state, tracked);
        compose_state(
            &layers,
            Self::read_sig(self.nav, tracked),
            Self::read_sig(self.lead_h, tracked),
            &Self::read_sig(self.palette, tracked),
            &Self::read_sig(self.locked_min, tracked),
            &Self::read_sig(self.locked_max, tracked),
        )
    }

    pub fn snapshot_state(&self) -> MapViewState {
        self.read_state(false)
    }

    pub fn state_fingerprint(&self) -> String {
        serde_json::to_string(&self.read_state(true)).unwrap_or_default()
    }

    pub fn apply_state(&self, state: MapViewState) {
        let state = state.canonical().unwrap_or_default();
        self.palette.set(state.color_scale.palette.clone());
        self.locked_min.set(
            state
                .color_scale
                .locked_min
                .map(|v| v.to_string())
                .unwrap_or_default(),
        );
        self.locked_max.set(
            state
                .color_scale
                .locked_max
                .map(|v| v.to_string())
                .unwrap_or_default(),
        );
        self.nav.set(state.viewport);
        self.lead_h.set(state.valid_time_lead_h);
        self.clamp_notice.set(None);
        self.layer_meta.set(HashMap::new());
        self.layer_tiles.set(TileLruCache::new());
        self.state.set(state.clone());
        self.layers_open.set(false);
        self.layer_note.set(String::new());
        self.bump_input_revision();
        if self.laid_out.get_untracked() {
            self.initial_view();
        }
        self.reprobe_unprobed();
        match self.field.get_untracked() {
            Some(field) => {
                let resolved = resolve_lead_with_clamp(
                    &field.result.run.reference_time_utc,
                    &field.result.run.valid_times,
                    state.valid_time_lead_h,
                );
                self.lead_target.set(None);
                if let Some(res) = resolved {
                    self.clamp_notice.set(res.clamp_notice);
                    self.refresh_notices();
                    if res.valid_time_utc != field.result.valid_time_utc {
                        self.request_field(Some(res.valid_time_utc));
                    } else {
                        self.refresh_tiles();
                    }
                } else {
                    self.refresh_tiles();
                }
            }
            None => self
                .lead_target
                .set((state.valid_time_lead_h > 0).then_some(state.valid_time_lead_h)),
        }
    }

    pub fn palette_cycle(&self) {
        let current = self.palette.get_untracked();
        let index = PALETTES.iter().position(|p| *p == current).unwrap_or(0);
        self.palette
            .set(PALETTES[(index + 1) % PALETTES.len()].to_owned());
        self.bump_input_revision();
        self.refresh_tiles();
    }

    pub fn on_worker_restart(&self) {
        self.sched.borrow_mut().worker_restarted();
        self.pending.borrow_mut().clear();
        self.loading.set(false);
        if self.exporting.get_untracked() {
            self.exporting.set(false);
            self.status.set(
                "The engine restarted before the map export finished; nothing was saved by this request."
                    .into(),
            );
        }
        self.layer_meta.update(|meta| {
            for entry in meta.values_mut() {
                if entry.error.is_none() {
                    entry.info = None;
                }
            }
        });
        self.reprobe_unprobed();
        if self.field.get_untracked().is_some() {
            self.field_stale
                .set(Some("The engine restarted; reloading the field…".into()));
            self.request_field(None);
        }
    }

    // ---- events ---------------------------------------------------------------------------

    pub fn handle_event(&self, kind: &str, request_id: u64, body: &Value) -> bool {
        if !self.pending.borrow().contains_key(&request_id) {
            return false;
        }
        match kind {
            "progress" => true,
            "result" | "export_committed" => {
                let pending = self.pending.borrow_mut().remove(&request_id);
                let (completion, actions) = self
                    .sched
                    .borrow_mut()
                    .complete(request_id, MapOutcome::Success(body.clone()));
                self.apply_completion(completion, pending);
                self.dispatch(actions, &mut Vec::new());
                self.after_progress();
                true
            }
            "failed" => {
                let pending = self.pending.borrow_mut().remove(&request_id);
                let message = body
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("worker request failed")
                    .to_owned();
                let fail_kind = parse_fail_kind(body);
                let cancelled = matches!(fail_kind, Some(FailKind::Cancelled(_)));
                let busy = fail_kind.as_ref().is_some_and(FailKind::is_worker_busy);
                let (completion, actions) = self.sched.borrow_mut().complete(
                    request_id,
                    MapOutcome::Failure {
                        message: message.clone(),
                        cancelled,
                    },
                );
                if busy && matches!(pending, Some(Pending::Field { .. })) {
                    if self.field_retries.get() < 60 {
                        self.field_retries.set(self.field_retries.get() + 1);
                        self.field_retry
                            .set(Some(Instant::now() + Duration::from_millis(1000)));
                    }
                } else if busy {
                    self.busy_retry
                        .set(Some(Instant::now() + Duration::from_millis(500)));
                }
                self.apply_completion_with_busy(completion, pending, busy);
                self.dispatch(actions, &mut Vec::new());
                self.after_progress();
                true
            }
            _ => false,
        }
    }

    fn after_progress(&self) {
        let loading = matches!(
            self.sched.borrow().running_request(),
            Some((_, MapRequestKind::LoadField))
        ) || self.sched.borrow().field_state_is_loading();
        if self.loading.get_untracked() != loading {
            self.loading.set(loading);
        }
    }

    fn apply_completion(&self, completion: MapCompletion, pending: Option<Pending>) {
        self.apply_completion_with_busy(completion, pending, false);
    }

    fn apply_completion_with_busy(
        &self,
        completion: MapCompletion,
        pending: Option<Pending>,
        busy: bool,
    ) {
        match completion {
            MapCompletion::FieldLoaded { revision, result } => {
                let (entry_id, catalog_version, stations_hash, doc_id, worker_generation) =
                    match pending {
                        Some(Pending::Field {
                            entry_id,
                            catalog_version,
                            stations_hash,
                            doc_id,
                            worker_generation,
                        }) => (
                            entry_id,
                            catalog_version,
                            stations_hash,
                            doc_id,
                            worker_generation,
                        ),
                        _ => (
                            self.series.selected_entry.get_untracked(),
                            self.series
                                .catalog
                                .get_untracked()
                                .map(|c| c.version)
                                .unwrap_or_default(),
                            self.series
                                .snapshot
                                .get_untracked()
                                .map(|s| s.content_sha256),
                            self.doc_id.get_untracked(),
                            self.worker_generation.get_untracked(),
                        ),
                    };
                match serde_json::from_value::<LoadFieldResult>(result) {
                    Ok(result) => {
                        let summary = format!(
                            "{} · valid {} · {}{}",
                            result.run.reference_time_utc,
                            result.valid_time_utc,
                            match result.source_layout {
                                SourceLayout::DataSpatial => "data_spatial",
                                SourceLayout::DataRun => "data_run",
                            },
                            result
                                .fallback_notice
                                .as_deref()
                                .map(|n| format!(" — {n}"))
                                .unwrap_or_default()
                        );
                        self.field_stale.set(None);
                        self.field_retries.set(0);
                        self.requested_valid_time
                            .set(Some(result.valid_time_utc.clone()));
                        self.status.set(format!("received, not saved · {summary}"));
                        let follow_up = self.lead_target.take().and_then(|lead| {
                            let resolved = resolve_lead_with_clamp(
                                &result.run.reference_time_utc,
                                &result.run.valid_times,
                                lead,
                            )?;
                            self.clamp_notice.set(resolved.clamp_notice);
                            self.refresh_notices();
                            if resolved.effective_lead_h != lead {
                                self.lead_h.set(resolved.effective_lead_h);
                            }
                            (resolved.valid_time_utc != result.valid_time_utc)
                                .then_some(resolved.valid_time_utc)
                        });
                        self.field.set(Some(LoadedField {
                            revision,
                            entry_id,
                            catalog_version,
                            stations_hash,
                            doc_id,
                            worker_generation,
                            result,
                        }));
                        self.readout.set(None);
                        self.refresh_tiles();
                        if let Some(valid) = follow_up {
                            self.request_field(Some(valid));
                        }
                    }
                    Err(error) => self
                        .status
                        .set(format!("The worker's field reply was invalid: {error}")),
                }
            }
            MapCompletion::FieldFailedStale(message) => {
                if !busy {
                    self.field_stale
                        .set(Some(format!("Field reload failed: {message}")));
                    self.status
                        .set(format!("Map retained as stale · {message}"));
                }
            }
            MapCompletion::FieldFailed(message) => {
                if !busy {
                    self.status
                        .set(format!("Could not load the field: {message}"));
                }
            }
            MapCompletion::FieldCancelled => {
                self.requested_valid_time.set(None);
                self.status.set("Forecast field load cancelled.".into());
            }
            MapCompletion::Completed {
                kind: MapRequestKind::RenderTiles,
                result,
                ..
            } => {
                let Some(Pending::Tiles { forecast, archives }) = pending else {
                    return;
                };
                match serde_json::from_value::<RenderTilesResult>(result) {
                    Ok(result) => self.install_tiles(forecast, archives, result),
                    Err(error) => self.status.set(format!("Invalid tile reply: {error}")),
                }
            }
            MapCompletion::Completed {
                kind: MapRequestKind::Inspect,
                result,
                ..
            } => {
                if let Ok(result) = serde_json::from_value::<InspectResult>(result) {
                    self.readout.set(Some(result));
                }
            }
            MapCompletion::Completed {
                kind: MapRequestKind::LayerProbe,
                result,
                ..
            } => match serde_json::from_value::<LayerProbeResult>(result) {
                Ok(probed) => {
                    self.layer_meta.update(|meta| {
                        let entry = meta.entry(probed.layer_id).or_default();
                        entry.info = Some(probed.info);
                        entry.error = None;
                    });
                    self.refresh_tiles();
                }
                Err(error) => self
                    .status
                    .set(format!("Invalid layer probe reply: {error}")),
            },
            MapCompletion::Completed {
                kind: MapRequestKind::Export,
                result,
                ..
            } => {
                self.exporting.set(false);
                let (path, key) = match pending {
                    Some(Pending::Export { path, key }) => (
                        result
                            .get("target")
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                            .unwrap_or_else(|| path.to_string_lossy().into_owned()),
                        key,
                    ),
                    _ => (
                        result
                            .get("target")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                        self.current_export_key(false),
                    ),
                };
                self.status.set(format!("Map exported to {path}"));
                self.export_saved.set(Some((key, path)));
            }
            MapCompletion::RequestCancelled { kind } => {
                if kind == MapRequestKind::Export {
                    self.exporting.set(false);
                    self.status.set("Map export cancelled.".into());
                }
            }
            MapCompletion::RequestFailed { kind, message } => match kind {
                MapRequestKind::Export => {
                    self.exporting.set(false);
                    self.status.set(format!("Map export failed: {message}"));
                }
                MapRequestKind::RenderTiles if !busy => {
                    self.status.set(format!("Tile rendering failed: {message}"));
                }
                MapRequestKind::LayerProbe if !busy => {
                    if let Some(Pending::Probe { layer_id }) = pending {
                        self.layer_meta.update(|meta| {
                            let entry = meta.entry(layer_id).or_default();
                            entry.info = None;
                            entry.error = Some(message);
                        });
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }

    fn install_tiles(
        &self,
        forecast: Option<RenderKey>,
        archives: HashMap<String, String>,
        result: RenderTilesResult,
    ) {
        if !result.layer_errors.is_empty() {
            self.layer_meta.update(|meta| {
                for error in &result.layer_errors {
                    if error.layer_id != FORECAST_LAYER {
                        let entry = meta.entry(error.layer_id.clone()).or_default();
                        entry.error = Some(error.message.clone());
                    }
                }
            });
        }
        if let Some(error) = result.layer_errors.first() {
            self.status
                .set(format!("Layer `{}`: {}", error.layer_id, error.message));
        }
        let live: HashSet<String> = self
            .state
            .with_untracked(|state| state.layers.iter().map(|layer| layer.id.clone()).collect());
        let tiles: Vec<RenderedTile> = result
            .tiles
            .into_iter()
            .filter(|tile| {
                if tile.layer_id == FORECAST_LAYER {
                    forecast.is_some()
                } else {
                    archives.contains_key(&tile.layer_id) && live.contains(&tile.layer_id)
                }
            })
            .collect();
        let this = self.clone();
        spawn(async move {
            let read = blocking(move || {
                tiles
                    .into_iter()
                    .map(|tile| {
                        let bytes = if tile.empty || tile.rgba_path.is_empty() {
                            None
                        } else {
                            std::fs::read(&tile.rgba_path).ok().filter(|bytes| {
                                bytes.len() == (TILE_SIZE as usize) * (TILE_SIZE as usize) * 4
                            })
                        };
                        (tile, bytes)
                    })
                    .collect::<Vec<_>>()
            })
            .await;
            let Ok(read) = read else { return };
            let mut forecast_tiles = Vec::new();
            let mut archive_tiles = Vec::new();
            for (tile, bytes) in read {
                let id = this.tile_ids.get() + 1;
                this.tile_ids.set(id);
                let image = bytes.map(|bytes| ImageSrc::rgba(TILE_SIZE, TILE_SIZE, bytes));
                if tile.layer_id == FORECAST_LAYER {
                    forecast_tiles.push((tile, id, image));
                } else {
                    archive_tiles.push((tile, id, image));
                }
            }
            if let Some(key) = forecast {
                this.tiles.update(|store| {
                    for (tile, id, image) in forecast_tiles {
                        store.insert(
                            (tile.z, tile.x, tile.y),
                            StoredTile {
                                id,
                                key: key.clone(),
                                image,
                            },
                        );
                    }
                });
            }
            if !archive_tiles.is_empty() {
                this.layer_tiles.update(|store| {
                    for (tile, id, image) in archive_tiles {
                        let Some(path) = archives.get(&tile.layer_id) else {
                            continue;
                        };
                        store.insert(
                            (tile.layer_id, (tile.z, tile.x, tile.y)),
                            StoredLayerTile {
                                id,
                                key: path.clone(),
                                image,
                            },
                        );
                    }
                });
            }
            this.prune_tiles();
            this.refresh_tiles();
        });
    }

    fn prune_tiles(&self) {
        let visible: HashSet<TileKey> = self
            .viewport
            .get_untracked()
            .visible_tiles()
            .into_iter()
            .map(|tile| (tile.coord.z, tile.coord.x, tile.coord.y))
            .collect();
        self.layer_tiles.update(|store| {
            store.evict_lru(MAX_CACHED_TILES * 2, MAX_CACHED_TILES, |(_, key)| {
                visible.contains(key)
            });
        });
        self.tiles.update(|store| {
            store.evict_lru(MAX_CACHED_TILES, MAX_CACHED_TILES / 2, |key| {
                visible.contains(key)
            });
        });
    }
}

/// True when every part of `inner` lies within `outer` (latitude band and longitude segments).
fn bbox_contains(outer: &MapBbox, inner: &MapBbox) -> bool {
    const EPS: f64 = 1e-6;
    if inner.lat_min < outer.lat_min - EPS || inner.lat_max > outer.lat_max + EPS {
        return false;
    }
    inner.lon_spans.iter().all(|span| {
        outer
            .lon_spans
            .iter()
            .any(|o| span[0] >= o[0] - EPS && span[1] <= o[1] + EPS)
    })
}

/// Text of the pointer readout HUD (`FR-011`, `FR-012`, `FR-017`).
pub(super) fn hud_text(
    hover: Option<(f64, f64)>,
    readout: Option<&InspectResult>,
    stale: bool,
    display_only_layers: &[String],
) -> String {
    let Some((lon, lat)) = hover else {
        return "Move the pointer over the map to read the forecast value.".into();
    };
    let coords = format!(
        "{:.3}°{} {:.3}°{}",
        lat.abs(),
        if lat >= 0.0 { "N" } else { "S" },
        lon.abs(),
        if lon >= 0.0 { "E" } else { "W" },
    );
    let display_suffix = if display_only_layers.is_empty() {
        String::new()
    } else {
        let list = display_only_layers
            .iter()
            .map(|name| format!("{name}: display only"))
            .collect::<Vec<_>>()
            .join(", ");
        format!("  ·  {list}")
    };
    let Some(readout) = readout else {
        return format!("{coords}{display_suffix}");
    };
    let value = match &readout.sample {
        ForecastSampleState::Value {
            value,
            unit,
            distance_m,
            stale: sample_stale,
            ..
        } => format!(
            "{value:.2} {unit} · native numeric · grid Δ {:.1} km{}",
            distance_m / 1000.0,
            if *sample_stale || stale {
                " · stale"
            } else {
                ""
            }
        ),
        ForecastSampleState::NoData { .. } => "no data at this grid cell".into(),
        ForecastSampleState::OutsideCachedExtent => "outside the loaded field extent".into(),
        ForecastSampleState::LayerHidden => "forecast layer hidden".into(),
        ForecastSampleState::NotLoaded => "field not loaded".into(),
    };
    format!("{coords}  ·  {value}{display_suffix}")
}

#[cfg(test)]
mod tests {
    use omo_protocol::map::{ForecastSampleState, InspectResult, ValueProvenance};

    use super::hud_text;

    #[test]
    fn hud_text_labels_forecast_native_numeric_stale_and_pmtiles_display_only() {
        let readout = InspectResult {
            latitude: 31.23,
            longitude: 121.47,
            sample: ForecastSampleState::Value {
                value: 26.7,
                unit: "°C".into(),
                grid_lat: 31.2,
                grid_lon: 121.5,
                distance_m: 1_200.0,
                provenance: ValueProvenance::NativeNumeric,
                stale: false,
            },
        };
        let text = hud_text(
            Some((121.47, 31.23)),
            Some(&readout),
            true,
            &["sample-raster.pmtiles".into()],
        );
        assert!(text.contains("26.70 °C · native numeric"), "{text}");
        assert!(text.contains(" · stale"), "{text}");
        assert!(
            text.contains("sample-raster.pmtiles: display only"),
            "{text}"
        );
    }
}
