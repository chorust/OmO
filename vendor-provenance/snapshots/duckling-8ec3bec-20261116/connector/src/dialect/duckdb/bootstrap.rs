use anyhow::{Context, Result};
use duckdb::{Config, Connection};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

pub const COMPILED_MANIFEST: &str =
  include_str!("../../../../scripts/studio/duckomo-artifacts.json");

const MAX_ARCHIVE_SIZE: u64 = 64 * 1024 * 1024;
const MAX_BINARY_SIZE: u64 = 128 * 1024 * 1024;
const RELEASE_TAG: &str = "v0.4.0";
const RELEASE_SOURCE: &str = "1bc76c3a36d26607b463909153cff9c255e19919";
const DUCKDB_VERSION: &str = "v2.0.0";
const EXTENSION_NAME: &str = "duckomo";
pub const OFFICIAL_EXTENSIONS_REPOSITORY: &str = "http://extensions.duckdb.org";
const HTTPFS_INSTALL_TIMEOUT: Duration = Duration::from_secs(15);

static APPLICATION_DESCRIPTOR: OnceLock<BootstrapDescriptor> = OnceLock::new();

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DuckomoArtifactManifest {
  schema_version: u32,
  extension_name: String,
  repository: String,
  release: DuckomoReleasePin,
  platforms: Vec<DuckomoArtifactPin>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DuckomoReleasePin {
  tag: String,
  source_commit: String,
  duckdb_version: String,
  prerelease: bool,
  formal_t1: String,
  signed: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DuckomoArtifactPin {
  rust_target: String,
  platform: String,
  asset_id: u64,
  asset_name: String,
  asset_url: String,
  archive_size: u64,
  archive_sha256: String,
  binary_size: u64,
  binary_sha256: String,
  resource_relative_path: String,
  verification: String,
}

#[derive(Clone, Debug)]
pub struct BootstrapDescriptor {
  resource_root: PathBuf,
  cache_root: PathBuf,
  extension_repository: String,
  install_timeout: Duration,
  pin: DuckomoArtifactPin,
}

#[derive(Debug)]
pub struct BootstrapError {
  pub stage: &'static str,
  pub code: &'static str,
  detail: String,
}

impl Display for BootstrapError {
  fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
    write!(formatter, "{} [{}]: {}", self.stage, self.code, self.detail)
  }
}

impl Error for BootstrapError {}

fn failure(stage: &'static str, code: &'static str, detail: impl Into<String>) -> anyhow::Error {
  anyhow::Error::new(BootstrapError {
    stage,
    code,
    detail: detail.into(),
  })
}

fn valid_sha256(value: &str) -> bool {
  value.len() == 64
    && value
      .bytes()
      .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn target_platform(target: &str) -> Option<&'static str> {
  match target {
    "x86_64-unknown-linux-gnu" => Some("linux_amd64"),
    "aarch64-unknown-linux-gnu" => Some("linux_arm64"),
    "x86_64-apple-darwin" => Some("osx_amd64"),
    "aarch64-apple-darwin" => Some("osx_arm64"),
    _ => None,
  }
}

fn safe_relative_resource(path: &str) -> bool {
  let path = Path::new(path);
  !path.is_absolute()
    && path
      .components()
      .all(|component| matches!(component, Component::Normal(_)))
    && path
      .file_name()
      .is_some_and(|name| name == "duckomo.duckdb_extension")
}

pub fn parse_manifest(contents: &str) -> Result<DuckomoArtifactManifest> {
  let manifest: DuckomoArtifactManifest = serde_json::from_str(contents).map_err(|error| {
    failure(
      "artifact_target",
      "artifact_target",
      format!("invalid manifest: {error}"),
    )
  })?;
  if manifest.schema_version != 1
    || manifest.extension_name != EXTENSION_NAME
    || manifest.repository != "chorust/duckomo"
    || manifest.release.tag != RELEASE_TAG
    || manifest.release.source_commit != RELEASE_SOURCE
    || manifest.release.duckdb_version != DUCKDB_VERSION
    || manifest.release.prerelease
    || manifest.release.formal_t1 != "OPEN"
    || manifest.release.signed
  {
    return Err(failure(
      "artifact_target",
      "artifact_target",
      "manifest does not match the reviewed DuckOMO v0.4.0 baseline pin",
    ));
  }
  if manifest.platforms.len() != 4 {
    return Err(failure(
      "artifact_target",
      "artifact_target",
      "manifest must contain exactly four reviewed targets",
    ));
  }

  let mut platforms = HashSet::new();
  let mut targets = HashSet::new();
  let mut resources = HashSet::new();
  for pin in &manifest.platforms {
    if !platforms.insert(pin.platform.as_str())
      || !targets.insert(pin.rust_target.as_str())
      || !resources.insert(pin.resource_relative_path.as_str())
    {
      return Err(failure(
        "artifact_target",
        "artifact_target",
        "manifest contains duplicate target, platform, or resource entries",
      ));
    }
    if target_platform(&pin.rust_target) != Some(pin.platform.as_str())
      || pin.asset_id == 0
      || pin.archive_size == 0
      || pin.archive_size > MAX_ARCHIVE_SIZE
      || pin.binary_size == 0
      || pin.binary_size > MAX_BINARY_SIZE
      || !valid_sha256(&pin.archive_sha256)
      || !valid_sha256(&pin.binary_sha256)
      || !safe_relative_resource(&pin.resource_relative_path)
      || pin.verification.trim().is_empty()
    {
      return Err(failure(
        "artifact_target",
        "artifact_target",
        format!("malformed pin for platform {}", pin.platform),
      ));
    }
    let expected_asset = format!(
      "duckomo-{RELEASE_TAG}-duckdb-{DUCKDB_VERSION}-{}.duckdb_extension.gz",
      pin.platform
    );
    let expected_url = format!(
      "https://github.com/chorust/duckomo/releases/download/{RELEASE_TAG}/{expected_asset}"
    );
    let expected_resource = format!(
      "extensions/{DUCKDB_VERSION}/{}/duckomo.duckdb_extension",
      pin.platform
    );
    if pin.asset_name != expected_asset
      || pin.asset_url != expected_url
      || pin.resource_relative_path != expected_resource
    {
      return Err(failure(
        "artifact_target",
        "artifact_target",
        format!(
          "asset or resource identity mismatch for platform {}",
          pin.platform
        ),
      ));
    }
  }
  Ok(manifest)
}

pub fn select_pin(
  manifest: &DuckomoArtifactManifest,
  engine: &str,
  platform: &str,
) -> Result<DuckomoArtifactPin> {
  if engine != manifest.release.duckdb_version {
    return Err(failure(
      "engine_compatibility",
      "engine_compatibility",
      format!(
        "expected engine {}, got {engine}",
        manifest.release.duckdb_version
      ),
    ));
  }
  manifest
    .platforms
    .iter()
    .find(|pin| pin.platform == platform)
    .cloned()
    .ok_or_else(|| {
      failure(
        "artifact_target",
        "artifact_target",
        format!("no reviewed DuckOMO pin for platform {platform}"),
      )
    })
}

pub fn resolve_resource_path(root: &Path, relative: &str) -> Result<PathBuf> {
  if !safe_relative_resource(relative) {
    return Err(failure(
      "artifact_path",
      "artifact_path",
      "resource path must be a safe relative DuckOMO resource path",
    ));
  }
  let canonical_root = root.canonicalize().with_context(|| {
    failure(
      "artifact_path",
      "artifact_path",
      "resource root is unavailable",
    )
  })?;
  let candidate = canonical_root.join(relative);
  let canonical_file = candidate.canonicalize().with_context(|| {
    failure(
      "artifact_path",
      "artifact_path",
      "DuckOMO resource is missing",
    )
  })?;
  if !canonical_file.starts_with(&canonical_root) || !canonical_file.is_file() {
    return Err(failure(
      "artifact_path",
      "artifact_path",
      "DuckOMO resource escapes its root or is not a regular file",
    ));
  }
  Ok(canonical_file)
}

pub fn verify_artifact(path: &Path, pin: &DuckomoArtifactPin) -> Result<String> {
  let metadata = fs::metadata(path).with_context(|| {
    failure(
      "artifact_integrity",
      "artifact_integrity",
      "DuckOMO resource is unavailable",
    )
  })?;
  if !metadata.is_file() || metadata.len() != pin.binary_size {
    return Err(failure(
      "artifact_integrity",
      "artifact_integrity",
      format!(
        "DuckOMO resource size mismatch: expected {}, got {}",
        pin.binary_size,
        metadata.len()
      ),
    ));
  }
  let mut reader = BufReader::new(File::open(path).with_context(|| {
    failure(
      "artifact_integrity",
      "artifact_integrity",
      "DuckOMO resource cannot be opened",
    )
  })?);
  let mut digest = Sha256::new();
  let mut buffer = [0_u8; 64 * 1024];
  loop {
    let count = reader.read(&mut buffer).with_context(|| {
      failure(
        "artifact_integrity",
        "artifact_integrity",
        "DuckOMO resource could not be hashed",
      )
    })?;
    if count == 0 {
      break;
    }
    digest.update(&buffer[..count]);
  }
  let actual = format!("{:x}", digest.finalize());
  if actual != pin.binary_sha256 {
    return Err(failure(
      "artifact_integrity",
      "artifact_integrity",
      format!(
        "DuckOMO resource SHA256 mismatch: expected {}, got {actual}",
        pin.binary_sha256
      ),
    ));
  }
  Ok(actual)
}

pub fn sql_literal(value: &str) -> String {
  format!("'{}'", value.replace('\'', "''"))
}

fn setting(conn: &Connection, name: &str, stage: &'static str) -> Result<String> {
  conn
    .query_row(
      "SELECT value::VARCHAR FROM duckdb_settings() WHERE name = ?",
      [name],
      |row| row.get(0),
    )
    .map_err(|error| failure(stage, stage, error.to_string()))
}

fn extension_path(conn: &Connection, name: &str, stage: &'static str) -> Result<PathBuf> {
  let path: String = conn
    .query_row(
      "SELECT install_path FROM duckdb_extensions() WHERE extension_name = ?",
      [name],
      |row| row.get(0),
    )
    .map_err(|error| failure(stage, stage, error.to_string()))?;
  Ok(PathBuf::from(path))
}

fn loaded_extensions(conn: &Connection, stage: &'static str) -> Result<Vec<String>> {
  let mut statement = conn
    .prepare("SELECT extension_name FROM duckdb_extensions() WHERE loaded ORDER BY extension_name")
    .map_err(|error| failure(stage, stage, error.to_string()))?;
  let rows = statement
    .query_map([], |row| row.get::<_, String>(0))
    .map_err(|error| failure(stage, stage, error.to_string()))?;
  rows
    .collect::<duckdb::Result<Vec<_>>>()
    .map_err(|error| failure(stage, stage, error.to_string()))
}

fn execute_bootstrap_sql(conn: &Connection, sql: &str, stage: &'static str) -> Result<()> {
  conn
    .execute_batch(sql)
    .map_err(|error| failure(stage, stage, error.to_string()))
}

enum HttpfsProvision {
  Loaded(PathBuf),
  Unavailable(String),
}

fn cached_httpfs_path(cache_root: &Path, engine: &str, platform: &str) -> PathBuf {
  cache_root
    .join(engine)
    .join(platform)
    .join("httpfs.duckdb_extension")
}

fn open_signed_dependency_probe(cache_root: &Path, repository: &str) -> Result<Connection> {
  let config = Config::default()
    .with("autoload_known_extensions", "false")
    .and_then(|config| config.with("autoinstall_known_extensions", "false"))
    .map_err(|error| failure("signed_dependency", "signed_dependency", error.to_string()))?;
  let conn = Connection::open_in_memory_with_flags(config)
    .map_err(|error| failure("signed_dependency", "signed_dependency", error.to_string()))?;
  if setting(&conn, "allow_unsigned_extensions", "signed_dependency")? != "false" {
    return Err(failure(
      "signed_dependency",
      "signed_dependency",
      "signed HTTPFS verification did not start with unsigned loading disabled",
    ));
  }
  let cache_sql = sql_literal(&cache_root.to_string_lossy());
  let repository_sql = sql_literal(repository);
  execute_bootstrap_sql(
    &conn,
    &format!(
      "SET extension_directories = [{cache_sql}]; SET custom_extension_repository = {repository_sql};",
    ),
    "signed_dependency",
  )?;
  Ok(conn)
}

fn load_signed_httpfs(
  cache_root: &Path,
  repository: &str,
  module_path: Option<&Path>,
) -> Result<PathBuf> {
  let conn = open_signed_dependency_probe(cache_root, repository)?;
  let sql = match module_path {
    Some(path) => format!("LOAD {};", sql_literal(&path.to_string_lossy())),
    None => format!(
      "FORCE INSTALL httpfs FROM {}; LOAD httpfs;",
      sql_literal(repository)
    ),
  };
  execute_bootstrap_sql(&conn, &sql, "signed_dependency")?;
  if !loaded_extensions(&conn, "signed_dependency")?
    .iter()
    .any(|name| name == "httpfs")
  {
    return Err(failure(
      "signed_dependency",
      "signed_dependency",
      "official HTTPFS did not load under default signature verification",
    ));
  }
  let path = match module_path {
    Some(module) => module.canonicalize().map_err(|error| {
      failure(
        "signed_dependency",
        "signed_dependency",
        format!("cached HTTPFS path is unavailable: {error}"),
      )
    })?,
    None => extension_path(&conn, "httpfs", "signed_dependency")?
      .canonicalize()
      .map_err(|error| failure("signed_dependency", "signed_dependency", error.to_string()))?,
  };
  if !path.starts_with(cache_root) || !path.is_file() {
    return Err(failure(
      "signed_dependency",
      "signed_dependency",
      "verified HTTPFS resolved outside the application extension cache",
    ));
  }
  Ok(path)
}

/// Provisions signed HTTPFS without making local SQL depend on repository reachability.
///
/// A cached artifact is loaded offline first. Only a missing or damaged cache triggers a
/// signed `INSTALL`, and an unreachable repository is reported as `Unavailable` instead of
/// failing the whole bootstrap, so local queries keep the M0.2 offline contract while
/// signature verification stays enabled in every attempt.
fn provision_signed_httpfs(
  cache_root: &Path,
  repository: &str,
  engine: &str,
  platform: &str,
  install_timeout: Duration,
) -> Result<HttpfsProvision> {
  let canonical_cache = match fs::create_dir_all(cache_root)
    .and_then(|_| cache_root.canonicalize())
    .map_err(|error| failure("signed_dependency", "signed_dependency", error.to_string()))
  {
    Ok(path) => path,
    Err(error) => {
      return Ok(HttpfsProvision::Unavailable(format!(
        "the application extension cache is unavailable: {error}"
      )))
    }
  };
  let mut attempts: Vec<String> = Vec::new();

  let cached = cached_httpfs_path(&canonical_cache, engine, platform);
  if cached.is_file() {
    match load_signed_httpfs(&canonical_cache, repository, Some(&cached)) {
      Ok(path) => return Ok(HttpfsProvision::Loaded(path)),
      Err(error) => attempts.push(format!("cached HTTPFS could not be loaded: {error}")),
    }
  }

  let (sender, receiver) = std::sync::mpsc::channel();
  let worker_cache = canonical_cache.clone();
  let worker_repository = repository.to_string();
  std::thread::spawn(move || {
    let _ = sender.send(load_signed_httpfs(&worker_cache, &worker_repository, None));
  });
  match receiver.recv_timeout(install_timeout) {
    Ok(Ok(path)) => Ok(HttpfsProvision::Loaded(path)),
    Ok(Err(error)) => {
      attempts.push(format!("signed HTTPFS installation failed: {error}"));
      Ok(HttpfsProvision::Unavailable(attempts.join("; ")))
    }
    Err(_) => {
      attempts.push(format!(
        "signed HTTPFS installation did not finish within {}s",
        install_timeout.as_secs_f32()
      ));
      Ok(HttpfsProvision::Unavailable(attempts.join("; ")))
    }
  }
}

pub fn open_local_connection(
  database_path: Option<&Path>,
  cwd: Option<&Path>,
) -> Result<Connection> {
  open_local_connection_with_note(database_path, cwd).map(|(conn, _)| conn)
}

/// Like [`open_local_connection`], also returning why remote reads are unavailable, if they are.
pub fn open_local_connection_with_note(
  database_path: Option<&Path>,
  cwd: Option<&Path>,
) -> Result<(Connection, Option<String>)> {
  if let Some(descriptor) = registered_application_descriptor() {
    return open_connection_with_note(database_path, descriptor, cwd);
  }

  let conn = match database_path {
    Some(path) => Connection::open_with_flags(path, Config::default()),
    None => Connection::open_in_memory_with_flags(Config::default()),
  }
  .map_err(|error| {
    failure(
      "engine_compatibility",
      "engine_compatibility",
      error.to_string(),
    )
  })?;
  if let Some(cwd) = cwd {
    conn
      .execute_batch(&format!(
        "SET file_search_path = {};",
        sql_literal(&cwd.to_string_lossy())
      ))
      .map_err(|error| failure("configuration", "configuration", error.to_string()))?;
  }
  Ok((conn, None))
}

pub fn open_connection(
  database_path: Option<&Path>,
  descriptor: &BootstrapDescriptor,
  cwd: Option<&Path>,
) -> Result<Connection> {
  open_connection_with_note(database_path, descriptor, cwd).map(|(conn, _)| conn)
}

/// Like [`open_connection`], also returning why remote reads are unavailable, if they are.
pub fn open_connection_with_note(
  database_path: Option<&Path>,
  descriptor: &BootstrapDescriptor,
  cwd: Option<&Path>,
) -> Result<(Connection, Option<String>)> {
  let config = Config::default()
    .with("allow_unsigned_extensions", "true")
    .and_then(|config| config.with("allow_extensions_metadata_mismatch", "false"))
    .and_then(|config| config.with("autoinstall_known_extensions", "false"))
    .and_then(|config| config.with("autoload_known_extensions", "false"))
    .map_err(|error| {
      failure(
        "configuration_seal",
        "configuration_seal",
        error.to_string(),
      )
    })?;
  let conn = match database_path {
    Some(path) => Connection::open_with_flags(path, config),
    None => Connection::open_in_memory_with_flags(config),
  }
  .map_err(|error| {
    failure(
      "engine_compatibility",
      "engine_compatibility",
      error.to_string(),
    )
  })?;
  bootstrap_connection_with_note(conn, descriptor, cwd)
}

pub fn bootstrap_connection(
  conn: Connection,
  descriptor: &BootstrapDescriptor,
  cwd: Option<&Path>,
) -> Result<Connection> {
  bootstrap_connection_with_note(conn, descriptor, cwd).map(|(conn, _)| conn)
}

fn bootstrap_connection_with_note(
  conn: Connection,
  descriptor: &BootstrapDescriptor,
  cwd: Option<&Path>,
) -> Result<(Connection, Option<String>)> {
  let engine = conn
    .query_row("SELECT version()", [], |row| row.get::<_, String>(0))
    .map_err(|error| {
      failure(
        "engine_compatibility",
        "engine_compatibility",
        error.to_string(),
      )
    })?;
  let platform = conn
    .query_row("PRAGMA platform", [], |row| row.get::<_, String>(0))
    .map_err(|error| {
      failure(
        "engine_compatibility",
        "engine_compatibility",
        error.to_string(),
      )
    })?;
  let unsigned = setting(&conn, "allow_unsigned_extensions", "extension_load")?;
  if unsigned != "true" {
    return Err(failure(
      "extension_load",
      "extension_load",
      "DuckOMO bootstrap requires its private startup unsigned window",
    ));
  }
  let manifest = parse_manifest(COMPILED_MANIFEST)?;
  let selected = select_pin(&manifest, &engine, &platform)?;
  if selected.platform != descriptor.pin.platform
    || selected.binary_sha256 != descriptor.pin.binary_sha256
    || selected.resource_relative_path != descriptor.pin.resource_relative_path
  {
    return Err(failure(
      "artifact_target",
      "artifact_target",
      "bootstrap descriptor does not match this handle's engine and platform",
    ));
  }
  let resource = descriptor.resource_path()?;
  verify_artifact(&resource, &selected)?;
  let mut httpfs_note: Option<String> = None;
  let httpfs = match provision_signed_httpfs(
    &descriptor.cache_root,
    &descriptor.extension_repository,
    &engine,
    &platform,
    descriptor.install_timeout,
  )? {
    HttpfsProvision::Loaded(path) => Some(path),
    HttpfsProvision::Unavailable(reason) => {
      log::warn!(
        "signed HTTPFS is unavailable for this connection; local queries still run: {reason}"
      );
      httpfs_note = Some(reason);
      None
    }
  };

  let cache_root = descriptor.cache_root.canonicalize().map_err(|error| {
    failure(
      "configuration_seal",
      "configuration_seal",
      error.to_string(),
    )
  })?;
  let cache_sql = sql_literal(&cache_root.to_string_lossy());
  let repository_sql = sql_literal(&descriptor.extension_repository);
  let mut bootstrap_sql = format!(
    "SET extension_directories = [{cache_sql}]; SET custom_extension_repository = {repository_sql};"
  );
  if let Some(cwd) = cwd {
    bootstrap_sql.push_str(&format!(
      " SET file_search_path = {};",
      sql_literal(&cwd.to_string_lossy())
    ));
  }
  execute_bootstrap_sql(&conn, &bootstrap_sql, "configuration_seal")?;
  execute_bootstrap_sql(
    &conn,
    "SET allowed_configs = []; SET autoinstall_known_extensions = false; SET autoload_known_extensions = false; SET allow_extensions_metadata_mismatch = false;",
    "configuration_seal",
  )?;
  let resource = descriptor.resource_path()?;
  verify_artifact(&resource, &selected)?;
  execute_bootstrap_sql(
    &conn,
    &format!("LOAD {};", sql_literal(&resource.to_string_lossy())),
    "extension_load",
  )?;
  execute_bootstrap_sql(
    &conn,
    "SET allow_unsigned_extensions = false;",
    "configuration_seal",
  )?;
  if let Some(httpfs) = &httpfs {
    execute_bootstrap_sql(
      &conn,
      &format!("LOAD {};", sql_literal(&httpfs.to_string_lossy())),
      "signed_dependency",
    )?;
  }
  let allowed_directories = if httpfs.is_some() {
    "['/','http://','https://','s3://']"
  } else {
    "['/']"
  };
  execute_bootstrap_sql(
    &conn,
    &format!(
      "SET allowed_directories = {allowed_directories}; SET enable_external_access = false; SET lock_configuration = true;"
    ),
    "configuration_seal",
  )?;

  for (name, expected) in [
    ("allow_unsigned_extensions", "false"),
    ("allow_extensions_metadata_mismatch", "false"),
    ("autoinstall_known_extensions", "false"),
    ("autoload_known_extensions", "false"),
    ("enable_external_access", "false"),
    ("lock_configuration", "true"),
  ] {
    let actual = setting(&conn, name, "configuration_seal")?;
    if actual != expected {
      return Err(failure(
        "configuration_seal",
        "configuration_seal",
        format!("final setting {name} is {actual}, expected {expected}"),
      ));
    }
  }
  if setting(&conn, "allowed_configs", "configuration_seal")? != "[]" {
    return Err(failure(
      "configuration_seal",
      "configuration_seal",
      "allowed_configs must remain empty",
    ));
  }
  let loaded = loaded_extensions(&conn, "extension_load")?;
  if !loaded.iter().any(|name| name == "duckomo") {
    return Err(failure(
      "extension_load",
      "extension_load",
      format!("required extensions are not loaded: {loaded:?}"),
    ));
  }
  if httpfs.is_some() && !loaded.iter().any(|name| name == "httpfs") {
    return Err(failure(
      "extension_load",
      "extension_load",
      format!("verified HTTPFS did not stay loaded: {loaded:?}"),
    ));
  }
  Ok((conn, httpfs_note))
}

pub fn register_application_descriptor(descriptor: BootstrapDescriptor) -> Result<()> {
  APPLICATION_DESCRIPTOR.set(descriptor).map_err(|_| {
    failure(
      "configuration_seal",
      "descriptor_already_registered",
      "the application DuckDB descriptor is immutable after registration",
    )
  })
}

pub fn registered_application_descriptor() -> Option<&'static BootstrapDescriptor> {
  APPLICATION_DESCRIPTOR.get()
}

pub fn descriptor_for_current_target(
  resource_root: impl Into<PathBuf>,
  app_data_root: impl Into<PathBuf>,
) -> Result<BootstrapDescriptor> {
  let conn = Connection::open_in_memory().map_err(|error| {
    failure(
      "engine_compatibility",
      "engine_compatibility",
      error.to_string(),
    )
  })?;
  let engine = conn
    .query_row("SELECT version()", [], |row| row.get::<_, String>(0))
    .map_err(|error| {
      failure(
        "engine_compatibility",
        "engine_compatibility",
        error.to_string(),
      )
    })?;
  let platform = conn
    .query_row("PRAGMA platform", [], |row| row.get::<_, String>(0))
    .map_err(|error| {
      failure(
        "engine_compatibility",
        "engine_compatibility",
        error.to_string(),
      )
    })?;
  BootstrapDescriptor::from_paths(resource_root, app_data_root, &engine, &platform)
}

impl BootstrapDescriptor {
  pub fn from_paths(
    resource_root: impl Into<PathBuf>,
    app_data_root: impl Into<PathBuf>,
    engine: &str,
    platform: &str,
  ) -> Result<Self> {
    let manifest = parse_manifest(COMPILED_MANIFEST)?;
    let pin = select_pin(&manifest, engine, platform)?;
    Ok(Self {
      resource_root: resource_root.into(),
      cache_root: app_data_root.into().join("duckdb/extensions"),
      extension_repository: OFFICIAL_EXTENSIONS_REPOSITORY.to_string(),
      install_timeout: HTTPFS_INSTALL_TIMEOUT,
      pin,
    })
  }

  /// Points signed-extension provisioning at another repository.
  ///
  /// This exists for offline validation fixtures that must prove behavior when the official
  /// repository is unreachable. Signature and metadata checks stay enabled, so it cannot admit
  /// unsigned code. Application descriptors keep the reviewed official repository.
  pub fn with_extension_repository(mut self, repository: impl Into<String>) -> Self {
    self.extension_repository = repository.into();
    self
  }

  /// Bounds how long a cold-cache HTTPFS install may delay the first local query.
  pub fn with_install_timeout(mut self, timeout: Duration) -> Self {
    self.install_timeout = timeout;
    self
  }

  pub fn resource_path(&self) -> Result<PathBuf> {
    resolve_resource_path(&self.resource_root, &self.pin.resource_relative_path)
  }

  pub fn cache_root(&self) -> &Path {
    &self.cache_root
  }

  pub fn pin(&self) -> &DuckomoArtifactPin {
    &self.pin
  }

  pub fn extension_repository(&self) -> &str {
    &self.extension_repository
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::fs;
  use std::path::{Path, PathBuf};
  use uuid::Uuid;

  struct TestDir(PathBuf);

  impl TestDir {
    fn new() -> Self {
      let path = std::env::temp_dir().join(format!("duckling-bootstrap-{}", Uuid::new_v4()));
      fs::create_dir_all(&path).unwrap();
      Self(path)
    }

    fn path(&self) -> &Path {
      &self.0
    }
  }

  impl Drop for TestDir {
    fn drop(&mut self) {
      let _ = fs::remove_dir_all(&self.0);
    }
  }

  fn compiled_manifest() -> DuckomoArtifactManifest {
    parse_manifest(COMPILED_MANIFEST).unwrap()
  }

  fn fixture_pin() -> DuckomoArtifactPin {
    let mut pin = select_pin(&compiled_manifest(), "v2.0.0", "linux_arm64").unwrap();
    pin.binary_size = 3;
    pin.binary_sha256 = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".into();
    pin
  }

  fn resource_relative_path() -> &'static str {
    "extensions/v2.0.0/linux_arm64/duckomo.duckdb_extension"
  }

  #[test]
  fn selects_only_the_exact_engine_and_platform_pin() {
    let manifest = compiled_manifest();
    let pin = select_pin(&manifest, "v2.0.0", "linux_arm64").unwrap();
    assert_eq!(pin.platform, "linux_arm64");
    assert_eq!(
      pin.binary_sha256,
      "2189a68bebc26f2eb7fa7b7b0f1fb2e5db9bd4cea543d5941ec165f05046a83e"
    );
    assert!(select_pin(&manifest, "v1.5.5", "linux_arm64").is_err());
    assert!(select_pin(&manifest, "v2.0.0", "windows_amd64").is_err());
  }

  #[test]
  fn rejects_duplicate_platforms_and_malformed_digests() {
    let mut value: serde_json::Value = serde_json::from_str(COMPILED_MANIFEST).unwrap();
    let duplicate = value["platforms"][0].clone();
    value["platforms"].as_array_mut().unwrap().push(duplicate);
    assert!(parse_manifest(&value.to_string()).is_err());

    let mut value: serde_json::Value = serde_json::from_str(COMPILED_MANIFEST).unwrap();
    value["platforms"][0]["binary_sha256"] = "not-a-sha256".into();
    assert!(parse_manifest(&value.to_string()).is_err());
  }

  #[test]
  fn verifies_raw_sha256_and_rejects_truncated_or_corrupt_files() {
    let temp = TestDir::new();
    let artifact = temp.path().join("duckomo.duckdb_extension");
    fs::write(&artifact, b"abc").unwrap();
    assert_eq!(
      verify_artifact(&artifact, &fixture_pin()).unwrap(),
      "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );

    fs::write(&artifact, b"ab").unwrap();
    assert!(verify_artifact(&artifact, &fixture_pin()).is_err());

    fs::write(&artifact, b"abd").unwrap();
    assert!(verify_artifact(&artifact, &fixture_pin()).is_err());
  }

  #[test]
  fn resolves_quoted_unicode_resource_paths_inside_the_canonical_root() {
    let temp = TestDir::new();
    let root = temp.path().join("resource root with ' quote 雪");
    let relative = Path::new(resource_relative_path());
    let artifact = root.join(relative);
    fs::create_dir_all(artifact.parent().unwrap()).unwrap();
    fs::write(&artifact, b"fixture").unwrap();

    let resolved = resolve_resource_path(&root, resource_relative_path()).unwrap();
    assert_eq!(resolved, fs::canonicalize(&artifact).unwrap());
    assert!(sql_literal(&resolved.to_string_lossy()).contains("''"));
    assert!(sql_literal(&resolved.to_string_lossy()).contains("雪"));
    assert!(resolve_resource_path(&root, "../outside/duckomo.duckdb_extension").is_err());
  }

  #[cfg(unix)]
  #[test]
  fn rejects_resource_symlinks_that_escape_the_canonical_root() {
    use std::os::unix::fs::symlink;

    let temp = TestDir::new();
    let root = temp.path().join("resource root");
    let outside = temp.path().join("outside");
    let relative = Path::new(resource_relative_path());
    let artifact = root.join(relative);
    fs::create_dir_all(artifact.parent().unwrap()).unwrap();
    fs::create_dir_all(&outside).unwrap();
    let outside_file = outside.join("duckomo.duckdb_extension");
    fs::write(&outside_file, b"outside").unwrap();
    symlink(&outside_file, &artifact).unwrap();

    assert!(resolve_resource_path(&root, resource_relative_path()).is_err());
  }

  #[test]
  fn unavailable_extension_cache_degrades_to_unavailable_httpfs() {
    let temp = TestDir::new();
    let blocker = temp.path().join("not a directory");
    fs::write(&blocker, b"file").unwrap();
    let provision = provision_signed_httpfs(
      &blocker.join("extensions"),
      "http://127.0.0.1:1",
      "v2.0.0",
      "linux_arm64",
      Duration::from_secs(15),
    )
    .unwrap();
    match provision {
      HttpfsProvision::Unavailable(reason) => assert!(reason.contains("cache is unavailable")),
      HttpfsProvision::Loaded(_) => panic!("an unusable cache must not report a loaded HTTPFS"),
    }

    let provision = provision_signed_httpfs(
      &temp.path().join("empty cache"),
      "http://127.0.0.1:1",
      "v2.0.0",
      "linux_arm64",
      Duration::from_secs(15),
    )
    .unwrap();
    match provision {
      HttpfsProvision::Unavailable(reason) => {
        assert!(reason.contains("signed HTTPFS installation failed"))
      }
      HttpfsProvision::Loaded(_) => panic!("an unreachable repository must not report a loaded HTTPFS"),
    }
  }

  #[test]
  fn hanging_repository_is_bounded_by_the_install_timeout() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let acceptor = std::thread::spawn(move || {
      let held: Vec<_> = listener.incoming().take(4).flatten().collect();
      std::thread::sleep(Duration::from_secs(6));
      drop(held);
    });
    let temp = TestDir::new();
    let started = std::time::Instant::now();
    let provision = provision_signed_httpfs(
      &temp.path().join("hang cache"),
      &format!("http://{address}"),
      "v2.0.0",
      "linux_arm64",
      Duration::from_millis(800),
    )
    .unwrap();
    assert!(started.elapsed() < Duration::from_secs(4), "install was not bounded");
    match provision {
      HttpfsProvision::Unavailable(reason) => assert!(reason.contains("did not finish within")),
      HttpfsProvision::Loaded(_) => panic!("a hanging repository must not report a loaded HTTPFS"),
    }
    drop(acceptor);
  }

  #[test]
  fn registers_immutable_application_descriptor_for_current_target() {
    if std::env::var_os("DUCKLING_DESCRIPTOR_TEST_CHILD").is_none() {
      let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "dialect::duckdb::bootstrap::tests::registers_immutable_application_descriptor_for_current_target", "--nocapture"])
        .env("DUCKLING_DESCRIPTOR_TEST_CHILD", "1")
        .output().unwrap();
      assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stdout));
      return;
    }
    let temp = TestDir::new();
    let descriptor = descriptor_for_current_target(temp.path(), temp.path()).unwrap();
    let expected_platform = descriptor.pin().platform.clone();
    let standalone = open_local_connection(None, None).unwrap();
    assert_eq!(
      setting(&standalone, "allow_unsigned_extensions", "extension_load").unwrap(),
      "false"
    );
    assert!(
      !loaded_extensions(&standalone, "extension_load")
        .unwrap()
        .iter()
        .any(|name| name == EXTENSION_NAME)
    );
    register_application_descriptor(descriptor).unwrap();
    assert_eq!(
      registered_application_descriptor().unwrap().pin().platform,
      expected_platform
    );

    let replacement = descriptor_for_current_target(temp.path(), temp.path()).unwrap();
    let error = register_application_descriptor(replacement).unwrap_err();
    assert!(error.to_string().contains("descriptor_already_registered"));
  }
}
