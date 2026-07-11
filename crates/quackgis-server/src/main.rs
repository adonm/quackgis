// SPDX-License-Identifier: Apache-2.0
//! DuckDB/official-DuckLake QuackGIS server entry point.

use std::sync::Arc;

use clap::Parser;
use tokio::net::TcpListener;
use tokio::signal;

use quackgis_server::auth::{AuthConfig, AuthMode, parse_read_allowlist, parse_write_allowlist};
use quackgis_server::cli::{Cli, CliAuthMode};
use quackgis_server::duckdb_adbc_storage::{DuckDbAdbcConfig, DuckDbAdbcStorage, ExtensionPolicy};
use quackgis_server::pgwire_server::ServerOptions;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(&cli.log))
        .write_style(env_logger::WriteStyle::Auto)
        .init();

    if cli.tls_cert.is_some() != cli.tls_key.is_some() {
        anyhow::bail!("--tls-cert and --tls-key must be specified together");
    }
    if cli.catalog_url.is_some()
        || cli.catalog_path.contains("://")
        || cli.data_path.contains("://")
    {
        anyhow::bail!("remote DuckLake profiles are not enabled yet; use local catalog/data paths");
    }

    let mut auth = match cli.auth_mode {
        CliAuthMode::Trust => AuthConfig::trust(),
        CliAuthMode::Password => AuthConfig::password(
            cli.readwrite_user.clone(),
            cli.readwrite_password.clone().ok_or_else(|| {
                anyhow::anyhow!("--readwrite-password is required with --auth-mode=password")
            })?,
            cli.readonly_password
                .clone()
                .map(|password| (cli.readonly_user.clone(), password)),
        )?,
    };
    if let Some(raw) = cli.write_allowlist.as_deref() {
        auth = auth.with_readwrite_allowlist(parse_write_allowlist(raw)?);
    }
    if let Some(raw) = cli.read_allowlist.as_deref() {
        auth = auth.with_read_allowlist(parse_read_allowlist(raw)?);
    }

    let driver_path = cli.duckdb_driver.clone().ok_or_else(|| {
        anyhow::anyhow!(
            "--duckdb-driver/QUACKGIS_DUCKDB_ADBC_DRIVER is required for the DuckDB runtime"
        )
    })?;
    let config = DuckDbAdbcConfig {
        driver_path,
        database_uri: cli.duckdb_database_uri.clone(),
        ducklake_uri: format!("ducklake:{}", cli.catalog_path),
        catalog_name: cli.ducklake_catalog_name.clone(),
        data_path: cli.data_path.clone(),
        extension_policy: ExtensionPolicy::LoadOnly,
    };
    let storage =
        Arc::new(tokio::task::spawn_blocking(move || DuckDbAdbcStorage::open(config)).await??);
    let options = ServerOptions::new()
        .with_host(cli.host.clone())
        .with_port(cli.port)
        .with_tls_cert_path(cli.tls_cert.clone())
        .with_tls_key_path(cli.tls_key.clone());

    let metrics_task = if let Some(port) = cli.metrics_port {
        let listener = TcpListener::bind(format!("{}:{port}", cli.metrics_host)).await?;
        log::info!(
            "quackgis metrics endpoint listening on http://{}/metrics",
            listener.local_addr()?
        );
        Some(tokio::spawn(quackgis_server::metrics::serve_listener(
            listener,
        )))
    } else {
        None
    };

    log::info!(
        "quackgis-server listening on {}:{} ({}; {}); engine=DuckDB; storage=official-DuckLake",
        cli.host,
        cli.port,
        if cli.tls_cert.is_some() {
            "TLS enabled"
        } else {
            "no TLS (dev mode)"
        },
        match auth.mode() {
            AuthMode::Trust => "trust auth (dev mode)",
            AuthMode::Password => "SCRAM password auth",
        }
    );

    let mut server = tokio::spawn(async move {
        quackgis_server::pgwire_server::serve_duckdb(storage, &options, auth).await
    });
    let outcome = tokio::select! {
        result = &mut server => match result {
            Ok(result) => result.map_err(anyhow::Error::from),
            Err(error) => Err(anyhow::Error::from(error)),
        },
        signal = shutdown_signal() => {
            signal?;
            log::info!("shutdown signal received, exiting");
            server.abort();
            let _ = server.await;
            Ok(())
        }
    };
    if let Some(task) = metrics_task {
        task.abort();
    }
    outcome
}

async fn shutdown_signal() -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        let mut terminate = signal::unix::signal(signal::unix::SignalKind::terminate())?;
        tokio::select! {
            result = signal::ctrl_c() => result.map_err(anyhow::Error::from),
            _ = terminate.recv() => Ok(()),
        }
    }
    #[cfg(not(unix))]
    {
        signal::ctrl_c().await.map_err(anyhow::Error::from)
    }
}
