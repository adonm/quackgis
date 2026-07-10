// SPDX-License-Identifier: Apache-2.0
//! Minimal Prometheus-compatible metrics endpoint.
//!
//! The endpoint is intentionally opt-in and process-local. It exposes only
//! counters that already exist for operations evidence and never includes SQL
//! text, object paths, user credentials, or client parameters.

use std::io;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::{catalog_metrics, ducklake_sql};

const METRICS_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

pub fn render_prometheus() -> String {
    let metrics = ducklake_sql::metrics_snapshot();
    format!(
        concat!(
            "# HELP quackgis_queries_started_total SQL statements observed at the pgwire hook boundary.\n",
            "# TYPE quackgis_queries_started_total counter\n",
            "quackgis_queries_started_total {}\n",
            "# HELP quackgis_transaction_ids_allocated_total DuckLake transaction staging identifiers allocated.\n",
            "# TYPE quackgis_transaction_ids_allocated_total counter\n",
            "quackgis_transaction_ids_allocated_total {}\n",
            "# HELP quackgis_write_denied_total Write or maintenance statements denied for read-only users.\n",
            "# TYPE quackgis_write_denied_total counter\n",
            "quackgis_write_denied_total {}\n",
            "# HELP quackgis_read_denied_total Read statements denied by an explicit QuackGIS read allowlist.\n",
            "# TYPE quackgis_read_denied_total counter\n",
            "quackgis_read_denied_total {}\n",
            "# HELP quackgis_catalog_refresh_total DuckLake catalog refreshes registered in this process.\n",
            "# TYPE quackgis_catalog_refresh_total counter\n",
            "quackgis_catalog_refresh_total {}\n",
            "# HELP quackgis_catalog_read_provider_calls_total PostgreSQL DuckLake MetadataProvider calls delegated by this process; excludes writes, pool setup, pgwire, object-store, and SQLite operations and does not claim physical network roundtrips.\n",
            "# TYPE quackgis_catalog_read_provider_calls_total counter\n",
            "quackgis_catalog_read_provider_calls_total {}\n",
            "# HELP quackgis_shared_catalog_read_refresh_total Shared-catalog refreshes triggered by read statements.\n",
            "# TYPE quackgis_shared_catalog_read_refresh_total counter\n",
            "quackgis_shared_catalog_read_refresh_total {}\n",
            "# HELP quackgis_shared_catalog_strong_refresh_total Shared-catalog refreshes forced by write, DDL, or maintenance statements.\n",
            "# TYPE quackgis_shared_catalog_strong_refresh_total counter\n",
            "quackgis_shared_catalog_strong_refresh_total {}\n",
            "# HELP quackgis_snapshot_reads_total Snapshot-pinned read queries that registered a snapshot catalog.\n",
            "# TYPE quackgis_snapshot_reads_total counter\n",
            "quackgis_snapshot_reads_total {}\n",
            "# HELP quackgis_snapshot_read_errors_total Snapshot-pinned read queries rejected before execution.\n",
            "# TYPE quackgis_snapshot_read_errors_total counter\n",
            "quackgis_snapshot_read_errors_total {}\n",
            "# HELP quackgis_native_delete_mutations_total Native DuckLake DELETE mutations committed.\n",
            "# TYPE quackgis_native_delete_mutations_total counter\n",
            "quackgis_native_delete_mutations_total {}\n",
            "# HELP quackgis_native_update_mutations_total Native DuckLake UPDATE mutations committed.\n",
            "# TYPE quackgis_native_update_mutations_total counter\n",
            "quackgis_native_update_mutations_total {}\n",
            "# HELP quackgis_native_compact_mutations_total Native bucket-compaction mutations committed.\n",
            "# TYPE quackgis_native_compact_mutations_total counter\n",
            "quackgis_native_compact_mutations_total {}\n",
            "# HELP quackgis_native_mutation_aborts_total Native DuckLake mutation attempts aborted before catalog commit.\n",
            "# TYPE quackgis_native_mutation_aborts_total counter\n",
            "quackgis_native_mutation_aborts_total {}\n",
            "# HELP quackgis_compactions_total Successful QuackGIS compaction calls.\n",
            "# TYPE quackgis_compactions_total counter\n",
            "quackgis_compactions_total {}\n",
        ),
        metrics.queries_started_total,
        metrics.transaction_ids_allocated_total,
        metrics.writes_denied_total,
        metrics.reads_denied_total,
        metrics.catalog_refresh_total,
        catalog_metrics::catalog_read_provider_calls_snapshot(),
        metrics.shared_catalog_read_refresh_total,
        metrics.shared_catalog_strong_refresh_total,
        metrics.snapshot_reads_total,
        metrics.snapshot_read_errors_total,
        metrics.native_delete_mutations_total,
        metrics.native_update_mutations_total,
        metrics.native_compact_mutations_total,
        metrics.native_mutation_aborts_total,
        metrics.compactions_total,
    )
}

pub async fn serve_listener(listener: TcpListener) -> io::Result<()> {
    loop {
        let (stream, peer) = listener.accept().await?;
        tokio::spawn(async move {
            if let Err(err) = handle_connection(stream).await {
                log::debug!("metrics scrape from {peer} failed: {err}");
            }
        });
    }
}

async fn handle_connection(mut stream: TcpStream) -> io::Result<()> {
    let mut buffer = [0_u8; 1024];
    let n = stream.read(&mut buffer).await?;
    let request = String::from_utf8_lossy(&buffer[..n]);
    let path = request_path(&request);
    let (status, content_type, body) = if path.is_some_and(is_metrics_path) {
        ("200 OK", METRICS_CONTENT_TYPE, render_prometheus())
    } else {
        (
            "404 Not Found",
            "text/plain; charset=utf-8",
            "not found\n".to_string(),
        )
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
        let body = render_prometheus();
        assert!(body.contains("# TYPE quackgis_queries_started_total counter"));
        assert!(body.contains("quackgis_write_denied_total"));
        assert!(body.contains("quackgis_read_denied_total"));
        assert!(body.contains("quackgis_catalog_refresh_total"));
        assert!(body.contains("# TYPE quackgis_catalog_read_provider_calls_total counter"));
        assert!(
            body.contains(
                "excludes writes, pool setup, pgwire, object-store, and SQLite operations and does not claim physical network roundtrips"
            )
        );
        assert!(body.contains("quackgis_snapshot_reads_total"));
        assert!(body.contains("quackgis_snapshot_read_errors_total"));
        assert!(body.contains("quackgis_native_delete_mutations_total"));
        assert!(body.contains("quackgis_native_mutation_aborts_total"));
        assert!(body.contains("quackgis_compactions_total"));
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
    }
}
