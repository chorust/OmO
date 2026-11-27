use std::{
    fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

use clap::{Parser, ValueEnum};
use omo_engine::{
    bootstrap::BootstrapReport,
    catalog::{self, Catalog, ConnMetaSource, RunRef},
    config::{BootstrapConfig, Channel, Platform},
    conn::{BootstrappedConn, open_bootstrapped},
    grid::reduced_gaussian::O1280,
    lock::ArtifactLock,
    series::{
        latest_scan_measurement, plan_a, plan_b,
        plan_g::{self, PointReader},
        read_stations,
        reference::{ReferenceComparison, compare_reference},
        schema::{STATION_SERIES_COLUMNS, write_csv},
    },
};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tempfile::Builder;

const DUCKOMO_VERSION: &str = "v0.4.0";
const DUCKDB_VERSION: &str = "v2.0.0";
const DEFAULT_STATIONS: &str = "fixtures/stations/m0.csv";
const DEFAULT_REFERENCE: &str = "fixtures/reference/station_series_reference.csv";
const DEFAULT_OFFICIAL_VALUES: &str =
    "fixtures/reference/duckomo-regions-v0.4.0/regions.reference.csv";
const DEFAULT_MANIFEST: &str = "fixtures/reference/station_series_manifest.json";
const DEFAULT_SERIES_PREFIX: &str = "fixtures/reference/duckomo-regions-v0.4.0/series/value";

#[derive(Debug, Parser)]
#[command(
    name = "omo-series-spike",
    about = "Compare a station-series plan against the retained independent oracle"
)]
struct Args {
    #[arg(long, value_enum)]
    plan: PlanArg,
    #[arg(long, value_enum, default_value = "github-release-unsigned")]
    channel: ChannelArg,
    #[arg(long)]
    artifact: PathBuf,
    #[arg(long, default_value = "artifacts/duckomo.lock.json")]
    lock: PathBuf,
    #[arg(long, default_value = DEFAULT_STATIONS)]
    stations: PathBuf,
    #[arg(long, default_value = DEFAULT_REFERENCE)]
    reference: PathBuf,
    #[arg(long, default_value = DEFAULT_OFFICIAL_VALUES)]
    official_values: PathBuf,
    #[arg(long, default_value = DEFAULT_MANIFEST)]
    manifest: PathBuf,
    #[arg(long, default_value = DEFAULT_SERIES_PREFIX)]
    series_prefix: PathBuf,
    #[arg(long)]
    output: Option<PathBuf>,
    #[arg(long, default_value = "target/omo-series-spike-data")]
    data_dir: PathBuf,
    #[arg(long, default_value = "target/omo-series-spike-extensions")]
    ext_cache_dir: PathBuf,
    #[arg(long, default_value_t = 1)]
    threads: usize,
    /// Plan G only. `latest` or an explicit `YYYY-MM-DDTHHZ` reference time.
    #[arg(long, default_value = "latest")]
    run: String,
    /// Plan G only. Number of flat indices to cross-check against duckomo lat/lon.
    #[arg(long, default_value_t = 0)]
    crosscheck: usize,
    /// Plan G predicate mode (`box`, `regions`, or `parallel-box`).
    #[arg(long, value_enum, default_value = "parallel-box")]
    select: SelectMode,
    /// Plan G timing repeats.
    #[arg(long, default_value_t = 1)]
    repeat: usize,
}

#[derive(Clone, Copy, Debug, Serialize, ValueEnum, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum SelectMode {
    Box,
    Regions,
    ParallelBox,
}

#[derive(Clone, Copy, Debug, Serialize, ValueEnum, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum PlanArg {
    #[value(name = "A", alias = "a")]
    A,
    #[value(name = "B", alias = "b")]
    B,
    #[value(name = "G", alias = "g")]
    G,
}

#[derive(Clone, Copy, Debug, Serialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
enum ChannelArg {
    CommunitySigned,
    GithubReleaseUnsigned,
}

impl ChannelArg {
    fn engine_channel(self) -> Channel {
        match self {
            Self::CommunitySigned => Channel::CommunitySigned,
            Self::GithubReleaseUnsigned => Channel::GithubReleaseUnsigned,
        }
    }
}

#[derive(Serialize)]
struct FileEvidence {
    path: String,
    bytes: u64,
    sha256: String,
}

#[derive(Serialize)]
struct SpikeReport {
    schema: &'static str,
    status: &'static str,
    plan: PlanArg,
    channel: ChannelArg,
    platform: String,
    locked_artifact_sha256: String,
    locked_artifact_bytes: u64,
    loaded_extension_sha256: String,
    station_count: usize,
    window_utc: [&'static str; 2],
    schema_columns: [&'static str; 9],
    rows_written: usize,
    unique_station_time_keys: usize,
    duplicate_rows_removed: usize,
    reference_comparison: ReferenceComparison,
    query_elapsed_ms: u64,
    scan_measurement: omo_engine::series::ScanMeasurement,
    plan_details: Value,
    bootstrap: BootstrapReport,
    inputs: Vec<FileEvidence>,
    output: FileEvidence,
    report_path: String,
}

fn main() -> ExitCode {
    let args = Args::parse();
    if args.plan == PlanArg::G {
        return match execute_plan_g(&args) {
            Ok(report) => {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&report).expect("serialize plan G report")
                );
                if report.get("status").and_then(|value| value.as_str()) == Some("passed") {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::FAILURE
                }
            }
            Err(error) => {
                eprintln!("omo-series-spike: {error}");
                ExitCode::from(2)
            }
        };
    }
    match execute(args) {
        Ok(report) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&report).expect("serialize spike report")
            );
            if report.status == "passed" {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Err(error) => {
            eprintln!("omo-series-spike: {error}");
            ExitCode::from(2)
        }
    }
}

fn execute(args: Args) -> Result<SpikeReport, String> {
    if args.threads == 0 {
        return Err("thread count must be greater than zero".to_owned());
    }
    let platform = Platform::current().map_err(|error| error.to_string())?;
    let channel = args.channel.engine_channel();
    let lock_path = canonical_existing(&args.lock, "artifact lock")?;
    let artifact_path = canonical_existing(&args.artifact, "Duckomo artifact")?;
    let stations_path = canonical_existing(&args.stations, "station CSV")?;
    let reference_path = canonical_existing(&args.reference, "station-series reference")?;
    let official_values_path =
        canonical_existing(&args.official_values, "official OM reader values")?;
    let manifest_path = canonical_existing(&args.manifest, "station-series fixture manifest")?;
    let series_prefix = canonical_existing(&args.series_prefix, "series prefix")?;
    let object_path = canonical_existing(&series_prefix.join("chunk_0000.om"), "series chunk")?;
    let stations = read_stations(&stations_path).map_err(|error| error.to_string())?;

    let manifest_bytes = fs::read(&manifest_path)
        .map_err(|error| format!("cannot read fixture manifest: {error}"))?;
    let manifest: Value = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| format!("invalid fixture manifest: {error}"))?;
    verify_manifest_hashes(&manifest, &object_path, &official_values_path)?;

    let lock_bytes =
        fs::read(&lock_path).map_err(|error| format!("cannot read artifact lock: {error}"))?;
    let lock = ArtifactLock::from_json(&lock_bytes)
        .map_err(|error| format!("invalid artifact lock: {error}"))?;
    let entry = lock
        .lookup_for_golden_measurement(
            channel.as_str(),
            DUCKOMO_VERSION,
            DUCKDB_VERSION,
            platform.as_str(),
        )
        .map_err(|error| error.to_string())?
        .clone();
    let locked_artifact_sha256 = entry.sha256.clone();
    let locked_artifact_bytes = entry.size;
    let golden_fixture = canonical_existing(Path::new(&entry.golden.fixture), "golden fixture")?;
    let data_root = prepare_directory(&args.data_dir, "series-spike data")?;
    let ext_cache_dir = prepare_directory(&args.ext_cache_dir, "extension cache")?;
    let output_path = args.output.unwrap_or_else(|| {
        PathBuf::from(format!(
            "target/omo-series-spike-{}.csv",
            match args.plan {
                PlanArg::A => "A",
                PlanArg::B => "B",
                PlanArg::G => "G",
            }
        ))
    });
    let output_parent = output_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(output_parent)
        .map_err(|error| format!("cannot create output directory: {error}"))?;
    let report_path = output_path.with_extension("report.json");
    let artifact_evidence = file_evidence(&artifact_path)?;

    let run_dir = Builder::new()
        .prefix("omo-series-spike-")
        .tempdir_in(&data_root)
        .map_err(|error| format!("cannot create per-run data directory: {error}"))?;
    let config = BootstrapConfig {
        data_dir: run_dir.path().to_path_buf(),
        ext_cache_dir,
        artifact_path,
        fixture_path: golden_fixture,
        artifact: entry,
        platform,
        threads: Some(args.threads),
        memory_limit: None,
        httpfs_timeout_ms: 15_000,
        no_network: false,
        install_spatial_before_lock: matches!(args.plan, PlanArg::B),
        database_path: None,
        duckomo_remote_max_request: None,
        duckomo_remote_merge_gap: None,
    };
    let mut connection = open_bootstrapped(config).map_err(|error| error.to_string())?;
    let run = match args.plan {
        PlanArg::A => plan_a::run(&connection, &stations, &series_prefix, &object_path),
        PlanArg::B => plan_b::run(&mut connection, &stations, &series_prefix, 0, 7_200),
        PlanArg::G => return Err("plan G is handled before the M0 comparison path".into()),
    }
    .map_err(|error| error.to_string())?;
    let bootstrap = connection.report().clone();
    drop(connection);
    drop(run_dir);

    let comparison =
        compare_reference(&run.rows, &reference_path).map_err(|error| error.to_string())?;
    write_csv(&output_path, &run.rows).map_err(|error| error.to_string())?;
    let output = file_evidence(&output_path)?;
    let inputs = [
        file_evidence(&stations_path)?,
        artifact_evidence,
        file_evidence(&reference_path)?,
        file_evidence(&official_values_path)?,
        file_evidence(&manifest_path)?,
        file_evidence(&object_path)?,
    ]
    .into_iter()
    .collect();
    let status = if comparison.passed() {
        "passed"
    } else {
        "failed"
    };
    let report = SpikeReport {
        schema: "omo-series-spike-report/1",
        status,
        plan: args.plan,
        channel: args.channel,
        platform: platform.as_str().to_owned(),
        locked_artifact_sha256,
        locked_artifact_bytes,
        loaded_extension_sha256: bootstrap.artifact_sha256.clone(),
        station_count: stations.len(),
        window_utc: ["1970-01-01T00:00:00Z", "1970-01-01T02:00:00Z"],
        schema_columns: STATION_SERIES_COLUMNS,
        rows_written: run.rows.len(),
        unique_station_time_keys: comparison.actual_unique_keys,
        duplicate_rows_removed: run.duplicate_rows_removed,
        reference_comparison: comparison,
        query_elapsed_ms: run.elapsed_ms,
        scan_measurement: run.scan,
        plan_details: run.plan_details,
        bootstrap,
        inputs,
        output,
        report_path: report_path.display().to_string(),
    };
    let report_bytes = serde_json::to_vec_pretty(&report).map_err(|error| error.to_string())?;
    fs::write(&report_path, report_bytes)
        .map_err(|error| format!("cannot write report {}: {error}", report_path.display()))?;
    Ok(report)
}

fn verify_manifest_hashes(
    manifest: &Value,
    object_path: &Path,
    reference_path: &Path,
) -> Result<(), String> {
    if manifest.get("schema").and_then(Value::as_str) != Some("omo-series-spike-fixture/1") {
        return Err("unsupported station-series fixture manifest schema".to_owned());
    }
    let expected_object = manifest
        .pointer("/source_fixture/source_object_sha256")
        .and_then(Value::as_str)
        .ok_or_else(|| "fixture manifest is missing source object hash".to_owned())?;
    let expected_reference = manifest
        .pointer("/source_fixture/official_value_reference_sha256")
        .and_then(Value::as_str)
        .ok_or_else(|| "fixture manifest is missing official reference hash".to_owned())?;
    let expected_object_bytes = manifest
        .pointer("/source_fixture/source_object_bytes")
        .and_then(Value::as_u64)
        .ok_or_else(|| "fixture manifest is missing source object byte size".to_owned())?;
    let shape = manifest
        .pointer("/array/shape")
        .and_then(Value::as_array)
        .ok_or_else(|| "fixture manifest is missing array shape".to_owned())?;
    if shape.iter().map(Value::as_u64).collect::<Option<Vec<_>>>() != Some(vec![2, 3, 5, 5])
        || manifest
            .pointer("/array/chunk_time_length")
            .and_then(Value::as_i64)
            != Some(3)
        || manifest
            .pointer("/array/time_step_seconds")
            .and_then(Value::as_i64)
            != Some(3_600)
        || manifest
            .pointer("/array/member_index")
            .and_then(Value::as_i64)
            != Some(0)
    {
        return Err("station-series spike currently requires the declared [2,3,5,5] fixture layout and [member,time,lat,lon] window".to_owned());
    }
    let object_hash = sha256_file(object_path)?;
    let reference_hash = sha256_file(reference_path)?;
    let object_bytes = fs::metadata(object_path)
        .map_err(|error| format!("cannot stat {}: {error}", object_path.display()))?
        .len();
    if object_bytes != expected_object_bytes {
        return Err(format!(
            "series chunk size mismatch: expected {expected_object_bytes}, got {object_bytes}"
        ));
    }
    if object_hash != expected_object {
        return Err(format!(
            "series chunk hash mismatch: expected {expected_object}, got {object_hash}"
        ));
    }
    if reference_hash != expected_reference {
        return Err(format!(
            "official value reference hash mismatch: expected {expected_reference}, got {reference_hash}"
        ));
    }
    Ok(())
}

fn canonical_existing(path: &Path, description: &str) -> Result<PathBuf, String> {
    path.canonicalize()
        .map_err(|error| format!("cannot resolve {description} {}: {error}", path.display()))
}

fn prepare_directory(path: &Path, description: &str) -> Result<PathBuf, String> {
    fs::create_dir_all(path).map_err(|error| {
        format!(
            "cannot create {description} directory {}: {error}",
            path.display()
        )
    })?;
    path.canonicalize().map_err(|error| {
        format!(
            "cannot resolve {description} directory {}: {error}",
            path.display()
        )
    })
}

fn file_evidence(path: &Path) -> Result<FileEvidence, String> {
    let metadata =
        fs::metadata(path).map_err(|error| format!("cannot stat {}: {error}", path.display()))?;
    Ok(FileEvidence {
        path: path.display().to_string(),
        bytes: metadata.len(),
        sha256: sha256_file(path)?,
    })
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let bytes =
        fs::read(path).map_err(|error| format!("cannot hash {}: {error}", path.display()))?;
    let digest = Sha256::digest(bytes);
    Ok(format!("{digest:x}"))
}

fn execute_plan_g(args: &Args) -> Result<Value, String> {
    if args.repeat == 0 {
        return Err("repeat count must be greater than zero".into());
    }
    let platform = Platform::current().map_err(|error| error.to_string())?;
    let channel = args.channel.engine_channel();
    let lock_path = canonical_existing(&args.lock, "artifact lock")?;
    let artifact_path = canonical_existing(&args.artifact, "Duckomo artifact")?;
    let stations_path = canonical_existing(&args.stations, "station CSV")?;
    let stations = read_plan_g_stations(&stations_path)?;
    let catalog =
        Catalog::load(Path::new("catalog/products.json")).map_err(|error| error.to_string())?;
    let entry = catalog.default_entry().clone();
    let lock = ArtifactLock::from_json(
        &fs::read(&lock_path).map_err(|error| format!("cannot read artifact lock: {error}"))?,
    )
    .map_err(|error| error.to_string())?;
    let locked = lock
        .lookup_for_golden_measurement(
            channel.as_str(),
            DUCKOMO_VERSION,
            DUCKDB_VERSION,
            platform.as_str(),
        )
        .map_err(|error| error.to_string())?
        .clone();
    let golden_fixture = canonical_existing(Path::new(&locked.golden.fixture), "golden fixture")?;
    let data_root = prepare_directory(&args.data_dir, "series-spike data")?;
    let ext_cache_dir = prepare_directory(&args.ext_cache_dir, "extension cache")?;
    let run_dir = Builder::new()
        .prefix("omo-series-spike-g-")
        .tempdir_in(&data_root)
        .map_err(|error| format!("cannot create per-run data directory: {error}"))?;
    let config = BootstrapConfig {
        data_dir: run_dir.path().to_path_buf(),
        ext_cache_dir,
        artifact_path,
        fixture_path: golden_fixture,
        artifact: locked,
        platform,
        threads: Some(args.threads),
        memory_limit: None,
        httpfs_timeout_ms: 15_000,
        no_network: false,
        install_spatial_before_lock: matches!(args.select, SelectMode::Regions),
        database_path: None,
        duckomo_remote_max_request: None,
        duckomo_remote_merge_gap: None,
    };
    let mut connection = open_bootstrapped(config.clone()).map_err(|error| error.to_string())?;
    let source = ConnMetaSource {
        conn: &connection,
        entry: &entry,
    };
    let run_ref = if args.run == "latest" || args.run == "latest_complete" {
        RunRef::LatestComplete
    } else {
        RunRef::Explicit(args.run.clone())
    };
    let forecast = catalog::resolve(&entry, &source, run_ref).map_err(|error| error.to_string())?;
    let https = catalog::open_meteo_https(&forecast.object_url);
    let dimensions = match plan_g::describe_om_columns(
        &connection,
        &forecast.object_url,
        &entry.grid.duckomo_domain,
    ) {
        Ok(columns) => serde_json::json!({"status": "ok", "columns": columns}),
        Err(error) => serde_json::json!({"status": "failed", "error": error.to_string()}),
    };
    let points = stations
        .iter()
        .map(|(id, lat, lon)| {
            let nearest = O1280::nearest(*lat, *lon);
            serde_json::json!({
                "station_id": id,
                "flat_index": nearest.flat_index,
                "grid_lat": nearest.lat,
                "grid_lon": nearest.lon,
                "distance_m": nearest.distance_m
            })
        })
        .collect::<Vec<_>>();
    let selected = stations
        .iter()
        .map(|(_, lat, lon)| {
            let nearest = O1280::nearest(*lat, *lon);
            plan_g::SelectedPoint {
                flat_index: nearest.flat_index,
                lat: nearest.lat,
                lon: nearest.lon,
            }
        })
        .collect::<Vec<_>>();
    let mut repeats = Vec::new();
    let mut read_error = None;
    for index in 0..args.repeat {
        let started = std::time::Instant::now();
        eprintln!(
            "omo-series-spike: plan G repeat {}/{} ({} points, mode={:?})",
            index + 1,
            args.repeat,
            selected.len(),
            args.select
        );
        let outcome = match args.select {
            SelectMode::Box => read_box(
                &connection,
                &forecast.object_url,
                &entry.grid.duckomo_domain,
                &selected,
            ),
            SelectMode::Regions => read_regions(
                &mut connection,
                &forecast.object_url,
                &entry.grid.duckomo_domain,
                &selected,
            ),
            SelectMode::ParallelBox => read_parallel_boxes(
                &config,
                &forecast.object_url,
                &entry.grid.duckomo_domain,
                &selected,
            ),
        };
        let elapsed_ms = started.elapsed().as_millis() as u64;
        eprintln!(
            "omo-series-spike: plan G repeat {} finished in {} ms",
            index + 1,
            elapsed_ms
        );
        match outcome {
            Ok(summary) => repeats.push(
                serde_json::json!({"repeat": index, "elapsed_ms": elapsed_ms, "summary": summary}),
            ),
            Err(error) => {
                repeats.push(serde_json::json!({"repeat": index, "elapsed_ms": elapsed_ms, "error": error.clone()}));
                read_error = Some(error);
                break;
            }
        }
    }
    let crosscheck = if args.crosscheck == 0 {
        serde_json::json!({"sampled": 0})
    } else {
        match crosscheck_points(
            &config,
            &forecast.object_url,
            &entry.grid.duckomo_domain,
            args.crosscheck,
        ) {
            Ok(value) => value,
            Err(error) => serde_json::json!({"status": "failed", "error": error}),
        }
    };
    let passed = read_error.is_none()
        && dimensions.get("status").and_then(|value| value.as_str()) == Some("ok")
        && crosscheck.get("status").and_then(|value| value.as_str()) != Some("failed");
    Ok(serde_json::json!({
        "schema": "omo-series-spike-plan-g/1",
        "status": if passed { "passed" } else { "failed" },
        "select": args.select,
        "station_count": stations.len(),
        "run": forecast,
        "object_https": https,
        "dimensions": dimensions,
        "selected_points": points,
        "repeats": repeats,
        "crosscheck": crosscheck,
        "bootstrap": connection.report(),
    }))
}

fn read_box(
    conn: &BootstrappedConn,
    object_url: &str,
    domain: &str,
    points: &[plan_g::SelectedPoint],
) -> Result<Value, String> {
    let mut reader = plan_g::DuckDbPointReader { conn, domain };
    let rows = reader
        .read_points(object_url, points)
        .map_err(|error| error.to_string())?;
    let exact = plan_g::verify_exact(points, &rows).is_ok();
    let scan = latest_scan_measurement(conn).ok();
    Ok(serde_json::json!({
        "mode": "box",
        "selected_points": points.len(),
        "returned_rows": rows.len(),
        "exact_match": exact,
        "scan_metrics": scan,
    }))
}

fn read_regions(
    conn: &mut BootstrappedConn,
    object_url: &str,
    domain: &str,
    points: &[plan_g::SelectedPoint],
) -> Result<Value, String> {
    conn.ensure_spatial().map_err(|error| error.to_string())?;
    let rows = plan_g::read_om_regions_points(conn, object_url, domain, points)
        .map_err(|error| error.to_string())?;
    let scan = latest_scan_measurement(conn).ok();
    Ok(serde_json::json!({
        "mode": "regions",
        "selected_points": points.len(),
        "returned_rows": rows.len(),
        "scan_metrics": scan,
    }))
}

fn read_parallel_boxes(
    config: &BootstrapConfig,
    object_url: &str,
    domain: &str,
    points: &[plan_g::SelectedPoint],
) -> Result<Value, String> {
    let workers = std::env::var("OMO_SPIKE_WORKERS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(20)
        .min(points.len().max(1));
    let mut reader = plan_g::ParallelDuckDbPointReader {
        config,
        domain,
        max_workers: workers,
        cancel: None,
        scan_stats: plan_g::PlanGScanStats::default(),
    };
    let rows = reader
        .read_points(object_url, points)
        .map_err(|error| error.to_string())?;
    plan_g::verify_exact(points, &rows).map_err(|error| error.to_string())?;
    Ok(serde_json::json!({
        "mode": "parallel-box",
        "workers": workers,
        "selected_points": points.len(),
        "returned_rows": rows.len(),
        "sum_bytes_fetched": reader.scan_stats.bytes_fetched,
        "sum_read_requests": reader.scan_stats.read_requests,
        "sum_data_bytes": reader.scan_stats.data_bytes,
    }))
}

fn crosscheck_points(
    config: &BootstrapConfig,
    object_url: &str,
    domain: &str,
    sample_count: usize,
) -> Result<Value, String> {
    let total = O1280::points() as usize;
    let step = (total / sample_count.max(1)).max(1);
    let mut indices = vec![
        0u32,
        10,
        19,
        (total - 20) as u32,
        (total - 10) as u32,
        (total - 1) as u32,
    ];
    let mut idx = 0usize;
    while indices.len() < sample_count && idx < total {
        let candidate = idx as u32;
        if !indices.contains(&candidate) {
            indices.push(candidate);
        }
        idx += step;
    }
    indices.truncate(sample_count);
    let points = indices
        .into_iter()
        .map(|flat_index| {
            let (lat, lon) = O1280::point(flat_index);
            plan_g::SelectedPoint {
                flat_index,
                lat,
                lon,
            }
        })
        .collect::<Vec<_>>();
    let workers = 20usize.min(points.len().max(1));
    let chunk_size = points.len().div_ceil(workers);
    let mut handles = Vec::new();
    for chunk in points.chunks(chunk_size) {
        let chunk = chunk.to_vec();
        let cfg = config.clone();
        let object_url = object_url.to_owned();
        let domain = domain.to_owned();
        handles.push(std::thread::spawn(
            move || -> Result<(usize, Vec<Value>), String> {
                let conn = open_bootstrapped(cfg).map_err(|error| error.to_string())?;
                let mut matched = 0usize;
                let mut diffs = Vec::new();
                for point in &chunk {
                    let coords =
                        plan_g::probe_om_point_coordinates(&conn, &object_url, &domain, point)
                            .map_err(|error| error.to_string())?;
                    if coords.len() == 1
                        && coords[0].0.to_bits() == point.lat.to_bits()
                        && coords[0].1.to_bits() == point.lon.to_bits()
                    {
                        matched += 1;
                    } else if diffs.len() < 5 {
                        diffs.push(serde_json::json!({
                            "flat_index": point.flat_index,
                            "expected": [point.lat, point.lon],
                            "actual": coords,
                        }));
                    }
                }
                Ok((matched, diffs))
            },
        ));
    }
    let mut matched = 0usize;
    let mut mismatches = Vec::new();
    for handle in handles {
        let (part_matched, part_diffs) = handle
            .join()
            .map_err(|_| "crosscheck worker panicked".to_owned())??;
        matched += part_matched;
        mismatches.extend(part_diffs);
    }
    Ok(serde_json::json!({
        "sampled": points.len(),
        "matched": matched,
        "mismatches": mismatches,
        "status": if mismatches.is_empty() && matched == points.len() { "passed" } else { "failed" }
    }))
}

fn read_plan_g_stations(path: &Path) -> Result<Vec<(String, f64, f64)>, String> {
    let text = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let mut lines = text.lines();
    let header = lines
        .next()
        .ok_or("station CSV is empty")?
        .trim_start_matches('\u{feff}');
    let delimiter = if header.contains(';') && !header.contains(',') {
        ';'
    } else {
        ','
    };
    let headers = header
        .split(delimiter)
        .map(str::trim)
        .map(|name| name.to_ascii_lowercase())
        .collect::<Vec<_>>();
    let index = |aliases: &[&str]| {
        headers
            .iter()
            .position(|name| aliases.iter().any(|alias| name == alias))
    };
    let id_index = index(&["id", "station_id", "code"]).ok_or("station CSV has no id column")?;
    let lat_index = index(&["lat", "latitude", "y"]).ok_or("station CSV has no latitude column")?;
    let lon_index =
        index(&["lon", "lng", "longitude", "x"]).ok_or("station CSV has no longitude column")?;
    let mut stations = Vec::new();
    for (line_index, line) in lines.enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let fields = line.split(delimiter).map(str::trim).collect::<Vec<_>>();
        let id = fields.get(id_index).copied().unwrap_or("").to_owned();
        let lat = fields
            .get(lat_index)
            .copied()
            .unwrap_or("")
            .parse::<f64>()
            .map_err(|error| format!("invalid latitude on line {}: {error}", line_index + 2))?;
        let lon = fields
            .get(lon_index)
            .copied()
            .unwrap_or("")
            .parse::<f64>()
            .map_err(|error| format!("invalid longitude on line {}: {error}", line_index + 2))?;
        stations.push((id, lat, lon));
    }
    if stations.is_empty() {
        return Err("station CSV contains no stations".into());
    }
    Ok(stations)
}
