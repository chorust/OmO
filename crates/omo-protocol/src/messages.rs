use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::state::FailKind;

pub const PROTOCOL_VERSION: u16 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Envelope<T = MessageBody> {
    pub v: u16,
    pub worker_generation: u64,
    pub job_id: u64,
    pub request_id: u64,
    pub kind: String,
    pub body: T,
}

impl<T> Envelope<T> {
    pub fn new(
        worker_generation: u64,
        job_id: u64,
        request_id: u64,
        kind: impl Into<String>,
        body: T,
    ) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            worker_generation,
            job_id,
            request_id,
            kind: kind.into(),
            body,
        }
    }

    pub fn map_body<U>(self, f: impl FnOnce(T) -> U) -> Envelope<U> {
        Envelope {
            v: self.v,
            worker_generation: self.worker_generation,
            job_id: self.job_id,
            request_id: self.request_id,
            kind: self.kind,
            body: f(self.body),
        }
    }
}

pub type MessageBody = serde_json::Value;

/// Build a contract-compliant `failed` event body (`{request_id, kind: FailKind, message}`).
pub fn failed_body(request_id: u64, kind: &FailKind, message: &str) -> MessageBody {
    json!({
        "request_id": request_id,
        "kind": kind,
        "message": message,
    })
}

/// Extract the structured `FailKind` from a `failed` event body.
pub fn parse_fail_kind(body: &MessageBody) -> Option<FailKind> {
    let value = body.get("kind")?.clone();
    serde_json::from_value(value).ok()
}
