use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use duckdb::{Config, Connection};
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;
use thiserror::Error;

pub use crate::config::Channel;

use crate::{
    config::BootstrapConfig,
    lock::{ArtifactLockEntry, ArtifactMismatch},
    probe::{self, ProbeReport},
};

static WRITE_PROBE_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Error)]
pub enum BootstrapError {
    #[error(transparent)]
    ArtifactMismatch(#[from] ArtifactMismatch),
    #[error("invalid bootstrap configuration: {reason}")]
    Configuration { reason: String },
    #[error("DuckDB failed during {stage}: {reason}")]
    DuckDb { stage: &'static str, reason: String },
    #[error("duckomo failed to load through {channel}: {reason}")]
    Duckomo {
        channel: &'static str,
        reason: String,
    },
    #[error("probe `{probe}` failed: expected {expected}, got {actual}")]
    ProbeFailed {
        probe: &'static str,
        expected: String,
        actual: String,
    },
    #[error("golden query result mismatch: expected {expected}, got {actual}")]
    GoldenMismatch { expected: String, actual: String },
    #[error("DuckDB configuration could not be locked: {reason}")]
    ConfigLock { reason: String },
    #[error("the connection was discarded after a failed pre-task probe")]
    ConnectionDiscarded,
    #[error("capability `{capability}` is unavailable: {reason}")]
    CapabilityUnavailable {
        capability: &'static str,
        reason: String,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "status", content = "reason", rename_all = "snake_case")]
pub enum HttpfsState {
    Loaded,
    Installed,
    Unavailable(String),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "status", content = "reason", rename_all = "snake_case")]
pub enum SpatialState {
    NotRequested,
    Loaded,
    Failed(String),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GoldenResult {
    pub fixture: String,
    pub query_id: String,
    pub expected: Option<String>,
    pub actual: String,
    pub rows: usize,
    pub ok: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct BootstrapTimings {
    pub open: u64,
    pub httpfs: u64,
    pub duckomo: u64,
    pub lock: u64,
    pub probes: u64,
    pub golden: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct R4Report {
    pub unsigned_close: String,
    pub community_close: String,
    pub repository_close: String,
    pub spatial_preinstall: String,
    pub spatial_after_lock: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BootstrapReport {
    pub duckdb_version: String,
    pub platform: String,
    pub duckomo_version: String,
    pub channel: Channel,
    pub artifact_sha256: String,
    pub httpfs: HttpfsState,
    pub spatial: SpatialState,
    pub probe: ProbeReport,
    pub golden: GoldenResult,
    pub timings_ms: BootstrapTimings,
    pub extension_settings: Vec<(String, String)>,
    pub r4: R4Report,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Capabilities {
    pub remote: bool,
    pub spatial: bool,
}

pub(crate) fn open_and_bootstrap(
    config: &BootstrapConfig,
    allow_golden_placeholder: bool,
) -> Result<(Connection, BootstrapReport), BootstrapError> {
    validate_config(config, allow_golden_placeholder)?;
    let channel = config
        .channel()
        .map_err(|reason| BootstrapError::Configuration { reason })?;
    let ext_dir =
        config
            .ext_cache_dir
            .canonicalize()
            .map_err(|error| BootstrapError::Configuration {
                reason: format!("cannot resolve extension cache directory: {error}"),
            })?;
    restrict_directory(&ext_dir).map_err(|error| BootstrapError::Configuration {
        reason: format!("cannot restrict extension cache directory permissions: {error}"),
    })?;

    let verified_artifact =
        verified_artifact_path(&config.artifact, &config.artifact_path, &ext_dir)?;
    let repository_metadata_dir = ext_dir.join("omo-extension-repositories");
    fs::create_dir_all(&repository_metadata_dir).map_err(|error| {
        BootstrapError::Configuration {
            reason: format!("cannot create extension repository metadata directory: {error}"),
        }
    })?;
    let repository_metadata_dir =
        repository_metadata_dir
            .canonicalize()
            .map_err(|error| BootstrapError::Configuration {
                reason: format!("cannot resolve extension repository metadata directory: {error}"),
            })?;
    if !repository_metadata_dir.starts_with(&ext_dir) {
        return Err(BootstrapError::Configuration {
            reason: "extension repository metadata directory escaped the configured cache"
                .to_owned(),
        });
    }
    restrict_directory(&repository_metadata_dir).map_err(|error| {
        BootstrapError::Configuration {
            reason: format!("cannot restrict extension repository metadata permissions: {error}"),
        }
    })?;
    let community_repository =
        (channel == Channel::CommunitySigned).then(|| community_repository_name(&config.artifact));

    let open_started = Instant::now();
    let mut duckdb_config = Config::default();
    for (name, value) in [
        (
            "extension_directory",
            ext_dir.to_string_lossy().into_owned(),
        ),
        (
            "extension_repository_directory",
            repository_metadata_dir.to_string_lossy().into_owned(),
        ),
        (
            "allow_unsigned_extensions",
            channel.allows_unsigned_extensions().to_string(),
        ),
        ("allow_community_extensions", "false".to_owned()),
        (
            "allow_extension_repositories",
            if community_repository.is_some() {
                "allowed".to_owned()
            } else {
                "undecided".to_owned()
            },
        ),
        ("allow_extensions_metadata_mismatch", "false".to_owned()),
        ("autoinstall_known_extensions", "false".to_owned()),
        ("autoload_known_extensions", "false".to_owned()),
    ] {
        duckdb_config =
            duckdb_config
                .with(name, value)
                .map_err(|error| BootstrapError::DuckDb {
                    stage: "open_config",
                    reason: error.to_string(),
                })?;
    }
    if let Some(threads) = config.threads {
        duckdb_config = duckdb_config
            .with("threads", threads.to_string())
            .map_err(|error| BootstrapError::DuckDb {
                stage: "open_config",
                reason: error.to_string(),
            })?;
    }
    if let Some(memory_limit) = &config.memory_limit {
        duckdb_config = duckdb_config
            .with("memory_limit", memory_limit)
            .map_err(|error| BootstrapError::DuckDb {
                stage: "open_config",
                reason: error.to_string(),
            })?;
    }
    let database_path = config.database_path();
    let conn = Connection::open_with_flags(&database_path, duckdb_config).map_err(|error| {
        BootstrapError::DuckDb {
            stage: "open",
            reason: error.to_string(),
        }
    })?;
    let open_ms = millis(open_started);

    let httpfs_started = Instant::now();
    let (httpfs, remote) =
        provision_httpfs(&conn, &ext_dir, config.no_network, config.httpfs_timeout());
    let httpfs_ms = millis(httpfs_started);
    if channel == Channel::GithubReleaseUnsigned {
        conn.execute_batch("SET allow_extension_repositories = 'forbidden';")
            .map_err(|error| BootstrapError::ConfigLock {
                reason: format!(
                    "could not close extension repository access before duckomo: {error}"
                ),
            })?;
    }

    let extension_settings_before_load = extension_settings_snapshot(&conn)?;
    let duckomo_started = Instant::now();
    let cache_lock = community_repository
        .as_deref()
        .map(|name| lock_extension_cache(&ext_dir, name))
        .transpose()?;
    let load_result = load_selected_channel(channel, || {
        let artifact = sql_literal(&verified_artifact.decompressed_path.to_string_lossy());
        match channel {
            Channel::CommunitySigned => {
                let repository = community_repository
                    .as_deref()
                    .ok_or_else(|| "community-signed channel has no local repository".to_owned())?;
                ensure_community_artifact(
                    &conn,
                    &config.artifact,
                    &verified_artifact.compressed,
                    config.platform,
                    &ext_dir,
                    repository,
                )?;
                conn.execute_batch(&format!(
                    "LOAD {} FROM {repository};",
                    config.artifact.extension
                ))
                .map_err(|error| {
                    format!(
                        "LOAD duckomo from the verified local community repository failed: {error}"
                    )
                })
            }
            Channel::GithubReleaseUnsigned => conn
                .execute_batch(&format!("LOAD {artifact};"))
                .map_err(|error| format!("LOAD verified artifact failed: {error}")),
        }
    });
    if let Err(error) = load_result {
        return Err(match error {
            BootstrapError::Duckomo { channel, reason } => BootstrapError::Duckomo {
                channel,
                reason: format!(
                    "{reason}; extension settings before load: {extension_settings_before_load:?}"
                ),
            },
            other => other,
        });
    }
    let duckomo_ms = millis(duckomo_started);
    let community_close = "disabled_at_open".to_owned();
    let community_extensions = setting(&conn, "allow_community_extensions", "configuration_lock")?;
    if community_extensions != "false" {
        return Err(BootstrapError::ConfigLock {
            reason: format!("allow_community_extensions remained {community_extensions}"),
        });
    }
    let repository_close = if channel == Channel::CommunitySigned {
        conn.execute_batch("SET allow_extension_repositories = 'forbidden';")
            .map_err(|error| BootstrapError::ConfigLock {
                reason: format!("could not close local extension repository access: {error}"),
            })?;
        "forbidden_after_load".to_owned()
    } else {
        "forbidden_before_duckomo".to_owned()
    };
    let repository_policy = setting(&conn, "allow_extension_repositories", "configuration_lock")?;
    if repository_policy != "forbidden" {
        return Err(BootstrapError::ConfigLock {
            reason: format!("allow_extension_repositories remained {repository_policy}"),
        });
    }
    drop(cache_lock);

    let spatial_preinstall = if config.install_spatial_before_lock {
        if config.no_network {
            "skipped: network access disabled by probe mode".to_owned()
        } else {
            match conn.execute_batch("INSTALL spatial;") {
                Ok(()) => "ok".to_owned(),
                Err(error) => format!("failed: {error}"),
            }
        }
    } else {
        "not_requested".to_owned()
    };

    let lock_started = Instant::now();
    let unsigned_close = if channel.allows_unsigned_extensions() {
        conn.execute_batch("SET allow_unsigned_extensions = false;")
            .map_err(|error| BootstrapError::ConfigLock {
                reason: format!("could not close the unsigned extension window: {error}"),
            })?;
        let actual = setting(&conn, "allow_unsigned_extensions", "configuration_lock")?;
        if actual != "false" {
            return Err(BootstrapError::ConfigLock {
                reason: format!("allow_unsigned_extensions remained {actual}"),
            });
        }
        "ok".to_owned()
    } else {
        "not_applicable".to_owned()
    };

    let mut allowed = vec![
        config.data_dir.to_string_lossy().into_owned(),
        config
            .fixture_path
            .parent()
            .unwrap_or_else(|| Path::new("/"))
            .to_string_lossy()
            .into_owned(),
        ext_dir.to_string_lossy().into_owned(),
        database_path
            .parent()
            .unwrap_or_else(|| Path::new("/"))
            .to_string_lossy()
            .into_owned(),
    ];
    allowed.sort();
    allowed.dedup();
    let mut allowed_sql = allowed
        .iter()
        .map(|path| sql_literal(path))
        .collect::<Vec<_>>();
    if remote {
        allowed_sql.extend([
            "'http://'".to_owned(),
            "'https://'".to_owned(),
            "'s3://'".to_owned(),
        ]);
    }
    let lock_sql = format!(
        "SET allowed_directories = [{}]; SET lock_configuration = true;",
        allowed_sql.join(",")
    );
    conn.execute_batch(&lock_sql)
        .map_err(|error| BootstrapError::ConfigLock {
            reason: error.to_string(),
        })?;
    let locked = setting(&conn, "lock_configuration", "configuration_lock")?;
    if locked != "true" {
        return Err(BootstrapError::ConfigLock {
            reason: format!("lock_configuration reported {locked}"),
        });
    }
    let extension_settings = extension_settings_snapshot(&conn)?;
    let lock_ms = millis(lock_started);

    let probe_started = Instant::now();
    let probe = probe::collect_bootstrap_probes(&conn, config, channel)?;
    let probes_ms = millis(probe_started);

    let golden_started = Instant::now();
    let (actual_golden, rows) = probe::golden_result_hash(&conn, &config.fixture_path)?;
    let is_placeholder =
        config.artifact.golden.result_sha256 == "<filled only after a verified bootstrap run>";
    let expected = (!is_placeholder).then(|| config.artifact.golden.result_sha256.clone());
    let golden_ok = expected
        .as_ref()
        .is_some_and(|value| value == &actual_golden);
    if let Some(expected) = &expected {
        if !golden_ok {
            return Err(BootstrapError::GoldenMismatch {
                expected: expected.clone(),
                actual: actual_golden,
            });
        }
    } else if !allow_golden_placeholder {
        return Err(BootstrapError::ArtifactMismatch(
            ArtifactMismatch::PlaceholderGoldenHash,
        ));
    }
    let golden_ms = millis(golden_started);

    let report = BootstrapReport {
        duckdb_version: probe.duckdb_version.clone(),
        platform: probe.platform.clone(),
        duckomo_version: probe.duckomo_version.clone(),
        channel,
        artifact_sha256: config.artifact.sha256_decompressed.clone(),
        httpfs,
        spatial: SpatialState::NotRequested,
        probe,
        golden: GoldenResult {
            fixture: config.artifact.golden.fixture.clone(),
            query_id: config.artifact.golden.query_id.clone(),
            expected,
            actual: actual_golden,
            rows,
            ok: golden_ok,
        },
        timings_ms: BootstrapTimings {
            open: open_ms,
            httpfs: httpfs_ms,
            duckomo: duckomo_ms,
            lock: lock_ms,
            probes: probes_ms,
            golden: golden_ms,
        },
        extension_settings,
        r4: R4Report {
            unsigned_close,
            community_close,
            repository_close,
            spatial_preinstall,
            spatial_after_lock: "not_run".to_owned(),
        },
    };
    Ok((conn, report))
}

/// First-run measurement only. The connection is dropped before this returns, so a placeholder
/// golden hash can never produce a task-capable `BootstrappedConn`.
pub fn measure_bootstrap(config: BootstrapConfig) -> Result<BootstrapReport, BootstrapError> {
    let (_, report) = open_and_bootstrap(&config, true)?;
    Ok(report)
}

fn validate_config(
    config: &BootstrapConfig,
    allow_golden_placeholder: bool,
) -> Result<(), BootstrapError> {
    if !config.requires_absolute_paths() {
        return Err(BootstrapError::Configuration {
            reason: "data, extension-cache, artifact, fixture, and database paths must be absolute"
                .to_owned(),
        });
    }
    if !config.data_dir.is_dir() {
        return Err(BootstrapError::Configuration {
            reason: format!(
                "data directory does not exist: {}",
                config.data_dir.display()
            ),
        });
    }
    verify_writable_directory(&config.data_dir)?;
    fs::create_dir_all(&config.ext_cache_dir).map_err(|error| BootstrapError::Configuration {
        reason: format!("cannot create extension cache directory: {error}"),
    })?;
    verify_writable_directory(&config.ext_cache_dir)?;
    if !config.artifact_path.is_file() {
        return Err(BootstrapError::Configuration {
            reason: format!(
                "locked artifact file does not exist: {}",
                config.artifact_path.display()
            ),
        });
    }
    if !config.fixture_path.is_file() {
        return Err(BootstrapError::Configuration {
            reason: format!(
                "golden fixture does not exist: {}",
                config.fixture_path.display()
            ),
        });
    }
    if let Some(parent) = config.database_path().parent() {
        if !parent.is_dir() {
            return Err(BootstrapError::Configuration {
                reason: format!(
                    "database parent directory does not exist: {}",
                    parent.display()
                ),
            });
        }
    }
    let channel = config
        .channel()
        .map_err(|reason| BootstrapError::Configuration { reason })?;
    if config.artifact.platform != config.platform.as_str() {
        return Err(BootstrapError::Configuration {
            reason: format!(
                "artifact platform {} does not match configured platform {}",
                config.artifact.platform,
                config.platform.as_str()
            ),
        });
    }
    if config.artifact.extension != "duckomo"
        || config.artifact.duckdb_version != probe::EXPECTED_DUCKDB_VERSION
        || config.artifact.channel != channel.as_str()
    {
        return Err(BootstrapError::Configuration {
            reason: "artifact entry does not match the supported engine, extension, or channel"
                .to_owned(),
        });
    }
    if allow_golden_placeholder {
        config.artifact.validate_for_golden_measurement()?;
    } else {
        config.artifact.validate()?;
    }
    if config
        .artifact_path
        .file_name()
        .and_then(|name| name.to_str())
        != Some(config.artifact.file.as_str())
    {
        return Err(BootstrapError::Configuration {
            reason: "artifact path filename does not match the lock entry".to_owned(),
        });
    }
    if !config
        .fixture_path
        .ends_with(Path::new(&config.artifact.golden.fixture))
    {
        return Err(BootstrapError::Configuration {
            reason: "fixture path does not match the lock entry".to_owned(),
        });
    }
    if config.httpfs_timeout_ms > 15_000 {
        return Err(BootstrapError::Configuration {
            reason: "HTTPFS installation timeout may not exceed 15000 ms".to_owned(),
        });
    }
    Ok(())
}

fn verify_writable_directory(path: &Path) -> Result<(), BootstrapError> {
    let id = WRITE_PROBE_ID.fetch_add(1, Ordering::Relaxed);
    let probe = path.join(format!(".omo-write-probe-{}-{id}", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
        .map_err(|error| BootstrapError::Configuration {
            reason: format!("data directory is not writable: {error}"),
        })?;
    file.write_all(b"omo")
        .map_err(|error| BootstrapError::Configuration {
            reason: format!("cannot write in data directory: {error}"),
        })?;
    drop(file);
    fs::remove_file(probe).map_err(|error| BootstrapError::Configuration {
        reason: format!("cannot remove data-directory write probe: {error}"),
    })
}

struct VerifiedArtifact {
    compressed: Vec<u8>,
    decompressed_path: PathBuf,
}

fn verified_artifact_path(
    entry: &ArtifactLockEntry,
    compressed_path: &Path,
    ext_dir: &Path,
) -> Result<VerifiedArtifact, BootstrapError> {
    let metadata =
        fs::metadata(compressed_path).map_err(|error| BootstrapError::Configuration {
            reason: format!("cannot inspect locked artifact: {error}"),
        })?;
    if metadata.len() != entry.size {
        return Err(ArtifactMismatch::Mismatch {
            field: "size",
            expected: entry.size.to_string(),
            actual: metadata.len().to_string(),
        }
        .into());
    }
    let compressed = fs::read(compressed_path).map_err(|error| BootstrapError::Configuration {
        reason: format!("cannot read locked artifact: {error}"),
    })?;
    entry.verify_compressed(&compressed)?;

    let mut decoder = GzDecoder::new(compressed.as_slice());
    let capacity = usize::try_from(entry.size_decompressed.min(128 * 1024 * 1024))
        .unwrap_or(128 * 1024 * 1024);
    let mut decompressed = Vec::with_capacity(capacity);
    decoder
        .by_ref()
        .take(entry.size_decompressed.saturating_add(1))
        .read_to_end(&mut decompressed)
        .map_err(|error| BootstrapError::DuckDb {
            stage: "artifact_decompress",
            reason: error.to_string(),
        })?;
    drop(decoder);
    entry.verify_decompressed(&decompressed)?;

    let cache_root = ext_dir
        .canonicalize()
        .map_err(|error| BootstrapError::Configuration {
            reason: format!("cannot resolve extension cache directory: {error}"),
        })?;
    let dest_dir = ext_dir
        .join("omo-artifacts")
        .join(entry.version.as_str())
        .join(entry.channel.as_str())
        .join(entry.platform.as_str());
    fs::create_dir_all(&dest_dir).map_err(|error| BootstrapError::Configuration {
        reason: format!("cannot create verified artifact cache: {error}"),
    })?;
    let dest_dir = dest_dir
        .canonicalize()
        .map_err(|error| BootstrapError::Configuration {
            reason: format!("cannot resolve verified artifact cache: {error}"),
        })?;
    if !dest_dir.starts_with(&cache_root) {
        return Err(BootstrapError::Configuration {
            reason: "verified artifact cache escaped the configured extension directory".to_owned(),
        });
    }
    // DuckDB derives the extension entrypoint from this basename. Keep the canonical name
    // even for versioned GitHub release assets; the digest is verified before this cache path.
    let dest = dest_dir.join("duckomo.duckdb_extension");
    match fs::symlink_metadata(&dest) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            fs::remove_file(&dest).map_err(|error| BootstrapError::Configuration {
                reason: format!("cannot remove symlinked cached duckomo artifact: {error}"),
            })?;
        }
        Ok(metadata) if metadata.is_file() => {
            let existing = fs::read(&dest).map_err(|error| BootstrapError::Configuration {
                reason: format!("cannot read cached duckomo artifact: {error}"),
            })?;
            if entry.verify_decompressed(&existing).is_ok() {
                return Ok(VerifiedArtifact {
                    compressed,
                    decompressed_path: dest,
                });
            }
            fs::remove_file(&dest).map_err(|error| BootstrapError::Configuration {
                reason: format!("cannot remove invalid cached duckomo artifact: {error}"),
            })?;
        }
        Ok(_) => {
            return Err(BootstrapError::Configuration {
                reason: format!(
                    "cached duckomo path is not a regular file: {}",
                    dest.display()
                ),
            });
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(BootstrapError::Configuration {
                reason: format!("cannot inspect cached duckomo artifact: {error}"),
            });
        }
    }
    let mut staged =
        NamedTempFile::new_in(&dest_dir).map_err(|error| BootstrapError::Configuration {
            reason: format!("cannot stage verified duckomo artifact: {error}"),
        })?;
    staged
        .write_all(&decompressed)
        .and_then(|()| staged.as_file().sync_all())
        .map_err(|error| BootstrapError::Configuration {
            reason: format!("cannot persist verified duckomo artifact: {error}"),
        })?;
    staged
        .persist(&dest)
        .map_err(|error| BootstrapError::Configuration {
            reason: format!("cannot publish verified duckomo artifact: {}", error.error),
        })?;
    Ok(VerifiedArtifact {
        compressed,
        decompressed_path: dest,
    })
}

fn community_repository_name(entry: &ArtifactLockEntry) -> String {
    let component = |value: &str| {
        value
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() {
                    character.to_ascii_lowercase()
                } else {
                    '_'
                }
            })
            .collect::<String>()
    };
    format!(
        "omo_duckomo_{}_{}_{}_{}",
        component(&entry.version),
        component(&entry.duckdb_version),
        component(&entry.platform),
        &entry.sha256[..8]
    )
}

fn lock_extension_cache(ext_dir: &Path, repository: &str) -> Result<std::fs::File, BootstrapError> {
    let lock_dir = ext_dir.join("omo-artifact-locks");
    fs::create_dir_all(&lock_dir).map_err(|error| BootstrapError::Configuration {
        reason: format!("cannot create extension artifact lock directory: {error}"),
    })?;
    let cache_root = ext_dir
        .canonicalize()
        .map_err(|error| BootstrapError::Configuration {
            reason: format!("cannot resolve extension cache directory: {error}"),
        })?;
    let lock_dir = lock_dir
        .canonicalize()
        .map_err(|error| BootstrapError::Configuration {
            reason: format!("cannot resolve extension artifact lock directory: {error}"),
        })?;
    if !lock_dir.starts_with(&cache_root) {
        return Err(BootstrapError::Configuration {
            reason: "extension artifact lock directory escaped the configured cache".to_owned(),
        });
    }
    restrict_directory(&lock_dir).map_err(|error| BootstrapError::Configuration {
        reason: format!("cannot restrict extension artifact lock directory permissions: {error}"),
    })?;
    let lock_path = lock_dir.join(format!("{repository}.lock"));
    if fs::symlink_metadata(&lock_path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(BootstrapError::Configuration {
            reason: format!(
                "extension artifact lock is a symlink: {}",
                lock_path.display()
            ),
        });
    }
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(&lock_path)
        .map_err(|error| BootstrapError::Configuration {
            reason: format!("cannot open extension artifact lock: {error}"),
        })?;
    restrict_file(&lock_path).map_err(|error| BootstrapError::Configuration {
        reason: format!("cannot restrict extension artifact lock permissions: {error}"),
    })?;
    lock.lock().map_err(|error| BootstrapError::Configuration {
        reason: format!("cannot lock extension artifact cache: {error}"),
    })?;
    Ok(lock)
}

fn ensure_community_artifact(
    conn: &Connection,
    entry: &ArtifactLockEntry,
    compressed: &[u8],
    platform: crate::config::Platform,
    ext_dir: &Path,
    repository: &str,
) -> Result<(), String> {
    entry.verify_compressed(compressed).map_err(|error| {
        format!("locked compressed community artifact failed verification: {error}")
    })?;
    let cache_root = ext_dir
        .canonicalize()
        .map_err(|error| format!("cannot resolve extension cache directory: {error}"))?;
    let repository_prefix = ext_dir.join("omo-community-repositories").join(repository);
    fs::create_dir_all(&repository_prefix)
        .map_err(|error| format!("cannot create local community repository: {error}"))?;
    let repository_prefix = repository_prefix
        .canonicalize()
        .map_err(|error| format!("cannot resolve local community repository: {error}"))?;
    if !repository_prefix.starts_with(&cache_root) {
        return Err("local community repository escaped the configured cache".to_owned());
    }
    restrict_directory(&repository_prefix).map_err(|error| {
        format!("cannot restrict local community repository permissions: {error}")
    })?;
    stage_community_repository_artifact(
        entry,
        compressed,
        platform,
        &repository_prefix,
        &cache_root,
    )?;

    let public_key = entry
        .signature_public_key_pem
        .as_deref()
        .ok_or_else(|| "community-signed lock entry has no pinned public key".to_owned())?;
    let create_repository = format!(
        "CREATE OR REPLACE EXTENSION REPOSITORY {repository} WITH PREFIX {} USING PUBLIC KEY {};",
        sql_literal(&repository_prefix.to_string_lossy()),
        sql_literal(public_key),
    );
    conn.execute_batch(&create_repository)
        .map_err(|error| format!("cannot register pinned local community repository: {error}"))?;

    let version_dir = ext_dir.join(&entry.duckdb_version);
    fs::create_dir_all(&version_dir)
        .map_err(|error| format!("cannot prepare DuckDB version cache: {error}"))?;
    let version_dir = version_dir
        .canonicalize()
        .map_err(|error| format!("cannot resolve DuckDB version cache: {error}"))?;
    if !version_dir.starts_with(&cache_root) {
        return Err("DuckDB version cache escaped the configured cache".to_owned());
    }
    restrict_directory(&version_dir)
        .map_err(|error| format!("cannot restrict DuckDB version cache permissions: {error}"))?;

    let extension_dir = version_dir.join(platform.as_str());
    fs::create_dir_all(&extension_dir)
        .map_err(|error| format!("cannot prepare repository extension cache: {error}"))?;
    let extension_dir = extension_dir
        .canonicalize()
        .map_err(|error| format!("cannot resolve repository extension cache: {error}"))?;
    if !extension_dir.starts_with(&cache_root) {
        return Err("repository extension cache escaped the configured cache".to_owned());
    }
    restrict_directory(&extension_dir).map_err(|error| {
        format!("cannot restrict repository extension cache permissions: {error}")
    })?;

    let repositories_dir = extension_dir.join("repositories");
    fs::create_dir_all(&repositories_dir)
        .map_err(|error| format!("cannot prepare extension repository cache: {error}"))?;
    let repositories_dir = repositories_dir
        .canonicalize()
        .map_err(|error| format!("cannot resolve extension repository cache: {error}"))?;
    if !repositories_dir.starts_with(&cache_root) {
        return Err("extension repository cache escaped the configured cache".to_owned());
    }
    restrict_directory(&repositories_dir).map_err(|error| {
        format!("cannot restrict extension repository cache permissions: {error}")
    })?;
    let installed_dir = repositories_dir.join(repository);
    fs::create_dir_all(&installed_dir)
        .map_err(|error| format!("cannot prepare repository-scoped extension cache: {error}"))?;
    let installed_dir = installed_dir
        .canonicalize()
        .map_err(|error| format!("cannot resolve repository-scoped extension cache: {error}"))?;
    if !installed_dir.starts_with(&cache_root) {
        return Err("repository-scoped extension cache escaped the configured cache".to_owned());
    }
    restrict_directory(&installed_dir)
        .map_err(|error| format!("cannot restrict repository-scoped cache permissions: {error}"))?;
    let installed = installed_dir.join(format!("{}.duckdb_extension", entry.extension));
    if community_artifact_matches(entry, &installed)? {
        return Ok(());
    }
    remove_unverified_community_artifact(&installed)?;
    let info = PathBuf::from(format!("{}.info", installed.display()));
    remove_unverified_community_artifact(&info)?;
    conn.execute_batch(&format!(
        "FORCE INSTALL {} FROM {repository};",
        entry.extension
    ))
    .map_err(|error| format!("cannot install Duckomo from the pinned local repository: {error}"))?;
    if !community_artifact_matches(entry, &installed)? {
        return Err(format!(
            "local repository installed a community artifact that does not match the locked hash at {}",
            installed.display()
        ));
    }
    restrict_file(&installed).map_err(|error| {
        format!("cannot restrict installed community artifact permissions: {error}")
    })?;
    Ok(())
}

fn stage_community_repository_artifact(
    entry: &ArtifactLockEntry,
    compressed: &[u8],
    platform: crate::config::Platform,
    repository_prefix: &Path,
    cache_root: &Path,
) -> Result<(), String> {
    entry.verify_compressed(compressed).map_err(|error| {
        format!("locked compressed artifact failed verification before staging: {error}")
    })?;
    let version_dir = repository_prefix.join(&entry.duckdb_version);
    fs::create_dir_all(&version_dir)
        .map_err(|error| format!("cannot create local repository version directory: {error}"))?;
    let version_dir = version_dir
        .canonicalize()
        .map_err(|error| format!("cannot resolve local repository version directory: {error}"))?;
    if !version_dir.starts_with(cache_root) {
        return Err("local repository version directory escaped the configured cache".to_owned());
    }
    restrict_directory(&version_dir).map_err(|error| {
        format!("cannot restrict local repository version directory permissions: {error}")
    })?;

    let artifact_dir = version_dir.join(platform.as_str());
    fs::create_dir_all(&artifact_dir)
        .map_err(|error| format!("cannot create local repository artifact directory: {error}"))?;
    let artifact_dir = artifact_dir
        .canonicalize()
        .map_err(|error| format!("cannot resolve local repository artifact directory: {error}"))?;
    if !artifact_dir.starts_with(cache_root) {
        return Err("local repository artifact directory escaped the configured cache".to_owned());
    }
    restrict_directory(&artifact_dir).map_err(|error| {
        format!("cannot restrict local repository artifact permissions: {error}")
    })?;
    let destination = artifact_dir.join(&entry.file);
    match fs::symlink_metadata(&destination) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            fs::remove_file(&destination).map_err(|error| {
                format!("cannot remove symlinked local repository artifact: {error}")
            })?;
        }
        Ok(metadata) if metadata.is_file() => {
            let existing = fs::read(&destination).map_err(|error| {
                format!("cannot read staged local repository artifact: {error}")
            })?;
            if entry.verify_compressed(&existing).is_ok() {
                return Ok(());
            }
            fs::remove_file(&destination).map_err(|error| {
                format!("cannot remove invalid local repository artifact: {error}")
            })?;
        }
        Ok(_) => {
            return Err(format!(
                "local repository artifact path is not a regular file: {}",
                destination.display()
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("cannot inspect local repository artifact: {error}")),
    }
    let mut staged = NamedTempFile::new_in(&artifact_dir)
        .map_err(|error| format!("cannot stage local repository artifact: {error}"))?;
    staged
        .write_all(compressed)
        .and_then(|()| staged.as_file().sync_all())
        .map_err(|error| format!("cannot persist local repository artifact: {error}"))?;
    staged
        .persist(&destination)
        .map_err(|error| format!("cannot publish local repository artifact: {}", error.error))?;
    let published = fs::read(&destination)
        .map_err(|error| format!("cannot verify published local repository artifact: {error}"))?;
    entry.verify_compressed(&published).map_err(|error| {
        format!("published local repository artifact failed verification: {error}")
    })
}

fn restrict_directory(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

fn restrict_file(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

fn community_artifact_matches(entry: &ArtifactLockEntry, path: &Path) -> Result<bool, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(format!("cannot inspect cached community artifact: {error}")),
    };
    if metadata.file_type().is_symlink() {
        return Ok(false);
    }
    if !metadata.is_file() {
        return Err(format!(
            "cached community artifact path is not a regular file: {}",
            path.display()
        ));
    }
    let bytes = fs::read(path)
        .map_err(|error| format!("cannot read cached community artifact: {error}"))?;
    Ok(entry.verify_decompressed(&bytes).is_ok())
}

fn remove_unverified_community_artifact(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() || metadata.file_type().is_symlink() => {
            fs::remove_file(path)
                .map_err(|error| format!("cannot remove unverified community artifact: {error}"))
        }
        Ok(_) => Err(format!(
            "refusing to remove non-file community artifact path: {}",
            path.display()
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "cannot inspect community artifact before install: {error}"
        )),
    }
}

fn provision_httpfs(
    conn: &Connection,
    ext_dir: &Path,
    no_network: bool,
    timeout: Duration,
) -> (HttpfsState, bool) {
    if no_network {
        return (
            HttpfsState::Unavailable("network access disabled by probe mode".to_owned()),
            false,
        );
    }
    if conn.execute_batch("LOAD httpfs;").is_ok() {
        return (HttpfsState::Loaded, true);
    }
    match install_httpfs_with_timeout(ext_dir, timeout) {
        Ok(()) => match conn.execute_batch("LOAD httpfs;") {
            Ok(()) => (HttpfsState::Installed, true),
            Err(error) => (
                HttpfsState::Unavailable(format!("installed HTTPFS could not load: {error}")),
                false,
            ),
        },
        Err(reason) => (HttpfsState::Unavailable(reason), false),
    }
}

fn install_httpfs_with_timeout(ext_dir: &Path, timeout: Duration) -> Result<(), String> {
    let (result_tx, result_rx) = mpsc::channel();
    let (interrupt_tx, interrupt_rx) = mpsc::channel();
    let ext_dir = ext_dir.to_path_buf();
    thread::spawn(move || {
        let result = (|| {
            let config = Config::default()
                .with("extension_directory", ext_dir.to_string_lossy())
                .and_then(|config| config.with("allow_unsigned_extensions", "false"))
                .and_then(|config| config.with("allow_extensions_metadata_mismatch", "false"))
                .and_then(|config| config.with("autoinstall_known_extensions", "false"))
                .and_then(|config| config.with("autoload_known_extensions", "false"))
                .map_err(|error| error.to_string())?;
            let conn =
                Connection::open_in_memory_with_flags(config).map_err(|error| error.to_string())?;
            let _ = interrupt_tx.send(conn.interrupt_handle());
            conn.execute_batch(
                "SET custom_extension_repository = 'http://extensions.duckdb.org'; INSTALL httpfs; LOAD httpfs;",
            )
            .map_err(|error| error.to_string())
        })();
        let _ = result_tx.send(result);
    });
    match result_rx.recv_timeout(timeout) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            if let Ok(interrupt) = interrupt_rx.try_recv() {
                interrupt.interrupt();
            }
            Err(format!(
                "signed HTTPFS installation exceeded {} ms",
                timeout.as_millis()
            ))
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            Err("signed HTTPFS installer exited without a result".to_owned())
        }
    }
}

fn extension_settings_snapshot(conn: &Connection) -> Result<Vec<(String, String)>, BootstrapError> {
    let mut statement = conn
        .prepare(
            "SELECT name, value FROM duckdb_settings() \
             WHERE lower(name) LIKE '%extension%' OR lower(name) LIKE '%signature%' \
             ORDER BY name",
        )
        .map_err(|error| BootstrapError::DuckDb {
            stage: "extension_settings",
            reason: error.to_string(),
        })?;
    statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| BootstrapError::DuckDb {
            stage: "extension_settings",
            reason: error.to_string(),
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| BootstrapError::DuckDb {
            stage: "extension_settings",
            reason: error.to_string(),
        })
}

fn setting(conn: &Connection, name: &str, stage: &'static str) -> Result<String, BootstrapError> {
    conn.query_row(
        "SELECT value FROM duckdb_settings() WHERE name = ?",
        [name],
        |row| row.get::<_, String>(0),
    )
    .map_err(|error| BootstrapError::DuckDb {
        stage,
        reason: error.to_string(),
    })
}

fn load_selected_channel(
    channel: Channel,
    load: impl FnOnce() -> Result<(), String>,
) -> Result<(), BootstrapError> {
    load().map_err(|reason| BootstrapError::Duckomo {
        channel: channel.as_str(),
        reason,
    })
}

fn sql_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn millis(started: Instant) -> u64 {
    started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

#[cfg(feature = "test-support")]
pub(crate) fn run_channel_load_for_test(
    channel: Channel,
    load: impl FnOnce(Channel) -> Result<(), String>,
) -> Result<(), BootstrapError> {
    load_selected_channel(channel, || load(channel))
}

#[cfg(test)]
mod community_artifact_tests {
    use super::*;
    use crate::lock::{ArtifactSource, GoldenLock};
    use sha2::{Digest, Sha256};

    const COMMUNITY_KEY_SHA256: &str =
        "b9aac70a9b1de896079041d76691284aceec44c4eef75a50c90ab2e99f9d54d1";
    const COMMUNITY_KEY_PEM: &str = "-----BEGIN PUBLIC KEY-----\nMIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAtXl28loGwAH3ZGQXXgJQ\n3omhIEiUb3z9Petjl+jmdtEQnMNUFEZiXkfJB02UFWBL1OoKKnjiGhcr5oGiIZKR\nCoaL6SfmWe//7o8STM44stE0exzZcv8W4tWwjrzSWQnwh2JgSnHN64xoDQjdvG3X\n9uQ1xXMXghWOKqEpgArpJQkHoPW3CD5sCS2NLFrBG6KgX0W+GTV5HaKhTMr2754F\nl260drcBJZhLFCeesze2DXtQC+R9D25Zwn2ehHHd2Fd1M10ZL/iKN8NeerB4Jnph\nw6E3orA0DusDLDLtpJUHhmpLoU/1eYQFQOpGw2ce5I88Tkx7SKnCRy1UiE7BA82W\nYQIDAQAB\n-----END PUBLIC KEY-----";

    fn entry_for(compressed: &[u8], decompressed: &[u8]) -> ArtifactLockEntry {
        ArtifactLockEntry {
            channel: Channel::CommunitySigned.as_str().to_owned(),
            extension: "duckomo".to_owned(),
            version: "v0.4.0".to_owned(),
            duckdb_version: "v2.0.0".to_owned(),
            platform: "linux_arm64".to_owned(),
            file: "duckomo.duckdb_extension.gz".to_owned(),
            sha256: hex::encode(Sha256::digest(compressed)),
            size: compressed.len() as u64,
            sha256_decompressed: hex::encode(Sha256::digest(decompressed)),
            size_decompressed: decompressed.len() as u64,
            signature_key_sha256: Some(COMMUNITY_KEY_SHA256.to_owned()),
            signature_public_key_pem: Some(COMMUNITY_KEY_PEM.to_owned()),
            source: ArtifactSource {
                url: "https://community-extensions.duckdb.org".to_owned(),
                pr: None,
                git_ref: None,
                release_tag: None,
                downloaded_at: "2026-11-20T00:00:00Z".to_owned(),
            },
            golden: GoldenLock {
                fixture: "fixtures/om/test.om".to_owned(),
                query_id: "golden-v1".to_owned(),
                result_sha256: "b".repeat(64),
            },
        }
    }

    #[test]
    fn exact_cached_community_binary_matches_offline() {
        let directory = tempfile::tempdir().unwrap();
        let compressed = b"locked compressed community artifact";
        let decompressed = b"locked community binary";
        let entry = entry_for(compressed, decompressed);
        let path = directory
            .path()
            .join("v2.0.0/linux_arm64/repositories/omo_duckomo_test/duckomo.duckdb_extension");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, decompressed).unwrap();

        assert!(community_artifact_matches(&entry, &path).unwrap());
    }

    #[test]
    fn mismatched_cached_community_binary_is_rejected_and_preserved() {
        let directory = tempfile::tempdir().unwrap();
        let entry = entry_for(b"locked compressed artifact", b"locked community binary");
        let path = directory
            .path()
            .join("v2.0.0/linux_arm64/repositories/omo_duckomo_test/duckomo.duckdb_extension");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"different extension binary").unwrap();

        assert!(!community_artifact_matches(&entry, &path).unwrap());
        assert_eq!(fs::read(&path).unwrap(), b"different extension binary");
    }

    #[test]
    fn local_repository_stages_only_the_locked_compressed_bytes() {
        let directory = tempfile::tempdir().unwrap();
        let cache_root = directory.path().canonicalize().unwrap();
        let repository = cache_root.join("omo-community-repositories/omo_duckomo_test");
        fs::create_dir_all(&repository).unwrap();
        let compressed = b"locked compressed community artifact";
        let entry = entry_for(compressed, b"locked community binary");

        stage_community_repository_artifact(
            &entry,
            compressed,
            crate::config::Platform::LinuxArm64,
            &repository,
            &cache_root,
        )
        .unwrap();
        let staged = repository.join("v2.0.0/linux_arm64/duckomo.duckdb_extension.gz");
        assert_eq!(fs::read(staged).unwrap(), compressed);
    }

    #[cfg(unix)]
    #[test]
    fn local_repository_cache_permissions_exclude_other_users() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let cache_root = directory.path().canonicalize().unwrap();
        let repository = cache_root.join("omo-community-repositories/omo_duckomo_test");
        fs::create_dir_all(&repository).unwrap();
        let compressed = b"locked compressed community artifact";
        let entry = entry_for(compressed, b"locked community binary");
        stage_community_repository_artifact(
            &entry,
            compressed,
            crate::config::Platform::LinuxArm64,
            &repository,
            &cache_root,
        )
        .unwrap();

        let version_dir = repository.join("v2.0.0");
        let artifact_dir = version_dir.join("linux_arm64");
        let staged = artifact_dir.join("duckomo.duckdb_extension.gz");
        assert_eq!(
            fs::metadata(version_dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(artifact_dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(staged).unwrap().permissions().mode() & 0o777,
            0o600
        );

        let installed_dir = cache_root.join("v2.0.0/linux_arm64/repositories/omo_duckomo_test");
        fs::create_dir_all(&installed_dir).unwrap();
        restrict_directory(&installed_dir).unwrap();
        let installed = installed_dir.join("duckomo.duckdb_extension");
        fs::write(&installed, b"locked community binary").unwrap();
        restrict_file(&installed).unwrap();
        assert_eq!(
            fs::metadata(&installed_dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(installed).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn local_repository_rejects_wrong_compressed_hash_before_writing() {
        let directory = tempfile::tempdir().unwrap();
        let cache_root = directory.path().canonicalize().unwrap();
        let repository = cache_root.join("omo-community-repositories/omo_duckomo_test");
        fs::create_dir_all(&repository).unwrap();
        let entry = entry_for(b"locked compressed artifact", b"locked community binary");

        let error = stage_community_repository_artifact(
            &entry,
            b"different compressed artifact",
            crate::config::Platform::LinuxArm64,
            &repository,
            &cache_root,
        )
        .unwrap_err();
        assert!(error.contains("failed verification before staging"));
        assert!(
            !repository
                .join("v2.0.0/linux_arm64/duckomo.duckdb_extension.gz")
                .exists()
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_cached_community_binary_is_not_accepted() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let decompressed = b"locked community binary";
        let entry = entry_for(b"locked compressed artifact", decompressed);
        let target = directory.path().join("outside.duckdb_extension");
        fs::write(&target, decompressed).unwrap();
        let path = directory
            .path()
            .join("repositories/omo_duckomo_test/duckomo.duckdb_extension");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        symlink(&target, &path).unwrap();

        assert!(!community_artifact_matches(&entry, &path).unwrap());
        assert_eq!(fs::read(target).unwrap(), decompressed);
    }

    #[cfg(unix)]
    #[test]
    fn local_repository_replaces_symlink_without_touching_target() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let cache_root = directory.path().canonicalize().unwrap();
        let repository = cache_root.join("omo-community-repositories/omo_duckomo_test");
        let artifact_dir = repository.join("v2.0.0/linux_arm64");
        fs::create_dir_all(&artifact_dir).unwrap();
        let destination = artifact_dir.join("duckomo.duckdb_extension.gz");
        let outside = cache_root.join("outside.gz");
        let compressed = b"locked compressed community artifact";
        fs::write(&outside, b"outside bytes").unwrap();
        symlink(&outside, &destination).unwrap();
        let entry = entry_for(compressed, b"locked community binary");

        stage_community_repository_artifact(
            &entry,
            compressed,
            crate::config::Platform::LinuxArm64,
            &repository,
            &cache_root,
        )
        .unwrap();

        assert!(
            !fs::symlink_metadata(&destination)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read(destination).unwrap(), compressed);
        assert_eq!(fs::read(outside).unwrap(), b"outside bytes");
    }
}
