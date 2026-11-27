use omo_engine::{
    BootstrapConfig, BootstrappedConn,
    catalog::{
        check_readiness,
        run::{ConnMetaSource, RunError, RunRef, effective_window, resolve, time_index_table},
    },
    limits::job_limits,
    open_bootstrapped,
    plot::create_plot_artifact,
    query::{QueryPurpose, execute_query},
    series::{
        SERIES_VARIABLE, SeriesError, plan_a, plan_b,
        plan_g::{
            self, ParallelDuckDbPointReader, ensure_stable_version, object_version_token,
            write_matches_arrow, write_series_parquet,
        },
        quote_sql_literal, read_canonical_stations, read_stations,
        schema::write_series_arrow,
    },
};
use omo_protocol::{
    result_ref::ResultLimits,
    series::{
        CatalogCheckRequest, CatalogCheckResult, ResolveRunRequest, ResolveRunResult, RunSelector,
        SeriesRequest, SeriesResultBody, SqlSessionRequest, SqlSessionResult,
    },
    state::FailKind,
};
use omo_worker::server::{BackendError, StageReporter, WorkerBackend, run_stdio};
use serde_json::Value;
use std::{collections::HashMap, io::Write, path::PathBuf, time::Instant};

#[derive(Clone)]
struct QueryInfo {
    sql: String,
    rows: u64,
}

#[derive(Default)]
struct EngineBackend {
    bootstrap_config: Option<BootstrapConfig>,
    conn: Option<BootstrappedConn>,
    result_root: PathBuf,
    generation: u64,
    query_by_result: HashMap<String, QueryInfo>,
    series_cancel: std::sync::Arc<omo_engine::series::SeriesCancel>,
    stage_reporter: Option<StageReporter>,
    fields: omo_engine::map::FieldCache,
    pmtiles: HashMap<String, CachedPmtiles>,
}

/// An open `.pmtiles` archive keyed by path; reopened when the file changes on disk.
struct CachedPmtiles {
    len: u64,
    modified: Option<std::time::SystemTime>,
    layer: std::sync::Arc<omo_engine::map::pmtiles_layer::PmtilesLayer>,
}

impl EngineBackend {
    /// Open (or reuse) the archive at `path`.
    fn open_pmtiles(
        &mut self,
        path: &str,
    ) -> Result<std::sync::Arc<omo_engine::map::pmtiles_layer::PmtilesLayer>, String> {
        let meta = std::fs::metadata(path).map_err(|e| format!("cannot read `{path}`: {e}"))?;
        let (len, modified) = (meta.len(), meta.modified().ok());
        if let Some(cached) = self.pmtiles.get(path)
            && cached.len == len
            && cached.modified == modified
        {
            return Ok(std::sync::Arc::clone(&cached.layer));
        }
        let layer = std::sync::Arc::new(
            omo_engine::map::pmtiles_layer::PmtilesLayer::open(std::path::Path::new(path))
                .map_err(|e| e.to_string())?,
        );
        if self.pmtiles.len() >= 8 {
            self.pmtiles.clear();
        }
        self.pmtiles.insert(
            path.to_owned(),
            CachedPmtiles {
                len,
                modified,
                layer: std::sync::Arc::clone(&layer),
            },
        );
        Ok(layer)
    }

    /// `export` with a `map_export` snapshot: render the composite PNG synchronously from the
    /// worker-resident field and mounted archives, then publish it with its `/3` sidecar.
    fn export_map_png(&mut self, body: &Value) -> Result<Value, BackendError> {
        use omo_engine::map::compose_export::{self, ComposeError};
        use omo_protocol::map::{ExportLayerKind, MapExportRequest};
        let report = self
            .conn
            .as_ref()
            .ok_or("engine is not bootstrapped")?
            .report()
            .clone();
        let target = PathBuf::from(
            body.get("target")
                .and_then(Value::as_str)
                .ok_or("export target is required")?,
        );
        if !target.is_absolute() {
            return Err("export target must be absolute".into());
        }
        if body.get("format").and_then(Value::as_str) != Some("png") {
            return Err("map export format must be png".into());
        }
        let confirmed = body
            .get("overwrite_confirmed")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let req: MapExportRequest = serde_json::from_value(
            body.get("map_export")
                .cloned()
                .ok_or("map_export snapshot is required")?,
        )
        .map_err(|e| e.to_string())?;
        // Fail before any file is staged when the target would need confirmation.
        if (target.exists() || omo_engine::export::sidecar_path(&target).exists()) && !confirmed {
            return Err(BackendError::new(
                omo_engine::export::ExportError::OverwriteConfirmationRequired.to_string(),
            ));
        }
        self.report_stage("reading");
        let mut archives = HashMap::new();
        for layer in &req.layers {
            if layer.kind != ExportLayerKind::LocalPmtiles {
                continue;
            }
            let path = layer.path.as_deref().unwrap_or_default();
            match self.open_pmtiles(path) {
                Ok(archive) => {
                    archives.insert(layer.id.clone(), archive);
                }
                Err(message) => {
                    return Err(BackendError::typed(
                        FailKind::InvalidLayer,
                        format!("layer `{}`: {message}", layer.id),
                    ));
                }
            }
        }
        self.report_stage("rendering");
        let cancel = std::sync::Arc::clone(&self.series_cancel);
        let field_id = req.field_id.clone();
        let field = match field_id.as_deref() {
            Some(id) => self.fields.get(id),
            None => None,
        };
        let composed = compose_export::compose(&req, field, &archives, &mut || {
            cancel.check().map_err(|_| ComposeError::Cancelled)
        })
        .map_err(|e| match e {
            ComposeError::Cancelled => BackendError::new("export cancelled"),
            ComposeError::Layer { .. } => {
                BackendError::typed(FailKind::InvalidLayer, e.to_string())
            }
            other => BackendError::new(other.to_string()),
        })?;
        let file_name = target
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or("export target has no file name")?
            .to_owned();
        let engine = serde_json::json!({
            "duckdb_version": report.duckdb_version,
            "duckomo_version": report.duckomo_version,
            "channel": report.channel.as_str(),
        });
        let exported_at = omo_protocol::utc::now_utc_rfc3339();
        let stage_reporter = self.stage_reporter.clone();
        let sha256 = omo_engine::export::export_map_png(
            &target,
            &composed.png,
            |sha| {
                compose_export::build_sidecar(
                    &req,
                    &composed,
                    sha,
                    &file_name,
                    engine,
                    &exported_at,
                )
            },
            confirmed,
            |stage| {
                if let Some(reporter) = &stage_reporter {
                    reporter.set_stage(stage);
                }
            },
            |_| Ok(()),
        )
        .map_err(|e| BackendError::new(e.to_string()))?;
        Ok(serde_json::json!({
            "request_id": body.get("request_id"),
            "target": target,
            "sidecar": omo_engine::export::sidecar_path(&target),
            "sidecar_path": omo_engine::export::sidecar_path(&target),
            "data_sha256": sha256,
            "sha256": sha256,
            "bytes": composed.png.len(),
        }))
    }

    fn report_stage(&self, stage: &'static str) {
        if let Some(reporter) = &self.stage_reporter {
            reporter.set_stage(stage);
        }
    }
}

impl WorkerBackend for EngineBackend {
    fn set_generation(&mut self, generation: u64) {
        self.generation = generation;
    }

    fn set_stage_reporter(&mut self, reporter: Option<StageReporter>) {
        self.stage_reporter = reporter;
    }

    fn bootstrap(&mut self, config: &Value, diagnostics: &mut dyn Write) -> Result<Value, String> {
        let config: BootstrapConfig =
            serde_json::from_value(config.clone()).map_err(|e| e.to_string())?;
        self.result_root = config.data_dir.join("results");
        self.bootstrap_config = Some(config.clone());
        let mut conn = open_bootstrapped(config).map_err(|e| e.to_string())?;
        conn.set_generation(self.generation);
        let report = serde_json::to_value(conn.report()).map_err(|e| e.to_string())?;
        writeln!(
            diagnostics,
            "engine bootstrap completed for generation {}",
            self.generation
        )
        .map_err(|e| e.to_string())?;
        self.conn = Some(conn);
        Ok(report)
    }

    fn probe_task(&mut self) -> Result<(), String> {
        self.conn
            .as_mut()
            .ok_or("engine is not bootstrapped")?
            .probe_before_task()
            .map_err(|e| e.to_string())
    }

    fn cancellation(&self) -> Option<Box<dyn Fn() + Send + Sync>> {
        let handle = self.conn.as_ref()?.interrupt_handle().ok()?;
        // Called once per accepted request, before its task starts: forget the last request.
        self.series_cancel.reset();
        let series_cancel = std::sync::Arc::clone(&self.series_cancel);
        Some(Box::new(move || {
            series_cancel.cancel();
            handle.interrupt();
        }))
    }

    fn query(&mut self, body: &Value) -> Result<Value, BackendError> {
        let started = Instant::now();
        self.report_stage("reading");
        let conn = self.conn.as_ref().ok_or("engine is not bootstrapped")?;
        let sql = body
            .get("sql")
            .and_then(Value::as_str)
            .ok_or("query sql is required")?;
        let id = body
            .get("result_id")
            .and_then(Value::as_str)
            .ok_or("result id is required")?;
        let limits: ResultLimits = serde_json::from_value(
            body.get("limits")
                .cloned()
                .ok_or("query limits are required")?,
        )
        .map_err(|e| e.to_string())?;
        let purpose = match body.get("purpose").and_then(Value::as_str) {
            Some("preview") => QueryPurpose::Preview,
            Some("job") => QueryPurpose::Job,
            _ => return Err("query purpose must be preview or job".into()),
        };
        let result = execute_query(conn, sql, purpose, limits, &self.result_root, id)
            .map_err(|e| e.to_string())?;
        let rows = result.rows;
        self.report_stage("writing");
        let plot = if purpose == QueryPurpose::Preview {
            Some(create_plot_artifact(&result, &self.result_root).map_err(|e| e.to_string())?)
        } else {
            None
        };
        let duration_ms = started.elapsed().as_millis() as u64;
        let scan_bytes = omo_engine::series::latest_scan_measurement(conn)
            .ok()
            .and_then(|m| {
                m.raw_metrics
                    .pointer("/legacy_v2/bytes_fetched")
                    .and_then(Value::as_u64)
                    .or(m.physical_read_bytes)
                    .or(m.data_bytes)
            })
            .unwrap_or_else(|| {
                std::fs::metadata(&result.path)
                    .map(|meta| meta.len())
                    .unwrap_or(0)
            });
        let result_ref = serde_json::to_value(result).map_err(|e| e.to_string())?;
        let plot_ref = plot
            .as_ref()
            .and_then(|artifact| artifact.reference.as_ref())
            .map(serde_json::to_value)
            .transpose()
            .map_err(|e| e.to_string())?;
        self.query_by_result.insert(
            id.to_owned(),
            QueryInfo {
                sql: sql.to_owned(),
                rows,
            },
        );
        Ok(serde_json::json!({
            "result_ref": result_ref,
            "plot_ref": plot_ref,
            "plot_source_rows": plot.map(|artifact| artifact.source_rows),
            "scan_metrics": {
                "bytes_read": scan_bytes,
                "duration_ms": duration_ms,
                "rows": rows,
            },
        }))
    }

    fn catalog_check(&mut self, body: &Value) -> Result<Value, BackendError> {
        let conn = self.conn.as_ref().ok_or("engine is not bootstrapped")?;
        let req: CatalogCheckRequest =
            serde_json::from_value(body.clone()).map_err(|e| e.to_string())?;
        let readiness = check_readiness(conn, &req.entry);
        let result = CatalogCheckResult {
            entry_id: req.entry.id,
            readiness,
        };
        serde_json::to_value(result).map_err(|e| e.to_string().into())
    }

    fn resolve_run(&mut self, body: &Value) -> Result<Value, BackendError> {
        let conn = self.conn.as_ref().ok_or("engine is not bootstrapped")?;
        if !conn.capabilities().remote {
            return Err(BackendError::typed(
                FailKind::Capability("capability `remote` is unavailable".into()),
                "capability `remote` is unavailable",
            ));
        }
        let req: ResolveRunRequest =
            serde_json::from_value(body.clone()).map_err(|e| e.to_string())?;
        let source = ConnMetaSource {
            conn,
            entry: &req.entry,
        };
        let selector = match &req.run {
            RunSelector::LatestComplete(_) => RunRef::LatestComplete,
            RunSelector::Explicit { reference_time_utc } => {
                RunRef::Explicit(reference_time_utc.clone())
            }
        };
        let run = resolve(&req.entry, &source, selector).map_err(map_run_error)?;
        serde_json::to_value(ResolveRunResult { run }).map_err(|e| e.to_string().into())
    }

    /// `map_load_field`: resolve the run and valid time, read the regional field, and keep it in
    /// the bounded per-generation cache for tile rendering and inspection.
    fn map_load_field(&mut self, body: &Value) -> Result<Value, BackendError> {
        use omo_engine::map::fetch::{self, LoadFieldParams};
        use omo_protocol::map::{LoadFieldRequest, LoadFieldResult, MapRunSelector};
        self.report_stage("planning");
        let config = self
            .bootstrap_config
            .clone()
            .ok_or("bootstrap config is missing")?;
        let req: LoadFieldRequest =
            serde_json::from_value(body.clone()).map_err(|e| e.to_string())?;
        let run = {
            let conn = self.conn.as_ref().ok_or("engine is not bootstrapped")?;
            if !conn.capabilities().remote {
                return Err(BackendError::typed(
                    FailKind::Capability("capability `remote` is unavailable".into()),
                    "capability `remote` is unavailable",
                ));
            }
            let source = ConnMetaSource {
                conn,
                entry: &req.catalog_entry,
            };
            let selector = match &req.run {
                MapRunSelector::LatestComplete => RunRef::LatestComplete,
                MapRunSelector::Explicit(utc) => RunRef::Explicit(utc.clone()),
            };
            resolve(&req.catalog_entry, &source, selector).map_err(map_run_error)?
        };
        let valid_time = match &req.valid_time_utc {
            None => run.valid_times.first().cloned(),
            Some(requested) => run
                .valid_times
                .iter()
                .find(|published| published.as_str() == requested.as_str())
                .cloned(),
        }
        .ok_or_else(|| {
            BackendError::typed(
                FailKind::ValidTimeUnavailable,
                format!(
                    "valid time {} is not published by run {}",
                    req.valid_time_utc.as_deref().unwrap_or("<first>"),
                    run.reference_time_utc
                ),
            )
        })?;
        self.series_cancel.check().map_err(map_series_error)?;
        let stage = self.stage_reporter.clone();
        let mut progress = |name: &'static str, _detail: Option<&str>| {
            if let Some(reporter) = &stage {
                reporter.set_stage(name);
            }
        };
        let loaded = fetch::load_field(
            &LoadFieldParams {
                config: &config,
                entry: &req.catalog_entry,
                field_id: &req.field_id,
                reference_time_utc: &run.reference_time_utc,
                valid_time_utc: &valid_time,
                bbox: &req.bbox,
                force_data_run: req.force_data_run,
                readers: std::env::var("OMO_MAP_READERS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(fetch::MAX_READERS),
                cancel: Some(std::sync::Arc::clone(&self.series_cancel)),
            },
            &mut progress,
        )
        .map_err(map_fetch_error)?;
        let field = loaded.field;
        let result = LoadFieldResult {
            field_id: field.field_id.clone(),
            run,
            valid_time_utc: field.valid_time_utc.clone(),
            source_layout: field.source_layout,
            fallback_notice: loaded.fallback_notice,
            source_object: field.source_object.clone(),
            object_version: field.object_version.clone(),
            stats: field.stats.clone(),
            coverage_bbox: field.coverage.clone(),
            scan_metrics: loaded.metrics.to_json(),
            readout_path: None,
        };
        self.fields.insert(field);
        serde_json::to_value(result).map_err(|e| e.to_string().into())
    }

    /// `map_render_tiles`: rasterize the visible tile batch from the cached numeric field and
    /// write each non-empty tile as generation-scoped straight RGBA8 (`256 x 256`).
    fn map_render_tiles(&mut self, body: &Value) -> Result<Value, BackendError> {
        use omo_engine::map::raster::{ColorLut, render_tile};
        use omo_protocol::map::{
            LayerError, RenderLayerKind, RenderTilesRequest, RenderTilesResult, RenderedTile,
        };
        const MAX_TILES_PER_BATCH: usize = 128;
        self.report_stage("reading");
        let req: RenderTilesRequest =
            serde_json::from_value(body.clone()).map_err(|e| e.to_string())?;
        if req.tiles.len() > MAX_TILES_PER_BATCH {
            return Err(format!(
                "tile batch of {} exceeds {MAX_TILES_PER_BATCH}",
                req.tiles.len()
            )
            .into());
        }
        if let Some(bad) = req.tiles.iter().find(|tile| !tile.is_valid()) {
            return Err(format!("invalid tile coordinate z{}/{}/{}", bad.z, bad.x, bad.y).into());
        }
        let lut = ColorLut::new(&req.color_scale.palette).map_err(|e| e.to_string())?;
        let dir = self.result_root.join(format!("g{}", self.generation));
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let mut tiles = Vec::new();
        let mut layer_errors = Vec::new();
        for layer in &req.layers {
            match layer.kind {
                RenderLayerKind::ForecastField => {
                    let Some(field_id) = req.field_id.as_deref() else {
                        layer_errors.push(LayerError {
                            layer_id: layer.layer_id.clone(),
                            message: "no field is loaded".into(),
                        });
                        continue;
                    };
                    let Some(field) = self.fields.get(field_id) else {
                        layer_errors.push(LayerError {
                            layer_id: layer.layer_id.clone(),
                            message: format!("field `{field_id}` is not loaded"),
                        });
                        continue;
                    };
                    let sampler = field.values.sampler();
                    for tile in &req.tiles {
                        self.series_cancel.check().map_err(map_series_error)?;
                        let (pixels, empty) = render_tile(
                            &sampler,
                            &lut,
                            req.color_scale.min,
                            req.color_scale.max,
                            tile.z,
                            tile.x,
                            tile.y,
                        );
                        let rgba_path = if empty {
                            String::new()
                        } else {
                            let path = dir.join(format!(
                                "tile_{}_{}_{}_{}_{}.rgba",
                                req.revision,
                                sanitize_layer_id(&layer.layer_id),
                                tile.z,
                                tile.x,
                                tile.y
                            ));
                            write_atomic(&path, &pixels).map_err(|e| e.to_string())?;
                            path.to_string_lossy().into_owned()
                        };
                        tiles.push(RenderedTile {
                            layer_id: layer.layer_id.clone(),
                            z: tile.z,
                            x: tile.x,
                            y: tile.y,
                            width: omo_protocol::map::MAP_TILE_SIZE,
                            height: omo_protocol::map::MAP_TILE_SIZE,
                            rgba_path,
                            empty,
                        });
                    }
                }
                RenderLayerKind::LocalPmtiles => {
                    let archive = layer
                        .path
                        .as_deref()
                        .ok_or_else(|| "local PMTiles layer has no path".to_owned())
                        .and_then(|path| self.open_pmtiles(path));
                    let archive = match archive {
                        Ok(archive) => archive,
                        Err(message) => {
                            layer_errors.push(LayerError {
                                layer_id: layer.layer_id.clone(),
                                message,
                            });
                            continue;
                        }
                    };
                    let mut failed = false;
                    for tile in &req.tiles {
                        self.series_cancel.check().map_err(map_series_error)?;
                        let rendered = if failed {
                            Ok(None)
                        } else {
                            archive.tile_rgba(tile.z, tile.x, tile.y)
                        };
                        let pixels = match rendered {
                            Ok(pixels) => pixels,
                            Err(error) => {
                                // One broken tile marks the layer; healthy layers still render.
                                failed = true;
                                layer_errors.push(LayerError {
                                    layer_id: layer.layer_id.clone(),
                                    message: error.to_string(),
                                });
                                None
                            }
                        };
                        let pixels =
                            pixels.filter(|px| px.as_chunks::<4>().0.iter().any(|p| p[3] != 0));
                        let rgba_path = match &pixels {
                            Some(pixels) => {
                                let path = dir.join(format!(
                                    "tile_{}_{}_{}_{}_{}.rgba",
                                    req.revision,
                                    sanitize_layer_id(&layer.layer_id),
                                    tile.z,
                                    tile.x,
                                    tile.y
                                ));
                                write_atomic(&path, pixels).map_err(|e| e.to_string())?;
                                path.to_string_lossy().into_owned()
                            }
                            None => String::new(),
                        };
                        tiles.push(RenderedTile {
                            layer_id: layer.layer_id.clone(),
                            z: tile.z,
                            x: tile.x,
                            y: tile.y,
                            width: omo_protocol::map::MAP_TILE_SIZE,
                            height: omo_protocol::map::MAP_TILE_SIZE,
                            rgba_path,
                            empty: pixels.is_none(),
                        });
                    }
                }
            }
        }
        serde_json::to_value(RenderTilesResult {
            result_id: req.result_id,
            revision: req.revision,
            tiles,
            layer_errors,
        })
        .map_err(|e| e.to_string().into())
    }

    /// `map_layer_probe`: validate a local `.pmtiles` archive and describe it.
    fn map_layer_probe(&mut self, body: &Value) -> Result<Value, BackendError> {
        use omo_protocol::map::{LayerProbeRequest, LayerProbeResult, RenderLayerKind};
        self.report_stage("reading");
        let req: LayerProbeRequest =
            serde_json::from_value(body.clone()).map_err(|e| e.to_string())?;
        let invalid = |message: String| BackendError::typed(FailKind::InvalidLayer, message);
        if req.kind != RenderLayerKind::LocalPmtiles {
            return Err(invalid(format!(
                "layer `{}` is not a local PMTiles layer",
                req.layer_id
            )));
        }
        let archive =
            omo_engine::map::pmtiles_layer::PmtilesLayer::open(std::path::Path::new(&req.path))
                .map_err(|e| invalid(format!("{}: {e}", req.path)))?;
        archive
            .smoke_check()
            .map_err(|e| invalid(format!("{}: {e}", req.path)))?;
        let info = archive.info(true);
        serde_json::to_value(LayerProbeResult {
            layer_id: req.layer_id,
            info,
        })
        .map_err(|e| e.to_string().into())
    }

    /// `map_inspect`: exact nearest-cell numeric readout from the cached field.
    fn map_inspect(&mut self, body: &Value) -> Result<Value, BackendError> {
        use omo_protocol::map::{ForecastSampleState, InspectRequest, InspectResult};
        self.report_stage("reading");
        let req: InspectRequest =
            serde_json::from_value(body.clone()).map_err(|e| e.to_string())?;
        let sample = match self.fields.peek(&req.field_id) {
            Some(field) => omo_engine::map::inspect_field(
                field,
                req.latitude,
                req.longitude,
                req.max_distance_m,
            ),
            None => ForecastSampleState::NotLoaded,
        };
        serde_json::to_value(InspectResult {
            latitude: req.latitude,
            longitude: req.longitude,
            sample,
        })
        .map_err(|e| e.to_string().into())
    }

    fn series(&mut self, body: &Value) -> Result<Value, BackendError> {
        let started = Instant::now();
        self.report_stage("planning");
        let config = self
            .bootstrap_config
            .clone()
            .ok_or("bootstrap config is missing")?;
        let req: SeriesRequest = serde_json::from_value(body.clone()).map_err(|e| e.to_string())?;
        let stations = read_canonical_stations(std::path::Path::new(&req.stations_ref))
            .map_err(|e| e.to_string())?;

        let (forecast_run, eff_window, time_index, version_before) = {
            let conn = self.conn.as_ref().ok_or("engine is not bootstrapped")?;
            if !conn.capabilities().remote {
                return Err(BackendError::typed(
                    FailKind::Capability("capability `remote` is unavailable".into()),
                    "capability `remote` is unavailable",
                ));
            }
            let source = ConnMetaSource {
                conn,
                entry: &req.catalog_entry,
            };
            let selector = match &req.run {
                RunSelector::LatestComplete(_) => RunRef::LatestComplete,
                RunSelector::Explicit { reference_time_utc } => {
                    RunRef::Explicit(reference_time_utc.clone())
                }
            };
            let forecast_run =
                resolve(&req.catalog_entry, &source, selector).map_err(map_run_error)?;
            let eff_window = effective_window(req.window, &forecast_run);
            let time_index = time_index_table(&forecast_run, &eff_window).map_err(map_run_error)?;
            let version_before =
                object_version_token(conn, &forecast_run.object_url).map_err(map_series_error)?;
            (forecast_run, eff_window, time_index, version_before)
        };

        let series_stations = stations
            .iter()
            .map(|s| plan_g::SeriesStation {
                station_id: s.station_id.clone(),
                latitude: s.latitude,
                longitude: s.longitude,
            })
            .collect::<Vec<_>>();

        self.report_stage("reading");
        let (plan_name, assembled, scan_stats) = match req.catalog_entry.grid.kind.as_str() {
            "reduced_gaussian_o1280" => {
                let mut reader = ParallelDuckDbPointReader {
                    config: &config,
                    domain: &req.catalog_entry.grid.duckomo_domain,
                    max_workers: 20,
                    cancel: Some(std::sync::Arc::clone(&self.series_cancel)),
                    scan_stats: plan_g::PlanGScanStats::default(),
                };
                let assembled = plan_g::assemble(
                    &series_stations,
                    &req.catalog_entry.variable.name,
                    &forecast_run.object_url,
                    &version_before,
                    f64::from(req.max_distance_m),
                    &time_index,
                    &forecast_run.object_url,
                    &mut reader,
                )
                .map_err(map_series_error)?;
                ("G", assembled, reader.scan_stats)
            }
            "regular_latlon" => {
                let conn = self.conn.as_mut().ok_or("engine is not bootstrapped")?;
                let (assembled, stats) = plan_b::run_remote_domain(
                    conn,
                    &series_stations,
                    &req.catalog_entry.variable.name,
                    &req.catalog_entry.grid.duckomo_domain,
                    &forecast_run.object_url,
                    &version_before,
                    f64::from(req.max_distance_m),
                    &time_index,
                )
                .map_err(map_series_error)?;
                ("B", assembled, stats)
            }
            other => {
                return Err(BackendError::typed(
                    FailKind::GridMismatch,
                    format!("unsupported catalog grid kind `{other}`"),
                ));
            }
        };

        let conn = self.conn.as_ref().ok_or("engine is not bootstrapped")?;
        let version_after =
            object_version_token(conn, &forecast_run.object_url).map_err(map_series_error)?;
        ensure_stable_version(&version_before, &version_after).map_err(map_series_error)?;
        self.series_cancel.check().map_err(map_series_error)?;

        self.report_stage("writing");
        let result_ref = write_series_arrow(
            &assembled.rows,
            self.generation,
            req.limits,
            &self.result_root,
            &req.result_id,
        )
        .map_err(map_series_error)?;
        let matches_ref = write_matches_arrow(
            &assembled.matches,
            self.generation,
            req.limits,
            &self.result_root,
            &req.result_id,
        )
        .map_err(map_series_error)?;
        let parquet_path = write_series_parquet(
            conn,
            &assembled.rows,
            self.generation,
            &self.result_root,
            &req.result_id,
        )
        .map_err(map_series_error)?;
        let parquet_lit = quote_sql_literal(&parquet_path.to_string_lossy());
        self.query_by_result.insert(
            req.result_id.clone(),
            QueryInfo {
                sql: format!("SELECT * FROM read_parquet({parquet_lit})"),
                rows: assembled.rows.len() as u64,
            },
        );
        let plot_artifact =
            create_plot_artifact(&result_ref, &self.result_root).map_err(|e| e.to_string())?;
        let mut forecast_run = forecast_run;
        if forecast_run.meta_etag.is_none() {
            forecast_run.meta_etag = Some(version_after.clone());
        }
        let duration_ms = started.elapsed().as_millis() as u64;
        let bytes_read = if scan_stats.bytes_fetched > 0 {
            scan_stats.bytes_fetched
        } else {
            scan_stats.data_bytes
        };

        let result_body = SeriesResultBody {
            result_ref: serde_json::to_value(&result_ref).map_err(|e| e.to_string())?,
            parquet_path: parquet_path.to_string_lossy().into_owned(),
            matches_ref: serde_json::to_value(&matches_ref).map_err(|e| e.to_string())?,
            plot_ref: plot_artifact
                .reference
                .as_ref()
                .map(serde_json::to_value)
                .transpose()
                .map_err(|e| e.to_string())?,
            run: forecast_run,
            effective_window: eff_window,
            plan: plan_name.to_owned(),
            scan_metrics: serde_json::json!({
                "version_token": version_after,
                "bytes_read": bytes_read,
                "bytes_fetched": scan_stats.bytes_fetched,
                "data_bytes": scan_stats.data_bytes,
                "read_requests": scan_stats.read_requests,
                "duration_ms": duration_ms,
                "matched_stations": assembled.matches.iter().filter(|m| matches!(m.status, plan_g::MatchStatus::Matched)).count(),
                "total_stations": assembled.matches.len(),
                "rows": assembled.rows.len(),
            }),
        };
        serde_json::to_value(result_body).map_err(|e| e.to_string().into())
    }

    fn sql_session(&mut self, body: &Value) -> Result<Value, BackendError> {
        let conn = self.conn.as_ref().ok_or("engine is not bootstrapped")?;
        let req: SqlSessionRequest =
            serde_json::from_value(body.clone()).map_err(|e| e.to_string())?;
        let views = omo_engine::query::setup_sql_session(
            conn,
            &req.stations_ref,
            req.series_parquet.as_deref(),
        )
        .map_err(|e| e.to_string())?;
        serde_json::to_value(SqlSessionResult { views }).map_err(|e| e.to_string().into())
    }

    fn series_plan(&mut self, body: &Value) -> Result<Value, BackendError> {
        let plan = body
            .get("plan")
            .and_then(Value::as_str)
            .ok_or("series_plan plan is required")?;
        if plan != "A" && plan != "B" {
            return Err(format!("series_plan plan must be \"A\" or \"B\", got `{plan}`").into());
        }
        let stations_path = body
            .get("stations_ref")
            .and_then(|value| {
                value
                    .as_str()
                    .or_else(|| value.get("path").and_then(Value::as_str))
            })
            .map(PathBuf::from)
            .ok_or("series_plan stations_ref path is required")?;
        let variable = body
            .get("variable")
            .and_then(Value::as_str)
            .ok_or("series_plan variable is required")?;
        if variable != SERIES_VARIABLE {
            return Err(format!(
                "unsupported series_plan variable `{variable}`; expected `{SERIES_VARIABLE}`"
            )
            .into());
        }
        let (window_start_seconds, window_end_seconds) = parse_window_utc(
            body.get("window_utc")
                .ok_or("series_plan window_utc is required")?,
        )?;
        let catalog_entry = body
            .get("catalog_entry")
            .ok_or("series_plan catalog_entry is required")?;
        let series_prefix = catalog_entry
            .as_str()
            .or_else(|| {
                catalog_entry
                    .get("series_prefix")
                    .or_else(|| catalog_entry.get("prefix"))
                    .or_else(|| catalog_entry.get("path"))
                    .and_then(Value::as_str)
            })
            .map(PathBuf::from)
            .ok_or("series_plan catalog_entry must provide series_prefix/path")?;
        let object_path = catalog_entry
            .get("object_path")
            .and_then(Value::as_str)
            .map(PathBuf::from)
            .unwrap_or_else(|| series_prefix.join("chunk_0000.om"));
        let id = body
            .get("result_id")
            .and_then(Value::as_str)
            .ok_or("result id is required")?;
        let limits: ResultLimits = match body.get("limits") {
            Some(value) => serde_json::from_value(value.clone()).map_err(|e| e.to_string())?,
            None => job_limits(),
        };

        let stations = read_stations(&stations_path).map_err(|e| e.to_string())?;
        let conn = self.conn.as_mut().ok_or("engine is not bootstrapped")?;
        let run = match plan {
            "A" => plan_a::run(conn, &stations, &series_prefix, &object_path),
            "B" => plan_b::run(
                conn,
                &stations,
                &series_prefix,
                window_start_seconds,
                window_end_seconds,
            ),
            _ => unreachable!(),
        }
        .map_err(|e| e.to_string())?;
        let result = write_series_arrow(&run.rows, self.generation, limits, &self.result_root, id)
            .map_err(|e| e.to_string())?;
        let result_ref = serde_json::to_value(result).map_err(|e| e.to_string())?;
        let scan_metrics = serde_json::to_value(run.scan).map_err(|e| e.to_string())?;
        Ok(serde_json::json!({
            "result_ref": result_ref,
            "scan_metrics": scan_metrics,
            "plan_details": run.plan_details,
        }))
    }

    fn export(&mut self, body: &Value) -> Result<Value, BackendError> {
        if body.get("map_export").is_some() {
            return self.export_map_png(body);
        }
        let conn = self.conn.as_ref().ok_or("engine is not bootstrapped")?;
        let (sql, known_rows) = if let Some(sql) = body.get("sql").and_then(Value::as_str) {
            (
                sql.to_owned(),
                body.get("row_count").and_then(Value::as_u64),
            )
        } else {
            let id = body
                .get("result_id")
                .and_then(Value::as_str)
                .ok_or("export sql or result_id is required")?;
            let query = self.query_by_result.get(id).ok_or("unknown result_id")?;
            (query.sql.clone(), Some(query.rows))
        };
        let target = PathBuf::from(
            body.get("target")
                .and_then(Value::as_str)
                .ok_or("export target is required")?,
        );
        if !target.is_absolute() {
            return Err("export target must be absolute".into());
        }
        let format = body
            .get("format")
            .and_then(Value::as_str)
            .ok_or("export format is required")?;
        let snapshot = body
            .get("job_snapshot")
            .cloned()
            .ok_or("export job snapshot is required")?;
        let confirmed = body
            .get("overwrite_confirmed")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let stage_reporter = self.stage_reporter.clone();
        let hash = omo_engine::export::export_sql_with_progress(
            conn,
            &sql,
            &target,
            format,
            snapshot,
            confirmed,
            known_rows,
            |stage| {
                if let Some(reporter) = &stage_reporter {
                    reporter.set_stage(stage);
                }
            },
        )
        .map_err(|e| e.to_string())?;
        Ok(
            serde_json::json!({"request_id":body.get("request_id"),"target":target,"sidecar":omo_engine::export::sidecar_path(&target),"data_sha256":hash}),
        )
    }

    fn interrupt(&mut self, _request_id: u64) -> Result<(), String> {
        Err("no active query".into())
    }
    fn ensure_spatial(&mut self) -> Result<Value, String> {
        let conn = self.conn.as_mut().ok_or("engine is not bootstrapped")?;
        conn.ensure_spatial().map_err(|e| e.to_string())?;
        serde_json::to_value(conn.report()).map_err(|e| e.to_string())
    }
}

fn parse_window_utc(value: &Value) -> Result<(i64, i64), String> {
    let pair = value
        .as_array()
        .filter(|items| items.len() == 2)
        .ok_or("series_plan window_utc must be a 2-element [start, end] array")?;
    let start = parse_utc_instant(&pair[0])?;
    let end = parse_utc_instant(&pair[1])?;
    if end < start {
        return Err("series_plan window_utc end must be >= start".into());
    }
    Ok((start, end))
}

fn parse_utc_instant(value: &Value) -> Result<i64, String> {
    if let Some(secs) = value.as_i64() {
        return Ok(secs);
    }
    let text = value
        .as_str()
        .ok_or("window_utc entries must be Unix seconds or RFC 3339 UTC strings")?;
    parse_rfc3339_utc_seconds(text)
}

fn parse_rfc3339_utc_seconds(text: &str) -> Result<i64, String> {
    if !text.ends_with('Z') {
        return Err(format!("window_utc timestamp must end with Z: {text}"));
    }
    omo_protocol::utc::parse_utc_rfc3339(text)
        .ok_or_else(|| format!("invalid RFC 3339 UTC timestamp: {text}"))
}

fn map_run_error(error: RunError) -> BackendError {
    match error {
        RunError::NoCompleteRun => BackendError::typed(FailKind::NoCompleteRun, error.to_string()),
        RunError::RunIncomplete { .. } => {
            BackendError::typed(FailKind::RunIncomplete, error.to_string())
        }
        RunError::VariableMissing { .. } => {
            BackendError::typed(FailKind::VariableMissing, error.to_string())
        }
        RunError::CapabilityUnavailable(ref msg) => {
            BackendError::typed(FailKind::Capability(msg.clone()), error.to_string())
        }
        other => BackendError::new(other.to_string()),
    }
}

fn map_series_error(error: SeriesError) -> BackendError {
    match error {
        SeriesError::GridMismatch(ref msg) => {
            BackendError::typed(FailKind::GridMismatch, msg.clone())
        }
        SeriesError::SourceChanged(ref msg) => {
            BackendError::typed(FailKind::SourceChanged, msg.clone())
        }
        other => BackendError::new(other.to_string()),
    }
}

/// Layer ids become part of a file name: keep `[A-Za-z0-9_-]`, replace everything else.
fn sanitize_layer_id(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Write `bytes` to `path` via a sibling temp file and rename so readers never see a partial tile.
fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("rgba.tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

fn map_fetch_error(error: omo_engine::map::fetch::FetchError) -> BackendError {
    use omo_engine::map::fetch::FetchError;
    match error {
        FetchError::ValidTimeUnavailable(ref msg) => {
            BackendError::typed(FailKind::ValidTimeUnavailable, msg.clone())
        }
        FetchError::SourceChanged(ref msg) => {
            BackendError::typed(FailKind::SourceChanged, msg.clone())
        }
        FetchError::GridMismatch(ref msg) => {
            BackendError::typed(FailKind::GridMismatch, msg.clone())
        }
        other => BackendError::new(other.to_string()),
    }
}

fn main() {
    match run_stdio(EngineBackend::default()) {
        Ok(
            omo_worker::server::WorkerExit::InputClosed | omo_worker::server::WorkerExit::Shutdown,
        ) => {}
        Ok(omo_worker::server::WorkerExit::BootstrapFailed) => std::process::exit(78),
        Err(error) => {
            eprintln!("worker protocol error: {error}");
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::parse_window_utc;

    #[test]
    fn parse_window_utc_accepts_unix_seconds_and_rfc3339() {
        assert_eq!(parse_window_utc(&json!([0, 7_200])).unwrap(), (0, 7_200));
        assert_eq!(
            parse_window_utc(&json!(["1970-01-01T00:00:00Z", "1970-01-01T02:00:00Z"])).unwrap(),
            (0, 7_200)
        );
        assert_eq!(
            parse_window_utc(&json!(["2024-02-29T23:59:59Z", "2024-03-01T00:00:00Z"])).unwrap(),
            (1_709_251_199, 1_709_251_200)
        );
        assert!(parse_window_utc(&json!([7_200, 0])).is_err());
        assert!(
            parse_window_utc(&json!([
                "2024-02-29T00:00:00+08:00",
                "2024-03-01T00:00:00Z"
            ]))
            .is_err()
        );
    }
}
