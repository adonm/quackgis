// SPDX-License-Identifier: Apache-2.0

use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::{Args, Parser, Subcommand};
use quackgis_migrate::connect::{ConnectionOptions, connect};
use quackgis_migrate::report::{read_bound_json, write_json_atomic};
use quackgis_migrate::{
    MigrationConfig, MigrationReport, MigrationState, PreflightStatus, PromotionState,
    RuntimeIdentityOptions, VerificationReport, VerificationState, begin_source_snapshot,
    build_preflight, build_staging_config, cleanup_configured_targets, cleanup_staging,
    collect_runtime_identity, promote_migration_report, run_migration, verify_migration_report,
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
    /// Reverify an exact staging report through a fresh target session.
    Verify(VerifyArgs),
    /// Atomically publish exact verified staging tables as the configured release.
    Promote(PromoteArgs),
    /// Explicitly drop only staging tables bound by an exact migration report.
    Cleanup(CleanupArgs),
    /// Preview-only reset of exact target tables named by a configuration.
    ResetConfiguredTargets(ResetConfiguredTargetsArgs),
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
    #[arg(long)]
    staging_id: Option<String>,
    #[arg(long, env = "QUACKGIS_MIGRATE_PROGRESS_OUT")]
    progress_out: Option<PathBuf>,
    #[command(flatten)]
    source: SourceConnectionArgs,
    #[command(flatten)]
    target: TargetConnectionArgs,
    #[command(flatten)]
    runtime: RuntimeIdentityArgs,
}

#[derive(Args)]
struct VerifyArgs {
    #[arg(long)]
    report: PathBuf,
    #[arg(long)]
    report_sha256: String,
    #[arg(long)]
    out: PathBuf,
    #[command(flatten)]
    target: TargetConnectionArgs,
    #[command(flatten)]
    runtime: RuntimeIdentityArgs,
}

#[derive(Args)]
struct CleanupArgs {
    #[arg(long)]
    report: PathBuf,
    #[arg(long)]
    report_sha256: String,
    #[arg(long)]
    out: PathBuf,
    #[arg(long)]
    confirm_cleanup_staging: bool,
    #[command(flatten)]
    target: TargetConnectionArgs,
    #[command(flatten)]
    runtime: RuntimeIdentityArgs,
}

#[derive(Args)]
struct PromoteArgs {
    #[arg(long)]
    report: PathBuf,
    #[arg(long)]
    report_sha256: String,
    #[arg(long)]
    verification_report: PathBuf,
    #[arg(long)]
    verification_report_sha256: String,
    #[arg(long)]
    out: PathBuf,
    #[arg(long)]
    confirm_promote: bool,
    #[command(flatten)]
    target: TargetConnectionArgs,
    #[command(flatten)]
    runtime: RuntimeIdentityArgs,
}

#[derive(Args)]
struct ResetConfiguredTargetsArgs {
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
struct RuntimeIdentityArgs {
    #[arg(long, env = "QUACKGIS_MIGRATE_RUNTIME_MANIFEST")]
    runtime_manifest: Option<PathBuf>,
    #[arg(long, env = "QUACKGIS_MIGRATE_TARGET_RUNTIME_IMAGE")]
    target_runtime_image: Option<String>,
}

impl RuntimeIdentityArgs {
    fn collect(&self) -> Result<quackgis_migrate::RuntimeIdentity> {
        collect_runtime_identity(&RuntimeIdentityOptions {
            artifact_manifest: self.runtime_manifest.clone(),
            target_runtime_image: self.target_runtime_image.clone(),
        })
    }
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
        Command::Verify(args) => verify(args).await,
        Command::Promote(args) => promote(args).await,
        Command::Cleanup(args) => cleanup(args).await,
        Command::ResetConfiguredTargets(args) => reset_configured_targets(args).await,
    }
}

async fn cleanup(args: CleanupArgs) -> Result<()> {
    if !args.confirm_cleanup_staging {
        bail!("cleanup requires --confirm-cleanup-staging");
    }
    let report: MigrationReport = read_bound_json(&args.report, &args.report_sha256)?;
    let cleanup = cleanup_staging(
        &report,
        args.report_sha256,
        &args.target.options(),
        args.runtime.collect()?,
    )
    .await?;
    write_json_atomic(&args.out, &cleanup)
}

async fn reset_configured_targets(args: ResetConfiguredTargetsArgs) -> Result<()> {
    if !args.confirm_drop_configured_targets {
        bail!("reset requires --confirm-drop-configured-targets");
    }
    let config = MigrationConfig::load(&args.config)?;
    let report = cleanup_configured_targets(&config, &args.target.options()).await?;
    write_json_atomic(&args.out, &report)
}

async fn promote(args: PromoteArgs) -> Result<()> {
    if !args.confirm_promote {
        bail!("promotion requires --confirm-promote");
    }
    let report: MigrationReport = read_bound_json(&args.report, &args.report_sha256)?;
    let verification: VerificationReport =
        read_bound_json(&args.verification_report, &args.verification_report_sha256)?;
    let promotion = promote_migration_report(
        &report,
        args.report_sha256,
        &verification,
        args.verification_report_sha256,
        &args.target.options(),
        args.runtime.collect()?,
    )
    .await;
    write_json_atomic(&args.out, &promotion)?;
    if promotion.state != PromotionState::Promoted {
        bail!("migration was not promoted; inspect the path-free report");
    }
    Ok(())
}

async fn run(args: RunArgs) -> Result<()> {
    let release_config = MigrationConfig::load(&args.config)?;
    let (config, staging) = match args.staging_id.as_deref() {
        Some(staging_id) => {
            let (config, staging) = build_staging_config(&release_config, staging_id)?;
            (config, Some(staging))
        }
        None => (release_config, None),
    };
    let report = run_migration(
        &config,
        &args.source.options(),
        &args.target.options(),
        args.runtime.collect()?,
        staging,
        args.progress_out.as_deref(),
    )
    .await?;
    write_json_atomic(&args.out, &report)?;
    if report.state != MigrationState::Verified {
        bail!("migration did not reach verified state; inspect the path-free report");
    }
    Ok(())
}

async fn verify(args: VerifyArgs) -> Result<()> {
    let report: MigrationReport = read_bound_json(&args.report, &args.report_sha256)?;
    let verification = verify_migration_report(
        &report,
        args.report_sha256,
        &args.target.options(),
        args.runtime.collect()?,
    )
    .await;
    write_json_atomic(&args.out, &verification)?;
    if verification.state != VerificationState::Verified {
        bail!("migration verification failed; inspect the path-free report");
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
