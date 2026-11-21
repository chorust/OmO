//! Recovery inspection for interrupted exports. A leftover is touched only when its
//! application-owned marker or sidecar schema identifies it as an Omobench transaction.
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
};

use omo_protocol::export::{SIDECAR_SCHEMA, STAGING_OWNER_MARKER, sidecar_path};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportVerdict {
    Finished,
    Incomplete,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ExportRecovery {
    pub target: PathBuf,
    pub verdict: ExportVerdict,
}

/// A sidecar is authoritative only when its schema and the data digest both match.
pub fn is_saved(target: &Path) -> io::Result<bool> {
    let sidecar = sidecar_path(target);
    if !is_known_sidecar(&sidecar)? {
        return Ok(false);
    }
    let bytes = fs::read(sidecar)?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let Some(expected) = value.pointer("/data/sha256").and_then(|v| v.as_str()) else {
        return Ok(false);
    };
    let mut file = match fs::File::open(target) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hash.finalize()) == expected)
}

/// Inspect leftovers in one recent export directory, deleting only completed transactions.
pub fn scan_export_dir(dir: &Path) -> io::Result<Vec<ExportRecovery>> {
    let mut leftovers: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        let filename = entry.file_name();
        let Some(name) = filename.to_str().and_then(|s| s.strip_prefix('.')) else {
            continue;
        };
        let target = if let Some(target) = name.strip_suffix(".omo-old") {
            (!target.is_empty() && metadata.file_type().is_file() && is_known_sidecar(&path)?)
                .then_some(target)
        } else if let Some((target, id)) = name.rsplit_once(".omo-tmp-") {
            let marker = path.join(".omo-owner");
            (!target.is_empty()
                && !id.is_empty()
                && metadata.file_type().is_dir()
                && fs::read(marker).is_ok_and(|bytes| bytes == STAGING_OWNER_MARKER))
            .then_some(target)
        } else {
            None
        };
        if let Some(target) = target {
            leftovers.entry(target.to_owned()).or_default().push(path);
        }
    }

    let mut results = Vec::new();
    for (name, paths) in leftovers {
        let target = dir.join(name);
        let verdict = if is_saved(&target)? {
            for path in paths {
                let metadata = fs::symlink_metadata(&path)?;
                if metadata.file_type().is_dir() {
                    fs::remove_dir_all(path)?;
                } else if metadata.file_type().is_file() {
                    fs::remove_file(path)?;
                }
            }
            ExportVerdict::Finished
        } else {
            ExportVerdict::Incomplete
        };
        results.push(ExportRecovery { target, verdict });
    }
    Ok(results)
}

fn is_known_sidecar(path: &Path) -> io::Result<bool> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    if !metadata.file_type().is_file() {
        return Ok(false);
    }
    let bytes = fs::read(path)?;
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return Ok(false);
    };
    Ok(value.get("schema").and_then(|v| v.as_str()) == Some(SIDECAR_SCHEMA))
}
