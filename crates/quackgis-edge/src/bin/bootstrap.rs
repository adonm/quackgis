// SPDX-License-Identifier: Apache-2.0

use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use quackgis_edge::CONTROL_ALPN;
use quackgis_edge::config::{BootstrapConfig, endpoint_document};
use quackgis_edge::runtime::{
    BootstrapAuthority, bind_endpoint, run_until_signal, serve_bootstrap,
};
use tokio::sync::watch;

#[derive(Parser)]
#[command(name = "quackgis-bootstrap", version)]
struct Cli {
    #[arg(long)]
    config: PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    let cli = Cli::parse();
    let config = BootstrapConfig::load(&cli.config)?;
    let secret = config.secret_key()?;
    let authority = BootstrapAuthority::new(
        secret.clone(),
        config.registered_credential()?,
        config.login_role.clone(),
        config.worker.parse()?,
        config.assignment_generation,
        config.lease_ttl_seconds,
    )?;
    let endpoint =
        bind_endpoint(secret, vec![CONTROL_ALPN.to_vec()], &config.relay_policy()?).await?;
    endpoint.online().await;
    println!("{}", endpoint_document(endpoint.id(), &endpoint.addr())?);
    let (_shutdown_guard, shutdown) = watch::channel(false);
    run_until_signal(
        endpoint.clone(),
        serve_bootstrap(endpoint, authority, config.max_connections, shutdown),
    )
    .await
}
