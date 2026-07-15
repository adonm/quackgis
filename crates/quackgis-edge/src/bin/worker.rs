// SPDX-License-Identifier: Apache-2.0

use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use quackgis_edge::EDGE_ALPN;
use quackgis_edge::config::{WorkerConfig, endpoint_document};
use quackgis_edge::runtime::{WorkerAuthority, bind_endpoint, run_until_signal, serve_worker};
use tokio::sync::watch;

#[derive(Parser)]
#[command(name = "quackgis-worker-edge", version)]
struct Cli {
    #[arg(long)]
    config: PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    let cli = Cli::parse();
    let config = WorkerConfig::load(&cli.config)?;
    let endpoint = bind_endpoint(
        config.secret_key()?,
        vec![EDGE_ALPN.to_vec()],
        &config.relay_policy()?,
    )
    .await?;
    endpoint.online().await;
    println!("{}", endpoint_document(endpoint.id(), &endpoint.addr())?);
    let authority = WorkerAuthority {
        bootstrap_public_key: config.bootstrap_public_key()?,
        backend: config.backend,
        max_streams_per_connection: config.max_streams_per_connection,
    };
    let (_shutdown_guard, shutdown) = watch::channel(false);
    run_until_signal(
        endpoint.clone(),
        serve_worker(endpoint, authority, config.max_connections, shutdown),
    )
    .await
}
