// SPDX-License-Identifier: Apache-2.0

use std::io::Write;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::{ArgGroup, Parser};
use iroh::SecretKey;
use quackgis_edge::config::load_secret_key;

#[derive(Parser)]
#[command(name = "quackgis-keygen", version)]
#[command(group(ArgGroup::new("action").required(true).multiple(false)))]
struct Cli {
    #[arg(long, group = "action")]
    out: Option<PathBuf>,
    #[arg(long, group = "action")]
    public_from: Option<PathBuf>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    if let Some(path) = cli.public_from {
        println!("{}", load_secret_key(&path)?.public());
        return Ok(());
    }
    let Some(out) = cli.out else {
        bail!("one key action is required");
    };
    let secret = SecretKey::generate();
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&out)
        .with_context(|| format!("cannot create key file {}", out.display()))?;
    writeln!(file, "{}", hex::encode(secret.to_bytes()))?;
    file.sync_all()?;
    println!("{}", secret.public());
    Ok(())
}
