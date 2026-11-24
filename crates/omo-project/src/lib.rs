//! Minimal project document model.

pub mod doc;
pub mod draft;
pub mod preview_key;
pub mod stations;

pub use doc::{
    CatalogSelection, ChartViewSettings, OpenOutcome, ProjectDoc, ProjectFileError,
    ProjectViewState, RevisionOverflow, SqlTab,
};
pub use preview_key::PreviewKey;

pub fn new_doc_id() -> String {
    uuid::Uuid::new_v4().to_string()
}
