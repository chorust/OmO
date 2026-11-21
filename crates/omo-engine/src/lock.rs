use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ArtifactLock {
    pub schema: String,
    pub entries: Vec<ArtifactLockEntry>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ArtifactLockEntry {
    pub channel: String,
    pub extension: String,
    pub version: String,
    pub duckdb_version: String,
    pub platform: String,
    pub file: String,
    pub sha256: String,
    pub size: u64,
    pub sha256_decompressed: String,
    pub size_decompressed: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature_key_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature_public_key_pem: Option<String>,
    pub source: ArtifactSource,
    pub golden: GoldenLock,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ArtifactSource {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr: Option<String>,
    #[serde(rename = "ref", default, skip_serializing_if = "Option::is_none")]
    pub git_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_tag: Option<String>,
    pub downloaded_at: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GoldenLock {
    pub fixture: String,
    pub query_id: String,
    pub result_sha256: String,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ArtifactMismatch {
    #[error("artifact lock has unsupported schema `{actual}`")]
    UnsupportedSchema { actual: String },
    #[error(
        "no artifact entry for channel={channel}, version={version}, DuckDB={duckdb_version}, platform={platform}"
    )]
    MissingEntry {
        channel: String,
        version: String,
        duckdb_version: String,
        platform: String,
    },
    #[error("artifact metadata field `{field}` is invalid")]
    InvalidMetadata { field: &'static str },
    #[error("artifact {field} mismatch: expected {expected}, got {actual}")]
    Mismatch {
        field: &'static str,
        expected: String,
        actual: String,
    },
    #[error("golden query hash is a placeholder or invalid")]
    PlaceholderGoldenHash,
}

impl ArtifactLock {
    pub const SCHEMA: &'static str = "omo-artifact-lock/1";

    pub fn from_json(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }

    pub fn lookup(
        &self,
        channel: &str,
        version: &str,
        duckdb_version: &str,
        platform: &str,
    ) -> Result<&ArtifactLockEntry, ArtifactMismatch> {
        self.lookup_with_golden_policy(channel, version, duckdb_version, platform, true)
    }

    /// Select a fully pinned artifact while allowing only the golden hash to remain a
    /// placeholder. This is for the explicit first-run measurement command, which reports
    /// the computed hash but never returns a task-capable connection.
    pub fn lookup_for_golden_measurement(
        &self,
        channel: &str,
        version: &str,
        duckdb_version: &str,
        platform: &str,
    ) -> Result<&ArtifactLockEntry, ArtifactMismatch> {
        self.lookup_with_golden_policy(channel, version, duckdb_version, platform, false)
    }

    fn lookup_with_golden_policy(
        &self,
        channel: &str,
        version: &str,
        duckdb_version: &str,
        platform: &str,
        require_golden: bool,
    ) -> Result<&ArtifactLockEntry, ArtifactMismatch> {
        if self.schema != Self::SCHEMA {
            return Err(ArtifactMismatch::UnsupportedSchema {
                actual: self.schema.clone(),
            });
        }
        let entry = self
            .entries
            .iter()
            .find(|entry| {
                entry.channel == channel
                    && entry.version == version
                    && entry.duckdb_version == duckdb_version
                    && entry.platform == platform
            })
            .ok_or_else(|| ArtifactMismatch::MissingEntry {
                channel: channel.to_owned(),
                version: version.to_owned(),
                duckdb_version: duckdb_version.to_owned(),
                platform: platform.to_owned(),
            })?;
        if require_golden {
            entry.validate()?;
        } else {
            entry.validate_for_golden_measurement()?;
        }
        Ok(entry)
    }
}

impl ArtifactLockEntry {
    pub fn validate_for_golden_measurement(&self) -> Result<(), ArtifactMismatch> {
        if self.golden.result_sha256 != "<filled only after a verified bootstrap run>" {
            return self.validate();
        }
        let mut measurement_entry = self.clone();
        measurement_entry.golden.result_sha256 = "b".repeat(64);
        measurement_entry.validate()
    }

    pub fn validate(&self) -> Result<(), ArtifactMismatch> {
        if self.extension != "duckomo" {
            return Err(ArtifactMismatch::InvalidMetadata { field: "extension" });
        }
        if self.size == 0 || self.size_decompressed == 0 {
            return Err(ArtifactMismatch::InvalidMetadata { field: "size" });
        }
        validate_sha256("sha256", &self.sha256)?;
        validate_sha256("sha256_decompressed", &self.sha256_decompressed)?;
        match self.channel.as_str() {
            "community-signed" => {
                let expected = self.signature_key_sha256.as_deref().ok_or(
                    ArtifactMismatch::InvalidMetadata {
                        field: "signature_key_sha256",
                    },
                )?;
                validate_sha256("signature_key_sha256", expected)?;
                let pem = self.signature_public_key_pem.as_deref().ok_or(
                    ArtifactMismatch::InvalidMetadata {
                        field: "signature_public_key_pem",
                    },
                )?;
                let actual = public_key_sha256(pem).ok_or(ArtifactMismatch::InvalidMetadata {
                    field: "signature_public_key_pem",
                })?;
                if !actual.eq_ignore_ascii_case(expected) {
                    return Err(ArtifactMismatch::Mismatch {
                        field: "signature_key_sha256",
                        expected: expected.to_owned(),
                        actual,
                    });
                }
            }
            "github-release-unsigned" => {
                if self.signature_key_sha256.is_some() || self.signature_public_key_pem.is_some() {
                    return Err(ArtifactMismatch::InvalidMetadata {
                        field: "signature_key_sha256",
                    });
                }
            }
            _ => return Err(ArtifactMismatch::InvalidMetadata { field: "channel" }),
        }
        validate_sha256("golden.result_sha256", &self.golden.result_sha256)
            .map_err(|_| ArtifactMismatch::PlaceholderGoldenHash)?;
        if self.golden.result_sha256.bytes().all(|byte| byte == b'0') {
            return Err(ArtifactMismatch::PlaceholderGoldenHash);
        }
        if self.golden.fixture.is_empty() || self.golden.query_id.is_empty() {
            return Err(ArtifactMismatch::InvalidMetadata { field: "golden" });
        }
        if self.source.url.is_empty() || self.source.downloaded_at.is_empty() {
            return Err(ArtifactMismatch::InvalidMetadata { field: "source" });
        }
        Ok(())
    }

    pub fn verify_compressed(&self, bytes: &[u8]) -> Result<(), ArtifactMismatch> {
        if bytes.len() as u64 != self.size {
            return Err(ArtifactMismatch::Mismatch {
                field: "size",
                expected: self.size.to_string(),
                actual: bytes.len().to_string(),
            });
        }
        let actual = hex::encode(Sha256::digest(bytes));
        if !actual.eq_ignore_ascii_case(&self.sha256) {
            return Err(ArtifactMismatch::Mismatch {
                field: "sha256",
                expected: self.sha256.clone(),
                actual,
            });
        }
        Ok(())
    }

    /// Run the supplied operation only after the downloaded artifact passes both lock checks.
    pub fn verify_compressed_and_then<T>(
        &self,
        bytes: &[u8],
        operation: impl FnOnce(&[u8]) -> T,
    ) -> Result<T, ArtifactMismatch> {
        self.verify_compressed(bytes)?;
        Ok(operation(bytes))
    }

    pub fn verify_decompressed(&self, bytes: &[u8]) -> Result<(), ArtifactMismatch> {
        if bytes.len() as u64 != self.size_decompressed {
            return Err(ArtifactMismatch::Mismatch {
                field: "size_decompressed",
                expected: self.size_decompressed.to_string(),
                actual: bytes.len().to_string(),
            });
        }
        let actual = hex::encode(Sha256::digest(bytes));
        if !actual.eq_ignore_ascii_case(&self.sha256_decompressed) {
            return Err(ArtifactMismatch::Mismatch {
                field: "sha256_decompressed",
                expected: self.sha256_decompressed.clone(),
                actual,
            });
        }
        Ok(())
    }

    /// Run the supplied operation only after the decompressed artifact passes both checks.
    pub fn verify_and_then<T>(
        &self,
        bytes: &[u8],
        operation: impl FnOnce(&[u8]) -> T,
    ) -> Result<T, ArtifactMismatch> {
        self.verify_decompressed(bytes)?;
        Ok(operation(bytes))
    }
}

fn public_key_sha256(pem: &str) -> Option<String> {
    let lines = pem.lines().collect::<Vec<_>>();
    if lines.first()? != &"-----BEGIN PUBLIC KEY-----" {
        return None;
    }
    let end = lines
        .iter()
        .position(|line| *line == "-----END PUBLIC KEY-----")?;
    if lines[end + 1..].iter().any(|line| !line.trim().is_empty())
        || lines[1..end].iter().any(|line| line.starts_with("-----"))
    {
        return None;
    }
    let encoded = lines[1..end]
        .iter()
        .map(|line| line.trim())
        .collect::<String>();
    let der = STANDARD.decode(encoded).ok()?;
    Some(hex::encode(Sha256::digest(der)))
}

fn validate_sha256(field: &'static str, value: &str) -> Result<(), ArtifactMismatch> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ArtifactMismatch::InvalidMetadata { field });
    }
    Ok(())
}
