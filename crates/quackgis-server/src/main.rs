// SPDX-License-Identifier: Apache-2.0
//! `quackgis-server` entry point. See [`cli::Cli`] for flags, [`context`] for
//! the session-construction integration point, and README.md for status.

use std::sync::Arc;

use clap::Parser;
use datafusion::prelude::SessionContext;
use datafusion_postgres::hooks::QueryHook;
use datafusion_postgres::hooks::cursor::CursorStatementHook;
use datafusion_postgres::hooks::set_show::SetShowHook;
use datafusion_postgres::hooks::transactions::TransactionStatementHook;
use datafusion_postgres::{ServerOptions, serve_with_hooks};
use tokio::signal;

use quackgis_server::catalog_compat::CatalogCompatHook;
use quackgis_server::cli::Cli;
use quackgis_server::ducklake_sql::DuckLakeSqlHook;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(&cli.log))
        .write_style(env_logger::WriteStyle::Auto)
        .init();

    let storage_paths =
        quackgis_server::context::StoragePaths::new(&cli.catalog_path, &cli.data_path)?;
    let ctx: Arc<SessionContext> =
        quackgis_server::context::build_session_context_with_storage(storage_paths.clone()).await?;

    let opts = ServerOptions::new()
        .with_host(cli.host.clone())
        .with_port(cli.port)
        .with_tls_cert_path(cli.tls_cert.clone())
        .with_tls_key_path(cli.tls_key.clone());

    let tls_note = if cli.tls_cert.is_some() {
        "TLS enabled"
    } else {
        "no TLS (dev mode)"
    };
    log::info!(
        "quackgis-server listening on {}:{} ({tls_note}); spatial engine: SedonaDB; pg_catalog: on",
        cli.host,
        cli.port
    );
    if cli.tls_cert.is_some() != cli.tls_key.is_some() {
        anyhow::bail!(
            "--tls-cert and --tls-key must be specified together (got cert={}, key={})",
            cli.tls_cert.is_some(),
            cli.tls_key.is_some()
        );
    }

    // datafusion-postgres' `serve` runs forever; we race it against a shutdown
    // signal so Ctrl-C produces a clean exit. The server has no built-in
    // cancellation today — when the signal wins we just exit the process,
    // which closes the listener and drops in-flight connections.
    let hooks: Vec<Arc<dyn QueryHook>> = vec![
        Arc::new(DuckLakeSqlHook::new(storage_paths)),
        Arc::new(CatalogCompatHook),
        Arc::new(CursorStatementHook),
        Arc::new(SetShowHook),
        Arc::new(TransactionStatementHook),
    ];
    let server = tokio::spawn(async move { serve_with_hooks(ctx, &opts, hooks).await });
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
