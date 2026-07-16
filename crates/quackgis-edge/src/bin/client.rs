// SPDX-License-Identifier: Apache-2.0

use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use quackgis_edge::compression::TransportMetrics;
use quackgis_edge::config::ClientConfig;
use quackgis_edge::runtime::{
    ClientConnector, LocalClientTls, bind_endpoint_at, run_until_signal,
    serve_local_client_with_tls,
};
use tokio::net::TcpListener;
use tokio::sync::watch;

#[derive(Parser)]
#[command(name = "quackgis-client", version)]
struct Cli {
    #[arg(long)]
    config: PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    let cli = Cli::parse();
    let config = ClientConfig::load(&cli.config)?;
    let credential = config.credential_secret_key()?;
    let endpoint = bind_endpoint_at(
        config.transport_secret_key()?,
        vec![],
        &config.relay_policy()?,
        config.bind,
    )
    .await?;
    let metrics = TransportMetrics::default();
    let connector = ClientConnector::new(endpoint.clone(), credential, config.bootstrap.parse()?)
        .with_compression(config.compression, metrics.clone());
    tokio::time::timeout(std::time::Duration::from_secs(10), connector.connect()).await??;
    let listener = TcpListener::bind(config.listen).await?;
    let local_tls = config
        .local_tls
        .as_ref()
        .map(|tls| {
            LocalClientTls::load(
                &tls.certificate_path,
                &tls.private_key_path,
                &tls.client_ca_path,
            )
        })
        .transpose()?;
    log::info!("tiny pgwire client listening on {}", listener.local_addr()?);
    let (shutdown_guard, shutdown) = watch::channel(false);
    let result = run_until_signal(
        endpoint.clone(),
        shutdown_guard,
        serve_local_client_with_tls(
            listener,
            connector,
            local_tls,
            config.max_connections,
            shutdown,
        ),
    )
    .await;
    log::info!(
        "transport_metrics={}",
        serde_json::to_string(&metrics.snapshot())?
    );
    result
}
