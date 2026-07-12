// SPDX-License-Identifier: Apache-2.0
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use futures::StreamExt;
use quackgis_server::duckdb_adbc_storage::{DuckDbAdbcConfig, DuckDbAdbcStorage, ExtensionPolicy};
use quackgis_server::pgwire_server::ServerOptions;
use serde_json::json;

mod support;
use support::evidence::{EvidenceEnvelope, EvidenceLevel, EvidenceProfile, ExecutionEnvironment};

const MIB: u64 = 1024 * 1024;

#[tokio::test]
#[ignore = "requires the pinned DuckDB ADBC runtime"]
async fn result_stream_profile() {
    let profile = ResultStreamProfile::from_environment();
    let output_path = std::env::var_os("QUACKGIS_PROFILE_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|| ".tmp/duckdb-result-stream/manifest.json".into());
    let temp = tempfile::tempdir().expect("profile tempdir");
    let catalog_path = temp.path().join("catalog.ducklake");
    let data_path = temp.path().join("data");
    std::fs::create_dir(&data_path).expect("profile data path");
    let storage = Arc::new(
        DuckDbAdbcStorage::open(DuckDbAdbcConfig {
            driver_path: std::env::var_os("QUACKGIS_DUCKDB_ADBC_DRIVER")
                .expect("set ADBC driver")
                .into(),
            database_uri: ":memory:".to_owned(),
            ducklake_uri: format!("ducklake:{}", catalog_path.display()),
            catalog_name: "quackgis".to_owned(),
            data_path: data_path.display().to_string(),
            extension_policy: ExtensionPolicy::LoadOnly,
        })
        .expect("profile storage"),
    );
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("profile listener");
    let port = listener.local_addr().expect("profile address").port();
    let server_storage = Arc::clone(&storage);
    let options = ServerOptions::new()
        .with_max_connections(4)
        .with_result_batch_bytes(8 * 1024 * 1024);
    let server = tokio::spawn(async move {
        quackgis_server::pgwire_server::serve_duckdb_on_listener(
            server_storage,
            listener,
            &options,
            quackgis_server::auth::AuthConfig::trust(),
        )
        .await
    });
    let (client, connection) = tokio_postgres::connect(
        &format!("host=127.0.0.1 port={port} user=postgres dbname=quackgis"),
        tokio_postgres::NoTls,
    )
    .await
    .expect("profile pgwire connection");
    let connection = tokio::spawn(connection);
    client
        .query_one("SELECT 1::INTEGER", &[])
        .await
        .expect("warm profile connection");
    let idle_rss = process_rss_bytes().expect("Linux process RSS");
    let peak_rss = Arc::new(AtomicU64::new(idle_rss));
    let sampling = Arc::new(AtomicBool::new(true));
    let sampler_peak = Arc::clone(&peak_rss);
    let sampler_sampling = Arc::clone(&sampling);
    let sampler = tokio::spawn(async move {
        while sampler_sampling.load(Ordering::Acquire) {
            if let Some(rss) = process_rss_bytes() {
                sampler_peak.fetch_max(rss, Ordering::Relaxed);
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    });

    let started = Instant::now();
    let rows = client
        .query_raw(
            &format!(
                "SELECT i::BIGINT FROM range({}) AS profile_rows(i)",
                profile.rows
            ),
            std::iter::empty::<&i32>(),
        )
        .await
        .expect("open streaming profile query");
    futures::pin_mut!(rows);
    let first = rows
        .next()
        .await
        .expect("streaming profile first row")
        .expect("streaming profile first result");
    let first_row_ms = started.elapsed().as_secs_f64() * 1000.0;
    let first_value = first.get::<_, i64>(0);
    let mut count = 1_u64;
    let mut sum = first_value as i128;
    while let Some(row) = rows.next().await {
        let value = row.expect("streaming profile row").get::<_, i64>(0);
        count += 1;
        sum += i128::from(value);
    }
    let total_ms = started.elapsed().as_secs_f64() * 1000.0;
    sampling.store(false, Ordering::Release);
    sampler.await.expect("RSS sampler");
    let peak_rss = peak_rss.load(Ordering::Relaxed);
    let rss_delta = peak_rss.saturating_sub(idle_rss);
    let expected_sum = i128::from(profile.rows) * i128::from(profile.rows - 1) / 2;
    let metrics = quackgis_server::metrics::render_prometheus(storage.lifecycle().as_ref());
    let batch_high_water = prometheus_u64(&metrics, "quackgis_query_batches_inflight_high_water")
        .expect("batch high-water metric");

    assert_eq!(first_value, 0);
    assert_eq!(count, profile.rows);
    assert_eq!(sum, expected_sum);
    assert!(first_row_ms < total_ms, "first row must precede completion");
    assert_eq!(batch_high_water, 1, "only one Arrow batch may be in flight");
    assert!(
        rss_delta <= profile.rss_budget_bytes,
        "RSS delta {} MiB exceeded {} MiB",
        rss_delta / MIB,
        profile.rss_budget_bytes / MIB
    );

    drop(client);
    connection.abort();
    server.abort();
    let evidence = EvidenceEnvelope::collect(
        EvidenceProfile::new(
            format!(
                "duckdb-result-stream-{}-r{}-v1",
                profile.level.as_str(),
                profile.rows
            ),
            profile.level,
            profile.environment,
            "single-client generated BIGINT result stream through pgwire; process RSS includes the in-process server and test client",
        ),
        json!({
            "rows": profile.rows,
            "logical_bytes": profile.rows * 8,
            "files": 0,
            "row_groups": 0,
        }),
        json!({
            "first_value": first_value,
            "row_count": count,
            "sum": sum.to_string(),
            "expected_sum": expected_sum.to_string(),
            "first_row_before_completion": first_row_ms < total_ms,
        }),
        json!({
            "idle_rss_bytes": idle_rss,
            "peak_rss_bytes": peak_rss,
            "rss_delta_bytes": rss_delta,
            "time_to_first_row_ms": first_row_ms,
            "total_ms": total_ms,
            "rows_per_second": profile.rows as f64 / (total_ms / 1000.0),
            "arrow_batches_inflight_high_water": batch_high_water,
            "rss_sample_interval_ms": 2,
        }),
        json!({
            "rss_delta_max_bytes": profile.rss_budget_bytes,
            "arrow_batches_inflight_high_water_max": 1,
            "first_row_before_completion": true,
        }),
    )
    .expect("collect result-stream evidence");
    evidence
        .write(&output_path)
        .expect("write result-stream evidence");
    println!(
        "duckdb_result_stream_profile_ok rows={} rss_delta_mib={} out={}",
        profile.rows,
        rss_delta / MIB,
        output_path.display()
    );
}

struct ResultStreamProfile {
    level: EvidenceLevel,
    environment: ExecutionEnvironment,
    rows: u64,
    rss_budget_bytes: u64,
}

impl ResultStreamProfile {
    fn from_environment() -> Self {
        let level = EvidenceLevel::parse(
            &std::env::var("QUACKGIS_EVIDENCE_LEVEL").unwrap_or_else(|_| "smoke".to_owned()),
        )
        .expect("valid evidence level");
        assert_ne!(
            level,
            EvidenceLevel::External,
            "external evidence is not local"
        );
        let environment = ExecutionEnvironment::parse(
            &std::env::var("QUACKGIS_EXECUTION_ENVIRONMENT")
                .unwrap_or_else(|_| "host_process".to_owned()),
        )
        .expect("valid execution environment");
        let rows = std::env::var("QUACKGIS_PROFILE_ROWS")
            .map(|value| value.parse::<u64>().expect("integer profile rows"))
            .unwrap_or(100_000);
        assert!(
            (1..=10_000_000).contains(&rows),
            "result profile rows must be between 1 and 10M"
        );
        let rss_budget_bytes = match level {
            EvidenceLevel::Smoke => 256 * MIB,
            EvidenceLevel::Local => 192 * MIB,
            EvidenceLevel::Reference => 128 * MIB,
            EvidenceLevel::External => unreachable!("external rejected above"),
        };
        Self {
            level,
            environment,
            rows,
            rss_budget_bytes,
        }
    }
}

fn process_rss_bytes() -> Option<u64> {
    let contents = std::fs::read_to_string("/proc/self/status").ok()?;
    let kib = contents.lines().find_map(|line| {
        let value = line.strip_prefix("VmRSS:")?;
        value.split_whitespace().next()?.parse::<u64>().ok()
    })?;
    kib.checked_mul(1024)
}

fn prometheus_u64(metrics: &str, name: &str) -> Option<u64> {
    metrics.lines().find_map(|line| {
        let (metric, value) = line.split_once(' ')?;
        (metric == name).then(|| value.parse().ok()).flatten()
    })
}

#[test]
fn parses_process_rss_and_prometheus_values() {
    assert!(process_rss_bytes().is_some_and(|rss| rss > 0));
    assert_eq!(prometheus_u64("metric 7\n", "metric"), Some(7));
    assert_eq!(prometheus_u64("metric 7\n", "missing"), None);
}
