//! DuckDB engine, verified bootstrap, query limits, and Arrow IPC results.

pub mod bootstrap;
pub mod catalog;
pub mod config;
pub mod conn;
pub mod export;
pub mod grid;
pub mod limits;
pub mod lock;
pub mod map;
pub mod plot;
pub mod probe;
pub mod query;
pub mod results;
pub mod series;
pub mod types;

#[cfg(feature = "test-support")]
pub mod test_support;

pub use bootstrap::{BootstrapError, BootstrapReport, measure_bootstrap};
pub use config::{BootstrapConfig, Channel, Platform};
pub use conn::{BootstrappedConn, open_bootstrapped};
