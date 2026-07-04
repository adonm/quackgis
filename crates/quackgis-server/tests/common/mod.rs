// SPDX-License-Identifier: Apache-2.0
//! Test harness: spin up `quackgis-server` on an ephemeral port for the duration
//! of a single test. Uses the same context-construction path as the real
//! binary so wire-test failures reflect real regressions.

use std::sync::Arc;

use datafusion::prelude::SessionContext;
use datafusion_postgres::{serve, ServerOptions};

use quackgis_server::context::build_session_context;

pub struct ServerHandle {
    /// Host:port clients connect to.
    host: String,
    port: u16,
    /// Keeps the server task alive; dropped on `ServerHandle` drop.
    _serve: tokio::task::JoinHandle<()>,
    /// Holds the SessionContext alive for the lifetime of the server.
    _ctx: Arc<SessionContext>,
}

impl ServerHandle {
    /// Start a server bound to `127.0.0.1:0` (OS-assigned ephemeral port) and
    /// wait until it is accepting connections. Panics on failure — tests only.
    pub async fn start() -> Self {
        // Bind a TCP listener ourselves first so we know the port before
        // handing it to datafusion-postgres (which would otherwise pick its
        // own and race the test).
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind ephemeral listener");
        let addr = listener.local_addr().expect("local_addr");
        drop(listener); // free the port so datafusion-postgres can rebind

        let ctx = build_session_context().await.expect("context builds");
        let ctx_for_server = Arc::clone(&ctx);

        let opts = ServerOptions::new()
            .with_host("127.0.0.1".to_string())
            .with_port(addr.port());

        let serve_task = tokio::spawn(async move {
            // Run forever; the task is aborted when ServerHandle drops.
            let _ = serve(ctx_for_server, &opts).await;
        });

        let handle = Self {
            host: "127.0.0.1".to_string(),
            port: addr.port(),
            _serve: serve_task,
            _ctx: ctx,
        };

        // Wait for the server to actually be listening by polling a TCP
        // connect. datafusion-postgres binds synchronously at the top of
        // `serve()`, so this normally succeeds within one retry.
        for _ in 0..50 {
            if tokio::net::TcpStream::connect((handle.host.as_str(), handle.port))
                .await
                .is_ok()
            {
                return handle;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!(
            "quackgis-server did not come up on {}:{}",
            handle.host, handle.port
        );
    }

    /// `tokio-postgres`-style connection string (`key=value key=value`).
    pub fn conn_str(&self) -> String {
        format!(
            "host={} port={} user=postgres dbname=quackgis",
            self.host, self.port
        )
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        self._serve.abort();
    }
}
