//! UI-free project persistence, auto-draft staging, and recovery helpers.

use std::{
    env,
    path::{Path, PathBuf},
};

use omo_project::{
    doc::{OpenOutcome, ProjectDoc, ProjectFileError},
    stations::import::write_canonical,
};

pub fn app_data_dir() -> PathBuf {
    if let Some(dir) = env::var_os("OMOBENCH_APP_DATA_DIR") {
        return PathBuf::from(dir);
    }
    if let Some(xdg) = env::var_os("XDG_DATA_HOME") {
        return PathBuf::from(xdg).join("omobench");
    }
    if let Some(home) = env::var_os("HOME") {
        return PathBuf::from(home).join(".local/share/omobench");
    }
    env::temp_dir().join("omobench-data")
}

#[derive(Debug)]
pub struct OpenProjectResult {
    pub outcome: OpenOutcome,
    pub canonical_stations_path: Option<PathBuf>,
}

pub fn open_project(path: &Path, app_data: &Path) -> Result<OpenProjectResult, ProjectFileError> {
    let outcome = ProjectDoc::open_from_path(path)?;
    let canonical_stations_path = if let Some(snapshot) = &outcome.doc.stations {
        let written = write_canonical(snapshot, &app_data.join("stations"))
            .map_err(|error| ProjectFileError::Io(std::io::Error::other(error.to_string())))?;
        Some(written)
    } else {
        None
    };
    Ok(OpenProjectResult {
        outcome,
        canonical_stations_path,
    })
}

pub fn materialize_stations(doc: &ProjectDoc, app_data: &Path) -> Option<PathBuf> {
    let snapshot = doc.stations.as_ref()?;
    write_canonical(snapshot, &app_data.join("stations")).ok()
}
