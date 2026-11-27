use duckdb::Connection;

use crate::{
    bootstrap::{
        BootstrapError, BootstrapReport, Capabilities, HttpfsState, SpatialState,
        open_and_bootstrap,
    },
    config::BootstrapConfig,
    probe,
};

/// A database connection that passed artifact verification, bootstrap probes, and the locked
/// golden query. Its raw DuckDB handle is never exposed to OM query callers.
pub struct BootstrappedConn {
    connection: Option<Connection>,
    capabilities: Capabilities,
    report: BootstrapReport,
    generation: u64,
}

impl std::fmt::Debug for BootstrappedConn {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BootstrappedConn")
            .field("has_connection", &self.connection.is_some())
            .field("capabilities", &self.capabilities)
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}

pub fn open_bootstrapped(config: BootstrapConfig) -> Result<BootstrappedConn, BootstrapError> {
    let (connection, report) = open_and_bootstrap(&config, false)?;
    let capabilities = Capabilities {
        remote: matches!(&report.httpfs, HttpfsState::Loaded | HttpfsState::Installed),
        spatial: matches!(&report.spatial, SpatialState::Loaded),
    };
    Ok(BootstrappedConn {
        connection: Some(connection),
        capabilities,
        report,
        generation: 0,
    })
}

impl BootstrappedConn {
    pub fn report(&self) -> &BootstrapReport {
        &self.report
    }

    pub fn capabilities(&self) -> &Capabilities {
        &self.capabilities
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Obtain DuckDB's thread-safe cancellation handle without exposing the connection.
    pub fn interrupt_handle(
        &self,
    ) -> Result<std::sync::Arc<duckdb::InterruptHandle>, BootstrapError> {
        Ok(self.connection()?.interrupt_handle())
    }

    pub fn probe_before_task(&mut self) -> Result<(), BootstrapError> {
        let Some(connection) = self.connection.as_ref() else {
            return Err(BootstrapError::ConnectionDiscarded);
        };
        if let Err(error) = probe::verify_required_functions(connection) {
            self.connection.take();
            self.capabilities.remote = false;
            self.capabilities.spatial = false;
            return Err(error);
        }
        Ok(())
    }

    /// Try to load spatial after configuration lock and retain the observed result for R4.
    pub fn ensure_spatial(&mut self) -> Result<(), BootstrapError> {
        let Some(connection) = self.connection.as_ref() else {
            return Err(BootstrapError::ConnectionDiscarded);
        };
        match connection.execute_batch("LOAD spatial;") {
            Ok(()) => {
                self.capabilities.spatial = true;
                self.report.spatial = SpatialState::Loaded;
                self.report.r4.spatial_after_lock = "ok".to_owned();
                Ok(())
            }
            Err(error) => {
                let reason = error.to_string();
                self.capabilities.spatial = false;
                self.report.spatial = SpatialState::Failed(reason.clone());
                self.report.r4.spatial_after_lock = format!("failed: {reason}");
                Err(BootstrapError::CapabilityUnavailable {
                    capability: "spatial",
                    reason,
                })
            }
        }
    }

    pub fn require_remote(&self) -> Result<(), BootstrapError> {
        if self.capabilities.remote {
            return Ok(());
        }
        let reason = match &self.report.httpfs {
            HttpfsState::Unavailable(reason) => reason.clone(),
            HttpfsState::Loaded | HttpfsState::Installed => {
                "HTTPFS is not active on this connection".to_owned()
            }
        };
        Err(BootstrapError::CapabilityUnavailable {
            capability: "remote",
            reason,
        })
    }

    pub(crate) fn connection(&self) -> Result<&Connection, BootstrapError> {
        self.connection
            .as_ref()
            .ok_or(BootstrapError::ConnectionDiscarded)
    }

    pub fn set_generation(&mut self, generation: u64) {
        self.generation = generation;
    }

    #[cfg(feature = "test-support")]
    pub fn has_connection(&self) -> bool {
        self.connection.is_some()
    }

    #[cfg(feature = "test-support")]
    pub fn from_test_connection(connection: Connection) -> Self {
        use crate::bootstrap::{
            BootstrapReport, BootstrapTimings, GoldenResult, HttpfsState, R4Report, SpatialState,
        };
        use crate::config::Channel;
        use crate::probe::ProbeReport;

        let probe = ProbeReport {
            duckdb_version: "test".to_owned(),
            platform: "linux_arm64".to_owned(),
            duckomo_version: "v0.4.0".to_owned(),
            channel: Channel::CommunitySigned,
            functions_present: Vec::new(),
        };
        let report = BootstrapReport {
            duckdb_version: "test".to_owned(),
            platform: "linux_arm64".to_owned(),
            duckomo_version: "v0.4.0".to_owned(),
            channel: Channel::CommunitySigned,
            artifact_sha256: "0".repeat(64),
            httpfs: HttpfsState::Unavailable("test".to_owned()),
            spatial: SpatialState::NotRequested,
            probe,
            golden: GoldenResult {
                fixture: "test".to_owned(),
                query_id: "test".to_owned(),
                expected: None,
                actual: String::new(),
                rows: 0,
                ok: false,
            },
            timings_ms: BootstrapTimings::default(),
            extension_settings: Vec::new(),
            r4: R4Report {
                unsigned_close: "not_applicable".to_owned(),
                community_close: "not_applicable".to_owned(),
                repository_close: "not_applicable".to_owned(),
                spatial_preinstall: "not_requested".to_owned(),
                spatial_after_lock: "not_run".to_owned(),
            },
            remote_tuning: None,
        };
        Self {
            connection: Some(connection),
            capabilities: Capabilities {
                remote: false,
                spatial: false,
            },
            report,
            generation: 0,
        }
    }
}
