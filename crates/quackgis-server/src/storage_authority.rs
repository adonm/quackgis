// SPDX-License-Identifier: Apache-2.0
//! Atomic local marker preventing incompatible writers from sharing a data root.

use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};

pub const AUTHORITY_MARKER_NAME: &str = "_quackgis/storage-authority-v1";
const MARKER: &str = "version=1\nauthority=duckdb-official-ducklake\n";

pub fn claim_local_root(root: &Path) -> Result<()> {
    std::fs::create_dir_all(root)
        .with_context(|| format!("creating storage root {}", root.display()))?;
    let marker = root.join(AUTHORITY_MARKER_NAME);
    if let Some(parent) = marker.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating marker directory {}", parent.display()))?;
    }
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&marker)
    {
        Ok(mut file) => {
            use std::io::Write;
            file.write_all(MARKER.as_bytes())?;
            file.sync_all()?;
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let metadata = std::fs::symlink_metadata(&marker)?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                bail!("storage authority marker is not a regular file");
            }
            let existing = std::fs::read_to_string(&marker)
                .map_err(|error| anyhow!("reading storage authority marker: {error}"))?;
            if existing == MARKER {
                Ok(())
            } else {
                bail!(
                    "storage root authority mismatch; migrate into a separate official DuckLake root"
                )
            }
        }
        Err(error) => Err(error).context("creating storage authority marker"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claim_is_idempotent_and_rejects_other_authority() {
        let root = tempfile::tempdir().expect("root");
        claim_local_root(root.path()).expect("first claim");
        claim_local_root(root.path()).expect("repeat claim");
        std::fs::write(
            root.path().join(AUTHORITY_MARKER_NAME),
            "authority=legacy\n",
        )
        .expect("replace marker");
        assert!(claim_local_root(root.path()).is_err());
    }
}
