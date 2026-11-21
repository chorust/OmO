use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct PreviewKey {
    pub doc_id: String,
    pub revision: u64,
    pub view_id: String,
    pub query_hash: String,
    pub stations_hash: Option<String>,
    pub catalog_version: String,
    pub worker_generation: u64,
}
