// SPDX-License-Identifier: Apache-2.0

use std::io::Write;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use iroh::SecretKey;

#[derive(Parser)]
#[command(name = "quackgis-keygen", version)]
struct Cli {
    #[arg(long)]
    out: PathBuf,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let secret = SecretKey::generate();
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&cli.out)
        .with_context(|| format!("cannot create key file {}", cli.out.display()))?;
    writeln!(file, "{}", hex::encode(secret.to_bytes()))?;
    file.sync_all()?;
    println!("{}", secret.public());
    Ok(())
}
