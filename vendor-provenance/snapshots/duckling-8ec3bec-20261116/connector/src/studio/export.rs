use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Serialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::CancelToken;
use crate::studio::error::{StudioError, StudioErrorCode};
use crate::studio::results::{LocalOperation, ResultLease, ResultManifest, error};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportReceipt {
  pub schema_version: u32,
  pub data_path: String,
  pub provenance_path: String,
  pub data_sha256: String,
  pub commit_id: String,
  pub completion: &'static str,
  pub row_count: String,
  pub recovery_paths: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Sidecar<'a> {
  schema_version: u32,
  data_sha256: &'a str,
  commit_id: &'a str,
  completion: &'static str,
  row_count: &'a str,
  remote_reread: bool,
  manifest: &'a ResultManifest,
}

fn check(cancel: &CancelToken, started: Instant) -> Result<(), StudioError> {
  if cancel.is_cancelled() { return Err(error(StudioErrorCode::Cancelled, "export cancelled")); }
  if started.elapsed() > Duration::from_secs(300) { return Err(error(StudioErrorCode::BudgetExceeded, "local export exceeded its deadline")); }
  Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ExportPart { Data, Provenance }

#[cfg_attr(not(test), allow(dead_code))]
struct ExportWrite { part: ExportPart, bytes: usize, error: Option<i32> }

struct ObservedFile<'a, F> {
  file: File,
  part: ExportPart,
  observe: &'a mut F,
  cancel: &'a CancelToken,
  started: Instant,
}

impl<F: FnMut(ExportWrite)> Write for ObservedFile<'_, F> {
  fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
    if self.cancel.is_cancelled() || self.started.elapsed() > Duration::from_secs(300) { return Err(io::Error::other("export stopped")); }
    let result = self.file.write(bytes);
    (self.observe)(ExportWrite { part: self.part, bytes: result.as_ref().copied().unwrap_or(0), error: result.as_ref().err().and_then(io::Error::raw_os_error) });
    result
  }
  fn flush(&mut self) -> io::Result<()> { self.file.flush() }
}

struct HashWriter<'a, F: FnMut(ExportWrite)> {
  file: BufWriter<ObservedFile<'a, F>>,
  digest: Sha256,
  cancel: &'a CancelToken,
  started: Instant,
}

impl<F: FnMut(ExportWrite)> Write for HashWriter<'_, F> {
  fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
    if self.cancel.is_cancelled() || self.started.elapsed() > Duration::from_secs(300) { return Err(io::Error::other("export stopped")); }
    let written = self.file.write(&bytes[..bytes.len().min(64 * 1024)])?;
    self.digest.update(&bytes[..written]);
    Ok(written)
  }
  fn flush(&mut self) -> io::Result<()> { self.file.flush() }
}

fn cell(output: &mut impl Write, value: &str) -> io::Result<()> {
  if !value.contains([',', '"', '\r', '\n']) { return output.write_all(value.as_bytes()); }
  output.write_all(b"\"")?;
  let mut segments = value.split('"').peekable();
  while let Some(segment) = segments.next() {
    output.write_all(segment.as_bytes())?;
    if segments.peek().is_some() { output.write_all(b"\"\"")?; }
  }
  output.write_all(b"\"")
}

fn write_csv(output: &mut impl Write, lease: &ResultLease) -> io::Result<()> {
  output.write_all(b"station_id,valid_time,variable_id,value,unit,target_lat,target_lon,source_lat,source_lon,source_point_index,source_object_id,quality,business_json\n")?;
  for index in 0..lease.batch().num_rows() {
    let row = lease.row(index);
    let value = row.value.map(|value| value.to_string()).unwrap_or_default();
    let coordinates = [row.target_lat.to_string(), row.target_lon.to_string(), row.source_lat.to_string(), row.source_lon.to_string()];
    let cells = [&row.station_id, &row.valid_time, &row.variable_id, &value, row.unit.as_deref().unwrap_or(""), &coordinates[0], &coordinates[1], &coordinates[2], &coordinates[3], &row.source_point_index, &row.source_object_id, &row.quality, lease.business_json(&row.station_id)];
    for (index, value) in cells.iter().enumerate() {
      if index != 0 { output.write_all(b",")?; }
      cell(output, value)?;
    }
    output.write_all(b"\n")?;
  }
  Ok(())
}

fn regular_or_missing(path: &Path) -> Result<bool, StudioError> {
  match fs::symlink_metadata(path) {
    Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => Ok(true),
    Ok(_) => Err(error(StudioErrorCode::ExportIo, "export target must be a regular file")),
    Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
    Err(_) => Err(error(StudioErrorCode::ExportIo, "cannot inspect export target")),
  }
}

fn create(path: &Path) -> Result<File, StudioError> {
  let mut options = OpenOptions::new();
  options.write(true).create_new(true);
  #[cfg(unix)]
  { use std::os::unix::fs::OpenOptionsExt; options.mode(0o600); }
  options.open(path).map_err(|_| error(StudioErrorCode::ExportIo, "cannot create export temporary file"))
}

struct PairCommit {
  data: PathBuf,
  sidecar: PathBuf,
  data_tmp: PathBuf,
  side_tmp: PathBuf,
  backups: Vec<(PathBuf, PathBuf)>,
  installed: Vec<(PathBuf, PathBuf)>,
  committed: bool,
  recovery_paths: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CommitStage { BackedUpData, BackedUpSidecar, InstalledData, InstalledSidecar }

fn same_file(left: &Path, right: &Path) -> bool {
  #[cfg(unix)]
  {
    use std::os::unix::fs::MetadataExt;
    match (fs::symlink_metadata(left), fs::symlink_metadata(right)) {
      (Ok(left), Ok(right)) => left.dev() == right.dev() && left.ino() == right.ino(),
      _ => false,
    }
  }
  #[cfg(not(unix))]
  { let _ = (left, right); false }
}

impl PairCommit {
  fn commit(&mut self, overwrite: bool, mut checkpoint: impl FnMut(CommitStage) -> Result<(), StudioError>) -> Result<(), StudioError> {
    let id = Uuid::new_v4().simple().to_string();
    for (index, target) in [&self.data, &self.sidecar].into_iter().enumerate() {
      if regular_or_missing(target)? {
        if !overwrite { return Err(StudioError::new(StudioErrorCode::ExportIo, serde_json::json!({"reason":"existing export requires confirmation", "requiresOverwriteConfirmation":true}))); }
        let backup = target.parent().unwrap().join(format!(".{id}-{index}.studio-backup"));
        fs::rename(target, &backup).map_err(|_| error(StudioErrorCode::ExportIo, "cannot preserve existing export"))?;
        self.backups.push((target.clone(), backup));
      }
      checkpoint(if index == 0 { CommitStage::BackedUpData } else { CommitStage::BackedUpSidecar })?;
    }
    for (index, (source, target)) in [(&self.data_tmp, &self.data), (&self.side_tmp, &self.sidecar)].into_iter().enumerate() {
      fs::hard_link(source, target).map_err(|_| error(StudioErrorCode::ExportIo, "cannot commit export without replacing an unconfirmed file"))?;
      self.installed.push((source.clone(), target.clone()));
      checkpoint(if index == 0 { CommitStage::InstalledData } else { CommitStage::InstalledSidecar })?;
    }
    File::open(self.data.parent().unwrap()).and_then(|directory| directory.sync_all()).map_err(|_| error(StudioErrorCode::ExportIo, "cannot sync export directory"))?;
    checkpoint(CommitStage::InstalledSidecar)?;
    self.committed = true;
    for (_, backup) in &self.backups {
      if fs::remove_file(backup).is_err() { self.recovery_paths.push(backup.to_string_lossy().into_owned()); }
    }
    Ok(())
  }

  fn rollback(&mut self) {
    for (source, target) in self.installed.drain(..).rev() {
      if same_file(&source, &target) { let _ = fs::remove_file(target); }
    }
    for (target, backup) in self.backups.drain(..).rev() {
      if fs::hard_link(&backup, &target).is_ok() {
        let _ = fs::remove_file(backup);
      } else { self.recovery_paths.push(backup.to_string_lossy().into_owned()); }
    }
  }
}

impl Drop for PairCommit {
  fn drop(&mut self) {
    if !self.committed { self.rollback(); }
    let _ = fs::remove_file(&self.data_tmp);
    let _ = fs::remove_file(&self.side_tmp);
  }
}

pub fn export_csv(lease: &ResultLease, destination: &Path, overwrite: bool, cancel: &CancelToken) -> Result<ExportReceipt, StudioError> {
  export_csv_with_commit(lease, destination, overwrite, cancel, || Ok(()))
}

pub fn export_csv_with_commit(lease: &ResultLease, destination: &Path, overwrite: bool, cancel: &CancelToken, begin_commit: impl FnOnce() -> Result<(), StudioError>) -> Result<ExportReceipt, StudioError> {
  let operation = lease.begin_operation(lease.export_memory_bytes())?;
  export_csv_reserved(lease, destination, overwrite, cancel, operation, begin_commit)
}

pub fn export_csv_reserved(lease: &ResultLease, destination: &Path, overwrite: bool, cancel: &CancelToken, operation: LocalOperation, begin_commit: impl FnOnce() -> Result<(), StudioError>) -> Result<ExportReceipt, StudioError> {
  export_csv_inner(lease, destination, overwrite, cancel, operation, begin_commit, create, |_| {})
}

fn export_csv_inner(
  lease: &ResultLease, destination: &Path, overwrite: bool, cancel: &CancelToken, operation: LocalOperation,
  begin_commit: impl FnOnce() -> Result<(), StudioError>, mut open_file: impl FnMut(&Path) -> Result<File, StudioError>,
  mut observe: impl FnMut(ExportWrite),
) -> Result<ExportReceipt, StudioError> {
  lease.validate_operation(&operation, lease.export_memory_bytes())?;
  let _operation = operation;
  let started = Instant::now();
  check(cancel, started)?;
  let stem = destination.file_name().and_then(|name| name.to_str()).filter(|name| !name.is_empty())
    .ok_or_else(|| error(StudioErrorCode::ExportIo, "invalid export filename"))?;
  let parent = destination.parent().unwrap_or(Path::new(".")).canonicalize()
    .map_err(|_| error(StudioErrorCode::ExportIo, "export directory is unavailable"))?;
  let destination = parent.join(stem);
  let sidecar = parent.join(format!("{stem}.provenance.json"));
  let data_exists = regular_or_missing(&destination)?;
  let side_exists = regular_or_missing(&sidecar)?;
  if (data_exists || side_exists) && !overwrite { return Err(StudioError::new(StudioErrorCode::ExportIo, serde_json::json!({"reason":"existing export requires confirmation", "requiresOverwriteConfirmation":true}))); }
  let commit_id = Uuid::new_v4().simple().to_string();
  let mut pair = PairCommit { data: destination.clone(), sidecar: sidecar.clone(), data_tmp: parent.join(format!(".{commit_id}.studio-data.partial")), side_tmp: parent.join(format!(".{commit_id}.studio-provenance.partial")), backups: Vec::new(), installed: Vec::new(), committed: false, recovery_paths: Vec::new() };
  let digest = {
    let mut output = HashWriter { file: BufWriter::with_capacity(64 * 1024, ObservedFile { file: open_file(&pair.data_tmp)?, part: ExportPart::Data, observe: &mut observe, cancel, started }), digest: Sha256::new(), cancel, started };
    if write_csv(&mut output, lease).and_then(|_| output.flush()).and_then(|_| output.file.get_ref().file.sync_all()).is_err() {
      check(cancel, started)?;
      return Err(error(StudioErrorCode::ExportIo, "CSV write failed"));
    }
    format!("{:x}", output.digest.finalize())
  };
  check(cancel, started)?;
  let manifest = lease.manifest();
  let sidecar_document = Sidecar { schema_version: 1, data_sha256: &digest, commit_id: &commit_id, completion: "complete", row_count: &manifest.row_count, remote_reread: false, manifest };
  let mut output = HashWriter { file: BufWriter::with_capacity(64 * 1024, ObservedFile { file: open_file(&pair.side_tmp)?, part: ExportPart::Provenance, observe: &mut observe, cancel, started }), digest: Sha256::new(), cancel, started };
  if serde_json::to_writer_pretty(&mut output, &sidecar_document).is_err() || output.flush().and_then(|_| output.file.get_ref().file.sync_all()).is_err() {
    check(cancel, started)?;
    return Err(error(StudioErrorCode::ExportIo, "provenance write failed"));
  }
  drop(output);
  check(cancel, started)?;
  begin_commit()?;
  if let Err(error) = pair.commit(overwrite, |_| check(cancel, started)) {
    pair.rollback();
    if !pair.recovery_paths.is_empty() {
      return Err(StudioError::new(StudioErrorCode::ExportIo, serde_json::json!({"reason": "export failed; preserved original files require recovery", "recoveryPaths": pair.recovery_paths.clone()})));
    }
    return Err(error);
  }
  Ok(ExportReceipt { schema_version: 1, data_path: destination.to_string_lossy().into_owned(), provenance_path: sidecar.to_string_lossy().into_owned(), data_sha256: digest, commit_id, completion: "complete", row_count: manifest.row_count.clone(), recovery_paths: pair.recovery_paths.clone() })
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::sync::Arc;
  use crate::dialect::duckdb::session::NativeSession;
  use crate::studio::model::{SeriesRow, format_utc_micros};
  use crate::studio::results::{Publication, RetainedPool};
  use crate::studio::stations::{confirm_rows, LongitudeConvention, WhitespacePolicy};

  struct Fixture {
    _session: Arc<NativeSession>,
    pool: RetainedPool,
    manifest: ResultManifest,
    root: PathBuf,
  }

  impl Fixture {
    async fn new() -> Self {
      let session = Arc::new(NativeSession::new_local(None, None));
      let owner = Arc::clone(&session);
      let pool = RetainedPool::new();
      let publishing = pool.clone();
      let manifest = session.run(None, move |connection, _| {
        let columns = vec!["id".into(), "lat".into(), "lon".into(), "name".into()];
        let station = std::collections::BTreeMap::from([("id".into(), "001".into()), ("lat".into(), "10".into()), ("lon".into(), "20".into()), ("name".into(), "北京,\"站点\"\n".into())]);
        let hash = "a".repeat(64);
        let snapshot = confirm_rows(&hash, &hash, &columns, &[station], "id", "lat", "lon", LongitudeConvention::Signed180, WhitespacePolicy::Preserve, &Default::default(), &[0])?;
        let rows = (0..64).map(|index| SeriesRow { station_id: "001".into(), valid_time: format_utc_micros(index * 3_600_000_000).unwrap(), variable_id: "test".into(), value: if index == 1 { None } else { Some(index as f32) }, unit: Some("°C".into()), target_lat: 10.0, target_lon: 20.0, source_lat: 10.0, source_lon: 20.0, source_point_index: "9007199254740993".into(), source_object_id: "x".repeat(8192), quality: "synthetic".into() }).collect();
        Ok(publishing.publish(publishing.reserve(16 * 1024 * 1024)?, Publication { session: Arc::clone(&owner), owner_id: "export-owner".into(), generation: owner.snapshot().generation, rows, snapshot, provenance: serde_json::json!({"synthetic":true, "attribution":"Test producer", "diagnostic":"x".repeat(80 * 1024)}) }, connection)?)
      }).await.unwrap();
      let root = std::env::temp_dir().join(format!("studio-export-stream-{}", Uuid::new_v4().simple()));
      fs::create_dir(&root).unwrap();
      Self { _session: session, pool, manifest, root }
    }
    fn lease(&self) -> ResultLease {
      self.pool.acquire(&self.manifest.handle, "export-owner", self._session.snapshot().generation).unwrap()
    }
    fn destination(&self) -> PathBuf { self.root.join("series.csv") }
    fn sidecar(&self) -> PathBuf { self.root.join("series.csv.provenance.json") }
    fn assert_recovered(&self) {
      assert_eq!(fs::read_to_string(self.destination()).unwrap(), "original data");
      assert_eq!(fs::read_to_string(self.sidecar()).unwrap(), "original provenance");
      assert_eq!(fs::read_dir(&self.root).unwrap().count(), 2);
      let first = self.pool.begin_operation(32 * 1024 * 1024).unwrap();
      let second = self.pool.begin_operation(32 * 1024 * 1024).unwrap();
      drop((first, second));
    }
    fn assert_successful_retry(&self, lease: &ResultLease) {
      let receipt = export_csv(lease, &self.destination(), true, &CancelToken::new()).unwrap();
      let bytes = fs::read(self.destination()).unwrap();
      let sidecar: serde_json::Value = serde_json::from_slice(&fs::read(self.sidecar()).unwrap()).unwrap();
      assert_eq!(sidecar["dataSha256"], format!("{:x}", Sha256::digest(&bytes)));
      assert_eq!(sidecar["commitId"], receipt.commit_id);
      assert_eq!(sidecar["completion"], "complete");
      assert_eq!(sidecar["rowCount"], "64");
      assert_eq!(sidecar["manifest"]["provenance"]["attribution"], "Test producer");
      assert_eq!(fs::read_dir(&self.root).unwrap().count(), 2);
    }
  }

  impl Drop for Fixture { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.root); } }

  #[tokio::test]
  async fn write_errors_in_both_streams_preserve_originals_and_allow_retry() {
    let fixture = Fixture::new().await;
    let lease = fixture.lease();
    let mut modes = vec![(ExportPart::Data, false), (ExportPart::Provenance, false)];
    #[cfg(target_os = "linux")]
    modes.extend([(ExportPart::Data, true), (ExportPart::Provenance, true)]);
    for (part, disk_full) in modes {
      fs::write(fixture.destination(), "original data").unwrap();
      fs::write(fixture.sidecar(), "original provenance").unwrap();
      let cancel = CancelToken::new();
      let mut data_written = 0;
      let mut saw_error = false;
      let error = export_csv_inner(&lease, &fixture.destination(), true, &cancel, lease.begin_operation(lease.export_memory_bytes()).unwrap(), || panic!("failed stream must not commit"), |path| {
        let file = create(path)?;
        let is_data = path.to_string_lossy().ends_with("studio-data.partial");
        if is_data == (part == ExportPart::Data) {
          drop(file);
          if disk_full { OpenOptions::new().write(true).open("/dev/full") } else { File::open(path) }.map_err(|_| error(StudioErrorCode::ExportIo, "cannot open failing test sink"))
        } else { Ok(file) }
      }, |event| {
        if event.part == ExportPart::Data { data_written += event.bytes; }
        if event.part == part && event.error.is_some() {
          saw_error = true;
          if disk_full { assert_eq!(event.error, Some(28)); }
        }
      }).unwrap_err();
      assert_eq!(error.code, StudioErrorCode::ExportIo);
      assert!(saw_error);
      if part == ExportPart::Provenance { assert!(data_written > 64 * 1024); }
      fixture.assert_recovered();
      fixture.assert_successful_retry(&lease);
      println!("export_write_fault part={} sink={} data_os_bytes={data_written} original_pair=preserved temporaries=removed retry=passed", if part == ExportPart::Data { "data" } else { "provenance" }, if disk_full { "dev_full_enospc_28" } else { "readonly_file" });
    }
  }

  #[tokio::test]
  async fn cancellation_after_os_writes_cleans_both_streams_and_keeps_leases_charged() {
    for part in [ExportPart::Data, ExportPart::Provenance] {
      let fixture = Fixture::new().await;
      let lease = fixture.lease();
      fs::write(fixture.destination(), "original data").unwrap();
      fs::write(fixture.sidecar(), "original provenance").unwrap();
      let cancel = CancelToken::new();
      let mut bytes_before_cancel = 0;
      let mut bytes_after_cancel = 0;
      let held_bytes = fixture.pool.charged_bytes();
      let error = export_csv_inner(&lease, &fixture.destination(), true, &cancel, lease.begin_operation(lease.export_memory_bytes()).unwrap(), || panic!("cancelled stream must not commit"), create, |event| {
        if cancel.is_cancelled() { bytes_after_cancel += event.bytes; }
        if event.part == part && event.bytes > 0 && !cancel.is_cancelled() {
          bytes_before_cancel += event.bytes;
          fixture.pool.close(&fixture.manifest.handle, "export-owner", fixture._session.snapshot().generation).unwrap();
          assert_eq!(fixture.pool.charged_bytes(), held_bytes);
          cancel.cancel();
        }
      }).unwrap_err();
      assert_eq!(error.code, StudioErrorCode::Cancelled);
      assert!(bytes_before_cancel > 0);
      assert_eq!(bytes_after_cancel, 0);
      fixture.assert_recovered();
      fixture.assert_successful_retry(&lease);
      assert_eq!(fixture.pool.charged_bytes(), held_bytes);
      drop(lease);
      assert_eq!(fixture.pool.charged_bytes(), 0);
      println!("export_cancel part={} os_bytes_before_cancel={bytes_before_cancel} os_bytes_after_cancel={bytes_after_cancel} original_pair=preserved temporaries=removed retained=finally_released retry=passed", if part == ExportPart::Data { "data" } else { "provenance" });
    }
  }

  #[tokio::test]
  async fn rejecting_commit_after_both_streams_preserves_the_original_pair() {
    let fixture = Fixture::new().await;
    let lease = fixture.lease();
    fs::write(fixture.destination(), "original data").unwrap();
    fs::write(fixture.sidecar(), "original provenance").unwrap();
    let cancel = CancelToken::new();
    let mut wrote = [0, 0];
    let error = export_csv_inner(&lease, &fixture.destination(), true, &cancel, lease.begin_operation(lease.export_memory_bytes()).unwrap(), || { cancel.cancel(); Err(error(StudioErrorCode::Cancelled, "cancel won before commit")) }, create, |event| { wrote[if event.part == ExportPart::Data { 0 } else { 1 }] += event.bytes; }).unwrap_err();
    assert_eq!(error.code, StudioErrorCode::Cancelled);
    assert!(wrote.iter().all(|bytes| *bytes > 64 * 1024));
    fixture.assert_recovered();
    fixture.assert_successful_retry(&lease);
  }

  #[cfg(unix)]
  #[test]
  fn filesystem_rename_denial_preserves_original_pair() {
    use std::os::unix::fs::PermissionsExt;
    let root = std::env::temp_dir().join(format!("studio-rename-denied-{}", Uuid::new_v4().simple()));
    fs::create_dir(&root).unwrap();
    let data = root.join("series.csv"); let sidecar = root.join("series.csv.provenance.json");
    let data_tmp = root.join("data.partial"); let side_tmp = root.join("side.partial");
    fs::write(&data, "original data").unwrap(); fs::write(&sidecar, "original provenance").unwrap();
    fs::write(&data_tmp, "new data").unwrap(); fs::write(&side_tmp, "new provenance").unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o500)).unwrap();
    if File::create(root.join("permission-control")).is_ok() {
      fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
      fs::remove_dir_all(&root).unwrap();
      println!("rename_denial=SKIPPED process bypasses directory write permissions");
      return;
    }
    let mut pair = PairCommit { data: data.clone(), sidecar: sidecar.clone(), data_tmp, side_tmp, backups: vec![], installed: vec![], committed: false, recovery_paths: vec![] };
    let failure = pair.commit(true, |_| Ok(()));
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(failure.unwrap_err().code, StudioErrorCode::ExportIo);
    assert!(pair.backups.is_empty());
    drop(pair);
    assert_eq!(fs::read_to_string(&data).unwrap(), "original data");
    assert_eq!(fs::read_to_string(&sidecar).unwrap(), "original provenance");
    assert_eq!(fs::read_dir(&root).unwrap().count(), 2);
    fs::remove_dir_all(root).unwrap();
    println!("rename_denial=PASS actual_directory_permissions original_pair=preserved temporaries=removed");
  }

  #[test]
  fn rollback_preserves_a_backup_when_an_unconfirmed_file_blocks_restoration() {
    let root = std::env::temp_dir().join(format!("studio-recovery-{}", Uuid::new_v4().simple()));
    fs::create_dir(&root).unwrap();
    let data = root.join("series.csv"); let sidecar = root.join("series.csv.provenance.json");
    let data_tmp = root.join("data.partial"); let side_tmp = root.join("side.partial");
    fs::write(&data, "original data").unwrap(); fs::write(&sidecar, "original provenance").unwrap();
    fs::write(&data_tmp, "new data").unwrap(); fs::write(&side_tmp, "new provenance").unwrap();
    let mut pair = PairCommit { data: data.clone(), sidecar: sidecar.clone(), data_tmp, side_tmp, backups: vec![], installed: vec![], committed: false, recovery_paths: vec![] };
    assert!(pair.commit(true, |stage| {
      if stage == CommitStage::BackedUpSidecar {
        fs::write(&sidecar, "unconfirmed replacement").unwrap();
        Err(error(StudioErrorCode::ExportIo, "simulated file race"))
      } else { Ok(()) }
    }).is_err());
    pair.rollback();
    assert_eq!(pair.recovery_paths.len(), 1);
    let backup = PathBuf::from(&pair.recovery_paths[0]);
    drop(pair);
    assert_eq!(fs::read_to_string(&data).unwrap(), "original data");
    assert_eq!(fs::read_to_string(&sidecar).unwrap(), "unconfirmed replacement");
    assert_eq!(fs::read_to_string(backup).unwrap(), "original provenance");
    assert_eq!(fs::read_dir(&root).unwrap().count(), 3);
    fs::remove_dir_all(root).unwrap();
  }

  #[test]
  fn commit_failures_preserve_both_original_files() {
    for failure in [CommitStage::BackedUpData, CommitStage::BackedUpSidecar, CommitStage::InstalledData, CommitStage::InstalledSidecar] {
      let root = std::env::temp_dir().join(format!("studio-pair-{}", Uuid::new_v4().simple()));
      fs::create_dir(&root).unwrap();
      let data = root.join("series.csv"); let sidecar = root.join("series.csv.provenance.json");
      let data_tmp = root.join("data.partial"); let side_tmp = root.join("side.partial");
      fs::write(&data, "original data").unwrap(); fs::write(&sidecar, "original provenance").unwrap();
      fs::write(&data_tmp, "new data").unwrap(); fs::write(&side_tmp, "new provenance").unwrap();
      let mut pair = PairCommit { data: data.clone(), sidecar: sidecar.clone(), data_tmp, side_tmp, backups: vec![], installed: vec![], committed: false, recovery_paths: vec![] };
      assert!(pair.commit(true, |stage| if stage == failure { Err(error(StudioErrorCode::ExportIo, "simulated commit failure")) } else { Ok(()) }).is_err());
      drop(pair);
      assert_eq!(fs::read_to_string(&data).unwrap(), "original data");
      assert_eq!(fs::read_to_string(&sidecar).unwrap(), "original provenance");
      assert_eq!(fs::read_dir(&root).unwrap().count(), 2);
      fs::remove_dir_all(root).unwrap();
    }
  }
}
