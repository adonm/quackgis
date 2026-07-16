// SPDX-License-Identifier: Apache-2.0

use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use quackgis_edge::CONTROL_ALPN;
use quackgis_edge::config::{BootstrapConfig, endpoint_document};
use quackgis_edge::runtime::{
    BootstrapAuthority, bind_endpoint_at, run_until_signal, serve_bootstrap,
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
    let relay_policy = config.relay_policy()?;
    let endpoint = bind_endpoint_at(
        secret,
        vec![CONTROL_ALPN.to_vec()],
        &relay_policy,
        config.bind,
    )
    .await?;
    if relay_policy != quackgis_edge::RelayPolicy::Disabled {
        tokio::time::timeout(std::time::Duration::from_secs(30), endpoint.online()).await?;
    }
    println!("{}", endpoint_document(endpoint.id(), &endpoint.addr())?);
    let (shutdown_guard, shutdown) = watch::channel(false);
    run_until_signal(
        endpoint.clone(),
        shutdown_guard,
        serve_bootstrap(endpoint, authority, config.max_connections, shutdown),
    )
    .await
}
