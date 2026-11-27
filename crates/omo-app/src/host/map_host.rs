//! UI-free M2 map host: the [`MapRequestScheduler`] state machine.
//!
//! `omo-worker` accepts one active request at a time (`contracts/worker-protocol-m2.md`), so map
//! work is serialized here, independently of the SQL `PreviewQueue`:
//! `map_load_field` > `map_layer_probe` > `map_render_tiles` > `map_inspect`. Pan/zoom tile batches and hover points
//! coalesce to the latest; a newer field revision interrupts a running load exactly once; a
//! failed reload keeps the last good field visible as `Stale`.

pub use super::map_view::{Viewport, VisibleTile};
use serde_json::Value;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MapRequestKind {
    LoadField,
    LayerProbe,
    /// Composite PNG export (`export` with a `map_export` snapshot).
    Export,
    RenderTiles,
    Inspect,
}

impl MapRequestKind {
    /// Wire `kind` string of the worker request.
    pub fn wire_kind(self) -> &'static str {
        match self {
            Self::LoadField => "map_load_field",
            Self::LayerProbe => "map_layer_probe",
            Self::Export => "export",
            Self::RenderTiles => "map_render_tiles",
            Self::Inspect => "map_inspect",
        }
    }

    pub fn from_wire(kind: &str) -> Option<Self> {
        match kind {
            "map_load_field" => Some(Self::LoadField),
            "map_layer_probe" => Some(Self::LayerProbe),
            "export" => Some(Self::Export),
            "map_render_tiles" => Some(Self::RenderTiles),
            "map_inspect" => Some(Self::Inspect),
            _ => None,
        }
    }
}

/// Request a field load (`body` is the complete `map_load_field` body).
#[derive(Clone, Debug, PartialEq)]
pub struct FieldIntent {
    pub revision: u64,
    pub body: Value,
}

/// Request one local-archive layer probe (`body` is the complete `map_layer_probe` body).
/// Probes are independent of the field revision and are never coalesced across layers.
#[derive(Clone, Debug, PartialEq)]
pub struct ProbeIntent {
    pub layer_id: String,
    pub body: Value,
}

/// Request one composite PNG export (`body` is the complete `export` body). The snapshot was
/// fixed against `field_revision` (`0` when it shows no forecast field), so it is dropped when a
/// newer field is requested before it starts.
#[derive(Clone, Debug, PartialEq)]
pub struct ExportIntent {
    pub field_revision: u64,
    pub body: Value,
}

/// Request one tile batch. `field_revision == 0` renders without a forecast field.
#[derive(Clone, Debug, PartialEq)]
pub struct TilesIntent {
    pub field_revision: u64,
    pub body: Value,
}

/// Request one point readout. `field_revision == 0` is never meaningful for inspect but kept
/// symmetric with tiles.
#[derive(Clone, Debug, PartialEq)]
pub struct InspectIntent {
    pub field_revision: u64,
    pub body: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub enum MapAction {
    Send {
        request_id: u64,
        kind: MapRequestKind,
        body: Value,
    },
    Interrupt {
        request_id: u64,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum MapOutcome {
    Success(Value),
    Failure { message: String, cancelled: bool },
}

#[derive(Clone, Debug, PartialEq)]
pub enum FieldState {
    Idle,
    Loading {
        revision: u64,
    },
    Ready {
        revision: u64,
        loaded: Value,
    },
    /// The latest reload failed (or the worker restarted); `loaded` is the last good field.
    Stale {
        revision: u64,
        loaded: Value,
        error: String,
    },
    Failed(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum MapCompletion {
    FieldLoaded {
        revision: u64,
        result: Value,
    },
    Completed {
        kind: MapRequestKind,
        field_revision: u64,
        result: Value,
    },
    FieldFailedStale(String),
    FieldFailed(String),
    FieldCancelled,
    RequestFailed {
        kind: MapRequestKind,
        message: String,
    },
    RequestCancelled {
        kind: MapRequestKind,
    },
    DiscardedSuperseded,
    DiscardedUnknown,
}

struct Running {
    request_id: u64,
    kind: MapRequestKind,
    /// Field revision this request belongs to (load: the revision being loaded).
    revision: u64,
    interrupt_sent: bool,
}

pub struct MapRequestScheduler {
    next_request_id: u64,
    running: Option<Running>,
    queued_field: Option<FieldIntent>,
    queued_probes: Vec<ProbeIntent>,
    queued_export: Option<ExportIntent>,
    queued_tiles: Option<TilesIntent>,
    queued_inspect: Option<InspectIntent>,
    /// Newest field revision ever submitted.
    latest_requested: u64,
    last_good: Option<(u64, Value)>,
    state: FieldState,
}

impl MapRequestScheduler {
    /// `first_request_id` starts the id space used for map requests.
    pub fn new(first_request_id: u64) -> Self {
        Self {
            next_request_id: first_request_id,
            running: None,
            queued_field: None,
            queued_probes: Vec::new(),
            queued_export: None,
            queued_tiles: None,
            queued_inspect: None,
            latest_requested: 0,
            last_good: None,
            state: FieldState::Idle,
        }
    }

    pub fn has_running(&self) -> bool {
        self.running.is_some()
    }

    pub fn running_request(&self) -> Option<(u64, MapRequestKind)> {
        self.running.as_ref().map(|r| (r.request_id, r.kind))
    }

    /// True while a field load is running or queued.
    pub fn field_state_is_loading(&self) -> bool {
        matches!(self.state, FieldState::Loading { .. })
    }

    pub fn field_state(&self) -> &FieldState {
        &self.state
    }

    /// Revision of the field the worker can render and inspect (the last good one).
    pub fn active_field_revision(&self) -> Option<u64> {
        self.last_good.as_ref().map(|(revision, _)| *revision)
    }

    fn accepts(&self, field_revision: u64) -> bool {
        field_revision == 0
            || Some(field_revision) == self.active_field_revision()
            || field_revision == self.latest_requested
    }

    fn start(&mut self, kind: MapRequestKind, revision: u64, body: Value) -> MapAction {
        let request_id = self.next_request_id;
        self.next_request_id += 1;
        self.running = Some(Running {
            request_id,
            kind,
            revision,
            interrupt_sent: false,
        });
        MapAction::Send {
            request_id,
            kind,
            body,
        }
    }

    pub fn submit_field(&mut self, intent: FieldIntent) -> Vec<MapAction> {
        self.latest_requested = self.latest_requested.max(intent.revision);
        // Work queued for older revisions is obsolete; the host resubmits after the load.
        if self
            .queued_tiles
            .as_ref()
            .is_some_and(|t| t.field_revision != 0)
        {
            self.queued_tiles = None;
        }
        self.queued_inspect = None;
        if self
            .queued_export
            .as_ref()
            .is_some_and(|export| export.field_revision != 0)
        {
            self.queued_export = None;
        }
        self.queued_field = Some(intent);
        match &mut self.running {
            None => self.pump(),
            Some(running)
                if running.kind == MapRequestKind::LoadField && !running.interrupt_sent =>
            {
                running.interrupt_sent = true;
                self.state = FieldState::Loading {
                    revision: self.latest_requested,
                };
                vec![MapAction::Interrupt {
                    request_id: running.request_id,
                }]
            }
            Some(_) => Vec::new(),
        }
    }

    /// Queue a layer probe; a newer probe of the same layer replaces a queued older one.
    pub fn submit_probe(&mut self, intent: ProbeIntent) -> Vec<MapAction> {
        self.queued_probes
            .retain(|queued| queued.layer_id != intent.layer_id);
        self.queued_probes.push(intent);
        self.pump_if_idle()
    }

    /// Queue a composite export; only one can wait, a newer one replaces it.
    pub fn submit_export(&mut self, intent: ExportIntent) -> Vec<MapAction> {
        if !self.accepts(intent.field_revision) {
            return Vec::new();
        }
        self.queued_export = Some(intent);
        self.pump_if_idle()
    }

    pub fn submit_tiles(&mut self, intent: TilesIntent) -> Vec<MapAction> {
        if !self.accepts(intent.field_revision) {
            return Vec::new();
        }
        self.queued_tiles = Some(intent);
        self.pump_if_idle()
    }

    pub fn submit_inspect(&mut self, intent: InspectIntent) -> Vec<MapAction> {
        if !self.accepts(intent.field_revision) {
            return Vec::new();
        }
        self.queued_inspect = Some(intent);
        self.pump_if_idle()
    }

    /// Cancel user-initiated map work (`FR-019`, `US1.6`): drop queued map requests and
    /// return the active request `(request_id, kind)` to interrupt if one has not already been
    /// interrupted.
    pub fn cancel_running(&mut self) -> Option<(u64, MapRequestKind)> {
        self.queued_field = None;
        self.queued_export = None;
        self.queued_tiles = None;
        self.queued_inspect = None;
        let running = self.running.as_mut()?;
        if running.interrupt_sent {
            return None;
        }
        running.interrupt_sent = true;
        Some((running.request_id, running.kind))
    }

    fn pump_if_idle(&mut self) -> Vec<MapAction> {
        if self.running.is_some() {
            Vec::new()
        } else {
            self.pump()
        }
    }

    /// Start the highest-priority queued request, if any. A tile or inspect request is held back
    /// while a newer field is queued or loading.
    fn pump(&mut self) -> Vec<MapAction> {
        if self.running.is_some() {
            return Vec::new();
        }
        if let Some(field) = self.queued_field.take() {
            self.state = FieldState::Loading {
                revision: field.revision,
            };
            return vec![self.start(MapRequestKind::LoadField, field.revision, field.body)];
        }
        if !self.queued_probes.is_empty() {
            let probe = self.queued_probes.remove(0);
            return vec![self.start(MapRequestKind::LayerProbe, 0, probe.body)];
        }
        if let Some(export) = self.queued_export.take() {
            return vec![self.start(MapRequestKind::Export, export.field_revision, export.body)];
        }
        if let Some(tiles) = self.queued_tiles.take() {
            return vec![self.start(
                MapRequestKind::RenderTiles,
                tiles.field_revision,
                tiles.body,
            )];
        }
        if let Some(inspect) = self.queued_inspect.take() {
            return vec![self.start(
                MapRequestKind::Inspect,
                inspect.field_revision,
                inspect.body,
            )];
        }
        Vec::new()
    }

    pub fn complete(
        &mut self,
        request_id: u64,
        outcome: MapOutcome,
    ) -> (MapCompletion, Vec<MapAction>) {
        if self.running.as_ref().map(|r| r.request_id) != Some(request_id) {
            return (MapCompletion::DiscardedUnknown, Vec::new());
        }
        let running = self.running.take().expect("matched above");
        let superseded = running.kind == MapRequestKind::LoadField && self.queued_field.is_some();
        let completion =
            match (running.kind, outcome) {
                (MapRequestKind::LoadField, _) if superseded => {
                    // A newer revision is waiting: never publish this one. Keep the previous field
                    // as the visible state until the newer load finishes.
                    MapCompletion::DiscardedSuperseded
                }
                (MapRequestKind::LoadField, MapOutcome::Success(result)) => {
                    self.last_good = Some((running.revision, result.clone()));
                    self.state = FieldState::Ready {
                        revision: running.revision,
                        loaded: result.clone(),
                    };
                    // Work queued for any other field is obsolete now.
                    if self.queued_tiles.as_ref().is_some_and(|t| {
                        t.field_revision != 0 && t.field_revision != running.revision
                    }) {
                        self.queued_tiles = None;
                    }
                    if self
                        .queued_inspect
                        .as_ref()
                        .is_some_and(|i| i.field_revision != running.revision)
                    {
                        self.queued_inspect = None;
                    }
                    MapCompletion::FieldLoaded {
                        revision: running.revision,
                        result,
                    }
                }
                (
                    MapRequestKind::LoadField,
                    MapOutcome::Failure {
                        cancelled: true, ..
                    },
                ) => match &self.last_good {
                    Some((revision, loaded)) => {
                        let note = String::from("Field load cancelled.");
                        self.state = FieldState::Stale {
                            revision: *revision,
                            loaded: loaded.clone(),
                            error: note.clone(),
                        };
                        MapCompletion::FieldFailedStale(note)
                    }
                    None => {
                        self.state = FieldState::Idle;
                        MapCompletion::FieldCancelled
                    }
                },
                (MapRequestKind::LoadField, MapOutcome::Failure { message, .. }) => {
                    match &self.last_good {
                        Some((revision, loaded)) => {
                            self.state = FieldState::Stale {
                                revision: *revision,
                                loaded: loaded.clone(),
                                error: message.clone(),
                            };
                            MapCompletion::FieldFailedStale(message)
                        }
                        None => {
                            self.state = FieldState::Failed(message.clone());
                            MapCompletion::FieldFailed(message)
                        }
                    }
                }
                (kind, MapOutcome::Success(result)) => MapCompletion::Completed {
                    kind,
                    field_revision: running.revision,
                    result,
                },
                (
                    kind,
                    MapOutcome::Failure {
                        cancelled: true, ..
                    },
                ) => MapCompletion::RequestCancelled { kind },
                (kind, MapOutcome::Failure { message, .. }) => {
                    MapCompletion::RequestFailed { kind, message }
                }
            };
        let next = self.pump();
        (completion, next)
    }

    /// The worker process was replaced: in-flight and queued work is gone, and the worker's
    /// field cache with it, so a previously good field is now only a stale picture.
    pub fn worker_restarted(&mut self) {
        self.running = None;
        self.queued_field = None;
        self.queued_probes.clear();
        self.queued_export = None;
        self.queued_tiles = None;
        self.queued_inspect = None;
        if let Some((revision, loaded)) = self.last_good.clone() {
            self.state = FieldState::Stale {
                revision,
                loaded,
                error: "the engine restarted; reload the field".into(),
            };
        } else if matches!(self.state, FieldState::Loading { .. }) {
            self.state = FieldState::Idle;
        }
    }
}
