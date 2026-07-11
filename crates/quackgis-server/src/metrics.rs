// SPDX-License-Identifier: Apache-2.0
//! Minimal Prometheus-compatible metrics endpoint.
//!
//! The endpoint is intentionally opt-in and process-local. It exposes only
//! counters that already exist for operations evidence and never includes SQL
//! text, object paths, user credentials, or client parameters.

use std::io;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const METRICS_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

pub fn render_prometheus() -> String {
    format!(
        concat!(
            "# HELP quackgis_write_denied_total DuckDB write statements denied by policy.\n",
            "# TYPE quackgis_write_denied_total counter\n",
            "quackgis_write_denied_total {}\n",
            "# HELP quackgis_read_denied_total DuckDB reads denied by policy.\n",
            "# TYPE quackgis_read_denied_total counter\n",
            "quackgis_read_denied_total {}\n",
        ),
        crate::statement_policy::writes_denied_total(),
        crate::statement_policy::reads_denied_total(),
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
        assert!(body.contains("quackgis_write_denied_total"));
        assert!(body.contains("quackgis_read_denied_total"));
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
