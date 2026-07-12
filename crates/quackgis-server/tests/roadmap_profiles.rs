// SPDX-License-Identifier: Apache-2.0
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use arrow_array::{BinaryArray, Int64Array, RecordBatch, RecordBatchReader, StringArray};
use arrow_schema::{ArrowError, DataType, Field, Schema, SchemaRef};
use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use quackgis_server::engine_api::{EngineTableRef, IngestDisposition};
use quackgis_server::pgwire_server::ServerOptions;
use serde_json::json;

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
        let (chunk, next) = copy_text_chunk(next_id, profile.rows, 60 * 1024);
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
            "copy_chunk_max_bytes": 60 * 1024,
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
            "copy_chunk_max_bytes": 60 * 1024,
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

#[test]
fn parses_process_rss_and_prometheus_values() {
    assert!(process_rss_bytes().is_some_and(|rss| rss > 0));
    assert_eq!(prometheus_u64("metric 7\n", "metric"), Some(7));
    assert_eq!(prometheus_u64("metric 7\n", "missing"), None);
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
