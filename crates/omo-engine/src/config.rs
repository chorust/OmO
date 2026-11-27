use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::lock::ArtifactLockEntry;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Channel {
    CommunitySigned,
    GithubReleaseUnsigned,
}

impl Channel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CommunitySigned => "community-signed",
            Self::GithubReleaseUnsigned => "github-release-unsigned",
        }
    }

    pub const fn allows_unsigned_extensions(self) -> bool {
        matches!(self, Self::GithubReleaseUnsigned)
    }
}

impl TryFrom<&str> for Channel {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "community-signed" => Ok(Self::CommunitySigned),
            "github-release-unsigned" => Ok(Self::GithubReleaseUnsigned),
            _ => Err(format!("unsupported duckomo channel `{value}`")),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Platform {
    LinuxAmd64,
    LinuxArm64,
    OsxArm64,
}

impl Platform {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LinuxAmd64 => "linux_amd64",
            Self::LinuxArm64 => "linux_arm64",
            Self::OsxArm64 => "osx_arm64",
        }
    }

    pub fn current() -> Result<Self, String> {
        match (std::env::consts::OS, std::env::consts::ARCH) {
            ("linux", "x86_64") => Ok(Self::LinuxAmd64),
            ("linux", "aarch64") => Ok(Self::LinuxArm64),
            ("macos", "aarch64") => Ok(Self::OsxArm64),
            (os, arch) => Err(format!("unsupported target platform {os}/{arch}")),
        }
    }
}

impl TryFrom<&str> for Platform {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "linux_amd64" => Ok(Self::LinuxAmd64),
            "linux_arm64" => Ok(Self::LinuxArm64),
            "osx_arm64" => Ok(Self::OsxArm64),
            _ => Err(format!("unsupported target platform `{value}`")),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BootstrapConfig {
    pub data_dir: PathBuf,
    pub ext_cache_dir: PathBuf,
    pub artifact_path: PathBuf,
    pub fixture_path: PathBuf,
    pub artifact: ArtifactLockEntry,
    pub platform: Platform,
    #[serde(default)]
    pub threads: Option<usize>,
    #[serde(default)]
    pub memory_limit: Option<String>,
    #[serde(default = "default_httpfs_timeout_ms")]
    pub httpfs_timeout_ms: u64,
    #[serde(default)]
    pub no_network: bool,
    #[serde(default)]
    pub install_spatial_before_lock: bool,
    #[serde(default)]
    pub database_path: Option<PathBuf>,
    /// Largest merged remote read in bytes (`duckomo_remote_max_request`). `None` selects
    /// [`DEFAULT_REMOTE_MAX_REQUEST`] for remote sessions. Must be set before the lock.
    #[serde(default)]
    pub duckomo_remote_max_request: Option<u64>,
    /// Largest gap in bytes bridged when merging reads (`duckomo_remote_merge_gap`). `None`
    /// selects [`DEFAULT_REMOTE_MERGE_GAP`] for remote sessions. Must be set before the lock.
    #[serde(default)]
    pub duckomo_remote_merge_gap: Option<u64>,
}

/// `4 MiB`: lets a regional `data_spatial` band move in a few large range requests.
pub const DEFAULT_REMOTE_MAX_REQUEST: u64 = 4 * 1024 * 1024;
/// `64 KiB`: bridge small gaps instead of issuing another request.
pub const DEFAULT_REMOTE_MERGE_GAP: u64 = 64 * 1024;

const fn default_httpfs_timeout_ms() -> u64 {
    15_000
}

impl BootstrapConfig {
    pub fn channel(&self) -> Result<Channel, String> {
        Channel::try_from(self.artifact.channel.as_str())
    }

    pub fn database_path(&self) -> PathBuf {
        self.database_path
            .clone()
            .unwrap_or_else(|| self.data_dir.join("omobench.duckdb"))
    }

    pub fn remote_max_request(&self) -> u64 {
        self.duckomo_remote_max_request
            .unwrap_or(DEFAULT_REMOTE_MAX_REQUEST)
    }

    pub fn remote_merge_gap(&self) -> u64 {
        self.duckomo_remote_merge_gap
            .unwrap_or(DEFAULT_REMOTE_MERGE_GAP)
    }

    pub fn httpfs_timeout(&self) -> std::time::Duration {
        std::time::Duration::from_millis(self.httpfs_timeout_ms)
    }

    pub fn requires_absolute_paths(&self) -> bool {
        [
            self.data_dir.as_path(),
            self.ext_cache_dir.as_path(),
            self.artifact_path.as_path(),
            self.fixture_path.as_path(),
            self.database_path
                .as_deref()
                .unwrap_or_else(|| self.data_dir.as_path()),
        ]
        .iter()
        .all(|path: &&Path| path.is_absolute())
    }
}
