use omo_engine::types::{BootstrappedConn, BootstrappedQuery};

/// A raw database connection cannot be passed to an OM query API.
/// ```compile_fail
/// use duckdb::Connection;
/// use omo_engine::types::{BootstrappedConn, BootstrappedQuery};
/// struct Query;
/// impl BootstrappedQuery for Query {
///     type Output = ();
///     fn query(&self, _conn: &BootstrappedConn, _sql: &str) {}
/// }
/// let raw = Connection::open_in_memory().unwrap();
/// Query.query(&raw, "select 1");
/// ```
#[test]
fn query_api_contract_requires_the_bootstrapped_connection_type() {
    struct Noop;
    impl BootstrappedQuery for Noop {
        type Output = ();
        fn query(&self, _conn: &BootstrappedConn, _sql: &str) {}
    }

    fn accepts_only_bootstrapped<Q: BootstrappedQuery>(query: &Q, conn: &BootstrappedConn) {
        query.query(conn, "select 1");
    }
    let _ = accepts_only_bootstrapped::<Noop>;
}
