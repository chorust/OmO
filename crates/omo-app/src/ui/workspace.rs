//! Main Omobench workbench composition.

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    path::PathBuf,
    rc::Rc,
    time::Duration,
};

use nsh_base::{
    button,
    dock::{
        DockArea, DockAreaApi, DockAreaLayout, DockLayout, DockPanel, DockPanelOptions,
        DockPlacement, DockRegion, DockTab, dock_area,
    },
    input,
};
use nsh_core::{
    desktop::MenuRole,
    element::{
        Root, Rows,
        desktop::{MenuItem, set_application_menu},
    },
    prelude::*,
};
use omo_project::PreviewKey;
use omo_protocol::{
    messages::parse_fail_kind,
    result_ref::ResultRef,
    state::{CancelLevel, FailKind},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::host::{
    preview_queue::{Completion, PreviewOutcome, PreviewQueue, PreviewRequest, QueueAction},
    runtime::{self, EngineHandle, RuntimeEvent},
};

use super::{dialogs, plot_view, sql_view, table_view, tasks_view};

const INITIAL_SQL: &str = "-- Edit this query, then run a preview.\nSELECT 1 AS value";
const INITIAL_SQL_ENV: &str = "OMOBENCH_INITIAL_SQL";
const PROGRESS_HZ_ENV: &str = "OMOBENCH_PROGRESS_HZ";
const DOC_ID: &str = "untitled-project";
const JOB_ID: u64 = 1;

fn sorted_query(sql: &str, sort: Option<nsh_base::DataTableSort>) -> String {
    let Some(sort) = sort else {
        return sql.to_owned();
    };
    let query = sql.trim().trim_end_matches(';').trim();
    let column = sort.key.to_string().replace('"', "\"\"");
    let direction = match sort.direction {
        nsh_base::DataTableSortDirection::Ascending => "ASC",
        nsh_base::DataTableSortDirection::Descending => "DESC",
    };
    format!("SELECT * FROM ({query}) AS omo_sorted ORDER BY \"{column}\" {direction}")
}

#[derive(Clone)]
struct PendingPreview {
    request_id: u64,
    key: PreviewKey,
    sql: String,
}

fn with_removed_pending<T, R>(
    pending: &RefCell<HashMap<u64, T>>,
    request_id: u64,
    after_remove: impl FnOnce(Option<T>) -> R,
) -> R {
    let removed = pending.borrow_mut().remove(&request_id);
    after_remove(removed)
}

fn send_preview(engine: &EngineHandle, preview: &PendingPreview) -> Result<(), String> {
    let mut body = json!({
        "sql": preview.sql,
        "purpose": "preview",
        "limits": {"rows": 100_000, "bytes": 64 * 1024 * 1024}
    });
    if let Some(hz) = std::env::var(PROGRESS_HZ_ENV)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
    {
        body["progress_hz"] = json!(hz.min(200));
    }
    engine.send_request(JOB_ID, preview.request_id, "query", body)
}

pub fn root() -> Root {
    Root::of::<Workspace>()
}

#[component]
fn Workspace() -> impl IntoElement {
    let sql = signal(std::env::var(INITIAL_SQL_ENV).unwrap_or_else(|_| String::from(INITIAL_SQL)));
    let revision = signal(0u64);
    let request_count = signal(0u64);
    let latest_preview_request = signal(0u64);
    let sort_state = signal(None::<nsh_base::DataTableSort>);
    let preview_queue = stored(PreviewQueue::default());
    let fixture_path = crate::host::fixture_result::env_path();
    let task_status = signal(if fixture_path.is_some() {
        String::from("Loading fixture result…")
    } else {
        String::from("Starting engine worker…")
    });
    let file_status = signal(String::from("No file action yet."));
    let manual_export_path = signal(String::new());
    let source_status = signal(String::from("No station CSV selected."));
    let source_path = signal(String::new());
    let result = signal(None::<ResultRef>);
    let plot_input = signal(None::<plot_view::PlotInput>);
    let stale_result = signal(None::<String>);
    let chart_rows = signal(Rows::default());
    let worker_ready = signal(false);
    let worker_generation = signal(0u64);
    let worker_state = signal(String::from("STARTING"));
    let bootstrap_report = signal(None::<Value>);
    let pending_previews = Rc::new(RefCell::new(HashMap::<u64, PendingPreview>::new()));
    let cancelled_previews = Rc::new(RefCell::new(HashSet::<u64>::new()));
    let export_requests = Rc::new(RefCell::new(HashMap::<u64, PathBuf>::new()));
    let (engine, event_receiver) = runtime::start_from_env();
    let layout = signal(default_layout());
    let dock_ref = component_ref::<DockArea>();

    if let Some(fixture_path) = fixture_path {
        let fixture_result = result;
        let fixture_status = task_status;
        let latest_preview = latest_preview_request;
        spawn(async move {
            let loaded =
                blocking(move || crate::host::fixture_result::load_from_path(&fixture_path)).await;
            if latest_preview.get_untracked() != 0 || fixture_result.get_untracked().is_some() {
                return;
            }
            match loaded {
                Ok(Ok(reference)) => {
                    let row_count = reference.rows;
                    fixture_result.set(Some(reference));
                    fixture_status.set(format!("Loaded fixture result ({row_count} rows)."));
                }
                Ok(Err(error)) => fixture_status.set(error),
                Err(error) => fixture_status.set(format!("Fixture load task failed: {error}")),
            }
        });
    }

    let dispatch_next_preview: Rc<dyn Fn()> = Rc::new({
        let preview_queue = preview_queue;
        let pending_previews = Rc::clone(&pending_previews);
        let worker_generation = worker_generation;
        let engine = engine.clone();
        let task_status = task_status;
        move || {
            let mut next = preview_queue.with(|queue| queue.take_dispatch(DOC_ID));
            while let Some(request) = next {
                let mut pending = pending_previews.borrow_mut().remove(&request.request_id);
                if let Some(pending) = pending.as_mut() {
                    let generation = worker_generation.get_untracked();
                    pending.key.worker_generation = generation;
                    preview_queue.with(|queue| {
                        queue.update_running_generation(DOC_ID, request.request_id, generation)
                    });
                }
                match pending {
                    Some(pending) => match send_preview(&engine, &pending) {
                        Ok(()) => {
                            pending_previews
                                .borrow_mut()
                                .insert(request.request_id, pending);
                            break;
                        }
                        Err(error) => {
                            preview_queue.with(|queue| {
                                queue.complete(
                                    request.request_id,
                                    PreviewOutcome::Failure(error.clone()),
                                )
                            });
                            task_status.set(format!(
                                "Could not dispatch preview {}: {error}",
                                request.request_id
                            ));
                        }
                    },
                    None => {
                        preview_queue.with(|queue| {
                            queue.complete(
                                request.request_id,
                                PreviewOutcome::Failure("preview request was lost".into()),
                            )
                        });
                        task_status.set(format!(
                            "Preview {} had no request data.",
                            request.request_id
                        ));
                    }
                }
                next = preview_queue.with(|queue| queue.take_dispatch(DOC_ID));
            }
        }
    });

    let run_preview: Rc<dyn Fn()> = Rc::new({
        let sql = sql;
        let revision = revision;
        let request_count = request_count;
        let latest_preview_request = latest_preview_request;
        let sort_state = sort_state;
        let preview_queue = preview_queue;
        let worker_ready = worker_ready;
        let worker_generation = worker_generation;
        let pending_previews = Rc::clone(&pending_previews);
        let result = result;
        let stale_result = stale_result;
        let engine = engine.clone();
        let task_status = task_status;
        move || {
            let request_id = request_count.get_untracked().saturating_add(1);
            request_count.set(request_id);
            latest_preview_request.set(request_id);
            let query = sorted_query(&sql.get_untracked(), sort_state.get_untracked());
            stale_result.set(
                result
                    .get_untracked()
                    .map(|_| "Preview running; showing the last successful result.".to_owned()),
            );
            let current_revision = revision.get_untracked();
            let generation = worker_generation.get_untracked();
            let key = PreviewKey {
                doc_id: DOC_ID.into(),
                revision: current_revision,
                view_id: "sql-main".into(),
                query_hash: format!("{:x}", Sha256::digest(query.as_bytes())),
                stations_hash: None,
                catalog_version: "omo-catalog/0".into(),
                worker_generation: generation,
            };
            let pending = PendingPreview {
                request_id,
                key: key.clone(),
                sql: query,
            };
            pending_previews
                .borrow_mut()
                .insert(request_id, pending.clone());
            let action =
                preview_queue.with(|queue| queue.submit(PreviewRequest::new(key, request_id)));
            match action {
                QueueAction::Start(id) => {
                    if let Err(error) = send_preview(&engine, &pending) {
                        pending_previews.borrow_mut().remove(&id);
                        preview_queue.with(|queue| {
                            queue.complete(id, PreviewOutcome::Failure(error.clone()))
                        });
                        task_status.set(format!("Could not dispatch preview {id}: {error}"));
                    } else if worker_ready.get_untracked() {
                        task_status.set(format!("Preview {id} is running."));
                    } else {
                        task_status.set(format!("Preview {id} is waiting for worker bootstrap."));
                    }
                }
                QueueAction::Interrupt { request_id: active } => {
                    match engine.cancel(JOB_ID, active) {
                        Ok(()) => task_status.set(format!(
                            "Preview {request_id} is waiting for request {active} to cancel."
                        )),
                        Err(error) => {
                            task_status.set(format!("Could not cancel preview {active}: {error}"))
                        }
                    }
                }
                QueueAction::Queued => task_status.set(format!(
                    "Preview {request_id} is coalesced behind the active request."
                )),
            }
        }
    });

    let on_edit: Rc<dyn Fn(String)> = Rc::new({
        let sql = sql;
        let revision = revision;
        let result = result;
        let stale_result = stale_result;
        move |next| {
            sql.set(next);
            revision.update(|value| *value = value.saturating_add(1));
            stale_result.set(
                result
                    .get_untracked()
                    .map(|_| "SQL edited; this table shows an earlier revision.".to_owned()),
            );
        }
    });

    {
        let events = event_receiver;
        let preview_queue = preview_queue;
        let pending_previews = Rc::clone(&pending_previews);
        let cancelled_previews = Rc::clone(&cancelled_previews);
        let export_requests = Rc::clone(&export_requests);
        let latest_preview_request = latest_preview_request;
        let current_revision = revision;
        let current_result = result;
        let current_plot_input = plot_input;
        let stale_result = stale_result;
        let worker_ready = worker_ready;
        let worker_generation = worker_generation;
        let worker_state = worker_state;
        let bootstrap_report = bootstrap_report;
        let task_status = task_status;
        let file_status = file_status;
        let dispatch_next = dispatch_next_preview.clone();
        spawn(async move {
            loop {
                loop {
                    match events.try_recv() {
                        Ok(RuntimeEvent::StartupFailed(message)) => {
                            worker_ready.set(false);
                            worker_state.set("FAILED".into());
                            bootstrap_report.set(None);
                            task_status.set(message);
                        }
                        Ok(RuntimeEvent::RecoveryScanFailed(message)) => {
                            file_status.set(format!("Could not scan saved exports: {message}"));
                        }
                        Ok(RuntimeEvent::Recovery(recovery)) => {
                            match recovery.verdict {
                                crate::host::export_scan::ExportVerdict::Finished => file_status
                                    .set(format!(
                                        "Recovered completed export: {}",
                                        recovery.target.display()
                                    )),
                                crate::host::export_scan::ExportVerdict::Incomplete => file_status
                                    .set(format!(
                                        "Incomplete export — re-run: {}",
                                        recovery.target.display()
                                    )),
                            }
                        }
                        Ok(RuntimeEvent::Worker(event)) => {
                            let previous_generation = worker_generation.get_untracked();
                            worker_generation.set(event.0.worker_generation);
                            match event.0.kind.as_str() {
                                "ready" => {
                                    if previous_generation != 0
                                        && previous_generation != event.0.worker_generation
                                        && current_result.get_untracked().is_some_and(|reference| {
                                            reference.generation != event.0.worker_generation
                                        })
                                    {
                                        stale_result.set(Some(
                                            "Worker restarted; displaying the last successful result.".into(),
                                        ));
                                    }
                                    if previous_generation == 0
                                        || previous_generation != event.0.worker_generation
                                    {
                                        if let Some(active) = preview_queue
                                            .with(|queue| queue.running_request(DOC_ID))
                                        {
                                            let was_cancelled =
                                                cancelled_previews.borrow_mut().remove(&active);
                                            if was_cancelled
                                                || active != latest_preview_request.get_untracked()
                                            {
                                                pending_previews.borrow_mut().remove(&active);
                                                preview_queue.with(|queue| {
                                                    queue.complete(
                                                        active,
                                                        PreviewOutcome::Failure("worker restarted while the preview was cancelled".into()),
                                                    )
                                                });
                                                dispatch_next();
                                            } else {
                                                if let Some(pending) =
                                                    pending_previews.borrow_mut().get_mut(&active)
                                                {
                                                    pending.key.worker_generation =
                                                        event.0.worker_generation;
                                                }
                                                preview_queue.with(|queue| {
                                                    queue.update_running_generation(
                                                        DOC_ID,
                                                        active,
                                                        event.0.worker_generation,
                                                    )
                                                });
                                            }
                                        }
                                    }
                                    worker_ready.set(true);
                                    worker_state.set("READY".into());
                                    bootstrap_report
                                        .set(event.0.body.get("bootstrap_report").cloned());
                                    task_status.set(format!(
                                        "Engine ready at generation {}.",
                                        event.0.worker_generation
                                    ));
                                }
                                "bootstrap_failed" => {
                                    worker_ready.set(false);
                                    worker_state.set("FAILED".into());
                                    let message = event
                                        .0
                                        .body
                                        .pointer("/error/message")
                                        .and_then(Value::as_str)
                                        .unwrap_or("worker bootstrap failed");
                                    task_status.set(format!("Bootstrap failed: {message}"));
                                }
                                "result" => {
                                    let request_id = event.0.request_id;
                                    with_removed_pending(
                                        &pending_previews,
                                        request_id,
                                        |pending| {
                                            if let Some(pending) = pending {
                                                let cancelled = cancelled_previews
                                                    .borrow_mut()
                                                    .remove(&request_id);
                                                if cancelled {
                                                    let completion = preview_queue.with(|queue| {
                                                        queue.complete(
                                                            request_id,
                                                            PreviewOutcome::Failure(
                                                                "preview cancelled by user".into(),
                                                            ),
                                                        )
                                                    });
                                                    if completion == Completion::RetainedStale {
                                                        stale_result.set(Some(
                                                    "Preview cancelled; displaying the last successful result.".into(),
                                                ));
                                                    }
                                                    if completion != Completion::DiscardedSuperseded
                                                    {
                                                        task_status.set(format!(
                                                            "Preview {request_id} was cancelled."
                                                        ));
                                                    }
                                                } else if let Some(value) =
                                                    event.0.body.get("result_ref").cloned()
                                                {
                                                    match serde_json::from_value::<ResultRef>(
                                                        value.clone(),
                                                    ) {
                                                        Ok(reference) => {
                                                            let new_plot_input = event
                                                                .0
                                                                .body
                                                                .get("plot_ref")
                                                                .filter(|value| !value.is_null())
                                                                .and_then(|value| {
                                                                    serde_json::from_value::<
                                                                        ResultRef,
                                                                    >(
                                                                        value.clone()
                                                                    )
                                                                    .ok()
                                                                })
                                                                .filter(|plot_ref| {
                                                                    plot_ref.generation
                                                                        == reference.generation
                                                                })
                                                                .map(|plot_ref| {
                                                                    plot_view::PlotInput {
                                                                        source_rows: event
                                                                            .0
                                                                            .body
                                                                            .get("plot_source_rows")
                                                                            .and_then(Value::as_u64)
                                                                            .unwrap_or(
                                                                                plot_ref.rows,
                                                                            ),
                                                                        reference: plot_ref,
                                                                        worker_sampled: true,
                                                                    }
                                                                });
                                                            let completion =
                                                                preview_queue.with(|queue| {
                                                                    queue.complete(
                                                                        request_id,
                                                                        PreviewOutcome::Success(
                                                                            value,
                                                                        ),
                                                                    )
                                                                });
                                                            let current = pending.key.revision
                                                                == current_revision.get_untracked()
                                                                && request_id
                                                                    == latest_preview_request
                                                                        .get_untracked()
                                                                && pending.key.worker_generation
                                                                    == event.0.worker_generation
                                                                && reference.generation
                                                                    == event.0.worker_generation;
                                                            if completion == Completion::Accepted
                                                                && current
                                                            {
                                                                stale_result.set(None);
                                                                let row_count = reference.rows;
                                                                let truncated = reference.truncated;
                                                                current_plot_input
                                                                    .set(new_plot_input);
                                                                current_result.set(Some(reference));
                                                                if truncated {
                                                                    task_status.set(format!(
                                                                        "Preview {request_id} completed with {row_count} rows (truncated to interactive limit)."
                                                                    ));
                                                                } else {
                                                                    task_status.set(format!(
                                                                        "Preview {request_id} completed with {row_count} rows."
                                                                    ));
                                                                }
                                                            } else if completion
                                                                == Completion::Accepted
                                                            {
                                                                task_status.set(format!(
                                                        "Discarded stale preview {request_id}; the SQL revision or worker generation changed."
                                                    ));
                                                            }
                                                        }
                                                        Err(error) => {
                                                            let message = format!(
                                                                "invalid result reference: {error}"
                                                            );
                                                            preview_queue.with(|queue| {
                                                                queue.complete(
                                                                    request_id,
                                                                    PreviewOutcome::Failure(
                                                                        message.clone(),
                                                                    ),
                                                                )
                                                            });
                                                            task_status.set(message);
                                                        }
                                                    }
                                                } else {
                                                    preview_queue.with(|queue| {
                                                        queue.complete(
                                                            request_id,
                                                            PreviewOutcome::Failure(
                                                                "worker result omitted result_ref"
                                                                    .into(),
                                                            ),
                                                        )
                                                    });
                                                    task_status.set(
                                                "Worker result omitted its result reference."
                                                    .into(),
                                            );
                                                }
                                                dispatch_next();
                                            }
                                        },
                                    );
                                }
                                "progress" => {
                                    let request_id = event.0.request_id;
                                    let sequence = event
                                        .0
                                        .body
                                        .get("sequence")
                                        .and_then(Value::as_u64)
                                        .unwrap_or(0);
                                    task_status.set(format!(
                                        "Preview {request_id} progress event {sequence}."
                                    ));
                                }
                                "failed" => {
                                    let request_id = event.0.request_id;
                                    let was_cancelled =
                                        cancelled_previews.borrow_mut().remove(&request_id);
                                    let message = event
                                        .0
                                        .body
                                        .get("message")
                                        .and_then(Value::as_str)
                                        .unwrap_or("worker request failed");
                                    let fail_kind = parse_fail_kind(&event.0.body);
                                    let is_cancelled = was_cancelled
                                        || matches!(fail_kind, Some(FailKind::Cancelled(_)));
                                    if matches!(
                                        fail_kind,
                                        Some(
                                            FailKind::Bootstrap(_)
                                                | FailKind::WorkerCrashed
                                                | FailKind::Timeout
                                                | FailKind::Cancelled(CancelLevel::Kill)
                                        )
                                    ) {
                                        worker_ready.set(false);
                                        worker_state.set("RESTARTING".into());
                                    }
                                    if let Some(path) =
                                        export_requests.borrow_mut().remove(&request_id)
                                    {
                                        file_status.set(format!(
                                            "Export failed for {}: {message}",
                                            path.display()
                                        ));
                                    }
                                    with_removed_pending(
                                        &pending_previews,
                                        request_id,
                                        |pending| {
                                            if let Some(pending) = pending {
                                                let completion = preview_queue.with(|queue| {
                                                    queue.complete(
                                                        request_id,
                                                        PreviewOutcome::Failure(message.to_owned()),
                                                    )
                                                });
                                                if completion == Completion::RetainedStale {
                                                    stale_result.set(Some(format!(
                                                "Preview failed: {message}; displaying the last successful result."
                                            )));
                                                }
                                                if completion != Completion::DiscardedSuperseded
                                                    && pending.key.revision
                                                        == current_revision.get_untracked()
                                                    && request_id
                                                        == latest_preview_request.get_untracked()
                                                {
                                                    if is_cancelled {
                                                        task_status.set(format!(
                                                            "Preview {request_id} was cancelled."
                                                        ));
                                                    } else {
                                                        task_status.set(format!(
                                                            "Preview failed: {message}"
                                                        ));
                                                    }
                                                }
                                                dispatch_next();
                                            }
                                        },
                                    );
                                }
                                "export_committed" => {
                                    let request_id = event.0.request_id;
                                    let path = export_requests
                                        .borrow_mut()
                                        .remove(&request_id)
                                        .or_else(|| {
                                            event
                                                .0
                                                .body
                                                .get("target")
                                                .and_then(Value::as_str)
                                                .map(PathBuf::from)
                                        });
                                    if let Some(path) = path {
                                        file_status
                                            .set(format!("Export saved: {}", path.display()));
                                    }
                                }
                                _ => {}
                            }
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => break,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            if worker_state.get_untracked() != "FAILED" {
                                worker_ready.set(false);
                                worker_state.set("FAILED".into());
                                task_status.set("Engine event bridge stopped unexpectedly.".into());
                            }
                            return;
                        }
                    }
                }
                timer(Duration::from_millis(25)).await;
            }
        });
    }

    let cancel_task: Rc<dyn Fn()> = Rc::new({
        let preview_queue = preview_queue;
        let cancelled_previews = Rc::clone(&cancelled_previews);
        let engine = engine.clone();
        let task_status = task_status;
        move || match preview_queue.with(|queue| queue.running_request(DOC_ID)) {
            Some(request_id) => match engine.cancel(JOB_ID, request_id) {
                Ok(()) => {
                    cancelled_previews.borrow_mut().insert(request_id);
                    task_status.set(format!("Cancellation requested for preview {request_id}."))
                }
                Err(error) => {
                    cancelled_previews.borrow_mut().remove(&request_id);
                    task_status.set(format!("Could not cancel preview {request_id}: {error}"))
                }
            },
            None => task_status.set("No worker query is active.".into()),
        }
    });

    let open_station_csv: Rc<dyn Fn()> = Rc::new({
        let source_path = source_path;
        let source_status = source_status;
        let file_status = file_status;
        move || {
            let selected_path = source_path;
            let selection_status = source_status;
            let dialog_status = file_status;
            dialogs::choose_stations_csv(
                move |path: PathBuf| {
                    let path = path.to_string_lossy().into_owned();
                    selected_path.set(path.clone());
                    selection_status
                        .set("CSV selected locally; engine loading is not connected yet.".into());
                    dialog_status.set(format!("Selected station file: {path}"));
                },
                move |error| {
                    selection_status.set(format!(
                        "File chooser unavailable ({error}). Enter a path below."
                    ));
                },
            );
        }
    });

    let submit_export: Rc<dyn Fn(PathBuf, bool)> = Rc::new({
        let file_status = file_status;
        let result = result;
        let worker_ready = worker_ready;
        let worker_generation = worker_generation;
        let revision = revision;
        let sql = sql;
        let sort_state = sort_state;
        let request_count = request_count;
        let export_requests = Rc::clone(&export_requests);
        let engine = engine.clone();
        move |path, overwrite_confirmed| {
            let Some(reference) = result.get_untracked() else {
                file_status.set("Run a query before exporting a result.".into());
                return;
            };
            if !worker_ready.get_untracked()
                || reference.generation != worker_generation.get_untracked()
            {
                file_status.set(
                    "This result belongs to a stopped worker generation and cannot be exported."
                        .into(),
                );
                return;
            }
            if !path.is_absolute() {
                file_status.set("Enter an absolute export path.".into());
                return;
            }
            let format = path
                .extension()
                .and_then(|value| value.to_str())
                .map(str::to_ascii_lowercase);
            let Some(format) = format.filter(|format| format == "csv" || format == "parquet")
            else {
                file_status.set("Choose a destination ending in .csv or .parquet.".into());
                return;
            };
            let request_id = request_count.get_untracked().saturating_add(1);
            request_count.set(request_id);
            let path_text = path.to_string_lossy().into_owned();
            let body = json!({
                "result_id": reference.id,
                "target": path_text,
                "format": format,
                "row_count": reference.rows,
                "overwrite_confirmed": overwrite_confirmed,
                "job_snapshot": {
                    "job_id": JOB_ID,
                    "revision": revision.get_untracked(),
                    "sql": sorted_query(&sql.get_untracked(), sort_state.get_untracked()),
                    "stations_hash": null,
                    "catalog_version": "omo-catalog/0"
                }
            });
            export_requests
                .borrow_mut()
                .insert(request_id, path.clone());
            match engine.send_request(JOB_ID, request_id, "export", body) {
                Ok(()) => file_status.set(format!("Exporting {}…", path.display())),
                Err(error) => {
                    export_requests.borrow_mut().remove(&request_id);
                    file_status.set(format!("Could not send export request: {error}"));
                }
            }
        }
    });

    let choose_export_path: Rc<dyn Fn()> = Rc::new({
        let file_status = file_status;
        let result = result;
        let worker_ready = worker_ready;
        let worker_generation = worker_generation;
        let submit_export = submit_export.clone();
        move || {
            let Some(reference) = result.get_untracked() else {
                file_status.set("Run a query before exporting a result.".into());
                return;
            };
            if !worker_ready.get_untracked()
                || reference.generation != worker_generation.get_untracked()
            {
                file_status.set(
                    "This result belongs to a stopped worker generation and cannot be exported."
                        .into(),
                );
                return;
            }
            let submit = submit_export.clone();
            let dialog_status = file_status;
            dialogs::choose_export_path(
                move |path, confirmed| submit(path, confirmed),
                move |error| {
                    dialog_status.set(format!(
                        "Save chooser unavailable ({error}); enter an absolute path in the Tasks panel."
                    ))
                },
            );
        }
    });

    let export_manually: Rc<dyn Fn()> = Rc::new({
        let manual_export_path = manual_export_path;
        let file_status = file_status;
        let submit_export = submit_export.clone();
        move || {
            let path = manual_export_path.get_untracked();
            if path.trim().is_empty() {
                file_status.set("Enter an absolute export path first.".into());
                return;
            }
            let submit = submit_export.clone();
            let error_status = file_status;
            dialogs::confirm_export_path(
                PathBuf::from(path.trim()),
                move |path, confirmed| submit(path, confirmed),
                move |error| error_status.set(format!("Could not prepare manual export: {error}")),
            );
        }
    });

    let sort_notice: Rc<dyn Fn(Option<nsh_base::DataTableSort>)> = Rc::new({
        let sort_state = sort_state;
        let task_status = task_status;
        let run_preview = run_preview.clone();
        move |sort| {
            let description = sort.as_ref().map(|value| value.key.to_string());
            sort_state.set(sort);
            task_status.set(match description {
                Some(column) => format!("Sorting by {column}; fetching a new result."),
                None => "Sort cleared; fetching the unsorted query result.".into(),
            });
            run_preview();
        }
    });

    let show_left = dock_ref;
    let show_bottom = dock_ref;
    let menu_run = run_preview.clone();
    let menu_cancel = cancel_task.clone();
    let menu_open = open_station_csv.clone();
    let menu_export = choose_export_path.clone();
    set_application_menu(vec![
        MenuItem::submenu(
            "Omobench",
            [
                MenuItem::role(MenuRole::About),
                MenuItem::separator(),
                MenuItem::role(MenuRole::Quit),
            ],
        ),
        MenuItem::submenu(
            "File",
            [
                MenuItem::new("Open station CSV…")
                    .shortcut("mod+o")
                    .on_select(move || menu_open()),
                MenuItem::new("Run preview")
                    .shortcut("mod+enter")
                    .on_select(move || menu_run()),
                MenuItem::new("Cancel task")
                    .shortcut("mod+.")
                    .on_select(move || menu_cancel()),
                MenuItem::separator(),
                MenuItem::new("Choose export destination…").on_select(move || menu_export()),
                MenuItem::separator(),
                MenuItem::role(MenuRole::Quit),
            ],
        ),
        MenuItem::submenu(
            "View",
            [
                MenuItem::new("Toggle sources panel").on_select(move || {
                    if let Some(area) = show_left.api::<DockAreaApi>() {
                        area.toggle_dock(DockPlacement::Left);
                    }
                }),
                MenuItem::new("Toggle task panel").on_select(move || {
                    if let Some(area) = show_bottom.api::<DockAreaApi>() {
                        area.toggle_dock(DockPlacement::Bottom);
                    }
                }),
            ],
        ),
    ]);

    let open_for_panel = open_station_csv.clone();
    let run_for_panel = run_preview.clone();
    let retry_for_panel = run_preview.clone();
    let cancel_for_panel = cancel_task.clone();
    let export_manual_for_panel = export_manually.clone();
    let status_for_footer = task_status;
    let worker_state_for_header = worker_state;

    view()
        .app_class("flex-col size-full bg-[#f2f0e8] font-sans text-[#24312c] dark:bg-[#121714] dark:text-[#e6e9e2]")
        .child(
            view()
                .app_class("h-14 shrink-0 flex-row items-center gap-4 border-b border-[#d9d5ca] bg-[#f9f8f3] px-5 dark:border-[#343b36] dark:bg-[#151a17]")
                .child(
                    view()
                        .app_class("size-8 items-center justify-center rounded-md bg-[#24584c] text-sm font-bold text-white")
                        .child(text("O")),
                )
                .child(
                    view()
                        .app_class("flex-col gap-0")
                        .child(text("OMOBENCH STUDIO").app_class("text-xs font-bold tracking-widest"))
                        .child(text("DATA WORKBENCH").app_class("text-[10px] tracking-wide text-[#78837b] dark:text-[#9ba69e]")),
                )
                .child(view().app_class("flex-1"))
                .child(
                    view()
                        .app_class("flex-row items-center gap-2 rounded-full border border-[#e1c8a6] bg-[#f6ecdf] px-3 py-1.5 dark:border-[#5a4631] dark:bg-[#2c251d]")
                        .child(view().app_class("size-1.5 rounded-full bg-[#bf7950]"))
                        .child(text(move || format!("ENGINE {}", worker_state_for_header.get())).app_class("text-[10px] font-semibold tracking-wide text-[#40594d] dark:text-[#bfd0c3]")),
                )
                .child({
                    let open = open_station_csv.clone();
                    button()
                        .app_class("h-8 rounded-md border border-[#d6d1c5] px-3 text-xs font-semibold hover:bg-[#eeece5] dark:border-[#404740] dark:hover:bg-[#252c27]")
                        .on_press(move || open())
                        .child(text("Open CSV"))
                })
                .child({
                    let export = choose_export_path.clone();
                    button()
                        .app_class("h-8 rounded-md bg-[#24584c] px-3 text-xs font-semibold text-white hover:bg-[#1c493e]")
                        .on_press(move || export())
                        .child(text("Export"))
                }),
        )
        .child(
            view()
                .app_class("min-h-0 flex-1 p-3")
                .child(
                    dock_area()
                        .layout(move || layout.get())
                        .panels(panel_options())
                        .render_panel(move |panel: &DockPanel| {
                            let panel_id = panel.id.to_string();
                            match panel_id.as_str() {
                                "sources" => source_panel(
                                    source_path,
                                    source_status,
                                    open_for_panel.clone(),
                                ).into_element(),
                                "query" => sql_view::panel(
                                    sql,
                                    on_edit.clone(),
                                    run_for_panel.clone(),
                                ).into_element(),
                                "results" => table_view::panel(
                                    result,
                                    stale_result,
                                    sort_notice.clone(),
                                ).into_element(),
                                "chart" => plot_view::panel(chart_rows, result, plot_input).into_element(),
                                "tasks" => tasks_view::panel(
                                    task_status,
                                    request_count,
                                    worker_state,
                                    worker_generation,
                                    bootstrap_report,
                                    manual_export_path,
                                    cancel_for_panel.clone(),
                                    retry_for_panel.clone(),
                                    export_manual_for_panel.clone(),
                                ).into_element(),
                                _ => view()
                                    .app_class("size-full items-center justify-center bg-[#fbfaf6] dark:bg-[#171c19]")
                                    .child(text("Panel unavailable"))
                                    .into_element(),
                            }
                        })
                        .render_tab(|tab: &DockTab| {
                            let label = match tab.panel.as_str() {
                                "sources" => "Sources",
                                "query" => "SQL editor",
                                "results" => "Results",
                                "chart" => "Plot",
                                "tasks" => "Tasks",
                                _ => tab.panel.as_str(),
                            };
                            let classes = if tab.active {
                                "h-9 flex-row items-center px-3 text-xs font-semibold text-[#24584c] dark:text-[#a6c5b6]"
                            } else {
                                "h-9 flex-row items-center px-3 text-xs font-medium text-[#78837b] hover:text-[#24312c] dark:text-[#9ba69e] dark:hover:text-[#e6e9e2]"
                            };
                            view().app_class(classes).child(text(label.to_owned()))
                        })
                        .group_class("flex-col min-w-0 min-h-0 overflow-hidden rounded-lg border border-[#dedacf] bg-[#fbfaf6] dark:border-[#343b36] dark:bg-[#171c19]")
                        .tab_bar_class("h-10 shrink-0 flex-row items-center gap-1 border-b border-[#e0ddd3] bg-[#f4f2eb] px-2 dark:border-[#343b36] dark:bg-[#1d241f]")
                        .tab_class("outline-none focus-visible:outline focus-visible:outline-2 focus-visible:outline-[#c6824b]")
                        .content_class("min-h-0 flex-1 overflow-hidden")
                        .dock_class("overflow-hidden rounded-lg border border-[#dedacf] bg-[#fbfaf6] dark:border-[#343b36] dark:bg-[#171c19]")
                        .handle_class("bg-[#9eaa9f]/40 group-hover:bg-[#bd7950] group-active:bg-[#bd7950]")
                        .on_layout_change(move |next: &DockAreaLayout| layout.set(next.clone()))
                        .component_ref(&dock_ref)
                        .size_full(),
                ),
        )
        .child(
            view()
                .app_class("h-8 shrink-0 flex-row items-center justify-between border-t border-[#d9d5ca] bg-[#f9f8f3] px-4 dark:border-[#343b36] dark:bg-[#151a17]")
                .child(text(move || status_for_footer.get()).app_class("text-[10px] text-[#77837b] dark:text-[#9ba69e]"))
                .child(text(move || file_status.get()).app_class("max-w-[55%] truncate text-[10px] text-[#77837b] dark:text-[#9ba69e]")),
        )
}

fn default_layout() -> DockAreaLayout {
    let mut sources = DockRegion::new(DockLayout::tabs().panel("sources"));
    sources.size = Some(px(252.0));
    sources.open = Some(true);

    let mut tasks = DockRegion::new(DockLayout::tabs().panel("tasks"));
    tasks.size = Some(px(210.0));
    tasks.open = Some(true);

    DockAreaLayout::new(
        DockLayout::tabs()
            .panel("query")
            .panel("results")
            .panel("chart"),
    )
    .with_dock(DockPlacement::Left, sources)
    .with_dock(DockPlacement::Bottom, tasks)
}

fn panel_options() -> Vec<DockPanelOptions> {
    ["sources", "query", "results", "chart", "tasks"]
        .into_iter()
        .map(|id| {
            let mut options = DockPanelOptions::new(id);
            options.closable = Some(false);
            options.zoomable = Some(true);
            options
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::with_removed_pending;
    use std::{cell::RefCell, collections::HashMap};

    #[test]
    fn removed_pending_callback_can_dispatch_and_reborrow_map() {
        let pending = RefCell::new(HashMap::from([(1, "active")]));

        let dispatched = with_removed_pending(&pending, 1, |removed| {
            assert_eq!(removed, Some("active"));
            pending
                .try_borrow_mut()
                .expect("next dispatch must be able to borrow pending previews")
                .insert(2, "queued");
            true
        });

        assert!(dispatched);
        assert_eq!(pending.borrow().get(&2), Some(&"queued"));
    }
}

fn source_panel(
    source_path: Signal<String>,
    source_status: Signal<String>,
    open_csv: Rc<dyn Fn()>,
) -> impl IntoElement {
    let open = open_csv.clone();
    view()
        .app_class("flex-col size-full gap-4 bg-[#fbfaf6] p-4 text-[#24312c] dark:bg-[#171c19] dark:text-[#e6e9e2]")
        .child(text("PROJECT").app_class("text-[10px] font-semibold tracking-widest text-[#7d877f] dark:text-[#9ba69e]"))
        .child(text("Untitled workspace").app_class("text-sm font-semibold"))
        .child(text("Project persistence is not connected in this M0 shell.").app_class("text-xs leading-5 text-[#7d877f] dark:text-[#9ba69e]"))
        .child(view().app_class("h-px bg-[#e0ddd3] dark:bg-[#343b36]"))
        .child(text("STATION DATA").app_class("text-[10px] font-semibold tracking-widest text-[#7d877f] dark:text-[#9ba69e]"))
        .child(
            button()
                .app_class("h-9 flex-row items-center justify-center gap-2 rounded-md border border-[#d6d1c5] px-3 text-xs font-semibold hover:bg-[#f0eee7] dark:border-[#404740] dark:hover:bg-[#252c27]")
                .on_press(move || open())
                .child(text("＋"))
                .child(text("Browse for CSV")),
        )
        .child(text("Or enter a local path if the file chooser is unavailable.")
            .app_class("text-xs leading-5 text-[#7d877f] dark:text-[#9ba69e]"))
        .child(
            input()
                .value(move || source_path.get())
                .placeholder("Absolute CSV path")
                .on_change(move |value: &str| source_path.set(value.to_owned()))
                .app_class("h-9 rounded-md border border-[#d6d1c5] bg-white px-2 text-xs dark:border-[#404740] dark:bg-[#202722]"),
        )
        .child({
            let path = source_path;
            let status = source_status;
            button()
                .app_class("h-8 self-start rounded-md bg-[#e7e9df] px-3 text-xs font-semibold text-[#43584d] hover:bg-[#dde2d6] dark:bg-[#2d3931] dark:text-[#c3d0c5]")
                .on_press(move || {
                    if path.get_untracked().trim().is_empty() {
                        status.set("Enter a CSV path or choose a file first.".into());
                    } else {
                        status.set("Path saved in this window only; CSV ingestion is not connected yet.".into());
                    }
                })
                .child(text("Use path"))
        })
        .child(text(move || source_status.get())
            .app_class("text-xs leading-5 text-[#6b786f] dark:text-[#a4ada6]"))
}
