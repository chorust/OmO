use std::{
    fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

use clap::{Parser, ValueEnum};
use omo_engine::{
    bootstrap::BootstrapReport,
    config::{BootstrapConfig, Channel, Platform},
    conn::open_bootstrapped,
    lock::ArtifactLock,
    series::{
        plan_a, plan_b, read_stations,
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
}

#[derive(Clone, Copy, Debug, Serialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
enum PlanArg {
    #[value(name = "A", alias = "a")]
    A,
    #[value(name = "B", alias = "b")]
    B,
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
    match execute(Args::parse()) {
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
    };
    let mut connection = open_bootstrapped(config).map_err(|error| error.to_string())?;
    let run = match args.plan {
        PlanArg::A => plan_a::run(&connection, &stations, &series_prefix, &object_path),
        PlanArg::B => plan_b::run(&mut connection, &stations, &series_prefix, 0, 7_200),
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
