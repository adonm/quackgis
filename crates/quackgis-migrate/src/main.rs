// SPDX-License-Identifier: Apache-2.0

use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::{Args, Parser, Subcommand};
use quackgis_migrate::connect::{ConnectionOptions, connect};
use quackgis_migrate::report::write_json_atomic;
use quackgis_migrate::{MigrationConfig, PreflightStatus, begin_source_snapshot, build_preflight};

#[derive(Parser)]
#[command(name = "quackgis-migrate", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Inventory a pinned source in one read-only repeatable-read snapshot.
    Preflight(PreflightArgs),
}

#[derive(Args)]
struct PreflightArgs {
    #[arg(long)]
    config: PathBuf,
    #[arg(long)]
    out: PathBuf,
    #[command(flatten)]
    source: SourceConnectionArgs,
}

#[derive(Args)]
struct SourceConnectionArgs {
    #[arg(long, env = "QUACKGIS_MIGRATE_SOURCE_URL", hide_env_values = true)]
    source_url: String,
    #[arg(long, env = "QUACKGIS_MIGRATE_SOURCE_PASSWORD_FILE")]
    source_password_file: Option<PathBuf>,
    #[arg(long, env = "QUACKGIS_MIGRATE_SOURCE_CA")]
    source_ca: Option<PathBuf>,
    #[arg(long, env = "QUACKGIS_MIGRATE_SOURCE_CLIENT_CERT")]
    source_client_cert: Option<PathBuf>,
    #[arg(long, env = "QUACKGIS_MIGRATE_SOURCE_CLIENT_KEY")]
    source_client_key: Option<PathBuf>,
    #[arg(long, env = "QUACKGIS_MIGRATE_ALLOW_PLAINTEXT_LOOPBACK")]
    allow_plaintext_loopback: bool,
}

impl SourceConnectionArgs {
    fn options(&self) -> ConnectionOptions {
        ConnectionOptions {
            url: self.source_url.clone(),
            password_file: self.source_password_file.clone(),
            ca_certificate: self.source_ca.clone(),
            client_certificate: self.source_client_cert.clone(),
            client_private_key: self.source_client_key.clone(),
            allow_plaintext_loopback: self.allow_plaintext_loopback,
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Preflight(args) => preflight(args).await,
    }
}

async fn preflight(args: PreflightArgs) -> Result<()> {
    let config = MigrationConfig::load(&args.config)?;
    let mut source = connect(&args.source.options()).await?;
    let (snapshot, inventory) = begin_source_snapshot(&mut source, &config).await?;
    let report = build_preflight(&config, inventory);
    write_json_atomic(&args.out, &report)?;
    snapshot.rollback().await?;
    if report.status == PreflightStatus::Rejected {
        bail!("migration preflight rejected; inspect the path-free report");
    }
    Ok(())
}
