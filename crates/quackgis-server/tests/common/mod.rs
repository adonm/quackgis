// SPDX-License-Identifier: Apache-2.0
//! Test harness: spin up `quackgis-server` on an ephemeral port with a fresh
//! DuckLake catalog in a per-test tempdir. Uses the same context-construction
//! path as the real binary so wire-test failures reflect real regressions.

use std::sync::Arc;

use datafusion::prelude::SessionContext;
use datafusion_postgres::ServerOptions;

use quackgis_server::context::{StoragePaths, build_session_context_with_storage};

pub struct ServerHandle {
    /// Host clients connect to.
    host: String,
    /// Port clients connect to.
    port: u16,
    /// Keeps the server task alive; dropped on `ServerHandle` drop.
    _serve: tokio::task::JoinHandle<()>,
    /// Holds the SessionContext alive for the lifetime of the server.
    _ctx: Arc<SessionContext>,
    /// Owns the on-disk DuckLake catalog + data files for this test.
    _tmp: tempfile::TempDir,
}

impl ServerHandle {
    /// Start a server with a fresh, empty DuckLake tempdir.
    #[allow(dead_code)]
    pub async fn start() -> Self {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        Self::start_with_tempdir(tmp).await
    }

    /// Start a server against an existing tempdir. Tests can pre-populate the
    /// DuckLake (writer API) before calling this to verify the server can read
    /// a specific on-disk state. The handle owns the tempdir and deletes it on
    /// drop.
    pub async fn start_with_tempdir(tmp: tempfile::TempDir) -> Self {
        let catalog_path = tmp.path().join("quackgis.db");
        let data_path = tmp.path().join("data");
        let paths = StoragePaths::new(catalog_path.to_str().unwrap(), data_path.to_str().unwrap())
            .expect("storage paths");
        Self::start_with_storage(tmp, paths).await
    }

    async fn start_with_storage(tmp: tempfile::TempDir, paths: StoragePaths) -> Self {
        // Bind a TCP listener ourselves first so we know the port before
        // handing it to datafusion-postgres (which would otherwise pick its
        // own and race the test).
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind ephemeral listener");
        let addr = listener.local_addr().expect("local_addr");
        drop(listener); // free the port so datafusion-postgres can rebind

        let ctx = build_session_context_with_storage(paths.clone())
            .await
            .expect("context builds");
        let ctx_for_server = Arc::clone(&ctx);

        let opts = ServerOptions::new()
            .with_host("127.0.0.1".to_string())
            .with_port(addr.port());

        let serve_task = tokio::spawn(async move {
            // Run forever; the task is aborted when ServerHandle drops.
            let _ = quackgis_server::pgwire_server::serve(ctx_for_server, &opts, paths).await;
        });

        let handle = Self {
            host: "127.0.0.1".to_string(),
            port: addr.port(),
            _serve: serve_task,
            _ctx: ctx,
            _tmp: tmp,
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

    /// Per-test tempdir path (for tests that want to inspect the on-disk
    /// DuckLake catalog or Parquet files directly).
    #[allow(dead_code)]
    pub fn tmp_dir(&self) -> &std::path::Path {
        self._tmp.path()
    }

    /// Storage paths backing this test server.
    #[allow(dead_code)]
    pub fn storage_paths(&self) -> StoragePaths {
        StoragePaths::new(
            self._tmp.path().join("quackgis.db").to_str().unwrap(),
            self._tmp.path().join("data").to_str().unwrap(),
        )
        .expect("storage paths")
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        self._serve.abort();
    }
}
