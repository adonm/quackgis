// SPDX-License-Identifier: Apache-2.0
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use adbc_core::options::IngestMode;
use arrow_array::builder::BinaryBuilder;
use arrow_array::{
    Array, ArrayRef, BinaryArray, Float64Array, Int64Array, RecordBatch, RecordBatchReader,
    StringArray, UInt64Array,
};
use arrow_schema::{ArrowError, DataType, Field, Schema, SchemaRef};
use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use quackgis_server::auth::AuthConfig;
use quackgis_server::engine_api::{
    EngineMaintenanceRequest, EngineStorageKernel, EngineTableRef, IngestDisposition,
};
use quackgis_server::pgwire_server::ServerOptions;
use serde_json::json;
use sha2::{Digest, Sha256};

#[path = "support/runtime.rs"]
mod runtime;
mod support;
use runtime::TestRuntime;
use support::evidence::{EvidenceEnvelope, EvidenceLevel, EvidenceProfile, ExecutionEnvironment};

const MIB: u64 = 1024 * 1024;
const COPY_WKB: [u8; 21] = [
    1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];
const COPY_WKB_HEX: &str = "010100000000000000000000000000000000000000";
const COPY_PROFILE_CHUNK_BYTES: usize = 60 * 1024;
const SPATIAL_QUERY_SAMPLES: usize = 5;

#[tokio::test]
#[ignore = "requires the pinned DuckDB ADBC runtime"]
async fn result_stream_profile() {
    let profile = ResultStreamProfile::from_environment();
    let output_path = std::env::var_os("QUACKGIS_PROFILE_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|| ".tmp/duckdb-result-stream/manifest.json".into());
    let options = ServerOptions::new()
        .with_max_connections(4)
        .with_result_batch_bytes(8 * 1024 * 1024);
    let runtime = TestRuntime::start(options).await;
    let (client, connection) = runtime.connect().await;
    client
        .query_one("SELECT 1::INTEGER", &[])
        .await
        .expect("warm profile connection");
    let sampler = RssSampler::start();

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
    let rss = sampler.finish().await;
    let expected_sum = i128::from(profile.rows) * i128::from(profile.rows - 1) / 2;
    let metrics =
        quackgis_server::metrics::render_prometheus(runtime.storage().lifecycle().as_ref());
    let batch_high_water = prometheus_u64(&metrics, "quackgis_query_batches_inflight_high_water")
        .expect("batch high-water metric");

    assert_eq!(first_value, 0);
    assert_eq!(count, profile.rows);
    assert_eq!(sum, expected_sum);
    assert!(first_row_ms < total_ms, "first row must precede completion");
    assert_eq!(batch_high_water, 1, "only one Arrow batch may be in flight");
    assert!(
        rss.delta <= profile.rss_budget_bytes,
        "RSS delta {} MiB exceeded {} MiB",
        rss.delta / MIB,
        profile.rss_budget_bytes / MIB
    );

    drop(client);
    connection.abort();
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
            "idle_rss_bytes": rss.idle,
            "peak_rss_bytes": rss.peak,
            "rss_delta_bytes": rss.delta,
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
        rss.delta / MIB,
        output_path.display()
    );
}

#[tokio::test]
#[ignore = "requires the pinned DuckDB ADBC runtime"]
async fn wide_result_stream_profile() {
    let profile = WideResultProfile::from_environment();
    let output_path = std::env::var_os("QUACKGIS_PROFILE_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|| ".tmp/duckdb-wide-result/manifest.json".into());
    let runtime = TestRuntime::start(
        ServerOptions::new()
            .with_max_connections(4)
            .with_result_batch_bytes(8 * 1024 * 1024),
    )
    .await;
    let (client, connection) = runtime.connect().await;
    client
        .query_one("SELECT 1::INTEGER", &[])
        .await
        .expect("warm wide-result connection");
    let baseline_metrics =
        quackgis_server::metrics::render_prometheus(runtime.storage().lifecycle().as_ref());
    let baseline_batches = prometheus_u64(&baseline_metrics, "quackgis_query_batches_total")
        .expect("baseline query batch metric");
    let sampler = RssSampler::start();
    let started = Instant::now();
    let rows = client
        .query_raw(
            &format!(
                "SELECT i::BIGINT, \
                 CASE WHEN i % 11 = 0 THEN NULL ELSE \
                   'row-' || i::VARCHAR || '-' || repeat('x', (i % {})::BIGINT) END, \
                 CASE WHEN i % 13 = 0 THEN NULL ELSE \
                   from_hex(repeat('ab', ((i % {}) + 1)::BIGINT)) END \
                 FROM range({}) AS wide_rows(i)",
                profile.max_text_bytes, profile.max_binary_bytes, profile.rows
            ),
            std::iter::empty::<&i32>(),
        )
        .await
        .expect("open wide-result profile query");
    futures::pin_mut!(rows);
    let mut count = 0_u64;
    let mut id_sum = 0_u128;
    let mut text_bytes = 0_u128;
    let mut binary_bytes = 0_u128;
    let mut text_nulls = 0_u64;
    let mut binary_nulls = 0_u64;
    let mut first_row_ms = None;
    while let Some(row) = rows.next().await {
        let row = row.expect("wide-result profile row");
        if first_row_ms.is_none() {
            first_row_ms = Some(started.elapsed().as_secs_f64() * 1000.0);
        }
        let expected_id = count;
        let id = row.get::<_, i64>(0);
        assert_eq!(id, expected_id as i64, "wide-result row order");
        id_sum += u128::from(expected_id);

        let label = row.get::<_, Option<String>>(1);
        if expected_id.is_multiple_of(11) {
            assert!(label.is_none(), "expected NULL label at row {expected_id}");
            text_nulls += 1;
        } else {
            let expected_label = format!(
                "row-{expected_id}-{}",
                "x".repeat((expected_id % profile.max_text_bytes) as usize)
            );
            let label = label.unwrap_or_else(|| panic!("missing label at row {expected_id}"));
            assert_eq!(label, expected_label, "label at row {expected_id}");
            text_bytes += label.len() as u128;
        }

        let payload = row.get::<_, Option<Vec<u8>>>(2);
        if expected_id.is_multiple_of(13) {
            assert!(
                payload.is_none(),
                "expected NULL payload at row {expected_id}"
            );
            binary_nulls += 1;
        } else {
            let payload =
                payload.unwrap_or_else(|| panic!("missing binary payload at row {expected_id}"));
            let expected_len = (expected_id % profile.max_binary_bytes + 1) as usize;
            assert_eq!(
                payload.len(),
                expected_len,
                "payload length at row {expected_id}"
            );
            assert!(
                payload.iter().all(|byte| *byte == 0xab),
                "payload bytes at row {expected_id}"
            );
            binary_bytes += payload.len() as u128;
        }
        count += 1;
    }
    let total_ms = started.elapsed().as_secs_f64() * 1000.0;
    let rss = sampler.finish().await;
    let first_row_ms = first_row_ms.expect("wide-result first row");
    let expected_sum = u128::from(profile.rows) * u128::from(profile.rows - 1) / 2;
    let metrics =
        quackgis_server::metrics::render_prometheus(runtime.storage().lifecycle().as_ref());
    let total_batches = prometheus_u64(&metrics, "quackgis_query_batches_total")
        .expect("wide-result query batch metric");
    let query_batches = total_batches.saturating_sub(baseline_batches);
    let batch_high_water = prometheus_u64(&metrics, "quackgis_query_batches_inflight_high_water")
        .expect("wide-result batch high-water metric");
    let batch_limit_rejections =
        prometheus_u64(&metrics, "quackgis_query_batch_limit_rejections_total")
            .expect("wide-result batch rejection metric");
    assert_eq!(count, profile.rows);
    assert_eq!(id_sum, expected_sum);
    assert!(first_row_ms < total_ms, "first row must precede completion");
    assert!(
        query_batches > 1,
        "wide result must cross native Arrow batches"
    );
    assert_eq!(batch_high_water, 1, "only one Arrow batch may be in flight");
    assert_eq!(batch_limit_rejections, 0);
    assert!(
        rss.delta <= profile.rss_budget_bytes,
        "wide-result RSS delta {} MiB exceeded {} MiB",
        rss.delta / MIB,
        profile.rss_budget_bytes / MIB
    );
    drop(client);
    connection.abort();

    let evidence = EvidenceEnvelope::collect(
        EvidenceProfile::new(
            format!(
                "duckdb-wide-result-{}-r{}-v1",
                profile.level.as_str(),
                profile.rows
            ),
            profile.level,
            profile.environment,
            "ordered BIGINT plus nullable variable-width VARCHAR/BLOB generated through pgwire; every row and byte pattern is checked",
        ),
        json!({
            "rows": profile.rows,
            "columns": 3,
            "max_text_payload_bytes": profile.max_text_bytes,
            "max_binary_payload_bytes": profile.max_binary_bytes,
            "files": 0,
            "row_groups": 0,
        }),
        json!({
            "row_count": count,
            "id_sum": id_sum.to_string(),
            "expected_id_sum": expected_sum.to_string(),
            "text_bytes": text_bytes.to_string(),
            "binary_bytes": binary_bytes.to_string(),
            "text_nulls": text_nulls,
            "binary_nulls": binary_nulls,
            "all_labels_exact": true,
            "all_binary_payloads_exact": true,
            "first_row_before_completion": true,
        }),
        json!({
            "idle_rss_bytes": rss.idle,
            "peak_rss_bytes": rss.peak,
            "rss_delta_bytes": rss.delta,
            "time_to_first_row_ms": first_row_ms,
            "total_ms": total_ms,
            "rows_per_second": profile.rows as f64 / (total_ms / 1000.0),
            "arrow_batches": query_batches,
            "arrow_batches_inflight_high_water": batch_high_water,
            "batch_limit_rejections": batch_limit_rejections,
            "rss_sample_interval_ms": 2,
        }),
        json!({
            "rss_delta_max_bytes": profile.rss_budget_bytes,
            "arrow_batches_min": 2,
            "arrow_batches_inflight_high_water_max": 1,
            "batch_limit_rejections_max": 0,
            "first_row_before_completion": true,
        }),
    )
    .expect("collect wide-result evidence");
    evidence
        .write(&output_path)
        .expect("write wide-result evidence");
    println!(
        "duckdb_wide_result_profile_ok rows={} batches={} rss_delta_mib={} out={}",
        profile.rows,
        query_batches,
        rss.delta / MIB,
        output_path.display()
    );
}

#[tokio::test]
#[ignore = "requires the pinned DuckDB ADBC runtime"]
async fn cancellation_profile() {
    let profile = CancellationProfile::from_environment();
    let output_path = std::env::var_os("QUACKGIS_PROFILE_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|| ".tmp/duckdb-cancellation/manifest.json".into());
    let runtime = TestRuntime::start(
        ServerOptions::new()
            .with_max_connections(8)
            .with_max_active_queries(4)
            .with_max_reader_queries(4)
            .with_max_blocking_workers(5)
            .with_statement_timeout(Duration::from_secs(30)),
    )
    .await;
    let mut latencies_ms = Vec::with_capacity(profile.iterations);
    let mut quarantined = 0_usize;
    for iteration in 0..profile.iterations {
        let (client, connection) = runtime.connect().await;
        let cancel = client.cancel_token();
        let rows = client
            .query_raw(
                "SELECT i::BIGINT FROM range(1000000000) AS cancel_rows(i)",
                std::iter::empty::<&i32>(),
            )
            .await
            .unwrap_or_else(|error| panic!("open cancellation sample {iteration}: {error}"));
        futures::pin_mut!(rows);
        rows.next()
            .await
            .unwrap_or_else(|| panic!("cancellation sample {iteration} has no first row"))
            .unwrap_or_else(|error| panic!("cancellation sample {iteration} first row: {error}"));
        let cancel_started = Instant::now();
        cancel
            .cancel_query(tokio_postgres::NoTls)
            .await
            .unwrap_or_else(|error| panic!("send cancellation sample {iteration}: {error}"));
        let error = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match rows.next().await {
                    Some(Ok(_)) => continue,
                    Some(Err(error)) => break error,
                    None => panic!("cancellation sample {iteration} completed"),
                }
            }
        })
        .await
        .unwrap_or_else(|_| panic!("cancellation sample {iteration} exceeded 5 seconds"));
        let latency_ms = cancel_started.elapsed().as_secs_f64() * 1000.0;
        assert_eq!(
            error.code(),
            Some(&tokio_postgres::error::SqlState::QUERY_CANCELED),
            "cancellation sample {iteration}"
        );
        let quarantine = client
            .query_one("SELECT 1::INTEGER", &[])
            .await
            .expect_err("cancelled session must be quarantined");
        assert_eq!(
            quarantine.code(),
            Some(&tokio_postgres::error::SqlState::INTERNAL_ERROR)
        );
        quarantined += 1;
        latencies_ms.push(latency_ms);
        drop(client);
        connection.abort();
    }
    let (fresh, fresh_connection) = runtime.connect().await;
    assert_eq!(
        fresh
            .query_one("SELECT 1::INTEGER", &[])
            .await
            .expect("fresh session after cancellation profile")
            .get::<_, i32>(0),
        1
    );
    drop(fresh);
    fresh_connection.abort();

    let summary = latency_summary(&latencies_ms);
    assert!(
        summary.p95_ms <= profile.p95_budget_ms,
        "cancellation p95 {:.3} ms exceeded {:.3} ms",
        summary.p95_ms,
        profile.p95_budget_ms
    );
    let metrics =
        quackgis_server::metrics::render_prometheus(runtime.storage().lifecycle().as_ref());
    let requested = prometheus_u64(&metrics, "quackgis_cancellations_requested_total")
        .expect("requested cancellations");
    let completed = prometheus_u64(&metrics, "quackgis_cancellations_completed_total")
        .expect("completed cancellations");
    let failed = prometheus_u64(&metrics, "quackgis_cancellations_failed_total")
        .expect("failed cancellations");
    assert_eq!(requested, profile.iterations as u64);
    assert_eq!(completed, profile.iterations as u64);
    assert_eq!(failed, 0);

    let evidence = EvidenceEnvelope::collect(
        EvidenceProfile::new(
            format!(
                "duckdb-cancellation-{}-n{}-v1",
                profile.level.as_str(),
                profile.iterations
            ),
            profile.level,
            profile.environment,
            "sequential long-query cancellation through pgwire; each cancelled session is explicitly quarantined and replaced",
        ),
        json!({
            "iterations": profile.iterations,
            "query_rows": 1_000_000_000_u64,
        }),
        json!({
            "query_canceled_sqlstate": "57014",
            "cancelled_sessions_quarantined": quarantined,
            "fresh_session_usable": true,
            "requested": requested,
            "completed": completed,
            "failed": failed,
        }),
        json!({
            "latencies_ms": latencies_ms,
            "min_ms": summary.min_ms,
            "p50_ms": summary.p50_ms,
            "p95_ms": summary.p95_ms,
            "p99_ms": summary.p99_ms,
            "max_ms": summary.max_ms,
        }),
        json!({
            "p95_max_ms": profile.p95_budget_ms,
            "failed_cancellations_max": 0,
            "quarantined_sessions": profile.iterations,
        }),
    )
    .expect("collect cancellation evidence");
    evidence
        .write(&output_path)
        .expect("write cancellation evidence");
    println!(
        "duckdb_cancellation_profile_ok iterations={} p95_ms={:.3} out={}",
        profile.iterations,
        summary.p95_ms,
        output_path.display()
    );
}

#[tokio::test]
#[ignore = "requires the pinned DuckDB ADBC runtime"]
async fn mixed_class_concurrency_profile() {
    const GLOBAL_LIMIT: usize = 3;
    const READER_LIMIT: usize = 2;
    const WRITER_LIMIT: usize = 1;
    const MAINTENANCE_LIMIT: usize = 1;

    let profile = MixedConcurrencyProfile::from_environment();
    let output_path = std::env::var_os("QUACKGIS_PROFILE_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|| ".tmp/duckdb-mixed-concurrency/manifest.json".into());
    let runtime = TestRuntime::start_with_auth(
        ServerOptions::new()
            .with_max_connections(12)
            .with_max_active_queries(GLOBAL_LIMIT)
            .with_max_reader_queries(READER_LIMIT)
            .with_max_writer_queries(WRITER_LIMIT)
            .with_max_maintenance_queries(MAINTENANCE_LIMIT)
            .with_max_queued_queries(8)
            .with_max_blocking_workers(8)
            .with_queue_timeout(Duration::from_secs(15)),
        AuthConfig::trust()
            .with_maintenance_user("postgres")
            .expect("maintenance identity"),
    )
    .await;
    runtime
        .storage()
        .execute_update(
            "CREATE TABLE quackgis.main.mixed_profile(id BIGINT, name VARCHAR); \
             INSERT INTO quackgis.main.mixed_profile VALUES (1, 'seed')",
        )
        .expect("seed mixed profile table");

    let mut holder_releases = Vec::new();
    let mut holder_tasks = Vec::new();
    let (entered_tx, mut entered_rx) = tokio::sync::mpsc::channel(GLOBAL_LIMIT);
    for id in 0..READER_LIMIT {
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        holder_releases.push(release_tx);
        let (mut client, connection) = runtime.connect().await;
        let entered_tx = entered_tx.clone();
        holder_tasks.push(tokio::spawn(async move {
            let transaction = client.transaction().await.expect("reader transaction");
            let statement = transaction
                .prepare("SELECT i::BIGINT FROM range(100000) AS mixed_rows(i)")
                .await
                .expect("reader statement");
            let portal = transaction
                .bind(&statement, &[])
                .await
                .expect("reader portal");
            let rows = transaction
                .query_portal(&portal, 1)
                .await
                .expect("reader first page");
            assert_eq!(rows[0].get::<_, i64>(0), 0);
            entered_tx.send(()).await.expect("reader entered");
            release_rx.await.expect("reader release");
            drop((portal, transaction));
            connection.abort();
            id
        }));
    }
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    holder_releases.push(release_tx);
    let (writer, writer_connection) = runtime.connect().await;
    let writer_entered = entered_tx.clone();
    holder_tasks.push(tokio::spawn(async move {
        let sink: tokio_postgres::CopyInSink<Bytes> = writer
            .copy_in("COPY quackgis.main.mixed_profile (id, name) FROM STDIN")
            .await
            .expect("writer COPY holder");
        let mut sink = Box::pin(sink);
        writer_entered.send(()).await.expect("writer entered");
        release_rx.await.expect("writer release");
        assert_eq!(sink.as_mut().finish().await.expect("finish empty COPY"), 0);
        drop(writer);
        writer_connection.abort();
        READER_LIMIT
    }));
    drop(entered_tx);
    for _ in 0..GLOBAL_LIMIT {
        tokio::time::timeout(Duration::from_secs(5), entered_rx.recv())
            .await
            .expect("holder admission timeout")
            .expect("holder admission");
    }
    assert_eq!(
        quackgis_server::execution_control::active_operations(),
        GLOBAL_LIMIT
    );

    let (queued_reader, queued_reader_connection) = runtime.connect().await;
    let reader_task = tokio::spawn(async move {
        let count = queued_reader
            .query_one(
                "SELECT count(*)::BIGINT FROM quackgis.main.mixed_profile",
                &[],
            )
            .await
            .expect("queued reader")
            .get::<_, i64>(0);
        queued_reader_connection.abort();
        count
    });
    let (queued_writer, queued_writer_connection) = runtime.connect().await;
    let writer_task = tokio::spawn(async move {
        queued_writer
            .batch_execute("BEGIN")
            .await
            .expect("queued writer begin");
        queued_writer
            .batch_execute("ROLLBACK")
            .await
            .expect("queued writer rollback");
        queued_writer_connection.abort();
    });
    let (maintenance, maintenance_connection) = runtime.connect().await;
    let maintenance_task = tokio::spawn(async move {
        maintenance
            .batch_execute(
                "CALL quackgis_merge_adjacent_files('public', 'mixed_profile', 8, 16777216, NULL)",
            )
            .await
            .expect("queued maintenance");
        maintenance_connection.abort();
    });

    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let metrics =
                quackgis_server::metrics::render_prometheus(runtime.storage().lifecycle().as_ref());
            if ["reader", "writer", "maintenance"].iter().all(|class| {
                prometheus_labeled_u64(&metrics, "quackgis_operations_class_queued", "class", class)
                    .is_some_and(|value| value >= 1)
            }) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("all operation classes queued");
    assert_eq!(
        quackgis_server::execution_control::active_operations(),
        GLOBAL_LIMIT,
        "configured global limit must remain saturated, never exceeded"
    );

    for release in holder_releases {
        release.send(()).expect("release holder");
    }
    for task in holder_tasks {
        task.await.expect("holder task");
    }
    assert_eq!(reader_task.await.expect("reader task"), 1);
    writer_task.await.expect("writer task");
    maintenance_task.await.expect("maintenance task");

    let metrics =
        quackgis_server::metrics::render_prometheus(runtime.storage().lifecycle().as_ref());
    let reader_high_water = prometheus_labeled_u64(
        &metrics,
        "quackgis_operations_class_high_water",
        "class",
        "reader",
    )
    .expect("reader high water");
    let writer_high_water = prometheus_labeled_u64(
        &metrics,
        "quackgis_operations_class_high_water",
        "class",
        "writer",
    )
    .expect("writer high water");
    let maintenance_high_water = prometheus_labeled_u64(
        &metrics,
        "quackgis_operations_class_high_water",
        "class",
        "maintenance",
    )
    .expect("maintenance high water");
    assert_eq!(reader_high_water, READER_LIMIT as u64);
    assert_eq!(writer_high_water, WRITER_LIMIT as u64);
    assert_eq!(maintenance_high_water, MAINTENANCE_LIMIT as u64);
    let rejected = prometheus_u64(&metrics, "quackgis_admission_rejected_total")
        .expect("admission rejection metric");
    let timed_out = prometheus_u64(&metrics, "quackgis_admission_queue_timeouts_total")
        .expect("admission timeout metric");
    assert_eq!(rejected, 0);
    assert_eq!(timed_out, 0);
    assert_eq!(quackgis_server::execution_control::active_operations(), 0);
    assert_eq!(quackgis_server::execution_control::queued_operations(), 0);

    let evidence = EvidenceEnvelope::collect(
        EvidenceProfile::new(
            format!("duckdb-mixed-concurrency-{}-v1", profile.level.as_str()),
            profile.level,
            profile.environment,
            "two retained pgwire reader portals and one retained pgwire COPY saturate global admission while reader, writer, and maintenance operations queue and then complete",
        ),
        json!({
            "clients": 6,
            "holder_readers": READER_LIMIT,
            "holder_writers": WRITER_LIMIT,
            "queued_readers": 1,
            "queued_writers": 1,
            "queued_maintenance": 1,
        }),
        json!({
            "reader_count": 1,
            "writer_transaction_completed": true,
            "maintenance_completed": true,
            "active_after_completion": 0,
            "queued_after_completion": 0,
        }),
        json!({
            "observed_global_active": GLOBAL_LIMIT,
            "reader_high_water": reader_high_water,
            "writer_high_water": writer_high_water,
            "maintenance_high_water": maintenance_high_water,
            "admission_rejections": rejected,
            "queue_timeouts": timed_out,
        }),
        json!({
            "global_active_max": GLOBAL_LIMIT,
            "reader_active_max": READER_LIMIT,
            "writer_active_max": WRITER_LIMIT,
            "maintenance_active_max": MAINTENANCE_LIMIT,
            "admission_rejections_max": 0,
            "queue_timeouts_max": 0,
        }),
    )
    .expect("collect mixed-concurrency evidence");
    evidence
        .write(&output_path)
        .expect("write mixed-concurrency evidence");
    println!(
        "duckdb_mixed_concurrency_profile_ok clients=6 active_limit={} out={}",
        GLOBAL_LIMIT,
        output_path.display()
    );
}

#[tokio::test]
#[ignore = "requires the pinned DuckDB ADBC runtime"]
async fn termination_atomicity_profile() {
    let profile = TerminationProfile::from_environment();
    let output_path = std::env::var_os("QUACKGIS_PROFILE_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|| ".tmp/duckdb-termination/manifest.json".into());
    let driver =
        PathBuf::from(std::env::var_os("QUACKGIS_DUCKDB_ADBC_DRIVER").expect("set ADBC driver"));
    let temp = tempfile::tempdir().expect("termination profile tempdir");
    let catalog = temp.path().join("catalog.ducklake");
    let data = temp.path().join("data");
    let first_port = unused_local_port().await;
    let mut first_server = spawn_profile_server(&driver, &catalog, &data, first_port);
    let (client, client_connection) = connect_profile_server(first_port, &mut first_server).await;
    client
        .batch_execute("CREATE TABLE quackgis.main.termination_profile(id BIGINT, name VARCHAR)")
        .await
        .expect("create termination table");
    client
        .batch_execute("INSERT INTO quackgis.main.termination_profile VALUES (1, 'committed')")
        .await
        .expect("commit baseline row");
    client
        .batch_execute("BEGIN")
        .await
        .expect("begin transaction");
    client
        .batch_execute("INSERT INTO quackgis.main.termination_profile VALUES (2, 'uncommitted')")
        .await
        .expect("insert uncommitted row");
    assert_eq!(
        client
            .query_one(
                "SELECT count(*)::BIGINT FROM quackgis.main.termination_profile",
                &[],
            )
            .await
            .expect("transaction sees own row")
            .get::<_, i64>(0),
        2
    );
    let (observer, observer_connection) = connect_profile_client(first_port).await;
    assert_eq!(
        observer
            .query_one(
                "SELECT count(*)::BIGINT FROM quackgis.main.termination_profile",
                &[],
            )
            .await
            .expect("observer isolation before termination")
            .get::<_, i64>(0),
        1
    );

    let termination_started = Instant::now();
    let signal_result = unsafe { libc::kill(first_server.0.id() as i32, libc::SIGTERM) };
    assert_eq!(signal_result, 0, "send termination signal");
    let first_status = wait_for_child(&mut first_server, Duration::from_secs(10)).await;
    let termination_ms = termination_started.elapsed().as_secs_f64() * 1000.0;
    assert!(first_status.success(), "first server exit: {first_status}");
    drop((client, observer));
    client_connection.abort();
    observer_connection.abort();

    let restart_started = Instant::now();
    let second_port = unused_local_port().await;
    let mut second_server = spawn_profile_server(&driver, &catalog, &data, second_port);
    let (restarted, restarted_connection) =
        connect_profile_server(second_port, &mut second_server).await;
    let restart_ms = restart_started.elapsed().as_secs_f64() * 1000.0;
    assert!(
        restart_ms <= profile.restart_budget_ms,
        "restart {restart_ms:.3} ms exceeded {:.3} ms",
        profile.restart_budget_ms
    );
    let recovered = restarted
        .query_one(
            "SELECT count(*)::BIGINT, sum(id)::BIGINT, \
             count(*) FILTER (WHERE id = 2)::BIGINT \
             FROM quackgis.main.termination_profile",
            &[],
        )
        .await
        .expect("recovered exact state");
    let recovered_count = recovered.get::<_, i64>(0);
    let recovered_sum = recovered.get::<_, i64>(1);
    let uncommitted_rows = recovered.get::<_, i64>(2);
    assert_eq!(
        (recovered_count, recovered_sum, uncommitted_rows),
        (1, 1, 0)
    );
    restarted
        .batch_execute("INSERT INTO quackgis.main.termination_profile VALUES (3, 'after-restart')")
        .await
        .expect("post-restart write");
    let final_state = restarted
        .query_one(
            "SELECT count(*)::BIGINT, sum(id)::BIGINT \
             FROM quackgis.main.termination_profile",
            &[],
        )
        .await
        .expect("post-restart exact state");
    let final_count = final_state.get::<_, i64>(0);
    let final_sum = final_state.get::<_, i64>(1);
    assert_eq!((final_count, final_sum), (2, 4));
    drop(restarted);
    restarted_connection.abort();
    let signal_result = unsafe { libc::kill(second_server.0.id() as i32, libc::SIGTERM) };
    assert_eq!(signal_result, 0, "stop restarted server");
    let second_status = wait_for_child(&mut second_server, Duration::from_secs(10)).await;
    assert!(
        second_status.success(),
        "second server exit: {second_status}"
    );

    let evidence = EvidenceEnvelope::collect(
        EvidenceProfile::new(
            format!("duckdb-termination-{}-v1", profile.level.as_str()),
            profile.level,
            profile.environment,
            "actual server process receives SIGTERM with one explicit uncommitted transaction, reaches its forced drain deadline, restarts on the same local DuckLake paths, and verifies exact committed state",
        ),
        json!({
            "baseline_rows": 1,
            "uncommitted_rows_attempted": 1,
            "shutdown_timeout_ms": 100,
        }),
        json!({
            "recovered_count": recovered_count,
            "recovered_sum": recovered_sum,
            "uncommitted_rows_visible": uncommitted_rows,
            "post_restart_write_succeeded": true,
            "final_count": final_count,
            "final_sum": final_sum,
        }),
        json!({
            "termination_ms": termination_ms,
            "restart_to_queryable_ms": restart_ms,
            "first_exit_success": first_status.success(),
            "second_exit_success": second_status.success(),
        }),
        json!({
            "restart_to_queryable_max_ms": profile.restart_budget_ms,
            "uncommitted_rows_visible_max": 0,
        }),
    )
    .expect("collect termination evidence");
    evidence
        .write(&output_path)
        .expect("write termination evidence");
    println!(
        "duckdb_termination_profile_ok termination_ms={:.3} restart_ms={:.3} out={}",
        termination_ms,
        restart_ms,
        output_path.display()
    );
}

#[tokio::test]
#[ignore = "requires the pinned DuckDB ADBC runtime"]
async fn tls_required_rotation_profile() {
    let profile = TlsRotationProfile::from_environment();
    let output_path = std::env::var_os("QUACKGIS_PROFILE_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|| ".tmp/duckdb-tls-rotation/manifest.json".into());
    let driver =
        PathBuf::from(std::env::var_os("QUACKGIS_DUCKDB_ADBC_DRIVER").expect("set ADBC driver"));
    let temp = tempfile::tempdir().expect("TLS rotation profile tempdir");
    let catalog = temp.path().join("catalog.ducklake");
    let data = temp.path().join("data");
    let cert_path = temp.path().join("server.pem");
    let key_path = temp.path().join("server.key");
    let first_identity = TestTlsIdentity::new();
    let second_identity = TestTlsIdentity::new();
    assert_ne!(first_identity.fingerprint, second_identity.fingerprint);
    first_identity.write(&cert_path, &key_path);

    let first_port = unused_local_port().await;
    let mut first_server = spawn_tls_profile_server(
        &driver,
        &catalog,
        &data,
        first_port,
        &cert_path,
        &key_path,
        "first-password",
    );
    let (first_client, first_connection) = connect_tls_profile_server(
        first_port,
        "first-password",
        &first_identity,
        &mut first_server,
    )
    .await;
    first_client
        .batch_execute("CREATE TABLE quackgis.main.tls_rotation_profile(id BIGINT, phase VARCHAR)")
        .await
        .expect("create baseline table through TLS and SCRAM");
    first_client
        .batch_execute("INSERT INTO quackgis.main.tls_rotation_profile VALUES (1, 'before')")
        .await
        .expect("write baseline through TLS and SCRAM");

    let plaintext_error = match tokio_postgres::connect(
        &format!(
            "host=127.0.0.1 port={first_port} user=postgres password=first-password \
             dbname=quackgis sslmode=disable"
        ),
        tokio_postgres::NoTls,
    )
    .await
    {
        Ok(_) => panic!("TLS-required server accepted plaintext startup"),
        Err(error) => error,
    };
    assert_eq!(
        plaintext_error.as_db_error().map(|error| error.code()),
        Some(&tokio_postgres::error::SqlState::INVALID_AUTHORIZATION_SPECIFICATION)
    );
    connect_tls_profile_client(first_port, "first-password", &second_identity)
        .await
        .expect_err("untrusted server certificate must fail");

    drop(first_client);
    first_connection.abort();
    let signal_result = unsafe { libc::kill(first_server.0.id() as i32, libc::SIGTERM) };
    assert_eq!(signal_result, 0, "stop first TLS server");
    assert!(
        wait_for_child(&mut first_server, Duration::from_secs(10))
            .await
            .success()
    );

    second_identity.write(&cert_path, &key_path);
    let rotation_started = Instant::now();
    let second_port = unused_local_port().await;
    let mut second_server = spawn_tls_profile_server(
        &driver,
        &catalog,
        &data,
        second_port,
        &cert_path,
        &key_path,
        "second-password",
    );
    let (second_client, second_connection) = connect_tls_profile_server(
        second_port,
        "second-password",
        &second_identity,
        &mut second_server,
    )
    .await;
    let rotation_to_queryable_ms = rotation_started.elapsed().as_secs_f64() * 1000.0;

    connect_tls_profile_client(second_port, "second-password", &first_identity)
        .await
        .expect_err("old certificate trust must fail after rotation");
    let old_password_error =
        connect_tls_profile_client(second_port, "first-password", &second_identity)
            .await
            .expect_err("old password must fail after rotation");
    assert_eq!(
        old_password_error.as_db_error().map(|error| error.code()),
        Some(&tokio_postgres::error::SqlState::INVALID_PASSWORD)
    );

    let recovered = second_client
        .query_one(
            "SELECT count(*)::BIGINT, sum(id)::BIGINT \
             FROM quackgis.main.tls_rotation_profile",
            &[],
        )
        .await
        .expect("read committed state after TLS rotation");
    assert_eq!(
        (recovered.get::<_, i64>(0), recovered.get::<_, i64>(1)),
        (1, 1)
    );
    second_client
        .batch_execute("INSERT INTO quackgis.main.tls_rotation_profile VALUES (2, 'after')")
        .await
        .expect("post-rotation write");
    let final_count = second_client
        .query_one(
            "SELECT count(*)::BIGINT FROM quackgis.main.tls_rotation_profile",
            &[],
        )
        .await
        .expect("post-rotation count")
        .get::<_, i64>(0);
    assert_eq!(final_count, 2);
    drop(second_client);
    second_connection.abort();
    let signal_result = unsafe { libc::kill(second_server.0.id() as i32, libc::SIGTERM) };
    assert_eq!(signal_result, 0, "stop rotated TLS server");
    assert!(
        wait_for_child(&mut second_server, Duration::from_secs(10))
            .await
            .success()
    );

    let evidence = EvidenceEnvelope::collect(
        EvidenceProfile::new(
            format!("duckdb-tls-rotation-{}-v1", profile.level.as_str()),
            profile.level,
            profile.environment,
            "actual server processes require TLS and SCRAM, reject plaintext and untrusted certificates, then restart on the same DuckLake paths with a replacement certificate and password",
        ),
        json!({
            "baseline_rows": 1,
            "rotation_mode": "process_restart",
            "server_name": "127.0.0.1",
            "first_certificate_sha256": first_identity.fingerprint,
            "second_certificate_sha256": second_identity.fingerprint,
        }),
        json!({
            "tls_scram_before_rotation": true,
            "plaintext_rejected_sqlstate": "28000",
            "untrusted_certificate_rejected": true,
            "old_certificate_trust_rejected": true,
            "old_password_rejected_sqlstate": "28P01",
            "committed_rows_preserved": 1,
            "post_rotation_write_succeeded": true,
            "final_count": final_count,
        }),
        json!({
            "rotation_to_queryable_ms": rotation_to_queryable_ms,
        }),
        json!({
            "plaintext_connections_allowed_max": 0,
            "old_credentials_allowed_max": 0,
            "committed_rows_lost_max": 0,
        }),
    )
    .expect("collect TLS rotation evidence");
    evidence
        .write(&output_path)
        .expect("write TLS rotation evidence");
    println!(
        "duckdb_tls_rotation_profile_ok rotation_ms={:.3} out={}",
        rotation_to_queryable_ms,
        output_path.display()
    );
}

#[tokio::test]
#[ignore = "requires the pinned DuckDB ADBC runtime"]
async fn copy_ingest_profile() {
    let profile = CopyProfile::from_environment();
    let output_path = std::env::var_os("QUACKGIS_PROFILE_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|| ".tmp/duckdb-copy/manifest.json".into());
    let runtime = TestRuntime::start(
        ServerOptions::new()
            .with_max_connections(4)
            .with_copy_batch_rows(8_192)
            .with_copy_batch_bytes(8 * 1024 * 1024)
            .with_copy_max_row_bytes(64 * 1024),
    )
    .await;
    for table in ["direct_copy_profile", "pgwire_copy_profile"] {
        runtime
            .storage()
            .execute_update(&format!(
                "CREATE TABLE quackgis.main.{table}(id BIGINT, name VARCHAR, geom_wkb BLOB)"
            ))
            .unwrap_or_else(|error| panic!("create {table}: {error}"));
    }

    let direct_schema = generated_copy_schema();
    let direct_reader = GeneratedBatchReader {
        schema: Arc::clone(&direct_schema),
        next_id: 0,
        rows: profile.rows,
        batch_rows: 8_192,
    };
    let direct_started = Instant::now();
    Arc::clone(runtime.storage())
        .start_ingest_operation()
        .expect("start direct ingest")
        .execute(
            &EngineTableRef {
                catalog: "quackgis".to_owned(),
                schema: "main".to_owned(),
                table: "direct_copy_profile".to_owned(),
            },
            Box::new(direct_reader),
            IngestDisposition::Append,
        )
        .expect("direct streaming ADBC ingest");
    let direct_ms = direct_started.elapsed().as_secs_f64() * 1000.0;

    let (client, connection) = runtime.connect().await;
    let sampler = RssSampler::start();
    let copy_started = Instant::now();
    let sink = client
        .copy_in("COPY quackgis.main.pgwire_copy_profile (id, name, geom_wkb) FROM STDIN")
        .await
        .expect("start pgwire COPY profile");
    let mut sink = Box::pin(sink);
    let mut next_id = 0_u64;
    let mut wire_bytes = 0_u64;
    while next_id < profile.rows {
        let (chunk, next) = copy_text_chunk(next_id, profile.rows, COPY_PROFILE_CHUNK_BYTES);
        next_id = next;
        wire_bytes += chunk.len() as u64;
        sink.as_mut()
            .send(Bytes::from(chunk))
            .await
            .expect("send COPY profile chunk");
    }
    let copied_rows = sink.as_mut().finish().await.expect("finish COPY profile");
    let pgwire_ms = copy_started.elapsed().as_secs_f64() * 1000.0;
    let rss = sampler.finish().await;
    assert_eq!(copied_rows, profile.rows);

    let expected_sum = u128::from(profile.rows) * u128::from(profile.rows - 1) / 2;
    for table in ["direct_copy_profile", "pgwire_copy_profile"] {
        let row = client
            .query_one(
                &format!(
                    "SELECT count(*)::BIGINT, sum(id)::HUGEINT::VARCHAR, \
                     min(hex(geom_wkb)), max(octet_length(geom_wkb))::BIGINT \
                     FROM quackgis.main.{table}"
                ),
                &[],
            )
            .await
            .unwrap_or_else(|error| panic!("verify {table}: {error}"));
        assert_eq!(row.get::<_, i64>(0), profile.rows as i64, "{table}");
        assert_eq!(row.get::<_, String>(1), expected_sum.to_string(), "{table}");
        assert_eq!(row.get::<_, String>(2), COPY_WKB_HEX, "{table}");
        assert_eq!(row.get::<_, i64>(3), 21, "{table}");
    }
    let direct_rows_per_second = profile.rows as f64 / (direct_ms / 1000.0);
    let pgwire_rows_per_second = profile.rows as f64 / (pgwire_ms / 1000.0);
    let throughput_ratio = pgwire_rows_per_second / direct_rows_per_second;
    assert!(
        rss.delta <= profile.rss_budget_bytes,
        "COPY RSS delta {} MiB exceeded {} MiB",
        rss.delta / MIB,
        profile.rss_budget_bytes / MIB
    );
    assert!(
        throughput_ratio >= profile.throughput_ratio_budget,
        "COPY/direct throughput ratio {throughput_ratio:.3} below {:.3}",
        profile.throughput_ratio_budget
    );
    let metrics =
        quackgis_server::metrics::render_prometheus(runtime.storage().lifecycle().as_ref());
    let copy_batches =
        prometheus_u64(&metrics, "quackgis_copy_batches_total").expect("COPY batch metric");
    assert!(copy_batches > 0);
    assert_eq!(
        prometheus_u64(&metrics, "quackgis_copy_rows_total"),
        Some(profile.rows)
    );
    assert_eq!(
        prometheus_u64(&metrics, "quackgis_copy_bytes_total"),
        Some(wire_bytes)
    );
    assert_eq!(
        prometheus_u64(&metrics, "quackgis_copy_completed_total"),
        Some(1)
    );
    assert_eq!(
        prometheus_u64(&metrics, "quackgis_copy_failed_total"),
        Some(0)
    );
    let commit_microseconds = prometheus_u64(&metrics, "quackgis_copy_commit_microseconds_total")
        .expect("COPY commit metric");
    drop(client);
    connection.abort();

    let evidence = EvidenceEnvelope::collect(
        EvidenceProfile::new(
            format!(
                "duckdb-copy-{}-r{}-v1",
                profile.level.as_str(),
                profile.rows
            ),
            profile.level,
            profile.environment,
            "generated id/name/WKB data through direct streaming ADBC and pgwire text COPY; process RSS sampled around pgwire COPY",
        ),
        json!({
            "rows": profile.rows,
            "wire_bytes": wire_bytes,
            "arrow_batch_rows": 8_192,
            "copy_chunk_max_bytes": COPY_PROFILE_CHUNK_BYTES,
            "wkb_bytes_per_row": 21,
        }),
        json!({
            "direct_count": profile.rows,
            "pgwire_count": profile.rows,
            "expected_sum": expected_sum.to_string(),
            "wkb_hex": COPY_WKB_HEX,
            "wkb_bytes": 21,
            "copy_completed": 1,
            "copy_failed": 0,
        }),
        json!({
            "direct_adbc_ms": direct_ms,
            "pgwire_copy_ms": pgwire_ms,
            "direct_rows_per_second": direct_rows_per_second,
            "pgwire_rows_per_second": pgwire_rows_per_second,
            "pgwire_to_direct_throughput_ratio": throughput_ratio,
            "idle_rss_bytes": rss.idle,
            "peak_rss_bytes": rss.peak,
            "rss_delta_bytes": rss.delta,
            "copy_batches": copy_batches,
            "copy_commit_ms": commit_microseconds as f64 / 1000.0,
            "rss_sample_interval_ms": 2,
        }),
        json!({
            "rss_delta_max_bytes": profile.rss_budget_bytes,
            "pgwire_to_direct_throughput_ratio_min": profile.throughput_ratio_budget,
            "copy_chunk_max_bytes": COPY_PROFILE_CHUNK_BYTES,
            "copy_failed_max": 0,
        }),
    )
    .expect("collect COPY evidence");
    evidence.write(&output_path).expect("write COPY evidence");
    println!(
        "duckdb_copy_profile_ok rows={} wire_mib={:.2} rss_delta_mib={} ratio={:.3} out={}",
        profile.rows,
        wire_bytes as f64 / MIB as f64,
        rss.delta / MIB,
        throughput_ratio,
        output_path.display()
    );
}

#[tokio::test]
#[ignore = "requires the pinned DuckDB ADBC runtime"]
async fn spatial_scan_profile() {
    let profile = SpatialScanProfile::from_environment();
    let output_path = std::env::var_os("QUACKGIS_PROFILE_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|| ".tmp/duckdb-spatial-scan/manifest.json".into());
    let runtime = TestRuntime::start(ServerOptions::new().with_max_connections(4)).await;
    runtime
        .storage()
        .execute_update(
            "CREATE TABLE quackgis.main.bbox_scan_profile(\
             id BIGINT, geom_wkb BLOB, _qg_minx DOUBLE, _qg_miny DOUBLE, \
             _qg_maxx DOUBLE, _qg_maxy DOUBLE); \
             CREATE TABLE quackgis.main.native_scan_profile(\
             id BIGINT, geom GEOMETRY)",
        )
        .expect("create spatial scan tables");

    let rows_per_file = profile.rows / profile.files;
    assert_eq!(rows_per_file * profile.files, profile.rows);
    let load_rss_sampler = RssSampler::start();
    let load_started = Instant::now();
    for file in 0..profile.files {
        let start = file * rows_per_file;
        let end = start + rows_per_file;
        runtime
            .storage()
            .ingest(
                "main",
                "bbox_scan_profile",
                vec![spatial_shape_batch(start, end)],
                IngestMode::Append,
            )
            .unwrap_or_else(|error| panic!("ingest bbox profile file {file}: {error}"));
        runtime
            .storage()
            .execute_update(&format!(
                "INSERT INTO quackgis.main.native_scan_profile \
                 SELECT i, CASE i % 3 \
                   WHEN 0 THEN ST_Point(i::DOUBLE, i::DOUBLE) \
                   WHEN 1 THEN ST_MakeLine([\
                     ST_Point(i::DOUBLE, i::DOUBLE), \
                     ST_Point(i::DOUBLE + 0.25, i::DOUBLE)]) \
                   ELSE ST_MakeEnvelope(\
                     i::DOUBLE, i::DOUBLE, \
                     i::DOUBLE + 0.25, i::DOUBLE + 0.25) \
                 END::GEOMETRY \
                 FROM range({start}, {end}) AS profile_shapes(i)"
            ))
            .unwrap_or_else(|error| panic!("ingest native profile file {file}: {error}"));
    }
    let load_ms = load_started.elapsed().as_secs_f64() * 1000.0;
    let load_rss = load_rss_sampler.finish().await;
    assert!(
        load_ms <= profile.load_duration_budget_ms,
        "spatial load {load_ms:.2} ms exceeded {:.2} ms",
        profile.load_duration_budget_ms
    );
    assert!(
        load_rss.delta <= profile.load_rss_budget_bytes,
        "spatial load RSS delta {} MiB exceeded {} MiB",
        load_rss.delta / MIB,
        profile.load_rss_budget_bytes / MIB
    );
    let bbox_files_before = table_files(runtime.storage(), "bbox_scan_profile");
    let native_files_before = table_files(runtime.storage(), "native_scan_profile");
    assert_eq!(bbox_files_before.len() as u64, profile.files);
    assert_eq!(native_files_before.len() as u64, profile.files);
    let bbox_row_group_bytes = parquet_row_group_bytes(runtime.storage(), &bbox_files_before);
    let native_row_group_bytes = parquet_row_group_bytes(runtime.storage(), &native_files_before);

    let (client, connection) = runtime.connect().await;
    let probe_max = rows_per_file.min(100).saturating_sub(1);
    let expected_count = probe_max + 1;
    let probe = format!("ST_MakeEnvelope(0, 0, {probe_max}, {probe_max})");
    let bbox_exact = format!("ST_Intersects(ST_GeomFromWKB(geom_wkb), {probe})");
    let bbox_query = format!(
        "SELECT count(*)::BIGINT FROM quackgis.main.bbox_scan_profile \
         WHERE {bbox_exact}"
    );
    let bbox_unpruned_query = format!(
        "SELECT count(*)::BIGINT FROM quackgis.main.bbox_scan_profile \
         WHERE ({bbox_exact}) IS TRUE"
    );
    let native_exact = format!("ST_Intersects(geom, {probe})");
    let native_query = format!(
        "SELECT count(*)::BIGINT FROM quackgis.main.native_scan_profile \
         WHERE ST_Intersects_Extent(geom, {probe}) AND {native_exact}"
    );
    let native_unpruned_query = format!(
        "SELECT count(*)::BIGINT FROM quackgis.main.native_scan_profile \
         WHERE ({native_exact}) IS TRUE"
    );
    for (label, query) in [
        ("bbox injected", bbox_query.as_str()),
        ("bbox exact", bbox_unpruned_query.as_str()),
        ("native stats", native_query.as_str()),
        ("native exact", native_unpruned_query.as_str()),
    ] {
        let count = client
            .query_one(query, &[])
            .await
            .unwrap_or_else(|error| panic!("{label} count: {error:?}"))
            .get::<_, i64>(0);
        assert_eq!(count, expected_count as i64, "{label} exact count");
    }

    runtime
        .storage()
        .execute_update(
            "CALL enable_profiling(\
             format := 'json', \
             metrics := ['EXTRA_INFO', 'OPERATOR_ROW_GROUPS_SCANNED', \
                         'OPERATOR_TOTAL_ROW_GROUPS_TO_SCAN'])",
        )
        .expect("enable spatial scan profiling metrics");
    let bbox_unpruned_plan = analyze_query(runtime.storage(), &bbox_unpruned_query);
    let bbox_plan = analyze_query(runtime.storage(), &bbox_query);
    let native_unpruned_plan = analyze_query(runtime.storage(), &native_unpruned_query);
    let native_plan = analyze_query(runtime.storage(), &native_query);
    runtime
        .storage()
        .execute_update("PRAGMA disable_profiling")
        .expect("disable spatial scan profiling");

    let bbox_unpruned = scan_metrics(&bbox_unpruned_plan);
    let bbox = scan_metrics(&bbox_plan);
    let native_unpruned = scan_metrics(&native_unpruned_plan);
    let native = scan_metrics(&native_plan);
    let bbox_plan_text = serde_json::to_string(&bbox_plan)
        .expect("serialize bbox plan")
        .to_ascii_lowercase();
    let native_plan_text = serde_json::to_string(&native_plan)
        .expect("serialize native plan")
        .to_ascii_lowercase();
    assert!(
        bbox_plan_text.contains("_qg_minx"),
        "bbox plan omitted candidate: {bbox_plan_text}"
    );
    assert!(bbox_plan_text.contains("st_intersects"));
    assert!(
        native_plan_text.contains("st_intersects_extent"),
        "native plan omitted extent candidate: {native_plan_text}"
    );
    assert!(native_plan_text.contains("st_intersects"));
    assert_scan_budget("maintained bbox", bbox, bbox_unpruned);
    assert_scan_budget("native geometry", native, native_unpruned);
    let bbox_bytes = scan_byte_metrics(&bbox_row_group_bytes, bbox, bbox_unpruned);
    let native_bytes = scan_byte_metrics(&native_row_group_bytes, native, native_unpruned);

    let rss_sampler = RssSampler::start();
    let resource_sampler = ResourceSampler::start(Arc::clone(runtime.storage()));
    let bbox_latency = pgwire_count_latency(
        &client,
        "bbox selective latency",
        &bbox_query,
        expected_count,
        SPATIAL_QUERY_SAMPLES,
    )
    .await;
    let native_latency = pgwire_count_latency(
        &client,
        "native selective latency",
        &native_query,
        expected_count,
        SPATIAL_QUERY_SAMPLES,
    )
    .await;
    let resources = resource_sampler.finish().await;
    let query_rss = rss_sampler.finish().await;
    assert_latency_budget("bbox", &bbox_latency, &profile);
    assert_latency_budget("native", &native_latency, &profile);
    assert!(
        query_rss.delta <= profile.query_rss_budget_bytes,
        "spatial query RSS delta {} MiB exceeded {} MiB",
        query_rss.delta / MIB,
        profile.query_rss_budget_bytes / MIB
    );
    assert!(
        resources.memory_delta <= profile.duckdb_memory_budget_bytes,
        "DuckDB memory delta {} MiB exceeded {} MiB",
        resources.memory_delta / MIB,
        profile.duckdb_memory_budget_bytes / MIB
    );
    assert!(
        resources.temporary_storage_peak <= profile.spill_budget_bytes,
        "DuckDB temporary storage {} bytes exceeded {} bytes",
        resources.temporary_storage_peak,
        profile.spill_budget_bytes
    );
    assert_eq!(resources.failures, 0, "DuckDB resource samples failed");

    for table in ["bbox_scan_profile", "native_scan_profile"] {
        runtime
            .storage()
            .maintain(EngineMaintenanceRequest::MergeAdjacentFiles {
                schema: "main".to_owned(),
                table: table.to_owned(),
                max_compacted_files: Some(profile.files),
                max_file_size: Some(1_073_741_824),
                min_file_size: None,
            })
            .unwrap_or_else(|error| panic!("compact {table}: {error}"));
    }
    let bbox_files_after = table_files(runtime.storage(), "bbox_scan_profile").len() as u64;
    let native_files_after = table_files(runtime.storage(), "native_scan_profile").len() as u64;
    assert!(
        bbox_files_after * 2 <= profile.files,
        "bbox compaction did not halve file count"
    );
    assert!(
        native_files_after * 2 <= profile.files,
        "native compaction did not halve file count"
    );
    for (label, query) in [
        ("compacted bbox", bbox_query.as_str()),
        ("compacted native", native_query.as_str()),
    ] {
        let count = client
            .query_one(query, &[])
            .await
            .unwrap_or_else(|error| panic!("{label} count: {error:?}"))
            .get::<_, i64>(0);
        assert_eq!(count, expected_count as i64, "{label} exact count");
    }

    drop(client);
    connection.abort();
    let bbox_ratio = bbox.scanned as f64 / bbox_unpruned.total as f64;
    let native_ratio = native.scanned as f64 / native_unpruned.total as f64;
    let bbox_improvement = bbox_unpruned.scanned as f64 / bbox.scanned as f64;
    let native_improvement = native_unpruned.scanned as f64 / native.scanned as f64;
    let evidence = EvidenceEnvelope::collect(
        EvidenceProfile::new(
            format!(
                "duckdb-spatial-scan-{}-r{}-v4",
                profile.level.as_str(),
                profile.rows
            ),
            profile.level,
            profile.environment,
            "ordered point, linestring, and polygon data in official DuckLake Parquet files; exact counts and timed selective samples run through pgwire while DuckDB row-group metrics compare exact-only scans with maintained WKB/bbox and native GEOMETRY candidates",
        ),
        json!({
            "rows_per_layout": profile.rows,
            "point_rows_per_layout": profile_shape_count(profile.rows, 0),
            "linestring_rows_per_layout": profile_shape_count(profile.rows, 1),
            "polygon_rows_per_layout": profile_shape_count(profile.rows, 2),
            "geometry_families": ["POINT", "LINESTRING", "POLYGON"],
            "files_per_layout": profile.files,
            "rows_per_file": rows_per_file,
            "probe_max_coordinate": probe_max,
            "bbox_row_groups": bbox_unpruned.total,
            "native_row_groups": native_unpruned.total,
            "bbox_file_bytes": bbox_files_before.iter().map(|file| file.bytes).sum::<u64>(),
            "native_file_bytes": native_files_before.iter().map(|file| file.bytes).sum::<u64>(),
        }),
        json!({
            "expected_count": expected_count,
            "bbox_injected_count": expected_count,
            "bbox_exact_count": expected_count,
            "native_stats_count": expected_count,
            "native_exact_count": expected_count,
            "bbox_plan_has_candidate": true,
            "bbox_plan_has_exact_recheck": true,
            "native_plan_has_candidate": true,
            "native_plan_has_exact_recheck": true,
            "bbox_compacted_count": expected_count,
            "native_compacted_count": expected_count,
        }),
        json!({
            "load": {
                "duration_ms": load_ms,
                "idle_rss_bytes": load_rss.idle,
                "peak_rss_bytes": load_rss.peak,
                "rss_delta_bytes": load_rss.delta,
            },
            "bbox_scan": {
                "row_groups_scanned": bbox.scanned,
                "row_groups_dispatched": bbox.total,
                "total_row_groups": bbox_unpruned.total,
                "scan_ratio": bbox_ratio,
                "row_group_improvement": bbox_improvement,
                "compressed_row_group_bytes": bbox_bytes.total,
                "scanned_bytes_upper_bound": bbox_bytes.scanned_upper_bound,
                "scan_byte_ratio_upper_bound": bbox_bytes.ratio_upper_bound,
            },
            "native_scan": {
                "row_groups_scanned": native.scanned,
                "row_groups_dispatched": native.total,
                "total_row_groups": native_unpruned.total,
                "scan_ratio": native_ratio,
                "row_group_improvement": native_improvement,
                "compressed_row_group_bytes": native_bytes.total,
                "scanned_bytes_upper_bound": native_bytes.scanned_upper_bound,
                "scan_byte_ratio_upper_bound": native_bytes.ratio_upper_bound,
            },
            "query_samples_per_layout": SPATIAL_QUERY_SAMPLES,
            "bbox_query_latency_ms": {
                "min": bbox_latency.min_ms,
                "p50": bbox_latency.p50_ms,
                "p95": bbox_latency.p95_ms,
                "p99": bbox_latency.p99_ms,
                "max": bbox_latency.max_ms,
            },
            "native_query_latency_ms": {
                "min": native_latency.min_ms,
                "p50": native_latency.p50_ms,
                "p95": native_latency.p95_ms,
                "p99": native_latency.p99_ms,
                "max": native_latency.max_ms,
            },
            "query_resources": {
                "idle_rss_bytes": query_rss.idle,
                "peak_rss_bytes": query_rss.peak,
                "rss_delta_bytes": query_rss.delta,
                "duckdb_memory_initial_bytes": resources.memory_initial,
                "duckdb_memory_peak_bytes": resources.memory_peak,
                "duckdb_memory_delta_bytes": resources.memory_delta,
                "duckdb_temporary_storage_peak_bytes": resources.temporary_storage_peak,
                "duckdb_samples": resources.samples,
                "duckdb_sample_failures": resources.failures,
            },
            "compaction": {
                "bbox_files_before": profile.files,
                "bbox_files_after": bbox_files_after,
                "native_files_before": profile.files,
                "native_files_after": native_files_after,
            },
        }),
        json!({
            "scan_ratio_max": 0.05,
            "scan_byte_ratio_upper_bound_max": 0.05,
            "row_group_improvement_min": 20.0,
            "compaction_file_reduction_min": 2.0,
            "exact_recheck_required": true,
            "load_duration_max_ms": profile.load_duration_budget_ms,
            "load_rss_delta_max_bytes": profile.load_rss_budget_bytes,
            "query_samples_per_layout": SPATIAL_QUERY_SAMPLES,
            "query_latency_p50_max_ms": profile.query_p50_budget_ms,
            "query_latency_p95_max_ms": profile.query_p95_budget_ms,
            "query_latency_p99_max_ms": profile.query_p99_budget_ms,
            "query_rss_delta_max_bytes": profile.query_rss_budget_bytes,
            "duckdb_memory_delta_max_bytes": profile.duckdb_memory_budget_bytes,
            "duckdb_temporary_storage_max_bytes": profile.spill_budget_bytes,
            "reference_rows": [10_000_000_u64, 100_000_000_u64],
            "required_consecutive_runs": 2,
        }),
    )
    .expect("collect spatial scan evidence");
    evidence
        .write(&output_path)
        .expect("write spatial scan evidence");
    println!(
        "duckdb_spatial_scan_profile_ok rows={} bbox={}/{} native={}/{} bbox_p95_ms={:.2} native_p95_ms={:.2} rss_delta_mib={} out={}",
        profile.rows,
        bbox.scanned,
        bbox_unpruned.total,
        native.scanned,
        native_unpruned.total,
        bbox_latency.p95_ms,
        native_latency.p95_ms,
        query_rss.delta / MIB,
        output_path.display()
    );
}

struct ResultStreamProfile {
    level: EvidenceLevel,
    environment: ExecutionEnvironment,
    rows: u64,
    rss_budget_bytes: u64,
}

struct WideResultProfile {
    level: EvidenceLevel,
    environment: ExecutionEnvironment,
    rows: u64,
    max_text_bytes: u64,
    max_binary_bytes: u64,
    rss_budget_bytes: u64,
}

impl WideResultProfile {
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
        let default_rows = match level {
            EvidenceLevel::Smoke => 10_000,
            EvidenceLevel::Local => 100_000,
            EvidenceLevel::Reference => 1_000_000,
            EvidenceLevel::External => unreachable!("external rejected above"),
        };
        let rows = std::env::var("QUACKGIS_PROFILE_ROWS")
            .map(|value| value.parse::<u64>().expect("integer profile rows"))
            .unwrap_or(default_rows);
        assert!(
            (10_000..=1_000_000).contains(&rows),
            "wide-result rows must be between 10k and 1M"
        );
        if level == EvidenceLevel::Reference {
            assert_eq!(
                rows, 1_000_000,
                "reference wide-result profile requires 1M rows"
            );
        }
        let rss_budget_bytes = match level {
            EvidenceLevel::Smoke => 384 * MIB,
            EvidenceLevel::Local => 320 * MIB,
            EvidenceLevel::Reference => 256 * MIB,
            EvidenceLevel::External => unreachable!("external rejected above"),
        };
        Self {
            level,
            environment,
            rows,
            max_text_bytes: 256,
            max_binary_bytes: 128,
            rss_budget_bytes,
        }
    }
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

struct CancellationProfile {
    level: EvidenceLevel,
    environment: ExecutionEnvironment,
    iterations: usize,
    p95_budget_ms: f64,
}

struct CopyProfile {
    level: EvidenceLevel,
    environment: ExecutionEnvironment,
    rows: u64,
    rss_budget_bytes: u64,
    throughput_ratio_budget: f64,
}

struct MixedConcurrencyProfile {
    level: EvidenceLevel,
    environment: ExecutionEnvironment,
}

struct TerminationProfile {
    level: EvidenceLevel,
    environment: ExecutionEnvironment,
    restart_budget_ms: f64,
}

struct TlsRotationProfile {
    level: EvidenceLevel,
    environment: ExecutionEnvironment,
}

struct SpatialScanProfile {
    level: EvidenceLevel,
    environment: ExecutionEnvironment,
    rows: u64,
    files: u64,
    load_duration_budget_ms: f64,
    load_rss_budget_bytes: u64,
    query_p50_budget_ms: f64,
    query_p95_budget_ms: f64,
    query_p99_budget_ms: f64,
    query_rss_budget_bytes: u64,
    duckdb_memory_budget_bytes: u64,
    spill_budget_bytes: u64,
}

impl SpatialScanProfile {
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
        let default_rows = match level {
            EvidenceLevel::Smoke => 100_000,
            EvidenceLevel::Local => 1_000_000,
            EvidenceLevel::Reference => 10_000_000,
            EvidenceLevel::External => unreachable!("external rejected above"),
        };
        let rows = std::env::var("QUACKGIS_PROFILE_ROWS")
            .map(|value| value.parse::<u64>().expect("integer profile rows"))
            .unwrap_or(default_rows);
        assert!(
            (100_000..=100_000_000).contains(&rows),
            "spatial scan rows must be between 100k and 100M"
        );
        if level == EvidenceLevel::Reference {
            assert!(
                matches!(rows, 10_000_000 | 100_000_000),
                "reference spatial scan profile requires 10M or 100M rows"
            );
        }
        let files = 25;
        assert_eq!(
            rows % files,
            0,
            "spatial scan rows must divide into 25 files"
        );
        Self {
            level,
            environment,
            rows,
            files,
            load_duration_budget_ms: match level {
                EvidenceLevel::Smoke => 30_000.0,
                EvidenceLevel::Local => 60_000.0,
                EvidenceLevel::Reference => 120_000.0,
                EvidenceLevel::External => unreachable!("external rejected above"),
            },
            load_rss_budget_bytes: match level {
                EvidenceLevel::Smoke => 512 * MIB,
                EvidenceLevel::Local => 768 * MIB,
                EvidenceLevel::Reference => 1_024 * MIB,
                EvidenceLevel::External => unreachable!("external rejected above"),
            },
            query_p50_budget_ms: 250.0,
            query_p95_budget_ms: 500.0,
            query_p99_budget_ms: 750.0,
            query_rss_budget_bytes: 128 * MIB,
            duckdb_memory_budget_bytes: 256 * MIB,
            spill_budget_bytes: 0,
        }
    }
}

impl TlsRotationProfile {
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
        Self { level, environment }
    }
}

impl TerminationProfile {
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
        Self {
            level,
            environment,
            restart_budget_ms: 60_000.0,
        }
    }
}

impl MixedConcurrencyProfile {
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
        Self { level, environment }
    }
}

impl CopyProfile {
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
        let default_rows = match level {
            EvidenceLevel::Smoke => 10_000,
            EvidenceLevel::Local => 1_000_000,
            EvidenceLevel::Reference => 10_000_000,
            EvidenceLevel::External => unreachable!("external rejected above"),
        };
        let rows = std::env::var("QUACKGIS_PROFILE_ROWS")
            .map(|value| value.parse::<u64>().expect("integer profile rows"))
            .unwrap_or(default_rows);
        assert!(
            (1..=10_000_000).contains(&rows),
            "COPY profile rows must be between 1 and 10M"
        );
        if level == EvidenceLevel::Reference {
            assert_eq!(rows, 10_000_000, "reference COPY profile requires 10M rows");
        }
        let (rss_budget_bytes, throughput_ratio_budget) = match level {
            EvidenceLevel::Smoke => (512 * MIB, 0.10),
            EvidenceLevel::Local => (384 * MIB, 0.25),
            EvidenceLevel::Reference => (256 * MIB, 0.50),
            EvidenceLevel::External => unreachable!("external rejected above"),
        };
        Self {
            level,
            environment,
            rows,
            rss_budget_bytes,
            throughput_ratio_budget,
        }
    }
}

impl CancellationProfile {
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
        let default_iterations = match level {
            EvidenceLevel::Smoke => 5,
            EvidenceLevel::Local => 25,
            EvidenceLevel::Reference => 100,
            EvidenceLevel::External => unreachable!("external rejected above"),
        };
        let iterations = std::env::var("QUACKGIS_PROFILE_ITERATIONS")
            .map(|value| value.parse::<usize>().expect("integer profile iterations"))
            .unwrap_or(default_iterations);
        assert!(
            (1..=100).contains(&iterations),
            "cancellation iterations must be between 1 and 100"
        );
        if level == EvidenceLevel::Reference {
            assert_eq!(
                iterations, 100,
                "reference cancellation profile requires 100 samples"
            );
        }
        let p95_budget_ms = match level {
            EvidenceLevel::Smoke => 2_000.0,
            EvidenceLevel::Local => 1_000.0,
            EvidenceLevel::Reference => 500.0,
            EvidenceLevel::External => unreachable!("external rejected above"),
        };
        Self {
            level,
            environment,
            iterations,
            p95_budget_ms,
        }
    }
}

struct LatencySummary {
    min_ms: f64,
    p50_ms: f64,
    p95_ms: f64,
    p99_ms: f64,
    max_ms: f64,
}

fn latency_summary(samples: &[f64]) -> LatencySummary {
    assert!(!samples.is_empty(), "latency samples must not be empty");
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    let percentile = |percent: f64| {
        let rank = (sorted.len() as f64 * percent).ceil() as usize;
        sorted[rank.saturating_sub(1).min(sorted.len() - 1)]
    };
    LatencySummary {
        min_ms: sorted[0],
        p50_ms: percentile(0.50),
        p95_ms: percentile(0.95),
        p99_ms: percentile(0.99),
        max_ms: sorted[sorted.len() - 1],
    }
}

struct RssSampler {
    idle: u64,
    peak: Arc<AtomicU64>,
    sampling: Arc<AtomicBool>,
    task: tokio::task::JoinHandle<()>,
}

struct RssSample {
    idle: u64,
    peak: u64,
    delta: u64,
}

struct ResourceSampler {
    storage: Arc<quackgis_server::duckdb_adbc_storage::DuckDbAdbcStorage>,
    memory_initial: u64,
    memory_peak: Arc<AtomicU64>,
    temporary_storage_peak: Arc<AtomicU64>,
    samples: Arc<AtomicU64>,
    failures: Arc<AtomicU64>,
    sampling: Arc<AtomicBool>,
    task: tokio::task::JoinHandle<()>,
}

struct ResourceSampleSummary {
    memory_initial: u64,
    memory_peak: u64,
    memory_delta: u64,
    temporary_storage_peak: u64,
    samples: u64,
    failures: u64,
}

impl RssSampler {
    fn start() -> Self {
        let idle = process_rss_bytes().expect("Linux process RSS");
        let peak = Arc::new(AtomicU64::new(idle));
        let sampling = Arc::new(AtomicBool::new(true));
        let sampler_peak = Arc::clone(&peak);
        let sampler_sampling = Arc::clone(&sampling);
        let task = tokio::spawn(async move {
            while sampler_sampling.load(Ordering::Acquire) {
                if let Some(rss) = process_rss_bytes() {
                    sampler_peak.fetch_max(rss, Ordering::Relaxed);
                }
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        });
        Self {
            idle,
            peak,
            sampling,
            task,
        }
    }

    async fn finish(self) -> RssSample {
        self.sampling.store(false, Ordering::Release);
        self.task.await.expect("RSS sampler");
        let peak = self.peak.load(Ordering::Relaxed);
        RssSample {
            idle: self.idle,
            peak,
            delta: peak.saturating_sub(self.idle),
        }
    }
}

impl ResourceSampler {
    fn start(storage: Arc<quackgis_server::duckdb_adbc_storage::DuckDbAdbcStorage>) -> Self {
        let initial = storage
            .resource_sample()
            .expect("initial DuckDB resource sample");
        let memory_peak = Arc::new(AtomicU64::new(initial.memory_bytes));
        let temporary_storage_peak = Arc::new(AtomicU64::new(initial.temporary_storage_bytes));
        let samples = Arc::new(AtomicU64::new(1));
        let failures = Arc::new(AtomicU64::new(0));
        let sampling = Arc::new(AtomicBool::new(true));
        let sampler_storage = Arc::clone(&storage);
        let sampler_memory_peak = Arc::clone(&memory_peak);
        let sampler_temporary_storage_peak = Arc::clone(&temporary_storage_peak);
        let sampler_samples = Arc::clone(&samples);
        let sampler_failures = Arc::clone(&failures);
        let sampler_sampling = Arc::clone(&sampling);
        let task = tokio::spawn(async move {
            while sampler_sampling.load(Ordering::Acquire) {
                let storage = Arc::clone(&sampler_storage);
                match tokio::task::spawn_blocking(move || storage.resource_sample()).await {
                    Ok(Ok(sample)) => {
                        sampler_memory_peak.fetch_max(sample.memory_bytes, Ordering::Relaxed);
                        sampler_temporary_storage_peak
                            .fetch_max(sample.temporary_storage_bytes, Ordering::Relaxed);
                        sampler_samples.fetch_add(1, Ordering::Relaxed);
                    }
                    Ok(Err(_)) | Err(_) => {
                        sampler_failures.fetch_add(1, Ordering::Relaxed);
                    }
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        });
        Self {
            storage,
            memory_initial: initial.memory_bytes,
            memory_peak,
            temporary_storage_peak,
            samples,
            failures,
            sampling,
            task,
        }
    }

    async fn finish(self) -> ResourceSampleSummary {
        self.sampling.store(false, Ordering::Release);
        self.task.await.expect("DuckDB resource sampler");
        match self.storage.resource_sample() {
            Ok(sample) => {
                self.memory_peak
                    .fetch_max(sample.memory_bytes, Ordering::Relaxed);
                self.temporary_storage_peak
                    .fetch_max(sample.temporary_storage_bytes, Ordering::Relaxed);
                self.samples.fetch_add(1, Ordering::Relaxed);
            }
            Err(_) => {
                self.failures.fetch_add(1, Ordering::Relaxed);
            }
        }
        let memory_peak = self.memory_peak.load(Ordering::Relaxed);
        ResourceSampleSummary {
            memory_initial: self.memory_initial,
            memory_peak,
            memory_delta: memory_peak.saturating_sub(self.memory_initial),
            temporary_storage_peak: self.temporary_storage_peak.load(Ordering::Relaxed),
            samples: self.samples.load(Ordering::Relaxed),
            failures: self.failures.load(Ordering::Relaxed),
        }
    }
}

struct GeneratedBatchReader {
    schema: SchemaRef,
    next_id: u64,
    rows: u64,
    batch_rows: u64,
}

impl Iterator for GeneratedBatchReader {
    type Item = Result<RecordBatch, ArrowError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next_id >= self.rows {
            return None;
        }
        let start = self.next_id;
        let end = self.rows.min(start + self.batch_rows);
        self.next_id = end;
        let ids = Int64Array::from_iter_values((start..end).map(|id| id as i64));
        let names = StringArray::from_iter_values((start..end).map(|id| format!("row-{id}")));
        let geometries = BinaryArray::from_iter_values((start..end).map(|_| COPY_WKB.as_slice()));
        Some(RecordBatch::try_new(
            Arc::clone(&self.schema),
            vec![Arc::new(ids), Arc::new(names), Arc::new(geometries)],
        ))
    }
}

impl RecordBatchReader for GeneratedBatchReader {
    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }
}

fn generated_copy_schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("name", DataType::Utf8, false),
        Field::new("geom_wkb", DataType::Binary, false),
    ]))
}

fn spatial_shape_batch(start: u64, end: u64) -> RecordBatch {
    let rows = usize::try_from(end - start).expect("spatial profile batch row count");
    let ids = Arc::new(Int64Array::from_iter_values(
        (start..end).map(|id| id as i64),
    )) as ArrayRef;
    let minimum_coordinates = Arc::new(Float64Array::from_iter_values(
        (start..end).map(|id| id as f64),
    )) as ArrayRef;
    let maximum_coordinates = Arc::new(Float64Array::from_iter_values((start..end).map(|id| {
        if id % 3 == 0 {
            id as f64
        } else {
            id as f64 + 0.25
        }
    }))) as ArrayRef;
    let mut geometries = BinaryBuilder::with_capacity(rows, rows * 52);
    for id in start..end {
        match id % 3 {
            0 => geometries.append_value(profile_point_wkb(id as f64)),
            1 => geometries.append_value(profile_linestring_wkb(id as f64)),
            2 => geometries.append_value(profile_polygon_wkb(id as f64)),
            _ => unreachable!("remainder modulo three"),
        }
    }
    RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("geom_wkb", DataType::Binary, false),
            Field::new("_qg_minx", DataType::Float64, false),
            Field::new("_qg_miny", DataType::Float64, false),
            Field::new("_qg_maxx", DataType::Float64, false),
            Field::new("_qg_maxy", DataType::Float64, false),
        ])),
        vec![
            ids,
            Arc::new(geometries.finish()),
            Arc::clone(&minimum_coordinates),
            minimum_coordinates,
            Arc::clone(&maximum_coordinates),
            maximum_coordinates,
        ],
    )
    .expect("spatial shape profile batch")
}

fn profile_point_wkb(coordinate: f64) -> [u8; 21] {
    let mut wkb = [0_u8; 21];
    wkb[0] = 1;
    wkb[1..5].copy_from_slice(&1_u32.to_le_bytes());
    wkb[5..13].copy_from_slice(&coordinate.to_le_bytes());
    wkb[13..21].copy_from_slice(&coordinate.to_le_bytes());
    wkb
}

fn profile_linestring_wkb(coordinate: f64) -> [u8; 41] {
    let mut wkb = [0_u8; 41];
    wkb[0] = 1;
    wkb[1..5].copy_from_slice(&2_u32.to_le_bytes());
    wkb[5..9].copy_from_slice(&2_u32.to_le_bytes());
    write_wkb_point(&mut wkb[9..25], coordinate, coordinate);
    write_wkb_point(&mut wkb[25..41], coordinate + 0.25, coordinate);
    wkb
}

fn profile_polygon_wkb(coordinate: f64) -> [u8; 93] {
    let mut wkb = [0_u8; 93];
    wkb[0] = 1;
    wkb[1..5].copy_from_slice(&3_u32.to_le_bytes());
    wkb[5..9].copy_from_slice(&1_u32.to_le_bytes());
    wkb[9..13].copy_from_slice(&5_u32.to_le_bytes());
    for (index, (x, y)) in [
        (coordinate, coordinate),
        (coordinate, coordinate + 0.25),
        (coordinate + 0.25, coordinate + 0.25),
        (coordinate + 0.25, coordinate),
        (coordinate, coordinate),
    ]
    .into_iter()
    .enumerate()
    {
        let start = 13 + index * 16;
        write_wkb_point(&mut wkb[start..start + 16], x, y);
    }
    wkb
}

fn write_wkb_point(target: &mut [u8], x: f64, y: f64) {
    target[..8].copy_from_slice(&x.to_le_bytes());
    target[8..16].copy_from_slice(&y.to_le_bytes());
}

fn profile_shape_count(rows: u64, remainder: u64) -> u64 {
    if rows <= remainder {
        0
    } else {
        (rows - 1 - remainder) / 3 + 1
    }
}

struct TableFile {
    path: String,
    bytes: u64,
}

fn table_files(
    storage: &quackgis_server::duckdb_adbc_storage::DuckDbAdbcStorage,
    table: &str,
) -> Vec<TableFile> {
    let batches = storage
        .query(&format!(
            "SELECT data_file, data_file_size_bytes \
             FROM ducklake_list_files('quackgis', '{}', schema => 'main') \
             ORDER BY data_file",
            table.replace('\'', "''")
        ))
        .unwrap_or_else(|error| panic!("list {table} DuckLake files: {error}"));
    let mut files = Vec::new();
    for batch in batches {
        let paths = batch
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("DuckLake data file paths");
        let sizes = batch
            .column(1)
            .as_any()
            .downcast_ref::<UInt64Array>()
            .expect("DuckLake data file sizes");
        for row in 0..batch.num_rows() {
            assert!(!paths.is_null(row));
            assert!(!sizes.is_null(row));
            files.push(TableFile {
                path: paths.value(row).to_owned(),
                bytes: sizes.value(row),
            });
        }
    }
    assert!(!files.is_empty(), "{table} has no DuckLake data files");
    files
}

fn parquet_row_group_bytes(
    storage: &quackgis_server::duckdb_adbc_storage::DuckDbAdbcStorage,
    files: &[TableFile],
) -> Vec<u64> {
    let paths = files
        .iter()
        .map(|file| format!("'{}'", file.path.replace('\'', "''")))
        .collect::<Vec<_>>()
        .join(", ");
    let batches = storage
        .query(&format!(
            "SELECT CAST(max(row_group_compressed_bytes) AS BIGINT) \
             FROM parquet_metadata([{paths}]) \
             GROUP BY file_name, row_group_id \
             ORDER BY file_name, row_group_id"
        ))
        .expect("read Parquet row-group metadata");
    let mut bytes = Vec::new();
    for batch in batches {
        let values = batch
            .column(0)
            .as_any()
            .downcast_ref::<Int64Array>()
            .expect("Parquet compressed row-group bytes");
        for row in 0..batch.num_rows() {
            assert!(!values.is_null(row));
            bytes.push(
                u64::try_from(values.value(row))
                    .expect("non-negative Parquet compressed row-group bytes"),
            );
        }
    }
    assert!(!bytes.is_empty(), "Parquet metadata has no row groups");
    bytes
}

fn analyze_query(
    storage: &quackgis_server::duckdb_adbc_storage::DuckDbAdbcStorage,
    query: &str,
) -> serde_json::Value {
    let batches = storage
        .query(&format!("EXPLAIN (ANALYZE, FORMAT JSON) {query}"))
        .unwrap_or_else(|error| panic!("analyze spatial query {query}: {error:?}"));
    let values = batches
        .first()
        .expect("DuckDB analyzed plan batch")
        .column(1)
        .as_any()
        .downcast_ref::<StringArray>()
        .expect("DuckDB analyzed plan JSON text");
    serde_json::from_str(values.value(0)).expect("DuckDB JSON analyzed plan")
}

#[derive(Clone, Copy)]
struct ScanMetrics {
    scanned: u64,
    total: u64,
}

struct ScanByteMetrics {
    total: u64,
    scanned_upper_bound: u64,
    ratio_upper_bound: f64,
}

fn scan_metrics(plan: &serde_json::Value) -> ScanMetrics {
    fn collect(value: &serde_json::Value, metrics: &mut ScanMetrics) {
        if let Some(object) = value.as_object() {
            let total = object
                .get("operator_total_row_groups_to_scan")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            if total > 0 {
                metrics.total += total;
                metrics.scanned += object
                    .get("operator_row_groups_scanned")
                    .and_then(serde_json::Value::as_u64)
                    .expect("row-group scan metric paired with total");
            }
        }
        if let Some(children) = value.get("children").and_then(serde_json::Value::as_array) {
            for child in children {
                collect(child, metrics);
            }
        }
    }

    let mut metrics = ScanMetrics {
        scanned: 0,
        total: 0,
    };
    collect(plan, &mut metrics);
    assert!(metrics.total > 0, "analyzed plan omitted row-group totals");
    assert!(metrics.scanned > 0, "selective query scanned no row groups");
    assert!(metrics.scanned <= metrics.total);
    metrics
}

fn assert_scan_budget(label: &str, selective: ScanMetrics, unpruned: ScanMetrics) {
    assert_eq!(
        unpruned.scanned, unpruned.total,
        "{label} exact-only oracle unexpectedly pruned row groups"
    );
    assert!(
        selective.total <= unpruned.total,
        "{label} selective plan dispatched more row groups than the exact-only plan"
    );
    let ratio = selective.scanned as f64 / unpruned.total as f64;
    let improvement = unpruned.scanned as f64 / selective.scanned as f64;
    assert!(
        ratio <= 0.05,
        "{label} scanned {:.2}% of row groups, above 5%",
        ratio * 100.0
    );
    assert!(
        improvement >= 20.0,
        "{label} row-group scan improvement {improvement:.2}x is below 20x"
    );
}

fn scan_byte_metrics(
    row_group_bytes: &[u64],
    selective: ScanMetrics,
    unpruned: ScanMetrics,
) -> ScanByteMetrics {
    assert_eq!(
        row_group_bytes.len() as u64,
        unpruned.total,
        "Parquet metadata and exact-only profile row-group counts differ"
    );
    let total = row_group_bytes.iter().sum::<u64>();
    assert!(
        total > 0,
        "compressed Parquet row-group bytes must be positive"
    );
    let mut largest = row_group_bytes.to_vec();
    largest.sort_unstable_by(|left, right| right.cmp(left));
    let scanned_upper_bound = largest
        .into_iter()
        .take(selective.scanned as usize)
        .sum::<u64>();
    let ratio_upper_bound = scanned_upper_bound as f64 / total as f64;
    assert!(
        ratio_upper_bound <= 0.05,
        "conservative compressed scan-byte upper bound {:.2}% exceeds 5%",
        ratio_upper_bound * 100.0
    );
    ScanByteMetrics {
        total,
        scanned_upper_bound,
        ratio_upper_bound,
    }
}

async fn pgwire_count_latency(
    client: &tokio_postgres::Client,
    label: &str,
    query: &str,
    expected_count: u64,
    samples: usize,
) -> LatencySummary {
    let mut latencies = Vec::with_capacity(samples);
    for sample in 0..samples {
        let started = Instant::now();
        let count = client
            .query_one(query, &[])
            .await
            .unwrap_or_else(|error| panic!("{label} sample {sample}: {error:?}"))
            .get::<_, i64>(0);
        latencies.push(started.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(
            count, expected_count as i64,
            "{label} sample {sample} exact count"
        );
    }
    latency_summary(&latencies)
}

fn assert_latency_budget(label: &str, latency: &LatencySummary, profile: &SpatialScanProfile) {
    for (percentile, actual, budget) in [
        ("p50", latency.p50_ms, profile.query_p50_budget_ms),
        ("p95", latency.p95_ms, profile.query_p95_budget_ms),
        ("p99", latency.p99_ms, profile.query_p99_budget_ms),
    ] {
        assert!(
            actual <= budget,
            "{label} {percentile} {actual:.2} ms exceeded {budget:.2} ms"
        );
    }
}

fn copy_text_chunk(start: u64, rows: u64, max_bytes: usize) -> (String, u64) {
    let mut chunk = String::with_capacity(max_bytes);
    let mut next = start;
    while next < rows {
        let line = format!("{next}\trow-{next}\t\\x{COPY_WKB_HEX}\n");
        if !chunk.is_empty() && chunk.len() + line.len() > max_bytes {
            break;
        }
        chunk.push_str(&line);
        next += 1;
    }
    (chunk, next)
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

fn prometheus_labeled_u64(
    metrics: &str,
    name: &str,
    label: &str,
    label_value: &str,
) -> Option<u64> {
    let metric_name = format!("{name}{{{label}=\"{label_value}\"}}");
    prometheus_u64(metrics, &metric_name)
}

struct ChildGuard(std::process::Child);

struct TestTlsIdentity {
    certificate_pem: String,
    private_key_pem: String,
    certificate_der: rustls::pki_types::CertificateDer<'static>,
    fingerprint: String,
}

impl TestTlsIdentity {
    fn new() -> Self {
        let rcgen::CertifiedKey { cert, signing_key } =
            rcgen::generate_simple_self_signed(vec!["127.0.0.1".to_owned()])
                .expect("generate test TLS identity");
        let certificate_der = cert.der().clone();
        let fingerprint = format!("{:x}", Sha256::digest(certificate_der.as_ref()));
        Self {
            certificate_pem: cert.pem(),
            private_key_pem: signing_key.serialize_pem(),
            certificate_der,
            fingerprint,
        }
    }

    fn write(&self, certificate: &std::path::Path, private_key: &std::path::Path) {
        std::fs::write(certificate, &self.certificate_pem).expect("write test certificate");
        std::fs::write(private_key, &self.private_key_pem).expect("write test private key");
    }

    fn connector(&self) -> tokio_postgres_rustls::MakeRustlsConnect {
        let mut roots = rustls::RootCertStore::empty();
        roots
            .add(self.certificate_der.clone())
            .expect("trust test certificate");
        let config = rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        tokio_postgres_rustls::MakeRustlsConnect::new(config)
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

fn spawn_profile_server(
    driver: &std::path::Path,
    catalog: &std::path::Path,
    data: &std::path::Path,
    port: u16,
) -> ChildGuard {
    ChildGuard(
        std::process::Command::new(env!("CARGO_BIN_EXE_quackgis-server"))
            .arg("--duckdb-driver")
            .arg(driver)
            .arg("--catalog-path")
            .arg(catalog)
            .arg("--data-path")
            .arg(data)
            .arg("--host=127.0.0.1")
            .arg(format!("--port={port}"))
            .arg("--shutdown-timeout-ms=100")
            .arg("--statement-timeout-ms=30000")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn profile server"),
    )
}

fn spawn_tls_profile_server(
    driver: &std::path::Path,
    catalog: &std::path::Path,
    data: &std::path::Path,
    port: u16,
    certificate: &std::path::Path,
    private_key: &std::path::Path,
    password: &str,
) -> ChildGuard {
    ChildGuard(
        std::process::Command::new(env!("CARGO_BIN_EXE_quackgis-server"))
            .arg("--duckdb-driver")
            .arg(driver)
            .arg("--catalog-path")
            .arg(catalog)
            .arg("--data-path")
            .arg(data)
            .arg("--host=127.0.0.1")
            .arg(format!("--port={port}"))
            .arg("--shutdown-timeout-ms=100")
            .arg("--statement-timeout-ms=30000")
            .arg("--tls-mode=required")
            .arg("--tls-cert")
            .arg(certificate)
            .arg("--tls-key")
            .arg(private_key)
            .arg("--auth-mode=password")
            .arg("--readwrite-user=postgres")
            .env("QUACKGIS_READWRITE_PASSWORD", password)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn TLS profile server"),
    )
}

async fn unused_local_port() -> u16 {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("ephemeral profile listener");
    listener.local_addr().expect("profile address").port()
}

async fn connect_profile_client(
    port: u16,
) -> (
    tokio_postgres::Client,
    tokio::task::JoinHandle<Result<(), tokio_postgres::Error>>,
) {
    let (client, connection) = tokio_postgres::connect(
        &format!("host=127.0.0.1 port={port} user=postgres dbname=quackgis"),
        tokio_postgres::NoTls,
    )
    .await
    .expect("connect profile client");
    (client, tokio::spawn(connection))
}

async fn connect_profile_server(
    port: u16,
    child: &mut ChildGuard,
) -> (
    tokio_postgres::Client,
    tokio::task::JoinHandle<Result<(), tokio_postgres::Error>>,
) {
    for _ in 0..400 {
        match tokio_postgres::connect(
            &format!("host=127.0.0.1 port={port} user=postgres dbname=quackgis"),
            tokio_postgres::NoTls,
        )
        .await
        {
            Ok((client, connection)) => return (client, tokio::spawn(connection)),
            Err(_) if child.0.try_wait().expect("profile server status").is_none() => {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            Err(error) => panic!("profile server exited before readiness: {error}"),
        }
    }
    panic!("profile server did not become ready")
}

async fn connect_tls_profile_client(
    port: u16,
    password: &str,
    identity: &TestTlsIdentity,
) -> Result<
    (
        tokio_postgres::Client,
        tokio::task::JoinHandle<Result<(), tokio_postgres::Error>>,
    ),
    tokio_postgres::Error,
> {
    let (client, connection) = tokio_postgres::connect(
        &format!(
            "host=127.0.0.1 port={port} user=postgres password={password} \
             dbname=quackgis sslmode=require"
        ),
        identity.connector(),
    )
    .await?;
    Ok((client, tokio::spawn(connection)))
}

async fn connect_tls_profile_server(
    port: u16,
    password: &str,
    identity: &TestTlsIdentity,
    child: &mut ChildGuard,
) -> (
    tokio_postgres::Client,
    tokio::task::JoinHandle<Result<(), tokio_postgres::Error>>,
) {
    for _ in 0..400 {
        match connect_tls_profile_client(port, password, identity).await {
            Ok(connection) => return connection,
            Err(_)
                if child
                    .0
                    .try_wait()
                    .expect("TLS profile server status")
                    .is_none() =>
            {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            Err(error) => panic!("TLS profile server exited before readiness: {error}"),
        }
    }
    panic!("TLS profile server did not become ready")
}

async fn wait_for_child(child: &mut ChildGuard, timeout: Duration) -> std::process::ExitStatus {
    tokio::time::timeout(timeout, async {
        loop {
            if let Some(status) = child.0.try_wait().expect("profile server status") {
                return status;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("profile server exit timeout")
}

#[test]
fn parses_process_rss_and_prometheus_values() {
    assert!(process_rss_bytes().is_some_and(|rss| rss > 0));
    assert_eq!(prometheus_u64("metric 7\n", "metric"), Some(7));
    assert_eq!(prometheus_u64("metric 7\n", "missing"), None);
    assert_eq!(
        prometheus_labeled_u64("metric{class=\"reader\"} 2\n", "metric", "class", "reader"),
        Some(2)
    );
    let summary = latency_summary(&[5.0, 1.0, 4.0, 2.0, 3.0]);
    assert_eq!(summary.min_ms, 1.0);
    assert_eq!(summary.p50_ms, 3.0);
    assert_eq!(summary.p95_ms, 5.0);
    assert_eq!(summary.max_ms, 5.0);
    let (chunk, next) = copy_text_chunk(0, 10, 80);
    assert!(!chunk.is_empty());
    assert!(chunk.len() <= 80);
    assert!((1..10).contains(&next));
}
