// SPDX-License-Identifier: Apache-2.0

use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::{Args, Parser, Subcommand};
use quackgis_migrate::connect::{ConnectionOptions, connect};
use quackgis_migrate::report::write_json_atomic;
use quackgis_migrate::{
    MigrationConfig, MigrationState, PreflightStatus, begin_source_snapshot, build_preflight,
    cleanup_configured_targets, run_migration,
};

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
    /// Copy and verify selected tables in one source and one target transaction.
    Run(RunArgs),
    /// Explicitly drop only the target tables named by the migration config.
    Cleanup(CleanupArgs),
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
struct RunArgs {
    #[arg(long)]
    config: PathBuf,
    #[arg(long)]
    out: PathBuf,
    #[command(flatten)]
    source: SourceConnectionArgs,
    #[command(flatten)]
    target: TargetConnectionArgs,
}

#[derive(Args)]
struct CleanupArgs {
    #[arg(long)]
    config: PathBuf,
    #[arg(long)]
    out: PathBuf,
    #[arg(long)]
    confirm_drop_configured_targets: bool,
    #[command(flatten)]
    target: TargetConnectionArgs,
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

#[derive(Args)]
struct TargetConnectionArgs {
    #[arg(long, env = "QUACKGIS_MIGRATE_TARGET_URL", hide_env_values = true)]
    target_url: String,
    #[arg(long, env = "QUACKGIS_MIGRATE_TARGET_PASSWORD_FILE")]
    target_password_file: Option<PathBuf>,
    #[arg(long, env = "QUACKGIS_MIGRATE_TARGET_CA")]
    target_ca: Option<PathBuf>,
    #[arg(long, env = "QUACKGIS_MIGRATE_TARGET_CLIENT_CERT")]
    target_client_cert: Option<PathBuf>,
    #[arg(long, env = "QUACKGIS_MIGRATE_TARGET_CLIENT_KEY")]
    target_client_key: Option<PathBuf>,
    #[arg(long, env = "QUACKGIS_MIGRATE_ALLOW_PLAINTEXT_TARGET_LOOPBACK")]
    allow_plaintext_target_loopback: bool,
}

impl TargetConnectionArgs {
    fn options(&self) -> ConnectionOptions {
        ConnectionOptions {
            url: self.target_url.clone(),
            password_file: self.target_password_file.clone(),
            ca_certificate: self.target_ca.clone(),
            client_certificate: self.target_client_cert.clone(),
            client_private_key: self.target_client_key.clone(),
            allow_plaintext_loopback: self.allow_plaintext_target_loopback,
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Preflight(args) => preflight(args).await,
        Command::Run(args) => run(args).await,
        Command::Cleanup(args) => cleanup(args).await,
    }
}

async fn cleanup(args: CleanupArgs) -> Result<()> {
    if !args.confirm_drop_configured_targets {
        bail!("cleanup requires --confirm-drop-configured-targets");
    }
    let config = MigrationConfig::load(&args.config)?;
    let report = cleanup_configured_targets(&config, &args.target.options()).await?;
    write_json_atomic(&args.out, &report)
}

async fn run(args: RunArgs) -> Result<()> {
    let config = MigrationConfig::load(&args.config)?;
    let report = run_migration(&config, &args.source.options(), &args.target.options()).await?;
    write_json_atomic(&args.out, &report)?;
    if report.state != MigrationState::Verified {
        bail!("migration did not reach verified state; inspect the path-free report");
    }
    Ok(())
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
