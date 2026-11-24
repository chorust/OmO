use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::preview_key::PreviewWindow;
use crate::stations::{StationSnapshot, import::canonical_bytes};

pub const PROJECT_SCHEMA_V1: &str = "omo-project/1";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CatalogSelection {
    pub entry_id: String,
    pub catalog_version: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SqlTab {
    pub title: String,
    pub sql: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChartViewSettings {
    pub show_legend: bool,
    pub y_auto: bool,
}

impl Default for ChartViewSettings {
    fn default() -> Self {
        Self {
            show_legend: true,
            y_auto: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ProjectViewState {
    #[serde(default)]
    pub selected_station_ids: Vec<String>,
    #[serde(default)]
    pub chart: ChartViewSettings,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProjectDoc {
    #[serde(default = "default_schema")]
    pub schema: String,
    doc_id: String,
    #[serde(skip)]
    revision: u64,
    #[serde(skip)]
    saved_revision: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stations: Option<StationSnapshot>,
    pub catalog_selection: CatalogSelection,
    pub window: PreviewWindow,
    pub max_distance_m: u32,
    #[serde(default)]
    pub sql_tabs: Vec<SqlTab>,
    #[serde(default)]
    pub view: ProjectViewState,
    #[serde(skip)]
    path: Option<PathBuf>,
}

fn default_schema() -> String {
    PROJECT_SCHEMA_V1.to_owned()
}

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
#[error("project revision overflow")]
pub struct RevisionOverflow;

#[derive(Debug, Error)]
pub enum ProjectFileError {
    #[error("unsupported project schema `{0}` (expected `{PROJECT_SCHEMA_V1}`)")]
    UnsupportedSchema(String),
    #[error(
        "CorruptSnapshot: stations snapshot hash mismatch (expected {expected}, computed {actual})"
    )]
    CorruptSnapshot { expected: String, actual: String },
    #[error("project JSON is invalid: {0}")]
    InvalidJson(#[from] serde_json::Error),
    #[error("project file I/O failed: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Clone, Debug, PartialEq)]
pub struct OpenOutcome {
    pub doc: ProjectDoc,
    pub source_changed_notice: Option<String>,
}

impl ProjectDoc {
    pub fn new(doc_id: impl Into<String>) -> Self {
        Self {
            schema: PROJECT_SCHEMA_V1.to_owned(),
            doc_id: doc_id.into(),
            revision: 0,
            saved_revision: Some(0),
            stations: None,
            catalog_selection: CatalogSelection {
                entry_id: "ecmwf_ifs.temperature_2m".into(),
                catalog_version: "2026-11-23.1".into(),
            },
            window: PreviewWindow::default(),
            max_distance_m: 7200,
            sql_tabs: Vec::new(),
            view: ProjectViewState::default(),
            path: None,
        }
    }

    pub fn doc_id(&self) -> &str {
        &self.doc_id
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn set_path(&mut self, path: Option<PathBuf>) {
        self.path = path;
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

    pub fn save_to_path(
        &mut self,
        target: &Path,
        app_data: Option<&Path>,
    ) -> Result<(), ProjectFileError> {
        self.schema = PROJECT_SCHEMA_V1.to_owned();
        let bytes = serde_json::to_vec_pretty(self)?;
        let parent = target.parent().unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(parent)?;
        let file_name = target
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("project.omoproj");
        let tmp_path = parent.join(format!(".{file_name}.tmp"));
        {
            use std::io::Write;
            let mut file = std::fs::File::create(&tmp_path)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
        }
        std::fs::rename(&tmp_path, target)?;
        if let Ok(dir) = std::fs::File::open(parent) {
            let _ = dir.sync_all();
        }
        self.path = Some(target.to_path_buf());
        self.mark_saved();
        if let Some(app_data) = app_data {
            let _ = crate::draft::discard(app_data, &self.doc_id);
        }
        Ok(())
    }

    pub fn open_from_path(target: &Path) -> Result<OpenOutcome, ProjectFileError> {
        let bytes = std::fs::read(target)?;
        let mut doc: ProjectDoc = serde_json::from_slice(&bytes)?;
        let major = doc
            .schema
            .strip_prefix("omo-project/")
            .and_then(|v| v.split('.').next())
            .and_then(|v| v.parse::<u32>().ok());
        if major != Some(1) {
            return Err(ProjectFileError::UnsupportedSchema(doc.schema));
        }
        let mut notice = None;
        if let Some(snapshot) = &doc.stations {
            let actual = format!("{:x}", Sha256::digest(canonical_bytes(&snapshot.rows)));
            if actual != snapshot.content_sha256 {
                return Err(ProjectFileError::CorruptSnapshot {
                    expected: snapshot.content_sha256.clone(),
                    actual,
                });
            }
            if let Ok(orig_bytes) = std::fs::read(&snapshot.source.orig_path) {
                let orig_sha = format!("{:x}", Sha256::digest(&orig_bytes));
                if orig_sha != snapshot.source.orig_sha256 {
                    notice = Some(format!(
                        "Original station CSV `{}` changed on disk; keeping saved snapshot.",
                        snapshot.source.orig_path
                    ));
                }
            }
        }
        doc.revision = 0;
        doc.saved_revision = Some(0);
        doc.path = Some(target.to_path_buf());
        Ok(OpenOutcome {
            doc,
            source_changed_notice: notice,
        })
    }
}
