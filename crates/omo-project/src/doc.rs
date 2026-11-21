use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProjectDoc {
    doc_id: String,
    revision: u64,
    saved_revision: Option<u64>,
    pub stations_source: Option<StationsSource>,
    pub sql_tabs: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StationsSource {
    pub orig_path: String,
    pub snapshot_path: String,
    pub sha256: String,
    pub mapping: serde_json::Value,
}

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
#[error("project revision overflow")]
pub struct RevisionOverflow;

impl ProjectDoc {
    pub fn new(doc_id: impl Into<String>) -> Self {
        Self {
            doc_id: doc_id.into(),
            revision: 0,
            saved_revision: Some(0),
            stations_source: None,
            sql_tabs: Vec::new(),
        }
    }

    pub fn doc_id(&self) -> &str {
        &self.doc_id
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn saved_revision(&self) -> Option<u64> {
        self.saved_revision
    }

    pub fn is_dirty(&self) -> bool {
        self.saved_revision != Some(self.revision)
    }

    pub fn record_edit(&mut self) -> Result<u64, RevisionOverflow> {
        self.revision = self.revision.checked_add(1).ok_or(RevisionOverflow)?;
        Ok(self.revision)
    }

    pub fn mark_saved(&mut self) {
        self.saved_revision = Some(self.revision);
    }
}
