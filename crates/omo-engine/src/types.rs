pub use crate::conn::BootstrappedConn;

/// Query implementations require a successfully bootstrapped connection capability.
pub trait BootstrappedQuery {
    type Output;

    fn query(&self, conn: &BootstrappedConn, sql: &str) -> Self::Output;
}
