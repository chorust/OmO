use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, Weak};

use arrow::array::RecordBatch;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::dialect::duckdb::duckdb_sync::DuckDbSyncConnection;
use crate::dialect::duckdb::session::NativeSession;
use crate::studio::error::{StudioError, StudioErrorCode};
use crate::studio::model::{SeriesRow, canonical_sha256, parse_utc_micros};
use crate::studio::profile::load_profile;
use crate::studio::result_relation;
use crate::studio::stations::StationSnapshot;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultManifest {
  pub schema_version: u32,
  pub handle: String,
  pub generation: String,
  pub manifest_hash: String,
  pub row_count: String,
  pub retained_bytes: String,
  pub variable_id: String,
  pub unit: Option<String>,
  pub station_ids: Vec<String>,
  pub alias: String,
  pub station_alias: String,
  pub sql_example: String,
  pub snapshot_hash: String,
  pub provenance: Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultSummary {
  pub schema_version: u32,
  pub handle: String,
  pub generation: String,
  pub row_count: String,
  pub variable_id: String,
  pub unit: Option<String>,
  pub default_station_id: Option<String>,
}

pub struct Publication {
  pub session: Arc<NativeSession>,
  pub owner_id: String,
  pub generation: u64,
  pub rows: Vec<SeriesRow>,
  pub snapshot: StationSnapshot,
  pub provenance: Value,
}

pub(super) struct Backing {
  pub batch: RecordBatch,
  pub stations: RecordBatch,
  pub manifest: ResultManifest,
  manifest_bytes: u64,
  export_bytes: u64,
}

struct Entry {
  owner: Weak<NativeSession>,
  owner_id: String,
  generation: u64,
  backing: Arc<Backing>,
  bytes: u64,
  leases: u64,
  closed: bool,
  ready: bool,
  aliases: Option<(u64, u64)>,
}

#[derive(Default)]
pub(super) struct PoolState {
  items: HashMap<String, Entry>,
  reserved_bytes: u64,
  reserved_handles: u64,
  active_operations: u64,
  operation_bytes: u64,
  observed_owners: HashSet<(String, u64)>,
}

#[derive(Clone, Default)]
pub struct RetainedPool {
  pub(super) state: Arc<Mutex<PoolState>>,
}

pub struct ResultLease {
  state: Arc<Mutex<PoolState>>,
  pub(super) backing: Arc<Backing>,
  handle: String,
}

pub struct PublicationHold {
  state: Arc<Mutex<PoolState>>,
  bytes: u64,
  active: bool,
}

pub struct LocalOperation {
  state: Arc<Mutex<PoolState>>,
  bytes: u64,
}

pub(super) fn error(code: StudioErrorCode, reason: &str) -> StudioError {
  StudioError::new(code, json!({"reason": reason}))
}

fn missing() -> StudioError { error(StudioErrorCode::InvalidRequest, "result is closed or unavailable") }

impl RetainedPool {
  pub fn new() -> Self { Self::default() }

  pub fn limits() -> (u64, u64) {
    let profile = load_profile().expect("compiled profile");
    (profile.results.max_retained_backing_bytes, profile.results.max_handles)
  }

  pub fn reserve(&self, bytes: u64) -> Result<PublicationHold, StudioError> {
    let mut state = self.state.lock().map_err(|_| missing())?;
    let (max_bytes, max_handles) = Self::limits();
    let used = state.items.values().map(|entry| entry.bytes).sum::<u64>();
    if bytes > max_bytes || used.saturating_add(state.reserved_bytes).saturating_add(bytes) > max_bytes
      || state.items.len() as u64 + state.reserved_handles >= max_handles {
      return Err(error(StudioErrorCode::ResultCapacityExceeded, "retained results are at capacity"));
    }
    state.reserved_bytes += bytes;
    state.reserved_handles += 1;
    Ok(PublicationHold { state: Arc::clone(&self.state), bytes, active: true })
  }

  pub fn begin_operation(&self, bytes: u64) -> Result<LocalOperation, StudioError> {
    let profile = load_profile()?;
    let mut state = self.state.lock().map_err(|_| missing())?;
    if state.active_operations >= profile.results.max_concurrent_operations
      || bytes > profile.results.max_live_operation_temporary_bytes
      || state.operation_bytes.saturating_add(bytes) > profile.results.max_live_operation_temporary_bytes {
      return Err(error(StudioErrorCode::BudgetExceeded, "local result operation capacity exceeded"));
    }
    state.active_operations += 1;
    state.operation_bytes += bytes;
    Ok(LocalOperation { state: Arc::clone(&self.state), bytes })
  }

  pub fn publish(&self, mut hold: PublicationHold, input: Publication, connection: &DuckDbSyncConnection) -> Result<ResultManifest, StudioError> {
    if !Arc::ptr_eq(&self.state, &hold.state) { return Err(missing()); }
    if !input.session.is_usable() || input.session.snapshot().generation != input.generation {
      return Err(error(StudioErrorCode::SessionExpired, "publication owner expired"));
    }
    let session = Arc::clone(&input.session);
    let upper_bound = result_relation::validate_publication(&input)?;
    if upper_bound > hold.bytes { return Err(error(StudioErrorCode::ResultCapacityExceeded, "publication exceeds its reservation before Arrow allocation")); }
    let handle = Uuid::new_v4().simple().to_string();
    let alias = format!("studio_result_{handle}");
    let station_alias = format!("studio_stations_{handle}");
    let (batch, stations) = result_relation::batches(&input)?;
    let bytes = batch.get_array_memory_size() as u64 + stations.get_array_memory_size() as u64 + upper_bound;
    if bytes > hold.bytes { return Err(error(StudioErrorCode::ResultCapacityExceeded, "Arrow backing exceeds its reservation")); }
    let mut manifest = ResultManifest {
      schema_version: 1, handle: handle.clone(), generation: input.generation.to_string(), manifest_hash: String::new(),
      row_count: batch.num_rows().to_string(), retained_bytes: bytes.to_string(),
      variable_id: input.rows[0].variable_id.clone(), unit: input.rows[0].unit.clone(),
      station_ids: input.snapshot.rows.iter().map(|row| row.station_id.clone()).collect(),
      alias: alias.clone(), station_alias: station_alias.clone(),
      sql_example: format!("SELECT r.*, s.business_json FROM {alias} r JOIN {station_alias} s USING (station_id) ORDER BY r.station_id, r.valid_time"),
      snapshot_hash: input.snapshot.snapshot_hash.clone(), provenance: input.provenance,
    };
    manifest.manifest_hash = canonical_sha256(&serde_json::to_value(&manifest).map_err(|_| missing())?);
    let manifest_bytes = crate::studio::result_relation::json_memory(&manifest.provenance) + manifest.station_ids.iter().map(|id| id.len() as u64 + 32).sum::<u64>() + manifest.variable_id.len() as u64 + manifest.unit.as_ref().map_or(0, |unit| unit.len() as u64) + 4096;
    let export_bytes = input.rows.iter().map(|row| [row.station_id.len(), row.valid_time.len(), row.variable_id.len(), row.unit.as_ref().map_or(0, String::len), row.source_point_index.len(), row.source_object_id.len(), row.quality.len()].into_iter().sum::<usize>() as u64).max().unwrap_or(0).saturating_add(512 * 1024).max(crate::studio::result_query::OPERATION_RESERVATION);
    let owner_id = input.owner_id;
    let generation = input.generation;
    {
      let mut state = self.state.lock().map_err(|_| missing())?;
      state.reserved_bytes -= hold.bytes;
      state.reserved_handles -= 1;
      hold.active = false;
      state.items.insert(handle.clone(), Entry { owner: Arc::downgrade(&session), owner_id: owner_id.clone(), generation,
        backing: Arc::new(Backing { batch, stations, manifest: manifest.clone(), manifest_bytes, export_bytes }), bytes, leases: 0, closed: false, ready: false, aliases: None });
    }
    let newly_observed = self.state.lock().map_err(|_| missing())?.observed_owners.insert((owner_id.clone(), generation));
    if newly_observed {
      let state = self.weak();
      let owner = owner_id.clone();
      session.on_retire(move || {
        let Some(state) = state.upgrade() else { return; };
        let removed = {
          let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
          state.observed_owners.remove(&(owner.clone(), generation));
          let mut removed = Vec::new();
          state.items.retain(|_, entry| {
            if entry.owner_id != owner || entry.generation != generation { return true; }
            entry.closed = true;
            if entry.leases == 0 { removed.push(Arc::clone(&entry.backing)); false } else { true }
          });
          removed
        };
        drop(removed);
      });
    }
    let installed = result_relation::install(self, connection, &handle, &owner_id, generation, &alias, &station_alias);
    match installed {
      Ok(identities) => {
        let mut state = self.state.lock().map_err(|_| missing())?;
        let entry = state.items.get_mut(&handle).ok_or_else(missing)?;
        entry.ready = true;
        entry.aliases = Some(identities);
        Ok(manifest)
      }
      Err(error) => {
        let removed = self.state.lock().map_err(|_| missing())?.items.remove(&handle);
        drop(removed);
        Err(error)
      }
    }
  }

  pub fn acquire(&self, handle: &str, owner_id: &str, generation: u64) -> Result<ResultLease, StudioError> {
    Self::acquire_state(&self.state, handle, owner_id, generation, false)
  }

  pub(super) fn acquire_state(state: &Arc<Mutex<PoolState>>, handle: &str, owner_id: &str, generation: u64, binding: bool) -> Result<ResultLease, StudioError> {
    let native_owner = {
      let locked = state.lock().map_err(|_| missing())?;
      locked.items.get(handle).ok_or_else(missing)?.owner.upgrade().ok_or_else(|| error(StudioErrorCode::SessionExpired, "result owner retired"))?
    };
    let mut locked = state.lock().map_err(|_| missing())?;
    let entry = locked.items.get_mut(handle).ok_or_else(missing)?;
    if entry.owner_id != owner_id || entry.generation != generation {
      return Err(error(StudioErrorCode::SessionExpired, "result belongs to a different owner or generation"));
    }
    if !native_owner.is_usable() || native_owner.snapshot().generation != generation {
      return Err(error(StudioErrorCode::SessionExpired, "result owner retired"));
    }
    if entry.closed || (!entry.ready && !binding) { return Err(missing()); }
    entry.leases += 1;
    Ok(ResultLease { state: Arc::clone(state), backing: Arc::clone(&entry.backing), handle: handle.to_string() })
  }

  pub fn list(&self, owner_id: &str, generation: u64) -> Result<Vec<ResultSummary>, StudioError> {
    let state = self.state.lock().map_err(|_| missing())?;
    let entries: Vec<_> = state.items.values().filter(|entry| entry.owner_id == owner_id && entry.generation == generation && entry.ready && !entry.closed).collect();
    let bytes: usize = entries.iter().map(|entry| { let manifest = &entry.backing.manifest; manifest.variable_id.len() + manifest.unit.as_ref().map_or(0, String::len) + manifest.station_ids.first().map_or(0, String::len) + 1024 }).sum();
    if bytes > crate::studio::result_query::MAX_RESPONSE_BYTES { return Err(error(StudioErrorCode::ResultPageTooLarge, "result summaries exceed the response limit")); }
    Ok(entries.into_iter().map(|entry| {
      let manifest = &entry.backing.manifest;
      ResultSummary { schema_version: 1, handle: manifest.handle.clone(), generation: manifest.generation.clone(), row_count: manifest.row_count.clone(), variable_id: manifest.variable_id.clone(), unit: manifest.unit.clone(), default_station_id: manifest.station_ids.first().cloned() }
    }).collect())
  }

  pub fn close(&self, handle: &str, owner_id: &str, generation: u64) -> Result<Vec<(String, u64)>, StudioError> {
    let (removed, aliases) = {
      let mut state = self.state.lock().map_err(|_| missing())?;
      let Some(entry) = state.items.get_mut(handle) else { return Ok(Vec::new()); };
      if entry.owner_id != owner_id || entry.generation != generation {
        return Err(error(StudioErrorCode::SessionExpired, "result belongs to a different owner or generation"));
      }
      entry.closed = true;
      let aliases = entry.aliases.map(|(result, station)| vec![(entry.backing.manifest.alias.clone(), result), (entry.backing.manifest.station_alias.clone(), station)]).unwrap_or_default();
      (if entry.leases == 0 { state.items.remove(handle) } else { None }, aliases)
    };
    drop(removed);
    Ok(aliases)
  }

  pub fn alias_identity(&self, handle: &str, owner_id: &str, generation: u64, connection: &DuckDbSyncConnection) -> Result<bool, StudioError> {
    let (alias, station_alias, identities) = {
      let state = self.state.lock().map_err(|_| missing())?;
      let entry = state.items.get(handle).ok_or_else(missing)?;
      if entry.owner_id != owner_id || entry.generation != generation { return Err(error(StudioErrorCode::SessionExpired, "wrong result owner")); }
      (entry.backing.manifest.alias.clone(), entry.backing.manifest.station_alias.clone(), entry.aliases)
    };
    let Some((result, stations)) = identities else { return Ok(false); };
    Ok(result_relation::view_identity(connection, &alias).ok() == Some(result)
      && result_relation::view_identity(connection, &station_alias).ok() == Some(stations))
  }

  pub fn charged_bytes(&self) -> u64 {
    let state = self.state.lock().expect("result pool");
    state.items.values().map(|item| item.bytes).sum::<u64>() + state.reserved_bytes
  }
  pub fn handle_count(&self) -> u64 {
    let state = self.state.lock().expect("result pool");
    state.items.len() as u64 + state.reserved_handles
  }
  pub(super) fn weak(&self) -> Weak<Mutex<PoolState>> { Arc::downgrade(&self.state) }
}

impl ResultLease {
  pub fn begin_operation(&self, bytes: u64) -> Result<LocalOperation, StudioError> {
    RetainedPool { state: Arc::clone(&self.state) }.begin_operation(bytes)
  }
  pub(super) fn validate_operation(&self, operation: &LocalOperation, required: u64) -> Result<(), StudioError> {
    if !Arc::ptr_eq(&self.state, &operation.state) || operation.bytes < required { return Err(error(StudioErrorCode::BudgetExceeded, "invalid local operation reservation")); }
    Ok(())
  }
  pub fn export_memory_bytes(&self) -> u64 { self.backing.export_bytes }
  pub fn manifest_memory_bytes(&self) -> u64 { self.backing.manifest_bytes }
  pub fn manifest(&self) -> &ResultManifest { &self.backing.manifest }
  pub fn batch(&self) -> &RecordBatch { &self.backing.batch }
  pub fn stations(&self) -> &RecordBatch { &self.backing.stations }
  pub fn row(&self, index: usize) -> SeriesRow { result_relation::row(self.batch(), index) }
  pub fn business_json(&self, station_id: &str) -> &str { result_relation::business_json(self.stations(), station_id) }
}

impl Drop for ResultLease {
  fn drop(&mut self) {
    let removed = {
      let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
      let Some(entry) = state.items.get_mut(&self.handle) else { return; };
      entry.leases = entry.leases.saturating_sub(1);
      if entry.closed && entry.leases == 0 { state.items.remove(&self.handle) } else { None }
    };
    drop(removed);
  }
}

impl Drop for PublicationHold {
  fn drop(&mut self) {
    if self.active {
      let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
      state.reserved_bytes -= self.bytes;
      state.reserved_handles -= 1;
    }
  }
}

impl Drop for LocalOperation {
  fn drop(&mut self) {
    let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
    state.active_operations -= 1;
    state.operation_bytes -= self.bytes;
  }
}

pub(super) fn validate_row(row: &SeriesRow) -> Result<(), StudioError> {
  parse_utc_micros(&row.valid_time)?;
  if row.station_id.is_empty() || row.value.is_some_and(|value| !value.is_finite())
    || [row.target_lat, row.target_lon, row.source_lat, row.source_lon].iter().any(|value| !value.is_finite()) {
    return Err(error(StudioErrorCode::InvalidRequest, "invalid result row"));
  }
  Ok(())
}
