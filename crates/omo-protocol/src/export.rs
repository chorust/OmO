//! Shared export transaction constants and path helpers.

use std::path::{Path, PathBuf};

/// Marker written to `.omo-owner` inside every export staging directory.
pub const STAGING_OWNER_MARKER: &[u8] = b"omo-export-staging/1\n";

/// Schema identifier stored in `<target>.omo.json` export provenance sidecars.
pub const SIDECAR_SCHEMA: &str = "omo-export-sidecar/1";

/// Filename suffix appended to an export target to name its provenance sidecar.
pub const SIDECAR_SUFFIX: &str = ".omo.json";

/// Return the provenance sidecar path `<target>.omo.json` beside `target`.
pub fn sidecar_path(target: &Path) -> PathBuf {
    let mut path = target.as_os_str().to_os_string();
    path.push(SIDECAR_SUFFIX);
    path.into()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::sidecar_path;

    #[test]
    fn sidecar_path_appends_suffix_to_target() {
        assert_eq!(
            sidecar_path(Path::new("/tmp/report.csv")),
            Path::new("/tmp/report.csv.omo.json")
        );
    }
}
