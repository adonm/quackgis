// SPDX-License-Identifier: Apache-2.0
//! `quackgis-server` entry point. See [`cli::Cli`] for flags, [`context`] for
//! the session-construction integration point, and README.md for status.

use std::sync::Arc;

use chrono::{Duration, Utc};
use clap::Parser;
use datafusion::prelude::SessionContext;
use datafusion_postgres::ServerOptions;
use tokio::net::TcpListener;
use tokio::signal;

use quackgis_server::auth::{AuthConfig, AuthMode, parse_read_allowlist, parse_write_allowlist};
use quackgis_server::cli::Cli;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(&cli.log))
        .write_style(env_logger::WriteStyle::Auto)
        .init();

    quackgis_server::ducklake_sql::configure_native_mutation_barrier_from_env()?;

    if cli.tls_cert.is_some() != cli.tls_key.is_some() {
        anyhow::bail!(
            "--tls-cert and --tls-key must be specified together (got cert={}, key={})",
            cli.tls_cert.is_some(),
            cli.tls_key.is_some()
        );
    }
    let mut auth = match cli.auth_mode {
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
    if let Some(raw_allowlist) = cli.write_allowlist.as_deref() {
        auth = auth.with_readwrite_allowlist(parse_write_allowlist(raw_allowlist)?);
    }
    if let Some(raw_allowlist) = cli.read_allowlist.as_deref() {
        auth = auth.with_read_allowlist(parse_read_allowlist(raw_allowlist)?);
    }

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
    if cli.orphan_inventory {
        if cli.orphan_min_age_seconds == 0 {
            anyhow::bail!("--orphan-min-age-seconds must be greater than zero");
        }
        let age_seconds = i64::try_from(cli.orphan_min_age_seconds)
            .map_err(|_| anyhow::anyhow!("--orphan-min-age-seconds is too large"))?;
        let cutoff = Utc::now()
            .checked_sub_signed(Duration::seconds(age_seconds))
            .ok_or_else(|| anyhow::anyhow!("orphan inventory cutoff is out of range"))?;
        if let Some(prefix) = cli.orphan_quarantine_prefix.as_deref() {
            let report = storage_paths
                .quarantine_orphan_candidates_before(cutoff, prefix, cli.orphan_quarantine_apply)
                .await?;
            println!(
                "quackgis_orphan_quarantine dry_run={} min_age_seconds={} candidates={} copied={} deleted={}",
                report.dry_run,
                cli.orphan_min_age_seconds,
                report.candidates.len(),
                report.copied_count,
                report.deleted_count
            );
            if cli.orphan_show_paths {
                for entry in report.candidates {
                    println!(
                        "orphan_quarantine_candidate source={} quarantine={}",
                        entry.source, entry.quarantine
                    );
                }
            }
        } else {
            let candidates = storage_paths.orphan_candidates_before(cutoff).await?;
            println!(
                "quackgis_orphan_inventory dry_run=true min_age_seconds={} candidates={}",
                cli.orphan_min_age_seconds,
                candidates.len()
            );
            if cli.orphan_show_paths {
                for path in candidates {
                    println!("orphan_candidate path={path}");
                }
            }
        }
        return Ok(());
    }
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

    let metrics_task = if let Some(metrics_port) = cli.metrics_port {
        let metrics_addr = format!("{}:{metrics_port}", cli.metrics_host);
        let listener = TcpListener::bind(&metrics_addr).await?;
        let local_addr = listener.local_addr()?;
        log::info!("quackgis metrics endpoint listening on http://{local_addr}/metrics");
        Some(tokio::spawn(quackgis_server::metrics::serve_listener(
            listener,
        )))
    } else {
        None
    };

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

    // datafusion-postgres' `serve` runs forever; race it against a shutdown
    // signal so Ctrl-C/SIGTERM produces a deterministic exit. The server has no built-in
    // cancellation today — when the signal wins we just exit the process,
    // which closes the listener and drops in-flight connections.
    let mut server = tokio::spawn(async move {
        quackgis_server::pgwire_server::serve_with_auth(ctx, &opts, storage_paths, auth).await
    });

    let outcome = tokio::select! {
        result = &mut server => {
            match result {
                Ok(result) => result.map_err(anyhow::Error::from),
                Err(err) => Err(anyhow::Error::from(err)),
            }
        }
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
