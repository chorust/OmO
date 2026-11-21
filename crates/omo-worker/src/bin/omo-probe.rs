use std::{
    fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

use clap::{Parser, ValueEnum};
use omo_engine::{
    bootstrap::{BootstrapError, BootstrapReport, measure_bootstrap},
    config::{BootstrapConfig, Channel, Platform},
    conn::open_bootstrapped,
    lock::ArtifactLock,
    query::{QueryError, QueryPurpose, execute_query},
};
use omo_protocol::result_ref::ResultLimits;
use serde::Serialize;
use serde_json::json;
use tempfile::Builder;

const DUCKOMO_VERSION: &str = "v0.4.0";
const DUCKDB_VERSION: &str = "v2.0.0";

#[derive(Debug, Parser)]
#[command(
    name = "omo-probe",
    about = "Verify Duckomo bootstrap and golden query"
)]
struct Args {
    #[arg(long, value_enum)]
    channel: ChannelArg,
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..))]
    repeat: u32,
    #[arg(long)]
    r4: bool,
    #[arg(long)]
    no_network: bool,
    #[arg(long)]
    verbose: bool,
    #[arg(long, default_value_t = 1)]
    threads: usize,
    #[arg(long)]
    artifact: PathBuf,
    #[arg(long, default_value = "artifacts/duckomo.lock.json")]
    lock: PathBuf,
    #[arg(long, default_value = "target/omo-probe-data")]
    data_dir: PathBuf,
    #[arg(long, default_value = "target/omo-probe-extensions")]
    ext_cache_dir: PathBuf,
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
struct AttemptReport {
    iteration: u32,
    elapsed_ms: u64,
    status: &'static str,
    bootstrap: Option<BootstrapReport>,
    r4: Option<serde_json::Value>,
    no_network_remote_probe: Option<String>,
    error: Option<String>,
}

#[derive(Serialize)]
struct ProbeReport {
    status: &'static str,
    channel: ChannelArg,
    platform: String,
    golden_mode: &'static str,
    repeat_requested: u32,
    attempts_completed: u32,
    no_network: bool,
    r4_requested: bool,
    attempts: Vec<AttemptReport>,
}

fn main() -> ExitCode {
    let args = Args::parse();
    match run(args) {
        Ok((report, success)) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&report).expect("serialize probe report")
            );
            if success {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Err(error) => {
            eprintln!("omo-probe: {error}");
            ExitCode::from(2)
        }
    }
}

fn run(args: Args) -> Result<(ProbeReport, bool), String> {
    if args.threads == 0 {
        return Err("thread count must be greater than zero".to_owned());
    }
    let platform = Platform::current().map_err(|error| error.to_string())?;
    let channel = args.channel.engine_channel();
    let lock_path = canonical_existing(&args.lock, "artifact lock")?;
    let artifact_path = canonical_existing(&args.artifact, "Duckomo artifact")?;
    let data_root = prepare_directory(&args.data_dir, "probe data")?;
    let ext_cache_dir = prepare_directory(&args.ext_cache_dir, "extension cache")?;
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
    let fixture_path = canonical_existing(Path::new(&entry.golden.fixture), "golden fixture")?;
    let is_measurement =
        entry.golden.result_sha256 == "<filled only after a verified bootstrap run>";

    let mut attempts = Vec::with_capacity(args.repeat as usize);
    let mut success = true;
    for iteration in 1..=args.repeat {
        let attempt_started = std::time::Instant::now();
        if args.verbose {
            eprintln!("omo-probe: attempt {iteration}/{} starting", args.repeat);
        }
        let run_data = Builder::new()
            .prefix("omo-probe-run-")
            .tempdir_in(&data_root)
            .map_err(|error| format!("cannot create per-run data directory: {error}"))?;
        let config = BootstrapConfig {
            data_dir: run_data.path().to_path_buf(),
            ext_cache_dir: ext_cache_dir.clone(),
            artifact_path: artifact_path.clone(),
            fixture_path: fixture_path.clone(),
            artifact: entry.clone(),
            platform,
            threads: Some(args.threads),
            memory_limit: None,
            httpfs_timeout_ms: 15_000,
            no_network: args.no_network,
            install_spatial_before_lock: args.r4,
            database_path: None,
        };

        let attempt = if is_measurement {
            match measure_bootstrap(config) {
                Ok(report) => AttemptReport {
                    iteration,
                    elapsed_ms: 0,
                    status: "measured",
                    bootstrap: Some(report),
                    r4: args.r4.then(|| json!({
                        "status": "not_run",
                        "reason": "measurement mode drops the connection; pin the independently verified golden hash first"
                    })),
                    no_network_remote_probe: None,
                    error: None,
                },
                Err(error) => failed_attempt(iteration, error),
            }
        } else {
            match open_bootstrapped(config) {
                Ok(mut connection) => {
                    let r4 = args.r4.then(|| {
                        let result = connection.ensure_spatial();
                        json!({
                            "status": if result.is_ok() { "loaded" } else { "failed" },
                            "spatial_preinstall": connection.report().r4.spatial_preinstall,
                            "spatial_after_lock": connection.report().r4.spatial_after_lock,
                            "unsigned_close": connection.report().r4.unsigned_close,
                            "community_close": connection.report().r4.community_close,
                            "error": result.err().map(|error| error.to_string())
                        })
                    });
                    let remote_probe = if args.no_network {
                        Some(run_no_network_remote_probe(&connection, run_data.path()))
                    } else {
                        None
                    };
                    let report = connection.report().clone();
                    drop(connection);
                    AttemptReport {
                        iteration,
                        elapsed_ms: 0,
                        status: "passed",
                        bootstrap: Some(report),
                        r4,
                        no_network_remote_probe: remote_probe,
                        error: None,
                    }
                }
                Err(error) => failed_attempt(iteration, error),
            }
        };

        let elapsed_ms = attempt_started
            .elapsed()
            .as_millis()
            .min(u128::from(u64::MAX)) as u64;
        let mut attempt = attempt;
        attempt.elapsed_ms = elapsed_ms;
        if args.verbose {
            eprintln!(
                "omo-probe: attempt {iteration}/{} {} in {elapsed_ms} ms",
                args.repeat, attempt.status
            );
        }
        if attempt.error.is_some() {
            success = false;
        }
        attempts.push(attempt);
        drop(run_data);
        if !success {
            break;
        }
    }

    let attempts_completed = attempts.len() as u32;
    let status = if !success {
        "failed"
    } else if is_measurement {
        "measured"
    } else {
        "passed"
    };
    Ok((
        ProbeReport {
            status,
            channel: args.channel,
            platform: platform.as_str().to_owned(),
            golden_mode: if is_measurement {
                "measurement_only"
            } else {
                "locked"
            },
            repeat_requested: args.repeat,
            attempts_completed,
            no_network: args.no_network,
            r4_requested: args.r4,
            attempts,
        },
        success,
    ))
}

fn failed_attempt(iteration: u32, error: BootstrapError) -> AttemptReport {
    AttemptReport {
        iteration,
        elapsed_ms: 0,
        status: "failed",
        bootstrap: None,
        r4: None,
        no_network_remote_probe: None,
        error: Some(error.to_string()),
    }
}

fn run_no_network_remote_probe(
    connection: &omo_engine::BootstrappedConn,
    session_tmp: &Path,
) -> String {
    let result = execute_query(
        connection,
        "SELECT * FROM read_csv('https://example.invalid/omo-no-network-probe.csv')",
        QueryPurpose::Preview,
        ResultLimits {
            rows: 10,
            bytes: 1024 * 1024,
        },
        session_tmp,
        "no-network-remote-probe",
    );
    match result {
        Err(QueryError::Bootstrap(BootstrapError::CapabilityUnavailable {
            capability: "remote",
            ..
        })) => "rejected_with_capability_unavailable_remote".to_owned(),
        Err(error) => format!("rejected_with_unexpected_error: {error}"),
        Ok(result_ref) => {
            let _ = fs::remove_file(result_ref.path);
            "unexpectedly_allowed".to_owned()
        }
    }
}

fn canonical_existing(path: &Path, label: &str) -> Result<PathBuf, String> {
    path.canonicalize()
        .map_err(|error| format!("cannot resolve {label} at {}: {error}", path.display()))
}

fn prepare_directory(path: &Path, label: &str) -> Result<PathBuf, String> {
    fs::create_dir_all(path).map_err(|error| {
        format!(
            "cannot create {label} directory {}: {error}",
            path.display()
        )
    })?;
    canonical_existing(path, label)
}
