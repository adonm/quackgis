// SPDX-License-Identifier: Apache-2.0
//! Minimal Prometheus-compatible metrics endpoint.
//!
//! The endpoint is intentionally opt-in and process-local. It exposes only
//! counters that already exist for operations evidence and never includes SQL
//! text, object paths, user credentials, or client parameters.

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::duckdb_adbc_storage::DuckDbAdbcStorage;
use crate::engine_api::{EngineResourceSample, EngineStorageKernel};
use crate::lifecycle::{ReadinessState, RuntimeLifecycle};

const METRICS_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";
static COPY_ACTIVE: AtomicUsize = AtomicUsize::new(0);
static COPY_STARTED: AtomicU64 = AtomicU64::new(0);
static COPY_COMPLETED: AtomicU64 = AtomicU64::new(0);
static COPY_FAILED: AtomicU64 = AtomicU64::new(0);
static COPY_ROWS: AtomicU64 = AtomicU64::new(0);
static COPY_BYTES: AtomicU64 = AtomicU64::new(0);
static COPY_BATCHES: AtomicU64 = AtomicU64::new(0);
static COPY_DURATION_MICROSECONDS: AtomicU64 = AtomicU64::new(0);
static COPY_COMMIT_MICROSECONDS: AtomicU64 = AtomicU64::new(0);
static QUERY_BATCHES: AtomicU64 = AtomicU64::new(0);
static QUERY_BATCHES_INFLIGHT: AtomicUsize = AtomicUsize::new(0);
static QUERY_BATCHES_INFLIGHT_HIGH_WATER: AtomicUsize = AtomicUsize::new(0);
static QUERY_BATCH_BYTES_HIGH_WATER: AtomicUsize = AtomicUsize::new(0);
static QUERY_BATCH_LIMIT_REJECTIONS: AtomicU64 = AtomicU64::new(0);
static CONNECTIONS_QUARANTINED: AtomicU64 = AtomicU64::new(0);
static DUCKDB_MEMORY_BYTES: AtomicU64 = AtomicU64::new(0);
static DUCKDB_MEMORY_BYTES_HIGH_WATER: AtomicU64 = AtomicU64::new(0);
static DUCKDB_TEMPORARY_STORAGE_BYTES: AtomicU64 = AtomicU64::new(0);
static DUCKDB_TEMPORARY_STORAGE_BYTES_HIGH_WATER: AtomicU64 = AtomicU64::new(0);
static DUCKDB_RESOURCE_SAMPLES: AtomicU64 = AtomicU64::new(0);
static DUCKDB_RESOURCE_SAMPLE_FAILURES: AtomicU64 = AtomicU64::new(0);
static DUCKDB_RESOURCE_LAST_SUCCESS_UNIX_SECONDS: AtomicU64 = AtomicU64::new(0);

fn update_high_water(target: &AtomicUsize, value: usize) {
    let mut observed = target.load(Ordering::Relaxed);
    while value > observed {
        match target.compare_exchange_weak(observed, value, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(actual) => observed = actual,
        }
    }
}

fn update_high_water_u64(target: &AtomicU64, value: u64) {
    let mut observed = target.load(Ordering::Relaxed);
    while value > observed {
        match target.compare_exchange_weak(observed, value, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(actual) => observed = actual,
        }
    }
}

pub fn record_duckdb_resource_sample(sample: EngineResourceSample) {
    DUCKDB_MEMORY_BYTES.store(sample.memory_bytes, Ordering::Relaxed);
    DUCKDB_TEMPORARY_STORAGE_BYTES.store(sample.temporary_storage_bytes, Ordering::Relaxed);
    update_high_water_u64(&DUCKDB_MEMORY_BYTES_HIGH_WATER, sample.memory_bytes);
    update_high_water_u64(
        &DUCKDB_TEMPORARY_STORAGE_BYTES_HIGH_WATER,
        sample.temporary_storage_bytes,
    );
    DUCKDB_RESOURCE_SAMPLES.fetch_add(1, Ordering::Relaxed);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    DUCKDB_RESOURCE_LAST_SUCCESS_UNIX_SECONDS.store(timestamp, Ordering::Relaxed);
}

pub fn record_duckdb_resource_sample_failure() {
    DUCKDB_RESOURCE_SAMPLE_FAILURES.fetch_add(1, Ordering::Relaxed);
}

/// Sample aggregate DuckDB allocations on an independent ADBC session. Scrapes
/// remain atomic-only and never execute native SQL on the HTTP task.
pub async fn sample_duckdb_resources(
    storage: Arc<DuckDbAdbcStorage>,
    lifecycle: Arc<RuntimeLifecycle>,
    interval: Duration,
) {
    let mut ticker = tokio::time::interval(interval);
    loop {
        ticker.tick().await;
        if !lifecycle.is_accepting() {
            return;
        }
        let storage = Arc::clone(&storage);
        match tokio::task::spawn_blocking(move || {
            storage.operational_readiness_probe()?;
            storage.resource_sample()
        })
        .await
        {
            Ok(Ok(sample)) => {
                lifecycle.mark_storage_ready();
                record_duckdb_resource_sample(sample);
            }
            Ok(Err(error)) => {
                lifecycle.mark_storage_unavailable();
                record_duckdb_resource_sample_failure();
                log::warn!("DuckDB readiness/resource sample failed: {error}");
            }
            Err(error) => {
                lifecycle.mark_storage_unavailable();
                record_duckdb_resource_sample_failure();
                log::warn!("DuckDB readiness/resource sampler worker failed: {error}");
            }
        }
    }
}

pub fn query_batch_started(bytes: usize) {
    QUERY_BATCHES.fetch_add(1, Ordering::Relaxed);
    update_high_water(&QUERY_BATCH_BYTES_HIGH_WATER, bytes);
    let inflight = QUERY_BATCHES_INFLIGHT.fetch_add(1, Ordering::Relaxed) + 1;
    update_high_water(&QUERY_BATCHES_INFLIGHT_HIGH_WATER, inflight);
}

pub fn query_batch_finished() {
    QUERY_BATCHES_INFLIGHT.fetch_sub(1, Ordering::Relaxed);
}

pub fn query_batch_rejected() {
    QUERY_BATCH_LIMIT_REJECTIONS.fetch_add(1, Ordering::Relaxed);
}

pub fn connection_quarantined() {
    CONNECTIONS_QUARANTINED.fetch_add(1, Ordering::Relaxed);
}

pub fn copy_started() {
    COPY_ACTIVE.fetch_add(1, Ordering::Relaxed);
    COPY_STARTED.fetch_add(1, Ordering::Relaxed);
}

pub fn copy_completed(
    rows: usize,
    bytes: usize,
    batches: usize,
    duration: Duration,
    commit_latency: Duration,
) {
    COPY_ACTIVE.fetch_sub(1, Ordering::Relaxed);
    COPY_COMPLETED.fetch_add(1, Ordering::Relaxed);
    COPY_ROWS.fetch_add(rows as u64, Ordering::Relaxed);
    COPY_BYTES.fetch_add(bytes as u64, Ordering::Relaxed);
    COPY_BATCHES.fetch_add(batches as u64, Ordering::Relaxed);
    COPY_DURATION_MICROSECONDS.fetch_add(duration.as_micros() as u64, Ordering::Relaxed);
    COPY_COMMIT_MICROSECONDS.fetch_add(commit_latency.as_micros() as u64, Ordering::Relaxed);
}

pub fn copy_failed(duration: Duration) {
    COPY_ACTIVE.fetch_sub(1, Ordering::Relaxed);
    COPY_FAILED.fetch_add(1, Ordering::Relaxed);
    COPY_DURATION_MICROSECONDS.fetch_add(duration.as_micros() as u64, Ordering::Relaxed);
}

pub fn render_prometheus(lifecycle: &RuntimeLifecycle) -> String {
    format!(
        concat!(
            "# HELP quackgis_write_denied_total DuckDB write statements denied by policy.\n",
            "# TYPE quackgis_write_denied_total counter\n",
            "quackgis_write_denied_total {}\n",
            "# HELP quackgis_read_denied_total DuckDB reads denied by policy.\n",
            "# TYPE quackgis_read_denied_total counter\n",
            "quackgis_read_denied_total {}\n",
            "# HELP quackgis_operations_active DuckDB operations currently admitted.\n",
            "# TYPE quackgis_operations_active gauge\n",
            "quackgis_operations_active {}\n",
            "# HELP quackgis_operations_queued DuckDB operations waiting for admission.\n",
            "# TYPE quackgis_operations_queued gauge\n",
            "quackgis_operations_queued {}\n",
            "# HELP quackgis_operations_class_active DuckDB operations currently admitted by class.\n",
            "# TYPE quackgis_operations_class_active gauge\n",
            "quackgis_operations_class_active{{class=\"reader\"}} {}\n",
            "quackgis_operations_class_active{{class=\"writer\"}} {}\n",
            "quackgis_operations_class_active{{class=\"maintenance\"}} {}\n",
            "# HELP quackgis_operations_class_queued DuckDB operations waiting for admission by class.\n",
            "# TYPE quackgis_operations_class_queued gauge\n",
            "quackgis_operations_class_queued{{class=\"reader\"}} {}\n",
            "quackgis_operations_class_queued{{class=\"writer\"}} {}\n",
            "quackgis_operations_class_queued{{class=\"maintenance\"}} {}\n",
            "# HELP quackgis_operations_class_high_water Highest observed admitted DuckDB operations by class.\n",
            "# TYPE quackgis_operations_class_high_water gauge\n",
            "quackgis_operations_class_high_water{{class=\"reader\"}} {}\n",
            "quackgis_operations_class_high_water{{class=\"writer\"}} {}\n",
            "quackgis_operations_class_high_water{{class=\"maintenance\"}} {}\n",
            "# HELP quackgis_operations_started_total DuckDB operations admitted.\n",
            "# TYPE quackgis_operations_started_total counter\n",
            "quackgis_operations_started_total {}\n",
            "# HELP quackgis_admission_rejected_total DuckDB operations rejected because the queue was full.\n",
            "# TYPE quackgis_admission_rejected_total counter\n",
            "quackgis_admission_rejected_total {}\n",
            "# HELP quackgis_admission_queue_timeouts_total DuckDB operations canceled while queued.\n",
            "# TYPE quackgis_admission_queue_timeouts_total counter\n",
            "quackgis_admission_queue_timeouts_total {}\n",
            "# HELP quackgis_cancellations_requested_total Native DuckDB cancellation requests received.\n",
            "# TYPE quackgis_cancellations_requested_total counter\n",
            "quackgis_cancellations_requested_total {}\n",
            "# HELP quackgis_cancellations_completed_total Native DuckDB cancellation calls completed.\n",
            "# TYPE quackgis_cancellations_completed_total counter\n",
            "quackgis_cancellations_completed_total {}\n",
            "# HELP quackgis_cancellations_failed_total Native DuckDB cancellation calls that failed.\n",
            "# TYPE quackgis_cancellations_failed_total counter\n",
            "quackgis_cancellations_failed_total {}\n",
            "# HELP quackgis_statement_timeouts_total DuckDB query streams canceled at their deadline.\n",
            "# TYPE quackgis_statement_timeouts_total counter\n",
            "quackgis_statement_timeouts_total {}\n",
            "# HELP quackgis_connections_quarantined_total DuckDB sessions quarantined after uncertain native cleanup.\n",
            "# TYPE quackgis_connections_quarantined_total counter\n",
            "quackgis_connections_quarantined_total {}\n",
            "# HELP quackgis_transactions_active Explicit DuckDB transactions currently open.\n",
            "# TYPE quackgis_transactions_active gauge\n",
            "quackgis_transactions_active {}\n",
            "# HELP quackgis_duckdb_memory_bytes Current aggregate DuckDB tracked memory usage.\n",
            "# TYPE quackgis_duckdb_memory_bytes gauge\n",
            "quackgis_duckdb_memory_bytes {}\n",
            "# HELP quackgis_duckdb_memory_bytes_high_water Highest sampled aggregate DuckDB tracked memory usage.\n",
            "# TYPE quackgis_duckdb_memory_bytes_high_water gauge\n",
            "quackgis_duckdb_memory_bytes_high_water {}\n",
            "# HELP quackgis_duckdb_temporary_storage_bytes Current aggregate DuckDB temporary storage usage.\n",
            "# TYPE quackgis_duckdb_temporary_storage_bytes gauge\n",
            "quackgis_duckdb_temporary_storage_bytes {}\n",
            "# HELP quackgis_duckdb_temporary_storage_bytes_high_water Highest sampled aggregate DuckDB temporary storage usage.\n",
            "# TYPE quackgis_duckdb_temporary_storage_bytes_high_water gauge\n",
            "quackgis_duckdb_temporary_storage_bytes_high_water {}\n",
            "# HELP quackgis_duckdb_resource_samples_total Successful DuckDB resource samples.\n",
            "# TYPE quackgis_duckdb_resource_samples_total counter\n",
            "quackgis_duckdb_resource_samples_total {}\n",
            "# HELP quackgis_duckdb_resource_sample_failures_total Failed DuckDB resource samples.\n",
            "# TYPE quackgis_duckdb_resource_sample_failures_total counter\n",
            "quackgis_duckdb_resource_sample_failures_total {}\n",
            "# HELP quackgis_duckdb_resource_last_success_unixtime_seconds Unix timestamp of the latest successful DuckDB resource sample.\n",
            "# TYPE quackgis_duckdb_resource_last_success_unixtime_seconds gauge\n",
            "quackgis_duckdb_resource_last_success_unixtime_seconds {}\n",
            "# HELP quackgis_query_batches_total Arrow result batches accepted by pgwire.\n",
            "# TYPE quackgis_query_batches_total counter\n",
            "quackgis_query_batches_total {}\n",
            "# HELP quackgis_query_batches_inflight Arrow result batches currently being encoded.\n",
            "# TYPE quackgis_query_batches_inflight gauge\n",
            "quackgis_query_batches_inflight {}\n",
            "# HELP quackgis_query_batches_inflight_high_water Highest observed in-flight Arrow result batches.\n",
            "# TYPE quackgis_query_batches_inflight_high_water gauge\n",
            "quackgis_query_batches_inflight_high_water {}\n",
            "# HELP quackgis_query_batch_bytes_high_water Largest accepted Arrow result batch in memory bytes.\n",
            "# TYPE quackgis_query_batch_bytes_high_water gauge\n",
            "quackgis_query_batch_bytes_high_water {}\n",
            "# HELP quackgis_query_batch_limit_rejections_total Arrow result batches rejected by the configured byte ceiling.\n",
            "# TYPE quackgis_query_batch_limit_rejections_total counter\n",
            "quackgis_query_batch_limit_rejections_total {}\n",
            "# HELP quackgis_blocking_workers_active Native blocking workers currently active.\n",
            "# TYPE quackgis_blocking_workers_active gauge\n",
            "quackgis_blocking_workers_active {}\n",
            "# HELP quackgis_blocking_regular_active Regular native blocking workers currently active.\n",
            "# TYPE quackgis_blocking_regular_active gauge\n",
            "quackgis_blocking_regular_active {}\n",
            "# HELP quackgis_blocking_control_active Reserved control workers currently active.\n",
            "# TYPE quackgis_blocking_control_active gauge\n",
            "quackgis_blocking_control_active {}\n",
            "# HELP quackgis_blocking_workers_queued Native operations waiting for a worker.\n",
            "# TYPE quackgis_blocking_workers_queued gauge\n",
            "quackgis_blocking_workers_queued {}\n",
            "# HELP quackgis_blocking_workers_high_water Highest observed active native workers.\n",
            "# TYPE quackgis_blocking_workers_high_water gauge\n",
            "quackgis_blocking_workers_high_water {}\n",
            "# HELP quackgis_copy_active COPY operations currently active.\n",
            "# TYPE quackgis_copy_active gauge\n",
            "quackgis_copy_active {}\n",
            "# HELP quackgis_copy_started_total COPY operations started.\n",
            "# TYPE quackgis_copy_started_total counter\n",
            "quackgis_copy_started_total {}\n",
            "# HELP quackgis_copy_completed_total COPY statements completed; an enclosing client transaction may still roll back.\n",
            "# TYPE quackgis_copy_completed_total counter\n",
            "quackgis_copy_completed_total {}\n",
            "# HELP quackgis_copy_failed_total COPY operations aborted.\n",
            "# TYPE quackgis_copy_failed_total counter\n",
            "quackgis_copy_failed_total {}\n",
            "# HELP quackgis_copy_rows_total Rows accepted by completed COPY statements.\n",
            "# TYPE quackgis_copy_rows_total counter\n",
            "quackgis_copy_rows_total {}\n",
            "# HELP quackgis_copy_bytes_total Wire bytes accepted by committed COPY operations.\n",
            "# TYPE quackgis_copy_bytes_total counter\n",
            "quackgis_copy_bytes_total {}\n",
            "# HELP quackgis_copy_batches_total Arrow batches accepted by completed COPY statements.\n",
            "# TYPE quackgis_copy_batches_total counter\n",
            "quackgis_copy_batches_total {}\n",
            "# HELP quackgis_copy_duration_microseconds_total Cumulative COPY duration.\n",
            "# TYPE quackgis_copy_duration_microseconds_total counter\n",
            "quackgis_copy_duration_microseconds_total {}\n",
            "# HELP quackgis_copy_commit_microseconds_total Cumulative COPY finish-to-commit latency.\n",
            "# TYPE quackgis_copy_commit_microseconds_total counter\n",
            "quackgis_copy_commit_microseconds_total {}\n",
        ),
        crate::statement_policy::writes_denied_total(),
        crate::statement_policy::reads_denied_total(),
        crate::execution_control::active_operations(),
        crate::execution_control::queued_operations(),
        crate::execution_control::active_operations_for(
            crate::execution_control::OperationClass::Reader
        ),
        crate::execution_control::active_operations_for(
            crate::execution_control::OperationClass::Writer
        ),
        crate::execution_control::active_operations_for(
            crate::execution_control::OperationClass::Maintenance
        ),
        crate::execution_control::queued_operations_for(
            crate::execution_control::OperationClass::Reader
        ),
        crate::execution_control::queued_operations_for(
            crate::execution_control::OperationClass::Writer
        ),
        crate::execution_control::queued_operations_for(
            crate::execution_control::OperationClass::Maintenance
        ),
        crate::execution_control::operations_high_water_for(
            crate::execution_control::OperationClass::Reader
        ),
        crate::execution_control::operations_high_water_for(
            crate::execution_control::OperationClass::Writer
        ),
        crate::execution_control::operations_high_water_for(
            crate::execution_control::OperationClass::Maintenance
        ),
        crate::execution_control::started_total(),
        crate::execution_control::rejected_total(),
        crate::execution_control::queue_timeouts_total(),
        crate::execution_control::cancellations_requested_total(),
        crate::execution_control::cancellations_completed_total(),
        crate::execution_control::cancellations_failed_total(),
        crate::execution_control::statement_timeouts_total(),
        CONNECTIONS_QUARANTINED.load(Ordering::Relaxed),
        lifecycle.active_transactions(),
        DUCKDB_MEMORY_BYTES.load(Ordering::Relaxed),
        DUCKDB_MEMORY_BYTES_HIGH_WATER.load(Ordering::Relaxed),
        DUCKDB_TEMPORARY_STORAGE_BYTES.load(Ordering::Relaxed),
        DUCKDB_TEMPORARY_STORAGE_BYTES_HIGH_WATER.load(Ordering::Relaxed),
        DUCKDB_RESOURCE_SAMPLES.load(Ordering::Relaxed),
        DUCKDB_RESOURCE_SAMPLE_FAILURES.load(Ordering::Relaxed),
        DUCKDB_RESOURCE_LAST_SUCCESS_UNIX_SECONDS.load(Ordering::Relaxed),
        QUERY_BATCHES.load(Ordering::Relaxed),
        QUERY_BATCHES_INFLIGHT.load(Ordering::Relaxed),
        QUERY_BATCHES_INFLIGHT_HIGH_WATER.load(Ordering::Relaxed),
        QUERY_BATCH_BYTES_HIGH_WATER.load(Ordering::Relaxed),
        QUERY_BATCH_LIMIT_REJECTIONS.load(Ordering::Relaxed),
        crate::execution_control::blocking_workers_active(),
        crate::execution_control::blocking_regular_active(),
        crate::execution_control::blocking_control_active(),
        crate::execution_control::blocking_workers_queued(),
        crate::execution_control::blocking_workers_high_water(),
        COPY_ACTIVE.load(Ordering::Relaxed),
        COPY_STARTED.load(Ordering::Relaxed),
        COPY_COMPLETED.load(Ordering::Relaxed),
        COPY_FAILED.load(Ordering::Relaxed),
        COPY_ROWS.load(Ordering::Relaxed),
        COPY_BYTES.load(Ordering::Relaxed),
        COPY_BATCHES.load(Ordering::Relaxed),
        COPY_DURATION_MICROSECONDS.load(Ordering::Relaxed),
        COPY_COMMIT_MICROSECONDS.load(Ordering::Relaxed),
    )
}

pub async fn serve_listener(
    listener: TcpListener,
    lifecycle: Arc<RuntimeLifecycle>,
) -> io::Result<()> {
    loop {
        let (stream, peer) = listener.accept().await?;
        let lifecycle = Arc::clone(&lifecycle);
        tokio::spawn(async move {
            if let Err(err) = handle_connection(stream, lifecycle).await {
                log::debug!("metrics scrape from {peer} failed: {err}");
            }
        });
    }
}

async fn handle_connection(
    mut stream: TcpStream,
    lifecycle: Arc<RuntimeLifecycle>,
) -> io::Result<()> {
    let mut buffer = [0_u8; 1024];
    let n = stream.read(&mut buffer).await?;
    let request = String::from_utf8_lossy(&buffer[..n]);
    let path = request_path(&request);
    let (status, content_type, body) = match path {
        Some(path) if is_metrics_path(path) => (
            "200 OK",
            METRICS_CONTENT_TYPE,
            render_prometheus(&lifecycle),
        ),
        Some("/healthz") => ("200 OK", "text/plain; charset=utf-8", "ok\n".to_owned()),
        Some("/readyz") if lifecycle.readiness() == ReadinessState::Ready => {
            ("200 OK", "text/plain; charset=utf-8", "ready\n".to_owned())
        }
        Some("/readyz") => (
            "503 Service Unavailable",
            "text/plain; charset=utf-8",
            format!(
                "{}\n",
                match lifecycle.readiness() {
                    ReadinessState::Starting => "starting",
                    ReadinessState::StorageUnavailable => "storage_unavailable",
                    ReadinessState::Draining => "draining",
                    ReadinessState::Ready => unreachable!("ready handled above"),
                }
            ),
        ),
        _ => (
            "404 Not Found",
            "text/plain; charset=utf-8",
            "not found\n".to_owned(),
        ),
    };
    let headers = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(headers.as_bytes()).await?;
    stream.write_all(body.as_bytes()).await?;
    stream.shutdown().await
}

fn request_path(request: &str) -> Option<&str> {
    request.lines().next()?.split_whitespace().nth(1)
}

fn is_metrics_path(path: &str) -> bool {
    path == "/metrics" || path.starts_with("/metrics?")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prometheus_render_includes_process_counters() {
        let body = render_prometheus(&RuntimeLifecycle::default());
        assert!(body.contains("quackgis_write_denied_total"));
        assert!(body.contains("quackgis_read_denied_total"));
        assert!(body.contains("quackgis_copy_batches_total"));
        assert!(body.contains("quackgis_copy_commit_microseconds_total"));
        assert!(body.contains("quackgis_connections_quarantined_total"));
        assert!(body.contains("quackgis_transactions_active"));
        assert!(body.contains("quackgis_duckdb_memory_bytes"));
        assert!(body.contains("quackgis_duckdb_temporary_storage_bytes"));
        assert!(!body.contains("QUACKGIS_S3_SECRET_ACCESS_KEY"));
        assert!(!body.contains("file:///"));
    }

    #[test]
    fn request_path_accepts_metrics_endpoint_only() {
        assert_eq!(
            request_path("GET /metrics HTTP/1.1\r\n\r\n"),
            Some("/metrics")
        );
        assert!(is_metrics_path("/metrics?format=prometheus"));
        assert!(!is_metrics_path("/metrics/private"));
        assert_eq!(
            request_path("GET /healthz HTTP/1.1\r\n\r\n"),
            Some("/healthz")
        );
        assert_eq!(
            request_path("GET /readyz HTTP/1.1\r\n\r\n"),
            Some("/readyz")
        );
    }

    #[tokio::test]
    async fn status_listener_serves_health_and_readiness() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind status");
        let address = listener.local_addr().expect("status address");
        let lifecycle = Arc::new(RuntimeLifecycle::default());
        let task = tokio::spawn(serve_listener(listener, Arc::clone(&lifecycle)));
        for (path, status, expected) in [
            ("/healthz", "200 OK", "ok\n"),
            ("/readyz", "503 Service Unavailable", "starting\n"),
        ] {
            let mut stream = TcpStream::connect(address).await.expect("connect status");
            stream
                .write_all(format!("GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes())
                .await
                .expect("write status request");
            let mut response = Vec::new();
            stream
                .read_to_end(&mut response)
                .await
                .expect("read status response");
            let response = String::from_utf8(response).expect("HTTP response text");
            assert!(response.starts_with(&format!("HTTP/1.1 {status}\r\n")));
            assert!(response.ends_with(expected));
        }
        lifecycle.mark_storage_ready();
        let mut stream = TcpStream::connect(address)
            .await
            .expect("connect ready status");
        stream
            .write_all(b"GET /readyz HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .await
            .expect("write ready request");
        let mut response = Vec::new();
        stream
            .read_to_end(&mut response)
            .await
            .expect("read ready response");
        let response = String::from_utf8(response).expect("HTTP response text");
        assert!(response.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(response.ends_with("ready\n"));
        lifecycle.mark_storage_unavailable();
        let mut stream = TcpStream::connect(address)
            .await
            .expect("connect failed status");
        stream
            .write_all(b"GET /readyz HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .await
            .expect("write failed request");
        let mut response = Vec::new();
        stream
            .read_to_end(&mut response)
            .await
            .expect("read failed response");
        let response = String::from_utf8(response).expect("HTTP response text");
        assert!(response.starts_with("HTTP/1.1 503 Service Unavailable\r\n"));
        assert!(response.ends_with("storage_unavailable\n"));
        lifecycle.mark_storage_ready();
        lifecycle.begin_drain();
        let mut stream = TcpStream::connect(address)
            .await
            .expect("connect draining status");
        stream
            .write_all(b"GET /readyz HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .await
            .expect("write draining readiness request");
        let mut response = Vec::new();
        stream
            .read_to_end(&mut response)
            .await
            .expect("read draining readiness response");
        let response = String::from_utf8(response).expect("HTTP response text");
        assert!(response.starts_with("HTTP/1.1 503 Service Unavailable\r\n"));
        assert!(response.ends_with("draining\n"));
        task.abort();
    }
}
