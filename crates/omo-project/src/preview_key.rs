use serde::{Deserialize, Serialize};

/// Closed lead-time window in hours. Equality participates in preview identity.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct PreviewWindow {
    pub start_h: u32,
    pub end_h: u32,
}

impl Default for PreviewWindow {
    fn default() -> Self {
        Self {
            start_h: 0,
            end_h: 168,
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct PreviewKey {
    pub doc_id: String,
    pub revision: u64,
    pub view_id: String,
    pub query_hash: String,
    pub stations_hash: Option<String>,
    pub catalog_version: String,
    pub run_ref: String,
    pub window: PreviewWindow,
    pub max_distance_m: u32,
    pub worker_generation: u64,
}

impl PreviewKey {
    pub fn m1_defaults() -> Self {
        Self {
            doc_id: String::new(),
            revision: 0,
            view_id: String::new(),
            query_hash: String::new(),
            stations_hash: None,
            catalog_version: String::new(),
            run_ref: "latest_complete".into(),
            window: PreviewWindow::default(),
            max_distance_m: 7200,
            worker_generation: 0,
        }
    }
}
