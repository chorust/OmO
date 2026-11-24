//! Crash-recovery drafts. Drafts never replace a project file (research R-05).

use std::{
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

use serde::{Deserialize, Serialize};

use crate::doc::ProjectDoc;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Draft {
    pub doc_id: String,
    pub project_path: Option<PathBuf>,
    pub base_saved_mtime_unix: Option<u64>,
    pub staged_at_utc: String,
    pub doc: ProjectDoc,
}

#[derive(Debug)]
pub struct RecoverableDraft {
    pub path: PathBuf,
    pub draft: Draft,
}

pub fn drafts_dir(app_data: &Path) -> PathBuf {
    app_data.join("drafts")
}

pub fn stage(
    doc: &ProjectDoc,
    app_data: &Path,
    staged_at_utc: &str,
) -> Result<PathBuf, std::io::Error> {
    let dir = drafts_dir(app_data);
    fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.json", doc.doc_id()));
    let draft = Draft {
        doc_id: doc.doc_id().to_owned(),
        project_path: doc.path().map(Path::to_path_buf),
        base_saved_mtime_unix: None,
        staged_at_utc: staged_at_utc.to_owned(),
        doc: doc.clone(),
    };
    let bytes = serde_json::to_vec_pretty(&draft).expect("draft JSON");
    let tmp = dir.join(format!(".{}.json.tmp", doc.doc_id()));
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, &path)?;
    Ok(path)
}

pub fn list_recoverable(app_data: &Path) -> Result<Vec<RecoverableDraft>, std::io::Error> {
    let dir = drafts_dir(app_data);
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut found = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let Ok(bytes) = fs::read(&path) else {
            continue;
        };
        let Ok(draft) = serde_json::from_slice::<Draft>(&bytes) else {
            continue;
        };
        if is_recoverable(&draft) {
            found.push(RecoverableDraft { path, draft });
        }
    }
    Ok(found)
}

pub fn is_recoverable(draft: &Draft) -> bool {
    match &draft.project_path {
        None => true,
        Some(path) => match fs::metadata(path).and_then(|meta| meta.modified()).ok() {
            Some(project_mtime) => draft_is_newer(&draft.staged_at_utc, project_mtime),
            None => true,
        },
    }
}

pub fn discard(app_data: &Path, doc_id: &str) -> Result<(), std::io::Error> {
    let path = drafts_dir(app_data).join(format!("{doc_id}.json"));
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

pub fn clear_after_save(app_data: &Path, doc_id: &str) -> Result<(), std::io::Error> {
    discard(app_data, doc_id)
}

fn draft_is_newer(staged_at_utc: &str, project_mtime: SystemTime) -> bool {
    let Ok(staged) = parse_utc(staged_at_utc) else {
        return true;
    };
    let project = project_mtime
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    staged > project
}

fn parse_utc(value: &str) -> Result<u64, ()> {
    let digits: String = value.chars().filter(|ch| ch.is_ascii_digit()).collect();
    if digits.len() < 14 {
        return Err(());
    }
    let year: i64 = digits[0..4].parse().map_err(|_| ())?;
    let month: u32 = digits[4..6].parse().map_err(|_| ())?;
    let day: u32 = digits[6..8].parse().map_err(|_| ())?;
    let hour: u32 = digits[8..10].parse().map_err(|_| ())?;
    let minute: u32 = digits[10..12].parse().map_err(|_| ())?;
    let second: u32 = digits[12..14].parse().map_err(|_| ())?;
    if !(1..=12).contains(&month) || day == 0 || hour > 23 || minute > 59 || second > 59 {
        return Err(());
    }
    let days = omo_protocol::utc::days_from_civil(year, i64::from(month), i64::from(day));
    Ok((days * 86400 + i64::from(hour) * 3600 + i64::from(minute) * 60 + i64::from(second)) as u64)
}
