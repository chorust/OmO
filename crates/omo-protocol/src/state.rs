use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const MAX_DRAIN_TIMEOUT_MS: u64 = 10_000;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", content = "detail", rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Bootstrapping,
    Planning,
    Reading,
    Writing,
    Committing,
    Saved,
    Failed(FailKind),
    Cancelled(CancelLevel),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "detail", rename_all = "snake_case")]
pub enum FailKind {
    WorkerCrashed,
    Bootstrap(String),
    Capability(String),
    Query(String),
    Cancelled(CancelLevel),
    Export(String),
    Timeout,
    NoCompleteRun,
    RunIncomplete,
    GridMismatch,
    VariableMissing,
    SourceChanged,
    /// The requested valid time is not published by the resolved run (M2).
    ValidTimeUnavailable,
    /// A map layer failed validation or decoding (M2).
    InvalidLayer,
}

impl FailKind {
    /// True when `omo-worker` rejected a request solely because another request is in flight.
    pub fn is_worker_busy(&self) -> bool {
        matches!(self, Self::Query(detail) if detail.starts_with("busy:"))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "level", content = "timeout_ms", rename_all = "snake_case")]
pub enum CancelLevel {
    Interrupt,
    Drain { timeout_ms: u64 },
    Kill,
}

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
#[error("drain timeout {requested_ms}ms exceeds maximum {maximum_ms}ms")]
pub struct DrainTimeoutError {
    pub requested_ms: u64,
    pub maximum_ms: u64,
}

impl CancelLevel {
    pub fn drain_for(timeout_ms: u64) -> Result<Self, DrainTimeoutError> {
        if timeout_ms > MAX_DRAIN_TIMEOUT_MS {
            return Err(DrainTimeoutError {
                requested_ms: timeout_ms,
                maximum_ms: MAX_DRAIN_TIMEOUT_MS,
            });
        }
        Ok(Self::Drain { timeout_ms })
    }

    pub fn timeout_ms(&self) -> Option<u64> {
        match self {
            Self::Drain { timeout_ms } => Some(*timeout_ms),
            Self::Interrupt | Self::Kill => None,
        }
    }
}
