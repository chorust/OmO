//! M1 "Series" workbench panel: station import, catalog readiness, series run, match report,
//! and the sidecar-v2 provenance snapshot.
//!
//! All heavy work (CSV parsing, canonical writes, Arrow match decoding) runs through
//! `blocking`; the UI thread only mutates signals.

use std::{cell::RefCell, collections::HashMap, path::PathBuf, rc::Rc};

use nsh_base::{button, input};
use nsh_core::prelude::*;
use omo_project::stations::{ColumnMapping, StationSnapshot};
use omo_protocol::{
    messages::parse_fail_kind,
    result_ref::{ResultLimits, ResultRef},
    series::{
        CatalogCheckResult, CatalogEntry, LeadWindow, Readiness, RunSelector, SeriesRequest,
        SeriesResultBody,
    },
};
use serde_json::{Value, json};

use super::plot_view::PlotInput;
use crate::host::{
    project_io,
    runtime::EngineHandle,
    series_host::{
        CatalogFile, MAX_PIVOT_STATIONS, StationImportDraft, StationMatchInfo, add_hours_iso,
        build_m1_sidecar_json, clamp_view_window, load_catalog, now_utc_iso, pivot_series_sql,
        read_matches_arrow,
    },
};

const JOB_ID: u64 = 1;
const SERIES_LIMITS: ResultLimits = ResultLimits {
    rows: 2_000_000,
    bytes: 256 * 1024 * 1024,
};

/// Immutable description of the inputs behind a received series result.
#[derive(Clone, PartialEq)]
pub(super) struct SeriesOutcome {
    pub body: SeriesResultBody,
    pub matches: Vec<StationMatchInfo>,
    pub key: String,
    pub result_id: String,
    pub entry_id: String,
}

enum Pending {
    Catalog(String),
    Series { result_id: String, key: String },
    SqlSession,
}

#[derive(Clone)]
pub(super) struct SeriesController {
    engine: EngineHandle,
    pending: Rc<RefCell<HashMap<u64, Pending>>>,
    active_series: Signal<Option<u64>>,
    request_count: Signal<u64>,
    worker_ready: Signal<bool>,
    worker_generation: Signal<u64>,
    result: Signal<Option<ResultRef>>,
    plot_input: Signal<Option<PlotInput>>,
    stale_result: Signal<Option<String>>,
    pub catalog: Signal<Option<CatalogFile>>,
    pub selected_entry: Signal<String>,
    pub readiness: Signal<HashMap<String, Readiness>>,
    pub draft: Signal<Option<StationImportDraft>>,
    pub snapshot: Signal<Option<StationSnapshot>>,
    pub stations_path: Signal<Option<PathBuf>>,
    pub window_start: Signal<String>,
    pub window_end: Signal<String>,
    pub max_distance: Signal<String>,
    pub outcome: Signal<Option<SeriesOutcome>>,
    pub saved_exports: Signal<HashMap<String, PathBuf>>,
    pub status: Signal<String>,
    pub source_path: Signal<String>,
    pub station_filter: Signal<String>,
    pub list_search: Signal<String>,
    pub view_start: Signal<String>,
    pub view_end: Signal<String>,
    pub view_note: Signal<String>,
}

impl SeriesController {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        engine: EngineHandle,
        request_count: Signal<u64>,
        worker_ready: Signal<bool>,
        worker_generation: Signal<u64>,
        result: Signal<Option<ResultRef>>,
        plot_input: Signal<Option<PlotInput>>,
        stale_result: Signal<Option<String>>,
    ) -> Self {
        let controller = Self {
            engine,
            pending: Rc::new(RefCell::new(HashMap::new())),
            active_series: signal(None),
            request_count,
            worker_ready,
            worker_generation,
            result,
            plot_input,
            stale_result,
            catalog: signal(None),
            selected_entry: signal(String::new()),
            readiness: signal(HashMap::new()),
            draft: signal(None),
            snapshot: signal(None),
            stations_path: signal(None),
            window_start: signal("0".to_owned()),
            window_end: signal("168".to_owned()),
            max_distance: signal("7200".to_owned()),
            outcome: signal(None),
            saved_exports: signal(HashMap::new()),
            status: signal(String::from("Loading product catalog…")),
            source_path: signal(String::new()),
            station_filter: signal(String::new()),
            list_search: signal(String::new()),
            view_start: signal("0".to_owned()),
            view_end: signal("168".to_owned()),
            view_note: signal(String::new()),
        };
        let this = controller.clone();
        spawn(async move {
            match blocking(load_catalog).await {
                Ok(Ok(catalog)) => {
                    if let Some(entry) = catalog.default_entry() {
                        if this.selected_entry.get_untracked().is_empty() {
                            this.selected_entry.set(entry.id.clone());
                            this.window_start.set(entry.default_window_h[0].to_string());
                            this.window_end.set(entry.default_window_h[1].to_string());
                            this.max_distance.set(entry.max_distance_m.to_string());
                            this.view_start.set(entry.default_window_h[0].to_string());
                            this.view_end.set(entry.default_window_h[1].to_string());
                        }
                    }
                    this.catalog.set(Some(catalog));
                    if this.status.get_untracked() == "Loading product catalog…" {
                        this.status.set("No station CSV selected.".into());
                    }
                    if this.worker_ready.get_untracked() {
                        this.check_all();
                    }
                }
                Ok(Err(error)) => {
                    this.status.set(format!("Catalog unavailable: {error}"));
                }
                Err(error) => {
                    this.status
                        .set(format!("Catalog load task failed: {error}"));
                }
            }
        });
        controller
    }

    fn next_request_id(&self) -> u64 {
        let id = self.request_count.get_untracked().saturating_add(1);
        self.request_count.set(id);
        id
    }

    pub fn selected_catalog_entry(&self) -> Option<CatalogEntry> {
        let id = self.selected_entry.get_untracked();
        self.catalog
            .get_untracked()
            .and_then(|catalog| catalog.entry_by_id(&id).cloned())
    }

    fn parsed_window(&self) -> Result<LeadWindow, String> {
        let start = self
            .window_start
            .get_untracked()
            .trim()
            .parse::<u32>()
            .map_err(|_| "Window start must be a whole number of hours.".to_owned())?;
        let end = self
            .window_end
            .get_untracked()
            .trim()
            .parse::<u32>()
            .map_err(|_| "Window end must be a whole number of hours.".to_owned())?;
        if end < start {
            return Err("Window end must not be before the start.".into());
        }
        Ok(LeadWindow::new(start, end))
    }

    fn parsed_max_distance(&self) -> Result<u32, String> {
        self.max_distance
            .get_untracked()
            .trim()
            .parse::<u32>()
            .ok()
            .filter(|value| *value > 0)
            .ok_or_else(|| "Max distance must be a positive number of metres.".to_owned())
    }

    /// Key identifying the inputs a result would be computed from (research R-05).
    pub fn current_key(&self) -> String {
        format!(
            "{}|{}|{}|{}|{}",
            self.snapshot
                .get()
                .map(|snapshot| snapshot.content_sha256)
                .unwrap_or_default(),
            self.selected_entry.get(),
            self.window_start.get(),
            self.window_end.get(),
            self.max_distance.get(),
        )
    }

    pub fn is_ready(&self, entry_id: &str) -> bool {
        matches!(
            self.readiness.get_untracked().get(entry_id),
            Some(Readiness::Ready { .. })
        )
    }

    // ---- catalog readiness -------------------------------------------------------------

    pub fn check_all(&self) {
        let Some(catalog) = self.catalog.get_untracked() else {
            return;
        };
        self.readiness.set(HashMap::new());
        for entry in catalog.entries {
            let request_id = self.next_request_id();
            self.pending
                .borrow_mut()
                .insert(request_id, Pending::Catalog(entry.id.clone()));
            if let Err(error) = self.engine.send_request(
                JOB_ID,
                request_id,
                "catalog_check",
                json!({ "entry": entry }),
            ) {
                self.pending.borrow_mut().remove(&request_id);
                self.status
                    .set(format!("Could not request catalog readiness: {error}"));
            }
        }
    }

    pub fn on_worker_ready(&self) {
        self.pending.borrow_mut().clear();
        self.active_series.set(None);
        self.check_all();
        self.sync_sql_session();
    }

    pub fn cycle_entry(&self) {
        let Some(catalog) = self.catalog.get_untracked() else {
            return;
        };
        if catalog.entries.is_empty() {
            return;
        }
        let current = self.selected_entry.get_untracked();
        let index = catalog
            .entries
            .iter()
            .position(|entry| entry.id == current)
            .map_or(0, |index| (index + 1) % catalog.entries.len());
        let entry = &catalog.entries[index];
        self.window_start.set(entry.default_window_h[0].to_string());
        self.window_end.set(entry.default_window_h[1].to_string());
        self.max_distance.set(entry.max_distance_m.to_string());
        self.selected_entry.set(entry.id.clone());
    }

    // ---- station import ----------------------------------------------------------------

    pub fn open_csv(&self) {
        let this = self.clone();
        let error_status = self.status;
        super::dialogs::choose_stations_csv(
            move |path: PathBuf| this.load_path(path.to_string_lossy().into_owned()),
            move |error| {
                error_status.set(format!(
                    "File chooser unavailable ({error}). Enter a path in the Series panel."
                ))
            },
        );
    }

    pub fn load_path(&self, path: String) {
        let path = path.trim().to_owned();
        if path.is_empty() {
            self.status
                .set("Enter a CSV path or choose a file first.".into());
            return;
        }
        self.source_path.set(path.clone());
        self.status.set(format!("Reading {path}…"));
        let draft = self.draft;
        let status = self.status;
        spawn(async move {
            let now = now_utc_iso();
            let parsed =
                blocking(move || StationImportDraft::from_path(std::path::Path::new(&path), &now))
                    .await;
            match parsed {
                Ok(next) => {
                    status.set(match &next.error {
                        Some(message) => message.clone(),
                        None if next.issues.is_empty() => {
                            "Review the column mapping, then confirm the import.".into()
                        }
                        None => format!(
                            "{} row issue(s) must be fixed in the CSV.",
                            next.issues.len()
                        ),
                    });
                    draft.set(Some(next));
                }
                Err(error) => status.set(format!("CSV import task failed: {error}")),
            }
        });
    }

    fn update_draft(&self, change: impl FnOnce(&mut StationImportDraft) + Send + 'static) {
        let Some(mut next) = self.draft.get_untracked() else {
            return;
        };
        let draft = self.draft;
        spawn(async move {
            let updated = blocking(move || {
                change(&mut next);
                next
            })
            .await;
            if let Ok(updated) = updated {
                draft.set(Some(updated));
            }
        });
    }

    pub fn toggle_delimiter(&self) {
        let Some(current) = self.draft.get_untracked() else {
            return;
        };
        let next = if current.delimiter == Some(b';') {
            b','
        } else {
            b';'
        };
        self.update_draft(move |draft| draft.set_delimiter(next));
    }

    pub fn cycle_mapping(&self, field: &'static str) {
        let Some(current) = self.draft.get_untracked() else {
            return;
        };
        let headers = current.headers().to_vec();
        if headers.is_empty() {
            return;
        }
        let mut mapping: ColumnMapping = current.mapping.clone();
        let advance = |value: &str, optional: bool| -> Option<String> {
            let position = headers.iter().position(|header| header == value);
            match position {
                None => Some(headers[0].clone()),
                Some(index) if index + 1 < headers.len() => Some(headers[index + 1].clone()),
                Some(_) if optional => None,
                Some(_) => Some(headers[0].clone()),
            }
        };
        match field {
            "id" => mapping.id = advance(&mapping.id, false).unwrap_or_default(),
            "lat" => mapping.lat = advance(&mapping.lat, false).unwrap_or_default(),
            "lon" => mapping.lon = advance(&mapping.lon, false).unwrap_or_default(),
            "name" => mapping.name = advance(mapping.name.as_deref().unwrap_or(""), true),
            "elevation" => {
                mapping.elevation = advance(mapping.elevation.as_deref().unwrap_or(""), true)
            }
            _ => return,
        }
        self.update_draft(move |draft| draft.set_mapping(mapping));
    }

    pub fn toggle_large_import_confirmed(&self) {
        self.update_draft(|draft| {
            draft.large_file_confirmed = !draft.large_file_confirmed;
        });
    }

    pub fn confirm_import(&self) {
        let Some(draft) = self.draft.get_untracked() else {
            return;
        };
        if !draft.can_confirm() {
            if draft.needs_large_confirmation() {
                self.status.set(
                    "CSV exceeds 1,000 stations. Acknowledge the large-import warning before confirming."
                        .into(),
                );
            } else {
                self.status
                    .set("Fix the listed issues before confirming the import.".into());
            }
            return;
        }
        let this = self.clone();
        self.status
            .set("Writing canonical station snapshot…".into());
        spawn(async move {
            let app_data = project_io::app_data_dir();
            let written = blocking(move || draft.confirm_and_write(&app_data)).await;
            match written {
                Ok(Ok((snapshot, path))) => {
                    this.status.set(format!(
                        "Imported {} stations (sha256 {}).",
                        snapshot.rows.len(),
                        &snapshot.content_sha256[..12.min(snapshot.content_sha256.len())]
                    ));
                    this.stations_path.set(Some(path));
                    this.snapshot.set(Some(snapshot));
                    this.draft.set(None);
                    this.sync_sql_session();
                }
                Ok(Err(message)) => this.status.set(message),
                Err(error) => this.status.set(format!("Import task failed: {error}")),
            }
        });
    }

    pub fn cancel_import(&self) {
        self.draft.set(None);
        self.status.set("Station import cancelled.".into());
    }

    // ---- series run --------------------------------------------------------------------

    pub fn run_series(&self) {
        if !self.worker_ready.get_untracked() {
            self.status.set("Engine worker is not ready.".into());
            return;
        }
        let Some(snapshot) = self.snapshot.get_untracked() else {
            self.status
                .set("Import and confirm a station CSV first.".into());
            return;
        };
        let Some(path) = self.stations_path.get_untracked() else {
            self.status
                .set("Station snapshot has no canonical file.".into());
            return;
        };
        let Some(entry) = self.selected_catalog_entry() else {
            self.status.set("Select a catalog entry first.".into());
            return;
        };
        if !self.is_ready(&entry.id) {
            self.status
                .set("The selected catalog entry is not ready; refresh readiness.".into());
            return;
        }
        let window = match self.parsed_window() {
            Ok(window) => window,
            Err(message) => {
                self.status.set(message);
                return;
            }
        };
        let max_distance_m = match self.parsed_max_distance() {
            Ok(value) => value,
            Err(message) => {
                self.status.set(message);
                return;
            }
        };
        if let Some(active) = self.active_series.get_untracked() {
            let _ = self.engine.cancel(JOB_ID, active);
            self.pending.borrow_mut().remove(&active);
        }
        let request_id = self.next_request_id();
        let result_id = format!("series-{request_id}");
        let key = self.current_key();
        let catalog_version = self
            .catalog
            .get_untracked()
            .map(|catalog| catalog.version)
            .unwrap_or_default();
        let request = SeriesRequest {
            stations_ref: path.to_string_lossy().into_owned(),
            stations_sha256: snapshot.content_sha256.clone(),
            catalog_entry: entry,
            catalog_version,
            run: RunSelector::latest_complete(),
            window,
            max_distance_m,
            result_id: result_id.clone(),
            limits: SERIES_LIMITS,
        };
        let body = match serde_json::to_value(&request) {
            Ok(body) => body,
            Err(error) => {
                self.status
                    .set(format!("Could not encode series request: {error}"));
                return;
            }
        };
        self.pending
            .borrow_mut()
            .insert(request_id, Pending::Series { result_id, key });
        match self.engine.send_request(JOB_ID, request_id, "series", body) {
            Ok(()) => {
                self.active_series.set(Some(request_id));
                self.status.set("Series request submitted…".into());
            }
            Err(error) => {
                self.pending.borrow_mut().remove(&request_id);
                self.status.set(format!("Could not submit series: {error}"));
            }
        }
    }

    pub fn has_active_series(&self) -> bool {
        self.active_series.get_untracked().is_some()
    }

    pub fn cancel_series(&self) {
        match self.active_series.get_untracked() {
            Some(request_id) => match self.engine.cancel(JOB_ID, request_id) {
                Ok(()) => self
                    .status
                    .set(format!("Cancellation requested for series {request_id}.")),
                Err(error) => self.status.set(format!("Could not cancel series: {error}")),
            },
            None => self.status.set("No series request is active.".into()),
        }
    }

    pub fn sync_sql_session(&self) {
        if !self.worker_ready.get_untracked() {
            return;
        }
        let Some(path) = self.stations_path.get_untracked() else {
            return;
        };
        let generation = self.worker_generation.get_untracked();
        let parquet = self
            .outcome
            .get_untracked()
            .filter(|_| {
                self.result
                    .get_untracked()
                    .is_some_and(|reference| reference.generation == generation)
            })
            .map(|outcome| outcome.body.parquet_path);
        let request_id = self.next_request_id();
        let mut body = json!({ "stations_ref": path.to_string_lossy() });
        if let Some(parquet) = parquet {
            body["series_parquet"] = json!(parquet);
        }
        self.pending
            .borrow_mut()
            .insert(request_id, Pending::SqlSession);
        if self
            .engine
            .send_request(JOB_ID, request_id, "sql_session", body)
            .is_err()
        {
            self.pending.borrow_mut().remove(&request_id);
        }
    }

    // ---- SQL helper (US3) --------------------------------------------------------------

    /// Stations of the confirmed snapshot as `(id, name)`.
    pub fn station_rows(&self) -> Vec<(String, Option<String>)> {
        self.snapshot
            .get_untracked()
            .map(|snapshot| {
                snapshot
                    .rows
                    .iter()
                    .map(|row| (row.station_id.clone(), row.name.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Ids selected through the list or typed into the filter.
    pub fn selected_ids(&self) -> Vec<String> {
        self.station_filter
            .get()
            .split(',')
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(str::to_owned)
            .collect()
    }

    pub fn set_selected(&self, ids: Vec<String>) {
        self.station_filter.set(ids.join(", "));
    }

    pub fn toggle_station(&self, id: &str, selected: bool) {
        let mut ids = self.selected_ids();
        ids.retain(|existing| existing != id);
        if selected {
            ids.push(id.to_owned());
        }
        self.set_selected(ids);
    }

    /// The lead window to view, clamped to the received range (never resubmits).
    fn clamped_view_window(&self) -> Option<([u32; 2], [u32; 2])> {
        let outcome = self.outcome.get_untracked()?;
        let effective = outcome.body.effective_window.effective;
        let parse = |value: String, fallback: u32| value.trim().parse::<u32>().unwrap_or(fallback);
        let view = [
            parse(self.view_start.get_untracked(), effective[0]),
            parse(self.view_end.get_untracked(), effective[1]),
        ];
        let (clamped, changed) = clamp_view_window(view, effective);
        self.view_note.set(if changed {
            format!(
                "View window clamped to the received {}–{} h.",
                effective[0], effective[1]
            )
        } else {
            String::new()
        });
        Some((clamped, effective))
    }

    /// Long-format SQL over `station_series` for the selected stations and view window.
    pub fn filtered_sql(&self) -> Option<String> {
        let (window, _) = self.clamped_view_window()?;
        let run_time = self.outcome.get_untracked()?.body.run.reference_time_utc;
        let mut predicates = Vec::new();
        if let (Some(from), Some(to)) = (
            add_hours_iso(&run_time, window[0]),
            add_hours_iso(&run_time, window[1]),
        ) {
            predicates.push(format!("valid_time >= '{from}' AND valid_time <= '{to}'"));
        }
        let ids = self
            .selected_ids()
            .iter()
            .map(|id| format!("'{}'", id.replace('\'', "''")))
            .collect::<Vec<_>>();
        if !ids.is_empty() {
            predicates.push(format!("station_id IN ({})", ids.join(", ")));
        }
        let predicate = if predicates.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", predicates.join(" AND "))
        };
        Some(format!(
            "SELECT station_id, valid_time, value FROM station_series{predicate} ORDER BY station_id, valid_time"
        ))
    }

    /// Wide SQL with one labelled column per selected station, for the chart.
    pub fn chart_sql(&self) -> Option<String> {
        let (window, _) = self.clamped_view_window()?;
        let run_time = self.outcome.get_untracked()?.body.run.reference_time_utc;
        let all = self.station_rows();
        let selected = self.selected_ids();
        let mut chosen: Vec<_> = if selected.is_empty() {
            all
        } else {
            all.into_iter()
                .filter(|(id, _)| selected.contains(id))
                .collect()
        };
        if chosen.len() > MAX_PIVOT_STATIONS {
            self.view_note.update(|note| {
                if !note.is_empty() {
                    note.push(' ');
                }
                note.push_str(&format!(
                    "Chart shows the first {MAX_PIVOT_STATIONS} of {} stations.",
                    chosen.len()
                ));
            });
            chosen.truncate(MAX_PIVOT_STATIONS);
        }
        Some(pivot_series_sql(&chosen, &run_time, window))
    }

    // ---- export provenance -------------------------------------------------------------

    pub fn mark_export_committed(&self, result_id: &str, path: PathBuf) {
        self.saved_exports.update(|map| {
            map.insert(result_id.to_owned(), path);
        });
    }

    pub fn preview_binding(&self) -> (Option<String>, String, String, [u32; 2], u32) {
        let stations_hash = self
            .snapshot
            .get_untracked()
            .map(|snapshot| snapshot.content_sha256);
        let catalog_version = self
            .catalog
            .get_untracked()
            .map(|catalog| catalog.version)
            .unwrap_or_else(|| "omo-catalog/1".to_owned());
        let run_ref = self
            .outcome
            .get_untracked()
            .map(|outcome| outcome.body.run.reference_time_utc)
            .unwrap_or_else(|| "latest_complete".to_owned());
        let window = self
            .parsed_window()
            .map(|window| [window.start_h, window.end_h])
            .unwrap_or([0, 168]);
        let max_distance_m = self.parsed_max_distance().unwrap_or(7200);
        (
            stations_hash,
            catalog_version,
            run_ref,
            window,
            max_distance_m,
        )
    }

    /// `m1_sidecar` for an export job snapshot (contracts/export-sidecar-v2.md).
    pub fn m1_sidecar(
        &self,
        exported_sql: Option<&str>,
        exported_result_id: &str,
    ) -> Option<Value> {
        let entry = self.selected_catalog_entry()?;
        let snapshot = self.snapshot.get_untracked();
        let outcome = self.outcome.get_untracked();
        let catalog_version = self
            .catalog
            .get_untracked()
            .map(|catalog| catalog.version)
            .unwrap_or_default();
        let is_series_result = outcome
            .as_ref()
            .is_some_and(|outcome| outcome.result_id == exported_result_id);
        let query = if is_series_result { None } else { exported_sql };
        Some(build_m1_sidecar_json(
            snapshot.as_ref().map(|s| s.content_sha256.as_str()),
            snapshot.as_ref().map_or(0, |s| s.rows.len()),
            &entry,
            &catalog_version,
            outcome.as_ref().map(|outcome| &outcome.body),
            query,
        ))
    }

    // ---- worker events -----------------------------------------------------------------

    /// Handle a worker event for an M1 request. Returns `true` when the event was consumed.
    pub fn handle_event(&self, kind: &str, request_id: u64, generation: u64, body: &Value) -> bool {
        match kind {
            "progress" => {
                if !self.pending.borrow().contains_key(&request_id) {
                    return false;
                }
                if let Some(stage) = body.get("stage").and_then(Value::as_str) {
                    self.status.set(format!("Series {stage}…"));
                }
                true
            }
            "result" => {
                let Some(pending) = self.pending.borrow_mut().remove(&request_id) else {
                    return false;
                };
                match pending {
                    Pending::Catalog(entry_id) => {
                        match serde_json::from_value::<CatalogCheckResult>(body.clone()) {
                            Ok(check) => self.readiness.update(|map| {
                                map.insert(entry_id, check.readiness);
                            }),
                            Err(error) => self
                                .status
                                .set(format!("Invalid catalog readiness reply: {error}")),
                        }
                    }
                    Pending::SqlSession => {}
                    Pending::Series { result_id, key } => {
                        self.active_series.set(None);
                        self.accept_series(body, generation, result_id, key);
                    }
                }
                true
            }
            "failed" => {
                let Some(pending) = self.pending.borrow_mut().remove(&request_id) else {
                    return false;
                };
                let message = body
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("worker request failed")
                    .to_owned();
                let detail = parse_fail_kind(body)
                    .map(|kind| format!("{kind:?}"))
                    .unwrap_or_default();
                match pending {
                    Pending::Catalog(entry_id) => {
                        self.readiness.update(|map| {
                            map.insert(
                                entry_id,
                                Readiness::NotReady {
                                    reason: omo_protocol::series::NotReadyReason::RemoteUnavailable,
                                    detail: message.clone(),
                                },
                            );
                        });
                    }
                    Pending::SqlSession => {
                        self.status
                            .set(format!("SQL session setup failed: {message}"));
                    }
                    Pending::Series { .. } => {
                        self.active_series.set(None);
                        if self.result.get_untracked().is_some()
                            || self.outcome.get_untracked().is_some()
                        {
                            self.stale_result.set(Some(format!(
                                "Series failed ({message}); displaying the last successful result."
                            )));
                        }
                        self.status
                            .set(format!("Series failed: {message} ({detail})"));
                    }
                }
                true
            }
            _ => false,
        }
    }

    fn accept_series(&self, body: &Value, generation: u64, result_id: String, key: String) {
        let parsed: SeriesResultBody = match serde_json::from_value(body.clone()) {
            Ok(parsed) => parsed,
            Err(error) => {
                self.status
                    .set(format!("Invalid series result reply: {error}"));
                return;
            }
        };
        let Ok(reference) = serde_json::from_value::<ResultRef>(parsed.result_ref.clone()) else {
            self.status
                .set("Series result omitted a usable result reference.".into());
            return;
        };
        if reference.generation != generation {
            self.status
                .set("Discarded a series result from an older worker generation.".into());
            return;
        }
        let matches_path = parsed
            .matches_ref
            .get("path")
            .and_then(Value::as_str)
            .map(PathBuf::from);
        let plot = parsed
            .plot_ref
            .clone()
            .and_then(|value| serde_json::from_value::<ResultRef>(value).ok())
            .filter(|plot| plot.generation == generation)
            .map(|plot| PlotInput {
                source_rows: reference.rows,
                reference: plot,
                worker_sampled: true,
            });
        let entry_id = self.selected_entry.get_untracked();
        let this = self.clone();
        let status = self.status;
        spawn(async move {
            let matches = match matches_path {
                Some(path) => blocking(move || read_matches_arrow(&path)).await,
                None => Ok(Err("series result omitted the matches file".to_owned())),
            };
            let matches = match matches {
                Ok(Ok(rows)) => rows,
                Ok(Err(message)) => {
                    status.set(format!("Could not read the match report: {message}"));
                    Vec::new()
                }
                Err(error) => {
                    status.set(format!("Match report task failed: {error}"));
                    Vec::new()
                }
            };
            if this.worker_generation.get_untracked() != generation || this.current_key() != key {
                return;
            }
            let matched = matches.iter().filter(|m| m.status == "Matched").count();
            let total = matches.len();
            let rows = reference.rows;
            this.result.set(Some(reference));
            this.plot_input.set(plot);
            this.stale_result.set(None);
            let effective = parsed.effective_window.effective;
            this.view_start.set(effective[0].to_string());
            this.view_end.set(effective[1].to_string());
            this.view_note.set(String::new());
            this.outcome.set(Some(SeriesOutcome {
                body: parsed,
                matches,
                key,
                result_id,
                entry_id,
            }));
            status.set(format!(
                "Series received, not saved: {rows} rows, {matched}/{total} stations matched."
            ));
            this.sync_sql_session();
        });
    }
}

// ---- views -----------------------------------------------------------------------------

const LABEL: &str = "text-[10px] font-semibold tracking-widest text-[#7d877f] dark:text-[#9ba69e]";
const HINT: &str = "text-xs leading-5 text-[#64736a] dark:text-[#a4ada6]";
const BTN: &str = "h-8 rounded-md border border-[#d6d1c5] px-3 text-xs font-semibold hover:bg-[#eeece5] dark:border-[#404740] dark:hover:bg-[#252c27]";
const BTN_PRIMARY: &str =
    "h-8 rounded-md bg-[#24584c] px-3 text-xs font-semibold text-white hover:bg-[#1c493e]";
const FIELD: &str = "h-8 w-24 rounded-md border border-[#d6d1c5] bg-white px-2 text-xs dark:border-[#404740] dark:bg-[#202722]";

fn mapping_value(draft: &StationImportDraft, field: &str) -> String {
    let value = match field {
        "id" => Some(draft.mapping.id.as_str()),
        "lat" => Some(draft.mapping.lat.as_str()),
        "lon" => Some(draft.mapping.lon.as_str()),
        "name" => draft.mapping.name.as_deref(),
        "elevation" => draft.mapping.elevation.as_deref(),
        _ => None,
    };
    match value {
        Some(value) if !value.is_empty() => value.to_owned(),
        _ => "— not mapped —".to_owned(),
    }
}

fn mapping_row(
    controller: &SeriesController,
    field: &'static str,
    label: &'static str,
) -> impl IntoElement {
    let draft = controller.draft;
    let cycle = controller.clone();
    view()
        .app_class("flex-row items-center gap-3")
        .child(text(label).app_class("w-24 text-xs text-[#7d877f] dark:text-[#9ba69e]"))
        .child(
            button()
                .app_class(BTN)
                .on_press(move || cycle.cycle_mapping(field))
                .child(text(move || {
                    draft
                        .get()
                        .map(|draft| mapping_value(&draft, field))
                        .unwrap_or_default()
                })),
        )
}

fn import_section(controller: &SeriesController) -> impl IntoElement {
    let draft = controller.draft;
    let preview = computed(move || {
        let Some(draft) = draft.get() else {
            return String::new();
        };
        let Some(parsed) = &draft.parsed else {
            return String::new();
        };
        let mut lines = vec![parsed.headers.join(" | ")];
        lines.extend(parsed.sample_rows.iter().map(|row| row.join(" | ")));
        lines.join("\n")
    });
    let issues = computed(move || {
        let Some(draft) = draft.get() else {
            return String::new();
        };
        draft
            .issues
            .iter()
            .take(50)
            .map(|issue| format!("line {} · {} · {:?}", issue.line, issue.field, issue.kind))
            .collect::<Vec<_>>()
            .join("\n")
    });
    let message = computed(move || {
        draft
            .get()
            .and_then(|draft| draft.error)
            .unwrap_or_default()
    });
    let large_warning = computed(move || {
        draft
            .get()
            .and_then(|draft| draft.large_warning_message())
            .unwrap_or_default()
    });
    let large_confirmed =
        computed(move || draft.get().is_some_and(|draft| draft.large_file_confirmed));
    let has_large_warning = computed(move || {
        draft
            .get()
            .is_some_and(|draft| draft.exceeds_large_warning())
    });
    let can_confirm = computed(move || draft.get().is_some_and(|draft| draft.can_confirm()));
    let confirm = controller.clone();
    let cancel = controller.clone();
    let delimiter = controller.clone();
    let toggle_large = controller.clone();
    view()
        .app_class("flex-col gap-2 rounded-lg border border-[#dedacf] p-3 dark:border-[#343b36]")
        .child(text("STATION IMPORT").app_class(LABEL))
        .child(text(move || message.get()).app_class(HINT))
        .child(
            text(move || large_warning.get())
                .app_class("text-xs text-[#7a5a1b] dark:text-[#e3c782]"),
        )
        .child(
            button()
                .app_class(BTN)
                .on_press(move || {
                    if has_large_warning.get_untracked() {
                        toggle_large.toggle_large_import_confirmed();
                    }
                })
                .child(text(move || {
                    if !has_large_warning.get() {
                        "Row count check: ≤ 1,000 rows".to_owned()
                    } else if large_confirmed.get() {
                        "Large import (>1,000 rows) confirmed ✓".to_owned()
                    } else {
                        "Acknowledge >1,000 station rows".to_owned()
                    }
                })),
        )
        .child(
            button()
                .app_class(BTN)
                .on_press(move || delimiter.toggle_delimiter())
                .child(text(move || match draft.get().and_then(|d| d.delimiter) {
                    Some(b';') => "Delimiter: semicolon (click to switch)".to_owned(),
                    Some(_) => "Delimiter: comma (click to switch)".to_owned(),
                    None => "Delimiter: choose (click)".to_owned(),
                })),
        )
        .child(mapping_row(controller, "id", "Station id"))
        .child(mapping_row(controller, "lat", "Latitude"))
        .child(mapping_row(controller, "lon", "Longitude"))
        .child(mapping_row(controller, "name", "Name"))
        .child(mapping_row(controller, "elevation", "Elevation"))
        .child(scroll().app_class("max-h-[140px]").child(
            text(move || preview.get()).app_class("whitespace-pre font-mono text-[10px] leading-4"),
        ))
        .child(
            scroll().app_class("max-h-[120px]").child(
                text(move || issues.get())
                    .app_class("whitespace-pre text-[11px] leading-4 text-[#b05243]"),
            ),
        )
        .child(
            view()
                .app_class("flex-row gap-2")
                .child(
                    button()
                        .app_class(BTN_PRIMARY)
                        .on_press(move || confirm.confirm_import())
                        .child(text(move || {
                            if can_confirm.get() {
                                "Confirm import".to_owned()
                            } else {
                                "Confirm import (fix issues first)".to_owned()
                            }
                        })),
                )
                .child(
                    button()
                        .app_class(BTN)
                        .on_press(move || cancel.cancel_import())
                        .child(text("Discard")),
                ),
        )
}

fn catalog_section(controller: &SeriesController) -> impl IntoElement {
    let catalog = controller.catalog;
    let selected = controller.selected_entry;
    let readiness = controller.readiness;
    let summary = computed(move || {
        let Some(catalog) = catalog.get() else {
            return "Catalog unavailable.".to_owned();
        };
        let Some(entry) = catalog.entry_by_id(&selected.get()) else {
            return "No catalog entry selected.".to_owned();
        };
        let state = match readiness.get().get(&entry.id) {
            None => "Checking readiness…".to_owned(),
            Some(Readiness::Ready { run }) => format!(
                "Ready · latest complete run {} ({} cycle, {} h horizon)",
                run.reference_time_utc, run.cycle, run.horizon_h
            ),
            Some(Readiness::NotReady { reason, detail }) => {
                format!("Not ready: {reason:?} — {detail}")
            }
        };
        let cycles = entry
            .cycles
            .iter()
            .map(|(cycle, horizon)| format!("{cycle}Z→{horizon} h"))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "{} {} · {} ({})\nTime steps: hourly to 90 h, 3-hourly to 144 h, 6-hourly after\nHorizons: {}\n{}\n{}",
            entry.product.provider,
            entry.product.model,
            entry.variable.label,
            entry.variable.unit,
            cycles,
            state,
            entry.attribution.join("\n"),
        )
    });
    let cycle = controller.clone();
    let refresh = controller.clone();
    view()
        .app_class("flex-col gap-2 rounded-lg border border-[#dedacf] p-3 dark:border-[#343b36]")
        .child(text("CATALOG").app_class(LABEL))
        .child(
            view()
                .app_class("flex-row gap-2")
                .child(
                    button()
                        .app_class(BTN)
                        .on_press(move || cycle.cycle_entry())
                        .child(text(move || format!("Entry: {}", selected.get()))),
                )
                .child(
                    button()
                        .app_class(BTN)
                        .on_press(move || refresh.check_all())
                        .child(text("Refresh readiness")),
                ),
        )
        .child(text(move || summary.get()).app_class("whitespace-pre-wrap text-xs leading-5"))
}

fn run_section(controller: &SeriesController, apply_sql: Rc<dyn Fn(String)>) -> impl IntoElement {
    let run = controller.clone();
    let cancel = controller.clone();
    let show = controller.clone();
    let show_chart = controller.clone();
    let apply_chart = Rc::clone(&apply_sql);
    let station_filter = controller.station_filter;
    let window_start = controller.window_start;
    let window_end = controller.window_end;
    let max_distance = controller.max_distance;
    let outcome = controller.outcome;
    let saved_exports = controller.saved_exports;
    let status = controller.status;
    let stale = {
        let controller = controller.clone();
        computed(move || {
            let current = controller.current_key();
            match controller.outcome.get() {
                Some(outcome) if outcome.key != current => {
                    "Result is out of date: stations, catalog, window, or distance changed."
                        .to_owned()
                }
                _ => String::new(),
            }
        })
    };
    let badge = computed(move || {
        let Some(outcome) = outcome.get() else {
            return String::new();
        };
        let window = &outcome.body.effective_window;
        let saved = saved_exports.get();
        let state_label = match saved.get(&outcome.result_id) {
            Some(path) => {
                let name = path
                    .file_name()
                    .and_then(|v| v.to_str())
                    .map(str::to_owned)
                    .unwrap_or_else(|| path.display().to_string());
                format!("Saved ({name})")
            }
            None => "Received, not saved".to_owned(),
        };
        let mut lines = vec![format!(
            "{state_label} · run {} · plan {}",
            outcome.body.run.reference_time_utc, outcome.body.plan
        )];
        if window.truncated {
            lines.push(format!(
                "{}–{} h of requested {}–{} h",
                window.effective[0], window.effective[1], window.requested[0], window.requested[1]
            ));
        }
        lines.join("\n")
    });
    let no_match = computed(move || {
        let Some(outcome) = outcome.get() else {
            return String::new();
        };
        let matched = outcome
            .matches
            .iter()
            .filter(|m| m.status == "Matched")
            .count();
        let mut lines = vec![format!(
            "Matched {matched} of {} stations",
            outcome.matches.len()
        )];
        lines.extend(
            outcome
                .matches
                .iter()
                .filter(|m| m.status != "Matched")
                .take(100)
                .map(|m| format!("{} — no match: {}", m.station_id, m.reason)),
        );
        lines.join("\n")
    });
    view()
        .app_class("flex-col gap-2 rounded-lg border border-[#dedacf] p-3 dark:border-[#343b36]")
        .child(text("SERIES").app_class(LABEL))
        .child(
            view()
                .app_class("flex-row items-center gap-2")
                .child(text("Window (h)").app_class("text-xs"))
                .child(
                    input()
                        .value(move || window_start.get())
                        .on_change(move |value: &str| window_start.set(value.to_owned()))
                        .app_class(FIELD),
                )
                .child(text("to").app_class("text-xs"))
                .child(
                    input()
                        .value(move || window_end.get())
                        .on_change(move |value: &str| window_end.set(value.to_owned()))
                        .app_class(FIELD),
                )
                .child(text("Max distance (m)").app_class("text-xs"))
                .child(
                    input()
                        .value(move || max_distance.get())
                        .on_change(move |value: &str| max_distance.set(value.to_owned()))
                        .app_class(FIELD),
                ),
        )
        .child(
            view()
                .app_class("flex-row gap-2")
                .child(
                    button()
                        .app_class(BTN_PRIMARY)
                        .on_press(move || run.run_series())
                        .child(text("Run series")),
                )
                .child(
                    button()
                        .app_class(BTN)
                        .on_press(move || cancel.cancel_series())
                        .child(text("Cancel")),
                ),
        )
        .child(text(move || status.get()).app_class(HINT))
        .child(text(move || badge.get()).app_class("whitespace-pre-wrap text-xs font-semibold text-[#24584c] dark:text-[#a6c5b6]"))
        .child(text(move || stale.get()).app_class("text-xs text-[#b05243]"))
        .child(
            scroll().app_class("max-h-[140px]").child(
                text(move || no_match.get())
                    .app_class("whitespace-pre-wrap text-[11px] leading-4"),
            ),
        )
        .child(text("Selected station ids (comma-separated, empty = all)").app_class("text-xs"))
        .child(
            view()
                .app_class("flex-row gap-2")
                .child(
                    input()
                        .value(move || station_filter.get())
                        .on_change(move |value: &str| station_filter.set(value.to_owned()))
                        .app_class("h-8 min-w-0 flex-1 rounded-md border border-[#d6d1c5] bg-white px-2 text-xs dark:border-[#404740] dark:bg-[#202722]"),
                )
                .child(
                    button()
                        .app_class(BTN)
                        .on_press(move || match show_chart.chart_sql() {
                            Some(sql) => apply_chart(sql),
                            None => show_chart.status.set("Run a series first.".into()),
                        })
                        .child(text("Chart selected")),
                )
                .child(
                    button()
                        .app_class(BTN)
                        .on_press(move || match show.filtered_sql() {
                            Some(sql) => apply_sql(sql),
                            None => show.status.set("Run a series first.".into()),
                        })
                        .child(text("Table (long)")),
                ),
        )
}

pub(crate) fn panel(
    controller: SeriesController,
    apply_sql: Rc<dyn Fn(String)>,
) -> impl IntoElement {
    let open = controller.clone();
    let use_path = controller.clone();
    let source_path = controller.source_path;
    let snapshot = controller.snapshot;
    let snapshot_text = computed(move || match snapshot.get() {
        Some(snapshot) => format!(
            "{} stations confirmed · {}",
            snapshot.rows.len(),
            snapshot.source.orig_path
        ),
        None => "No stations confirmed yet.".to_owned(),
    });
    let content = view()
        .app_class("flex-col gap-3 p-4 text-[#24312c] dark:text-[#e6e9e2]")
        .child(
            view()
                .app_class("flex-col gap-2 rounded-lg border border-[#dedacf] p-3 dark:border-[#343b36]")
                .child(text("STATIONS").app_class(LABEL))
                .child(text(move || snapshot_text.get()).app_class("text-xs"))
                .child(
                    view()
                        .app_class("flex-row gap-2")
                        .child(
                            button()
                                .app_class(BTN)
                                .on_press(move || open.open_csv())
                                .child(text("Open CSV…")),
                        )
                        .child(
                            input()
                                .value(move || source_path.get())
                                .placeholder("Absolute CSV path")
                                .on_change(move |value: &str| source_path.set(value.to_owned()))
                                .app_class("h-8 min-w-0 flex-1 rounded-md border border-[#d6d1c5] bg-white px-2 text-xs dark:border-[#404740] dark:bg-[#202722]"),
                        )
                        .child(
                            button()
                                .app_class(BTN)
                                .on_press(move || use_path.load_path(source_path.get_untracked()))
                                .child(text("Use path")),
                        ),
                ),
        )
        .child(import_section(&controller))
        .child(catalog_section(&controller))
        .child(run_section(&controller, apply_sql))
        .child(super::stations_view::panel(&controller))
        .child(super::window_control::panel(&controller));
    scroll()
        .app_class("size-full bg-[#fbfaf6] dark:bg-[#171c19]")
        .child(content)
}
