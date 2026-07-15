// SPDX-License-Identifier: Apache-2.0
use std::sync::Arc;

use quackgis_server::auth::AuthConfig;
use quackgis_server::duckdb_adbc_storage::{DuckDbAdbcConfig, DuckDbAdbcStorage, ExtensionPolicy};
use quackgis_server::pgwire_server::ServerOptions;

pub struct TestRuntime {
    storage: Arc<DuckDbAdbcStorage>,
    server: tokio::task::JoinHandle<Result<(), std::io::Error>>,
    _temp: tempfile::TempDir,
    port: u16,
}

impl TestRuntime {
    pub async fn start(options: ServerOptions) -> Self {
        Self::start_with_auth(options, AuthConfig::trust()).await
    }

    pub async fn start_with_auth(options: ServerOptions, auth: AuthConfig) -> Self {
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
        let server = tokio::spawn(async move {
            quackgis_server::pgwire_server::serve_duckdb_on_listener(
                server_storage,
                listener,
                &options,
                auth,
            )
            .await
        });
        Self {
            storage,
            server,
            _temp: temp,
            port,
        }
    }

    pub async fn connect(
        &self,
    ) -> (
        tokio_postgres::Client,
        tokio::task::JoinHandle<Result<(), tokio_postgres::Error>>,
    ) {
        let (client, connection) = tokio_postgres::connect(
            &format!(
                "host=127.0.0.1 port={} user=postgres dbname=quackgis",
                self.port
            ),
            tokio_postgres::NoTls,
        )
        .await
        .expect("profile pgwire connection");
        (client, tokio::spawn(connection))
    }

    pub fn storage(&self) -> &Arc<DuckDbAdbcStorage> {
        &self.storage
    }

    #[allow(dead_code)]
    pub fn port(&self) -> u16 {
        self.port
    }
}

impl Drop for TestRuntime {
    fn drop(&mut self) {
        self.server.abort();
    }
}
