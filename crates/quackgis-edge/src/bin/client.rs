// SPDX-License-Identifier: Apache-2.0

use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use quackgis_edge::compression::TransportMetrics;
use quackgis_edge::config::ClientConfig;
use quackgis_edge::runtime::{
    ClientConnector, bind_endpoint, run_until_signal, serve_local_client,
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
    let endpoint = bind_endpoint(
        config.transport_secret_key()?,
        vec![],
        &config.relay_policy()?,
    )
    .await?;
    let metrics = TransportMetrics::default();
    let connector = ClientConnector::new(endpoint.clone(), credential, config.bootstrap.parse()?)
        .with_compression(config.compression, metrics.clone());
    connector.connect().await?;
    let listener = TcpListener::bind(config.listen).await?;
    log::info!("tiny pgwire client listening on {}", listener.local_addr()?);
    let (_shutdown_guard, shutdown) = watch::channel(false);
    let result = run_until_signal(
        endpoint.clone(),
        serve_local_client(listener, connector, config.max_connections, shutdown),
    )
    .await;
    log::info!(
        "transport_metrics={}",
        serde_json::to_string(&metrics.snapshot())?
    );
    result
}
