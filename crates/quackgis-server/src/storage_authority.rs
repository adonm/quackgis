// SPDX-License-Identifier: Apache-2.0
//! Durable writer-authority marker for DuckLake data roots.
//!
//! A root may be claimed by exactly one storage implementation. The marker is
//! deliberately small and contains no catalog URL or credentials. Object-store
//! creation uses a create-only conditional write so racing authorities cannot
//! both succeed.

use std::fmt;
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use object_store::path::Path as ObjectPath;
use object_store::{ObjectStore, ObjectStoreExt, PutMode, PutOptions};

pub const AUTHORITY_MARKER_NAME: &str = "_quackgis/storage-authority-v1";
const MARKER_VERSION: &str = "1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageAuthority {
    LegacyDataFusionDuckLake,
    DuckDbOfficialDuckLake,
}

impl StorageAuthority {
    pub const fn id(self) -> &'static str {
        match self {
            Self::LegacyDataFusionDuckLake => "legacy-datafusion-ducklake",
            Self::DuckDbOfficialDuckLake => "duckdb-official-ducklake",
        }
    }

    fn marker(self) -> String {
        format!("version={MARKER_VERSION}\nauthority={}\n", self.id())
    }
}

impl fmt::Display for StorageAuthority {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.id())
    }
}

pub fn marker_key(prefix: &str) -> ObjectPath {
    let prefix = prefix.trim_matches('/');
    if prefix.is_empty() {
        ObjectPath::from(AUTHORITY_MARKER_NAME)
    } else {
        ObjectPath::from(format!("{prefix}/{AUTHORITY_MARKER_NAME}"))
    }
}

pub async fn claim_object_store(
    store: &dyn ObjectStore,
    marker: &ObjectPath,
    requested: StorageAuthority,
) -> Result<()> {
    let options = PutOptions {
        mode: PutMode::Create,
        ..PutOptions::default()
    };
    match store
        .put_opts(marker, requested.marker().into(), options)
        .await
    {
        Ok(_) => Ok(()),
        Err(object_store::Error::AlreadyExists { .. }) => {
            let existing = store
                .get(marker)
                .await
                .context("reading existing storage-authority marker")?
                .bytes()
                .await
                .context("reading storage-authority marker bytes")?;
            validate_marker(existing.as_ref(), requested)
        }
        Err(error) => Err(error).context("creating storage-authority marker"),
    }
}

pub fn claim_local_root(root: &Path, requested: StorageAuthority) -> Result<()> {
    std::fs::create_dir_all(root)
        .with_context(|| format!("creating storage-authority root {}", root.display()))?;
    let marker = root.join(AUTHORITY_MARKER_NAME);
    if let Some(parent) = marker.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating authority marker directory {}", parent.display()))?;
    }

    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    match options.open(&marker) {
        Ok(mut file) => {
            use std::io::Write;
            file.write_all(requested.marker().as_bytes())
                .context("writing storage-authority marker")?;
            file.sync_all()
                .context("syncing storage-authority marker")?;
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let metadata = std::fs::symlink_metadata(&marker).with_context(|| {
                format!(
                    "reading storage-authority marker metadata {}",
                    marker.display()
                )
            })?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                bail!(
                    "storage-authority marker is not a regular file: {}",
                    marker.display()
                );
            }
            let existing = std::fs::read(&marker).with_context(|| {
                format!("reading storage-authority marker {}", marker.display())
            })?;
            validate_marker(&existing, requested)
        }
        Err(error) => Err(error)
            .with_context(|| format!("creating storage-authority marker {}", marker.display())),
    }
}

fn validate_marker(existing: &[u8], requested: StorageAuthority) -> Result<()> {
    let existing = std::str::from_utf8(existing)
        .map_err(|_| anyhow!("storage-authority marker is not valid UTF-8"))?;
    if existing == requested.marker() {
        return Ok(());
    }
    let existing_authority = existing
        .lines()
        .find_map(|line| line.strip_prefix("authority="))
        .unwrap_or("malformed-or-unknown");
    bail!(
        "storage root authority mismatch: requested {}, existing {}; use separate catalog/data roots and a tested export/import",
        requested,
        existing_authority
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use object_store::memory::InMemory;

    #[tokio::test]
    async fn object_store_claim_is_idempotent_and_rejects_mixed_writers() {
        let store = InMemory::new();
        let marker = marker_key("datasets/city");
        claim_object_store(&store, &marker, StorageAuthority::LegacyDataFusionDuckLake)
            .await
            .expect("initial authority claim");
        claim_object_store(&store, &marker, StorageAuthority::LegacyDataFusionDuckLake)
            .await
            .expect("idempotent authority claim");

        let error = claim_object_store(&store, &marker, StorageAuthority::DuckDbOfficialDuckLake)
            .await
            .expect_err("mixed storage authorities must fail closed");
        assert!(error.to_string().contains("authority mismatch"));
    }

    #[test]
    fn local_claim_is_idempotent_and_rejects_mixed_writers() {
        let temp = tempfile::tempdir().expect("temporary authority root");
        claim_local_root(temp.path(), StorageAuthority::DuckDbOfficialDuckLake)
            .expect("initial authority claim");
        claim_local_root(temp.path(), StorageAuthority::DuckDbOfficialDuckLake)
            .expect("idempotent authority claim");

        let error = claim_local_root(temp.path(), StorageAuthority::LegacyDataFusionDuckLake)
            .expect_err("mixed storage authorities must fail closed");
        assert!(error.to_string().contains("authority mismatch"));
    }
}
