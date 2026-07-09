// SPDX-License-Identifier: Apache-2.0
//! `quackgis-server` entry point. See [`cli::Cli`] for flags, [`context`] for
//! the session-construction integration point, and README.md for status.

use std::sync::Arc;

use clap::Parser;
use datafusion::prelude::SessionContext;
use datafusion_postgres::ServerOptions;
use tokio::net::TcpListener;
use tokio::signal;

use quackgis_server::auth::{AuthConfig, AuthMode};
use quackgis_server::cli::Cli;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(&cli.log))
        .write_style(env_logger::WriteStyle::Auto)
        .init();

    if cli.tls_cert.is_some() != cli.tls_key.is_some() {
        anyhow::bail!(
            "--tls-cert and --tls-key must be specified together (got cert={}, key={})",
            cli.tls_cert.is_some(),
            cli.tls_key.is_some()
        );
    }
    let auth = match cli.auth_mode {
        quackgis_server::cli::CliAuthMode::Trust => AuthConfig::trust(),
        quackgis_server::cli::CliAuthMode::Password => AuthConfig::password(
            cli.readwrite_user.clone(),
            cli.readwrite_password.clone().ok_or_else(|| {
                anyhow::anyhow!(
                    "--readwrite-password/QUACKGIS_READWRITE_PASSWORD is required when --auth-mode=password"
                )
            })?,
            cli.readonly_password
                .clone()
                .map(|password| (cli.readonly_user.clone(), password)),
        )?,
    };

    let s3 = quackgis_server::context::S3StorageOptions::new(
        cli.s3_endpoint.clone(),
        cli.s3_access_key_id.clone(),
        cli.s3_secret_access_key.clone(),
        cli.s3_region.clone(),
        cli.s3_allow_http,
    )?;
    let storage_paths = if let Some(catalog_url) = cli.catalog_url.clone() {
        quackgis_server::context::StoragePaths::postgres(
            catalog_url,
            cli.ducklake_catalog_name.clone(),
            cli.data_path.clone(),
            s3,
        )?
    } else {
        if s3.is_some() {
            anyhow::bail!("S3 options require --catalog-url and an s3:// --data-path");
        }
        quackgis_server::context::StoragePaths::new(&cli.catalog_path, &cli.data_path)?
    };
    let ctx: Arc<SessionContext> =
        quackgis_server::context::build_session_context_with_storage_and_auth(
            storage_paths.clone(),
            &auth,
        )
        .await?;

    let opts = ServerOptions::new()
        .with_host(cli.host.clone())
        .with_port(cli.port)
        .with_tls_cert_path(cli.tls_cert.clone())
        .with_tls_key_path(cli.tls_key.clone());

    if let Some(metrics_port) = cli.metrics_port {
        let metrics_addr = format!("{}:{metrics_port}", cli.metrics_host);
        let listener = TcpListener::bind(&metrics_addr).await?;
        let local_addr = listener.local_addr()?;
        log::info!("quackgis metrics endpoint listening on http://{local_addr}/metrics");
        tokio::spawn(quackgis_server::metrics::serve_listener(listener));
    }

    let tls_note = if cli.tls_cert.is_some() {
        "TLS enabled"
    } else {
        "no TLS (dev mode)"
    };
    let auth_note = match auth.mode() {
        AuthMode::Trust => "trust auth (dev mode)",
        AuthMode::Password => "SCRAM password auth",
    };
    log::info!(
        "quackgis-server listening on {}:{} ({tls_note}; {auth_note}); spatial engine: SedonaDB; pg_catalog: on",
        cli.host,
        cli.port
    );

    // datafusion-postgres' `serve` runs forever; we race it against a shutdown
    // signal so Ctrl-C produces a clean exit. The server has no built-in
    // cancellation today — when the signal wins we just exit the process,
    // which closes the listener and drops in-flight connections.
    let server = tokio::spawn(async move {
        quackgis_server::pgwire_server::serve_with_auth(ctx, &opts, storage_paths, auth).await
    });
    let shutdown = tokio::spawn(async move {
        let _ = signal::ctrl_c().await;
        log::info!("ctrl-c received, exiting");
    });

    tokio::select! {
        _ = server => {
            log::error!("server task exited unexpectedly");
        }
        _ = shutdown => {
            // Ctrl-C path: process exit closes the listener.
        }
    }

    Ok(())
}
