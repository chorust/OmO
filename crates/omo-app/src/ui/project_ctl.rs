//! Project document state for the workbench: dirty tracking, save/open/new, close guard,
//! auto-draft staging, and startup recovery (research R-05, R-07).

use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
    time::{Duration, Instant},
};

use nsh_base::alert_dialog;
use nsh_core::{desktop::WindowAttributes, element::desktop::main_window, prelude::*};
use omo_project::{
    doc::{CatalogSelection, ProjectDoc, SqlTab},
    new_doc_id,
    preview_key::PreviewWindow,
};

use super::{map_view::MapController, series_panel::SeriesController};
use crate::host::{project_io, series_host::now_utc_iso};

const DRAFT_DEBOUNCE: Duration = Duration::from_secs(2);
const DRAFT_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Clone)]
pub(super) struct ProjectController {
    series: SeriesController,
    map: MapController,
    sql: Signal<String>,
    on_edit: Rc<dyn Fn(String)>,
    doc: Rc<RefCell<ProjectDoc>>,
    path: Signal<Option<PathBuf>>,
    saved_fingerprint: Signal<String>,
    staged_fingerprint: Signal<Option<String>>,
    pub status: Signal<String>,
}

impl ProjectController {
    pub fn new(
        series: SeriesController,
        map: MapController,
        sql: Signal<String>,
        on_edit: Rc<dyn Fn(String)>,
    ) -> Self {
        let doc = ProjectDoc::new(new_doc_id());
        map.set_doc_id(doc.doc_id().to_owned());
        let this = Self {
            series,
            map,
            sql,
            on_edit,
            doc: Rc::new(RefCell::new(doc)),
            path: signal(None),
            saved_fingerprint: signal(String::new()),
            staged_fingerprint: signal(None),
            status: signal(String::new()),
        };
        this.saved_fingerprint.set(this.fingerprint());
        this
    }

    /// Cheap change fingerprint; tracks every persisted input reactively.
    pub fn fingerprint(&self) -> String {
        format!(
            "{}|{}|{}|{}|{}|{}|{}|{}",
            self.series
                .snapshot
                .get()
                .map(|snapshot| snapshot.content_sha256)
                .unwrap_or_default(),
            self.series.selected_entry.get(),
            self.series.window_start.get(),
            self.series.window_end.get(),
            self.series.max_distance.get(),
            self.series.station_filter.get(),
            self.sql.get(),
            self.map.state_fingerprint(),
        )
    }

    pub fn is_dirty(&self) -> bool {
        self.fingerprint() != self.saved_fingerprint.get()
    }

    pub fn is_draft_staged(&self) -> bool {
        let fp = self.fingerprint();
        fp != self.saved_fingerprint.get()
            && self.staged_fingerprint.get().as_deref() == Some(fp.as_str())
    }

    pub fn title(&self) -> String {
        let name = self
            .path
            .get()
            .as_deref()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .map_or_else(|| "Untitled project".to_owned(), str::to_owned);
        let state = if !self.is_dirty() {
            ""
        } else if self.is_draft_staged() {
            " ● [draft staged]"
        } else {
            " ● [edited]"
        };
        format!("{name}{state} — Omobench Studio")
    }

    fn snapshot_doc(&self) -> ProjectDoc {
        let mut doc = self.doc.borrow().clone();
        doc.set_path(self.path.get_untracked());
        doc.stations = self.series.snapshot.get_untracked();
        let catalog_version = self
            .series
            .catalog
            .get_untracked()
            .map(|catalog| catalog.version)
            .unwrap_or_default();
        doc.catalog_selection = CatalogSelection {
            entry_id: self.series.selected_entry.get_untracked(),
            catalog_version,
        };
        let default = PreviewWindow::default();
        doc.window = PreviewWindow {
            start_h: self
                .series
                .window_start
                .get_untracked()
                .trim()
                .parse()
                .unwrap_or(default.start_h),
            end_h: self
                .series
                .window_end
                .get_untracked()
                .trim()
                .parse()
                .unwrap_or(default.end_h),
        };
        doc.max_distance_m = self
            .series
            .max_distance
            .get_untracked()
            .trim()
            .parse()
            .unwrap_or(7200);
        doc.sql_tabs = vec![SqlTab {
            title: "Query 1".into(),
            sql: self.sql.get_untracked(),
        }];
        doc.view.selected_station_ids = self
            .series
            .station_filter
            .get_untracked()
            .split(',')
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(str::to_owned)
            .collect();
        doc.map = self.map.snapshot_state();
        doc
    }

    fn apply_doc(&self, doc: ProjectDoc, stations_path: Option<PathBuf>, dirty: bool) {
        // The map first: its saved viewport must be in place before the new stations trigger
        // the automatic framing.
        self.map.set_doc_id(doc.doc_id().to_owned());
        self.map.apply_state(doc.map.clone());
        self.path.set(doc.path().map(Path::to_path_buf));
        self.series.snapshot.set(doc.stations.clone());
        self.series.stations_path.set(stations_path);
        self.series
            .selected_entry
            .set(doc.catalog_selection.entry_id.clone());
        self.series.window_start.set(doc.window.start_h.to_string());
        self.series.window_end.set(doc.window.end_h.to_string());
        self.series.max_distance.set(doc.max_distance_m.to_string());
        self.series
            .station_filter
            .set(doc.view.selected_station_ids.join(", "));
        self.series.outcome.set(None);
        self.series.draft.set(None);
        if let Some(tab) = doc.sql_tabs.first() {
            (self.on_edit)(tab.sql.clone());
        }
        *self.doc.borrow_mut() = doc;
        self.staged_fingerprint.set(None);
        self.saved_fingerprint.set(if dirty {
            String::new()
        } else {
            self.fingerprint()
        });
        self.series.sync_sql_session();
    }

    // ---- save / open / new -------------------------------------------------------------

    pub fn save(&self, then: Option<Rc<dyn Fn()>>) {
        match self.path.get_untracked() {
            Some(path) => self.write(path, then),
            None => self.save_as(then),
        }
    }

    pub fn save_as(&self, then: Option<Rc<dyn Fn()>>) {
        let this = self.clone();
        let status = self.status;
        super::dialogs::choose_project_save(
            move |mut path| {
                if path.extension().is_none() {
                    path.set_extension("omoproj");
                }
                this.write(path, then.clone());
            },
            move |error| status.set(format!("Save chooser unavailable: {error}")),
        );
    }

    fn write(&self, target: PathBuf, then: Option<Rc<dyn Fn()>>) {
        let mut doc = self.snapshot_doc();
        let fingerprint = self.fingerprint();
        let this = self.clone();
        self.status.set(format!("Saving {}…", target.display()));
        spawn(async move {
            let app_data = project_io::app_data_dir();
            let saved = blocking(move || {
                let result = doc.save_to_path(&target, Some(&app_data));
                (doc, result)
            })
            .await;
            match saved {
                Ok((doc, Ok(()))) => {
                    this.path.set(doc.path().map(Path::to_path_buf));
                    this.status.set(format!(
                        "Saved {}",
                        doc.path()
                            .map(|p| p.display().to_string())
                            .unwrap_or_default()
                    ));
                    *this.doc.borrow_mut() = doc;
                    this.staged_fingerprint.set(None);
                    this.saved_fingerprint.set(fingerprint);
                    if let Some(then) = then {
                        then();
                    }
                }
                Ok((_, Err(error))) => this.status.set(format!("Save failed: {error}")),
                Err(error) => this.status.set(format!("Save task failed: {error}")),
            }
        });
    }

    pub fn open(&self) {
        let this = self.clone();
        let status = self.status;
        super::dialogs::choose_project_open(
            move |path| {
                let this = this.clone();
                this.guard(Rc::new({
                    let this = this.clone();
                    let path = path.clone();
                    move || this.open_path(path.clone())
                }));
            },
            move |error| status.set(format!("Open chooser unavailable: {error}")),
        );
    }

    pub fn open_path(&self, path: PathBuf) {
        let this = self.clone();
        self.status.set(format!("Opening {}…", path.display()));
        spawn(async move {
            let app_data = project_io::app_data_dir();
            let target = path.clone();
            let opened = blocking(move || project_io::open_project(&target, &app_data)).await;
            match opened {
                Ok(Ok(opened)) => {
                    let mut doc = opened.outcome.doc;
                    doc.set_path(Some(path.clone()));
                    this.apply_doc(doc, opened.canonical_stations_path, false);
                    this.status.set(
                        opened
                            .outcome
                            .source_changed_notice
                            .unwrap_or_else(|| format!("Opened {}", path.display())),
                    );
                }
                Ok(Err(error)) => this.status.set(format!("Open failed: {error}")),
                Err(error) => this.status.set(format!("Open task failed: {error}")),
            }
        });
    }

    pub fn new_project(&self) {
        let this = self.clone();
        self.guard(Rc::new(move || {
            let mut doc = ProjectDoc::new(new_doc_id());
            doc.set_path(None);
            if let Some(entry) = this
                .series
                .catalog
                .get_untracked()
                .and_then(|catalog| catalog.default_entry().cloned())
            {
                doc.catalog_selection.entry_id = entry.id;
                doc.window = PreviewWindow {
                    start_h: entry.default_window_h[0],
                    end_h: entry.default_window_h[1],
                };
                doc.max_distance_m = entry.max_distance_m;
            }
            this.apply_doc(doc, None, false);
            this.status.set("New project.".into());
        }));
    }

    /// Run `then` immediately when clean; otherwise offer Save / Discard / Cancel first.
    pub fn guard(&self, then: Rc<dyn Fn()>) {
        if !self.is_dirty() {
            then();
            return;
        }
        let this = self.clone();
        spawn(async move {
            let answer = alert_dialog()
                .title("Save changes to this project?")
                .description("Unsaved changes will be lost if you discard them.")
                .action("save", "Save")
                .destructive("discard", "Discard")
                .cancel("cancel", "Cancel")
                .show()
                .await;
            match answer {
                Ok(answer) => match answer.action.as_deref() {
                    Some("save") => this.save(Some(then)),
                    Some("discard") => {
                        let app_data = project_io::app_data_dir();
                        let doc_id = this.doc.borrow().doc_id().to_owned();
                        let _ =
                            blocking(move || omo_project::draft::discard(&app_data, &doc_id)).await;
                        then();
                    }
                    _ => {}
                },
                Err(error) => this.status.set(format!("Close guard unavailable: {error}")),
            }
        });
    }

    // ---- window integration ------------------------------------------------------------

    /// Reactive window title and the unsaved-changes close guard. Call while rendering.
    pub fn install_window(&self) {
        let title_source = self.clone();
        effect(move || {
            let title = title_source.title();
            let mut attributes = WindowAttributes::default();
            attributes.title = Some(title);
            attributes.close_on_press = Some(false);
            main_window().set(attributes);
        });
        let this = self.clone();
        main_window().on_close_press(move |_reason| {
            let close = Rc::new(|| main_window().close());
            this.guard(close);
        });
    }

    // ---- drafts ------------------------------------------------------------------------

    pub fn start_draft_scheduler(&self) {
        let this = self.clone();
        spawn(async move {
            let mut last_fingerprint = String::new();
            let mut last_change = Instant::now();
            let mut last_staged = Instant::now();
            let mut staged_fingerprint = String::new();
            loop {
                timer(Duration::from_millis(500)).await;
                let fingerprint = this.fingerprint();
                if fingerprint != last_fingerprint {
                    last_fingerprint = fingerprint.clone();
                    last_change = Instant::now();
                }
                if fingerprint == this.saved_fingerprint.get_untracked()
                    || fingerprint == staged_fingerprint
                {
                    continue;
                }
                if last_change.elapsed() < DRAFT_DEBOUNCE && last_staged.elapsed() < DRAFT_INTERVAL
                {
                    continue;
                }
                let doc = this.snapshot_doc();
                let app_data = project_io::app_data_dir();
                let now = now_utc_iso();
                let staged =
                    blocking(move || omo_project::draft::stage(&doc, &app_data, &now)).await;
                match staged {
                    Ok(Ok(_)) => {
                        staged_fingerprint = fingerprint.clone();
                        this.staged_fingerprint.set(Some(fingerprint));
                        this.status.set("Draft staged (project not saved).".into());
                        last_staged = Instant::now();
                    }
                    Ok(Err(error)) => this.status.set(format!("Draft not saved: {error}")),
                    Err(error) => this.status.set(format!("Draft task failed: {error}")),
                }
            }
        });
    }

    pub fn offer_recovery(&self) {
        let this = self.clone();
        spawn(async move {
            let app_data = project_io::app_data_dir();
            let scan_dir = app_data.clone();
            let found = blocking(move || omo_project::draft::list_recoverable(&scan_dir)).await;
            let Ok(Ok(mut drafts)) = found else {
                return;
            };
            drafts.sort_by(|a, b| b.draft.staged_at_utc.cmp(&a.draft.staged_at_utc));
            let Some(latest) = drafts.into_iter().next() else {
                return;
            };
            let answer = alert_dialog()
                .title("Recover unsaved project?")
                .description(format!(
                    "An unsaved draft from {} was found.",
                    latest.draft.staged_at_utc
                ))
                .action("restore", "Restore")
                .destructive("discard", "Discard")
                .show()
                .await;
            let doc_id = latest.draft.doc_id.clone();
            match answer {
                Ok(answer) if answer.action.as_deref() == Some("restore") => {
                    let mut doc = latest.draft.doc.clone();
                    doc.set_path(latest.draft.project_path.clone());
                    let doc_for_io = doc.clone();
                    let stations_path =
                        blocking(move || project_io::materialize_stations(&doc_for_io, &app_data))
                            .await
                            .ok()
                            .flatten();
                    this.apply_doc(doc, stations_path, true);
                    this.staged_fingerprint.set(Some(this.fingerprint()));
                    this.status
                        .set("Recovered unsaved draft (not yet saved to project file).".into());
                }
                Ok(answer) if answer.action.as_deref() == Some("discard") => {
                    let _ = blocking(move || omo_project::draft::discard(&app_data, &doc_id)).await;
                }
                _ => {}
            }
        });
    }
}
