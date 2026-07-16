// SPDX-License-Identifier: Apache-2.0
//! DuckDB/official-DuckLake QuackGIS server entry point.

use std::sync::Arc;

use clap::Parser;
use tokio::net::TcpListener;
use tokio::signal;

use quackgis_server::auth::{AuthConfig, AuthMode, parse_read_allowlist, parse_write_allowlist};
use quackgis_server::cli::{Cli, CliAuthMode, CliTlsMode};
use quackgis_server::duckdb_adbc_storage::{
    DuckDbAdbcConfig, DuckDbAdbcStorage, DuckDbResourceConfig, ExtensionPolicy,
};
use quackgis_server::pgwire_server::{MAX_COPY_BATCHES_PER_CHUNK, ServerOptions};
use quackgis_server::role::RoleCatalog;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(&cli.log))
        .write_style(env_logger::WriteStyle::Auto)
        .init();

    if cli.tls_cert.is_some() != cli.tls_key.is_some() {
        anyhow::bail!("--tls-cert and --tls-key must be specified together");
    }
    if cli.dev_ducklake_extension.is_some() != cli.dev_ducklake_extension_sha256.is_some() {
        anyhow::bail!(
            "--dev-ducklake-extension and --dev-ducklake-extension-sha256 must be specified together"
        );
    }
    if cli.tls_mode == CliTlsMode::Required && cli.tls_cert.is_none() {
        anyhow::bail!("--tls-mode=required requires --tls-cert and --tls-key");
    }
    if cli.catalog_url.is_some()
        || cli.catalog_path.contains("://")
        || cli.data_path.contains("://")
    {
        anyhow::bail!("remote DuckLake profiles are not enabled yet; use local catalog/data paths");
    }
    if cli.max_connections == 0
        || cli.max_active_queries == 0
        || cli.max_reader_queries == 0
        || cli.max_writer_queries == 0
        || cli.max_maintenance_queries == 0
        || cli.max_reader_queries > cli.max_active_queries
        || cli.max_writer_queries > cli.max_active_queries
        || cli.max_maintenance_queries > cli.max_active_queries
        || cli.max_queued_queries == 0
        || cli.max_blocking_workers < 2
        || cli.max_active_queries >= cli.max_blocking_workers
        || cli.duckdb_threads == Some(0)
        || cli.duckdb_memory_limit_bytes == Some(0)
        || cli.duckdb_max_temp_directory_bytes == Some(0)
        || cli.copy_batch_rows == 0
        || cli.copy_batch_bytes == 0
        || cli.copy_max_row_bytes == 0
        || cli.result_batch_bytes == 0
        || cli.pgwire_max_frame_bytes < 4
        || cli.pgwire_max_frame_bytes < cli.copy_batch_bytes.saturating_add(4)
        || cli.shutdown_timeout_ms == 0
        || cli
            .copy_batch_rows
            .saturating_mul(MAX_COPY_BATCHES_PER_CHUNK)
            < cli.copy_batch_bytes
    {
        anyhow::bail!(
            "resource limits must be positive, operation-class limits must not exceed the global active-query limit, active queries must leave one reserved blocking control worker, the pgwire frame limit must accommodate COPY batch bytes plus its four-byte declared length, and COPY batch rows must bound one wire chunk to at most {MAX_COPY_BATCHES_PER_CHUNK} batches"
        );
    }

    if cli.auth_mode == CliAuthMode::EdgePreauthenticated {
        let host = cli.host.parse::<std::net::IpAddr>().map_err(|_| {
            anyhow::anyhow!("--auth-mode=edge-preauthenticated requires a literal loopback --host")
        })?;
        if !host.is_loopback() {
            anyhow::bail!("--auth-mode=edge-preauthenticated requires a literal loopback --host");
        }
        if cli.readwrite_password.is_some() || cli.readonly_password.is_some() {
            anyhow::bail!(
                "password settings are not accepted with --auth-mode=edge-preauthenticated"
            );
        }
    }
    let role_catalog = if let Some(path) = cli.role_config.as_deref() {
        let metadata = std::fs::metadata(path)?;
        if !metadata.is_file() || metadata.len() > 1_048_576 {
            anyhow::bail!(
                "--role-config must name a regular JSON file no larger than 1048576 bytes"
            );
        }
        let raw = std::fs::read_to_string(path)?;
        Some(RoleCatalog::from_json(&raw)?)
    } else {
        None
    };
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
        CliAuthMode::EdgePreauthenticated => {
            AuthConfig::edge_preauthenticated(role_catalog.clone().ok_or_else(|| {
                anyhow::anyhow!("--role-config is required with --auth-mode=edge-preauthenticated")
            })?)?
        }
    };
    if let Some(raw) = cli.write_allowlist.as_deref() {
        auth = auth.with_readwrite_allowlist(parse_write_allowlist(raw)?);
    }
    if let Some(raw) = cli.read_allowlist.as_deref() {
        auth = auth.with_read_allowlist(parse_read_allowlist(raw)?);
    }
    if let Some(user) = cli.maintenance_user.as_deref() {
        auth = auth.with_maintenance_user(user)?;
    }
    if cli.auth_mode != CliAuthMode::EdgePreauthenticated
        && let Some(role_catalog) = role_catalog
    {
        auth = auth.with_role_catalog(role_catalog)?;
    }

    let driver_path = cli.duckdb_driver.clone().ok_or_else(|| {
        anyhow::anyhow!(
            "--duckdb-driver/QUACKGIS_DUCKDB_ADBC_DRIVER is required for the DuckDB runtime"
        )
    })?;
    let extension_policy = match (
        cli.dev_ducklake_extension.clone(),
        cli.dev_ducklake_extension_sha256.clone(),
    ) {
        (Some(path), Some(sha256)) => {
            log::warn!(
                "using checksum-pinned unsigned development DuckLake extension {}; this configuration is not release-supported",
                path.display()
            );
            ExtensionPolicy::DevelopmentDuckLake { path, sha256 }
        }
        (None, None) => ExtensionPolicy::LoadOnly,
        _ => unreachable!("development extension arguments were validated as a pair"),
    };
    let development_ducklake = matches!(
        extension_policy,
        ExtensionPolicy::DevelopmentDuckLake { .. }
    );
    let config = DuckDbAdbcConfig {
        driver_path,
        database_uri: cli.duckdb_database_uri.clone(),
        ducklake_uri: format!("ducklake:{}", cli.catalog_path),
        catalog_name: cli.ducklake_catalog_name.clone(),
        data_path: cli.data_path.clone(),
        extension_policy,
    };
    let mut resources = DuckDbResourceConfig::for_data_path(&cli.data_path);
    if let Some(threads) = cli.duckdb_threads {
        resources.threads = threads;
    }
    if let Some(memory_limit_bytes) = cli.duckdb_memory_limit_bytes {
        resources.memory_limit_bytes = memory_limit_bytes;
    }
    if let Some(max_temp_directory_bytes) = cli.duckdb_max_temp_directory_bytes {
        resources.max_temp_directory_bytes = max_temp_directory_bytes;
    }
    if let Some(temp_directory) = cli.duckdb_temp_directory.clone() {
        resources.temp_directory = temp_directory;
    }
    log::info!(
        "DuckDB resources: threads={}, memory_limit_bytes={}, spill_directory={}, max_spill_bytes={}",
        resources.threads,
        resources.memory_limit_bytes,
        resources.temp_directory.display(),
        resources.max_temp_directory_bytes,
    );
    let storage = Arc::new(
        tokio::task::spawn_blocking(move || {
            DuckDbAdbcStorage::open_with_resources(config, resources)
        })
        .await??,
    );
    let lifecycle = storage.lifecycle();
    let options = ServerOptions::new()
        .with_host(cli.host.clone())
        .with_port(cli.port)
        .with_max_connections(cli.max_connections)
        .with_max_active_queries(cli.max_active_queries)
        .with_max_reader_queries(cli.max_reader_queries)
        .with_max_writer_queries(cli.max_writer_queries)
        .with_max_maintenance_queries(cli.max_maintenance_queries)
        .with_max_queued_queries(cli.max_queued_queries)
        .with_max_blocking_workers(cli.max_blocking_workers)
        .with_queue_timeout(std::time::Duration::from_millis(cli.queue_timeout_ms))
        .with_statement_timeout(std::time::Duration::from_millis(cli.statement_timeout_ms))
        .with_result_batch_bytes(cli.result_batch_bytes)
        .with_pgwire_max_frame_bytes(cli.pgwire_max_frame_bytes)
        .with_copy_batch_rows(cli.copy_batch_rows)
        .with_copy_batch_bytes(cli.copy_batch_bytes)
        .with_copy_max_row_bytes(cli.copy_max_row_bytes)
        .with_tls_cert_path(cli.tls_cert.clone())
        .with_tls_key_path(cli.tls_key.clone())
        .with_tls_required(cli.tls_mode == CliTlsMode::Required);

    let pgwire_listener = TcpListener::bind(format!("{}:{}", cli.host, cli.port)).await?;
    let pgwire_address = pgwire_listener.local_addr()?;
    let readiness_storage = Arc::clone(&storage);
    tokio::task::spawn_blocking(move || readiness_storage.operational_readiness_probe()).await??;
    lifecycle.mark_storage_ready();

    let (metrics_task, resource_sampler_task) = if let Some(port) = cli.metrics_port {
        let resource_session = Arc::new(storage.open_session()?);
        let listener = TcpListener::bind(format!("{}:{port}", cli.metrics_host)).await?;
        log::info!(
            "quackgis HTTP status/metrics endpoint listening on http://{} (/healthz, /readyz, /metrics)",
            listener.local_addr()?
        );
        (
            Some(tokio::spawn(quackgis_server::metrics::serve_listener(
                listener,
                Arc::clone(&lifecycle),
            ))),
            Some(tokio::spawn(
                quackgis_server::metrics::sample_duckdb_resources(
                    resource_session,
                    Arc::clone(&lifecycle),
                    std::time::Duration::from_secs(15),
                ),
            )),
        )
    } else {
        (None, None)
    };

    log::info!(
        "quackgis-server listening on {}:{} ({}; {}); engine=DuckDB; storage={}",
        pgwire_address.ip(),
        pgwire_address.port(),
        if cli.tls_mode == CliTlsMode::Required {
            "TLS required"
        } else if cli.tls_cert.is_some() {
            "TLS available"
        } else {
            "no TLS (dev mode)"
        },
        match auth.mode() {
            AuthMode::Trust => "trust auth (dev mode)",
            AuthMode::Password => "SCRAM password auth",
            AuthMode::EdgePreauthenticated => "loopback edge-preauthenticated auth",
        },
        if development_ducklake {
            "checksum-pinned-development-DuckLake"
        } else {
            "official-DuckLake"
        },
    );

    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let shutdown_timeout = std::time::Duration::from_millis(cli.shutdown_timeout_ms);
    let mut server = tokio::spawn(async move {
        quackgis_server::pgwire_server::serve_duckdb_on_listener_until(
            storage,
            pgwire_listener,
            &options,
            auth,
            shutdown_rx,
        )
        .await
    });
    let outcome = tokio::select! {
        result = &mut server => match result {
            Ok(result) => result.map_err(anyhow::Error::from),
            Err(error) => Err(anyhow::Error::from(error)),
        },
        signal = shutdown_signal() => {
            signal?;
            lifecycle.begin_drain();
            let _ = shutdown_tx.send(true);
            log::info!("shutdown signal received, draining connections and transactions");
            match tokio::time::timeout(shutdown_timeout, &mut server).await {
                Ok(Ok(result)) => result.map_err(anyhow::Error::from),
                Ok(Err(error)) => Err(anyhow::Error::from(error)),
                Err(_) => {
                    log::warn!("graceful shutdown deadline elapsed; aborting remaining connections");
                    server.abort();
                    let _ = server.await;
                    Ok(())
                }
            }
        }
    };
    if let Some(task) = metrics_task {
        task.abort();
    }
    if let Some(task) = resource_sampler_task {
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
