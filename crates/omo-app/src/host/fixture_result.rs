//! Development seam: publish an existing Arrow IPC file as the initial result.
//!
//! The N2/N3 acceptance workloads need a 1,000,000 x 20 result and a
//! 20 x 10,000 plot result in the table and chart. Interactive preview queries
//! are capped at 100,000 rows (FR-015), and M0 has no job runner, so the
//! workloads are produced by `omo-harness gen-result` and published here. The
//! file is read through the same `ResultRef` and `IpcPageReader` paths a worker
//! result uses; only the provenance of the reference differs.

use std::{
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
};

use arrow_ipc::reader::FileReader;
use omo_protocol::result_ref::{ResultLimits, ResultRef, schema_hash_from_debug};

/// Environment variable naming a generated Arrow IPC file to publish.
pub const ENV_VAR: &str = "OMOBENCH_FIXTURE_RESULT";

/// Return the fixture result path configured via `OMOBENCH_FIXTURE_RESULT`, if any.
/// Performs no filesystem I/O.
pub fn env_path() -> Option<PathBuf> {
    std::env::var_os(ENV_VAR)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// Open and describe a fixture Arrow IPC file off the UI thread.
pub fn load_from_path(path: &Path) -> Result<ResultRef, String> {
    describe(path).map_err(|error| {
        format!(
            "Could not publish fixture result {}: {error}",
            path.display()
        )
    })
}

/// Read `OMOBENCH_FIXTURE_RESULT`, if set, and describe the file it names.
pub fn from_env() -> (Option<ResultRef>, Option<String>) {
    let Some(path) = env_path() else {
        return (None, None);
    };
    match load_from_path(&path) {
        Ok(reference) => (Some(reference), None),
        Err(error) => (None, Some(error)),
    }
}

/// Count the file's rows and hash its schema, reusing the result-reference shape.
fn describe(path: &Path) -> Result<ResultRef, String> {
    let file = File::open(path).map_err(|error| error.to_string())?;
    let mut reader =
        FileReader::try_new(BufReader::new(file), None).map_err(|error| error.to_string())?;
    let schema = reader.schema();
    let mut rows = 0u64;
    for batch in &mut reader {
        let batch = batch.map_err(|error| error.to_string())?;
        rows = rows.saturating_add(batch.num_rows() as u64);
    }
    Ok(ResultRef {
        id: "fixture-result".into(),
        generation: 0,
        path: path.to_path_buf(),
        rows,
        schema_hash: schema_hash_from_debug(&format!("{schema:#?}")),
        truncated: false,
        limits: ResultLimits { rows, bytes: 0 },
    })
}
