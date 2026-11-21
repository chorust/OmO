use std::collections::HashMap;

use omo_project::PreviewKey;
use serde_json::Value;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreviewRequest {
    pub key: PreviewKey,
    pub request_id: u64,
}

impl PreviewRequest {
    pub fn new(key: PreviewKey, request_id: u64) -> Self {
        Self { key, request_id }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum QueueAction {
    Start(u64),
    Interrupt { request_id: u64 },
    Queued,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PreviewOutcome {
    Success(Value),
    Failure(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreviewValue {
    pub key: PreviewKey,
    pub value: Value,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Completion {
    Accepted,
    RetainedStale,
    FailedWithoutPreview,
    DiscardedSuperseded,
    DiscardedUnknown,
}

#[derive(Default)]
pub struct PreviewQueue {
    documents: HashMap<String, DocumentQueue>,
}

#[derive(Default)]
struct DocumentQueue {
    running: Option<PreviewRequest>,
    queued: Option<PreviewRequest>,
    interrupt_sent: bool,
    dispatch_pending: Option<PreviewRequest>,
    last_good: Option<PreviewValue>,
    stale_error: Option<String>,
}

impl PreviewQueue {
    pub fn submit(&mut self, request: PreviewRequest) -> QueueAction {
        let doc_id = request.key.doc_id.clone();
        let state = self.documents.entry(doc_id).or_default();
        if state.running.is_none() {
            state.running = Some(request.clone());
            return QueueAction::Start(request.request_id);
        }

        let active_request_id = state.running.as_ref().unwrap().request_id;
        state.queued = Some(request);
        if state.interrupt_sent {
            QueueAction::Queued
        } else {
            state.interrupt_sent = true;
            QueueAction::Interrupt {
                request_id: active_request_id,
            }
        }
    }

    pub fn complete(&mut self, request_id: u64, outcome: PreviewOutcome) -> Completion {
        let Some((doc_id, state)) = self.documents.iter_mut().find(|(_, state)| {
            state
                .running
                .as_ref()
                .is_some_and(|request| request.request_id == request_id)
        }) else {
            return Completion::DiscardedUnknown;
        };
        let running = state.running.take().expect("running request was matched");
        state.interrupt_sent = false;

        let completion = if state.queued.is_some() {
            Completion::DiscardedSuperseded
        } else {
            match outcome {
                PreviewOutcome::Success(value) => {
                    state.last_good = Some(PreviewValue {
                        key: running.key,
                        value,
                    });
                    state.stale_error = None;
                    Completion::Accepted
                }
                PreviewOutcome::Failure(error) => {
                    state.stale_error = Some(error);
                    if state.last_good.is_some() {
                        Completion::RetainedStale
                    } else {
                        Completion::FailedWithoutPreview
                    }
                }
            }
        };

        if let Some(next) = state.queued.take() {
            state.dispatch_pending = Some(next.clone());
            state.running = Some(next);
        } else if state.running.is_none() {
            state.dispatch_pending = None;
        }
        let _ = doc_id;
        completion
    }

    /// Returns the next coalesced request promoted after a completion.
    pub fn take_dispatch(&mut self, doc_id: &str) -> Option<PreviewRequest> {
        self.documents.get_mut(doc_id)?.dispatch_pending.take()
    }

    pub fn running_request(&self, doc_id: &str) -> Option<u64> {
        self.documents
            .get(doc_id)?
            .running
            .as_ref()
            .map(|request| request.request_id)
    }

    pub fn update_running_generation(
        &mut self,
        doc_id: &str,
        request_id: u64,
        generation: u64,
    ) -> bool {
        let Some(state) = self.documents.get_mut(doc_id) else {
            return false;
        };
        let Some(request) = state.running.as_mut() else {
            return false;
        };
        if request.request_id != request_id {
            return false;
        }
        request.key.worker_generation = generation;
        true
    }

    pub fn last_good(&self, doc_id: &str) -> Option<&PreviewValue> {
        self.documents.get(doc_id)?.last_good.as_ref()
    }

    pub fn stale_error(&self, doc_id: &str) -> Option<&str> {
        self.documents.get(doc_id)?.stale_error.as_deref()
    }
}
