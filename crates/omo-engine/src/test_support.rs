use duckdb::Connection;

use crate::{
    bootstrap::{BootstrapError, run_channel_load_for_test},
    config::Channel,
    conn::BootstrappedConn,
};

pub fn run_channel_load(
    channel: Channel,
    load: impl FnOnce(Channel) -> Result<(), String>,
) -> Result<(), BootstrapError> {
    run_channel_load_for_test(channel, load)
}

pub fn new_unbootstrapped_conn() -> Result<BootstrappedConn, BootstrapError> {
    let connection = Connection::open_in_memory().map_err(|error| BootstrapError::DuckDb {
        stage: "test_connection",
        reason: error.to_string(),
    })?;
    Ok(BootstrappedConn::from_test_connection(connection))
}
