// SPDX-License-Identifier: Apache-2.0
//! Opt-in compatibility probe for Martin's upstream PostGIS fixtures.
//!
//! This intentionally loads Martin's SQL fixture unmodified. A passing test is
//! evidence of real wire/SQL compatibility; failures are gaps to fix in
//! QuackGIS or the pgwire SQL preprocessor, not fixture-translation tasks.

mod common;

use common::ServerHandle;
use std::path::{Path, PathBuf};
use tokio_postgres::NoTls;

fn martin_table_fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(".tmp/ref/martin/tests/fixtures/tables")
        .join(name)
}

fn strip_line_comments(sql: &str) -> String {
    sql.lines()
        .filter(|line| !line.trim_start().starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires .tmp/ref/martin checkout"]
async fn martin_table_source_fixture_loads_unmodified() {
    let fixture_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(".tmp/ref/martin/tests/fixtures/tables/table_source.sql");
    if !fixture_path.exists() {
        eprintln!("skipping: {} not found", fixture_path.display());
        return;
    }

    let server = ServerHandle::start().await;
    let (client, connection) = tokio_postgres::connect(&server.conn_str(), NoTls)
        .await
        .expect("connect");
    tokio::spawn(connection);

    let fixture = std::fs::read_to_string(fixture_path).expect("read Martin fixture");

    let clean = strip_line_comments(&fixture);

    // Send the entire fixture as one batch, matching psql/fixture-loader
    // behaviour. Splitting on semicolons is intentionally avoided because
    // Postgres dollar-quoted PL/pgSQL blocks contain semicolons.
    client
        .simple_query(&clean)
        .await
        .expect("Martin table_source.sql should load as one batch");

    let count: i64 = client
        .query_one("SELECT count(*) FROM table_source", &[])
        .await
        .expect("count table_source rows")
        .get(0);
    assert_eq!(count, 28, "all Martin table_source rows should load");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires .tmp/ref/martin checkout"]
async fn martin_table_fixture_coverage_report() {
    let fixtures_dir = martin_table_fixture_path("");
    if !fixtures_dir.exists() {
        eprintln!("skipping: {} not found", fixtures_dir.display());
        return;
    }

    let mut fixtures: Vec<_> = std::fs::read_dir(&fixtures_dir)
        .expect("read Martin table fixtures dir")
        .map(|entry| entry.expect("read fixture entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "sql"))
        .collect();
    fixtures.sort();

    let server = ServerHandle::start().await;
    let (client, connection) = tokio_postgres::connect(&server.conn_str(), NoTls)
        .await
        .expect("connect");
    tokio::spawn(connection);

    let mut passed = Vec::new();
    let mut failed = Vec::new();
    for path in fixtures {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let sql = std::fs::read_to_string(&path).expect("read Martin table fixture");
        let sql = strip_line_comments(&sql);
        match client.simple_query(&sql).await {
            Ok(_) => passed.push(name),
            Err(e) => failed.push((name, format!("{e:?}"))),
        }
    }

    eprintln!(
        "Martin table fixture coverage: {}/{} passed",
        passed.len(),
        passed.len() + failed.len()
    );
    for name in &passed {
        eprintln!("  PASS {name}");
    }
    for (name, error) in &failed {
        eprintln!("  FAIL {name}: {error}");
    }
}
