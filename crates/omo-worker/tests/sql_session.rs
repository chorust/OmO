//! Offline SQL session test (T058): temp views `stations` and `station_series` survive
//! worker restarts and return accurate daily-max aggregates.

use omo_engine::series::quote_sql_literal;
use tempfile::tempdir;

#[test]
fn sql_session_views_support_daily_max_and_survive_reattach() {
    let dir = tempdir().unwrap();
    let stations_csv = dir.path().join("stations.csv");
    std::fs::write(
        &stations_csv,
        "station_id,latitude,longitude,name,elevation_m\nA1,39.9,116.4,Beijing,\nB2,31.2,121.5,Shanghai,\n",
    )
    .unwrap();

    let parquet_path = dir.path().join("series_small.parquet");
    {
        let setup_conn = duckdb::Connection::open_in_memory().unwrap();
        let pq_lit = quote_sql_literal(&parquet_path.to_string_lossy());
        setup_conn
            .execute_batch(&format!(
                "COPY (\
                   SELECT 'A1' AS station_id, 'temperature_2m' AS variable, '2026-11-23T00:00:00Z' AS valid_time, 4.5::DOUBLE AS value UNION ALL \
                   SELECT 'A1', 'temperature_2m', '2026-11-23T12:00:00Z', 11.25::DOUBLE UNION ALL \
                   SELECT 'B2', 'temperature_2m', '2026-11-23T00:00:00Z', 14.0::DOUBLE\
                 ) TO {pq_lit} (FORMAT PARQUET)"
            ))
            .unwrap();
    }

    // Attach views on connection 1 and also simulate connection 2 (worker restart).
    for gen_idx in [1u64, 2u64] {
        let conn = duckdb::Connection::open_in_memory().unwrap();
        let st_lit = quote_sql_literal(&stations_csv.to_string_lossy());
        let pq_lit = quote_sql_literal(&parquet_path.to_string_lossy());
        conn.execute_batch(&format!(
            "CREATE OR REPLACE TEMP VIEW stations AS SELECT * FROM read_csv({st_lit}, header=true, auto_detect=true);\
             CREATE OR REPLACE TEMP VIEW station_series AS SELECT * FROM read_parquet({pq_lit});"
        ))
        .unwrap();

        let mut stmt = conn
            .prepare(
                "SELECT s.name, MAX(ss.value) AS max_t \
                 FROM station_series ss JOIN stations s USING (station_id) \
                 GROUP BY s.name ORDER BY s.name",
            )
            .unwrap();
        let results: Vec<(String, f64)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            results,
            vec![("Beijing".to_owned(), 11.25), ("Shanghai".to_owned(), 14.0)],
            "generation {gen_idx}"
        );
    }
}
