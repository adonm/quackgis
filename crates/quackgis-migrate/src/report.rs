// SPDX-License-Identifier: Apache-2.0

use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};

const MAX_INPUT_REPORT_BYTES: u64 = 32 * 1024 * 1024;

pub fn read_bound_json<T: DeserializeOwned>(path: &Path, expected_sha256: &str) -> Result<T> {
    if !valid_sha256(expected_sha256) {
        bail!("expected report SHA-256 must be 64 lowercase hexadecimal characters");
    }
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("cannot inspect migration report {}", path.display()))?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() > MAX_INPUT_REPORT_BYTES
    {
        bail!(
            "migration report must be a non-symlink regular file no larger than {MAX_INPUT_REPORT_BYTES} bytes"
        );
    }
    let raw = std::fs::read(path)
        .with_context(|| format!("cannot read migration report {}", path.display()))?;
    let actual = hex::encode(Sha256::digest(&raw));
    if actual != expected_sha256 {
        bail!("migration report SHA-256 does not match the operator-provided digest");
    }
    serde_json::from_slice(&raw).context("invalid migration report JSON")
}

pub fn file_sha256(path: &Path) -> Result<String> {
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("cannot inspect migration report {}", path.display()))?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() > MAX_INPUT_REPORT_BYTES
    {
        bail!(
            "migration report must be a non-symlink regular file no larger than {MAX_INPUT_REPORT_BYTES} bytes"
        );
    }
    Ok(hex::encode(Sha256::digest(std::fs::read(path)?)))
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub fn write_json_atomic(path: &Path, value: &impl Serialize) -> Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)
        .with_context(|| format!("cannot create report directory {}", parent.display()))?;
    if path.file_name().is_none() {
        bail!("report path must name a file");
    }
    let temporary = path.with_extension(format!(
        "{}.tmp-{}",
        path.extension()
            .and_then(|value| value.to_str())
            .unwrap_or("json"),
        std::process::id()
    ));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| {
        let mut file = options
            .open(&temporary)
            .with_context(|| format!("cannot create temporary report {}", temporary.display()))?;
        serde_json::to_writer_pretty(&mut file, value)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        std::fs::rename(&temporary, path).with_context(|| {
            format!(
                "cannot publish migration report {} -> {}",
                temporary.display(),
                path.display()
            )
        })?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publishes_complete_json_and_replaces_existing_report() {
        let directory = std::env::temp_dir().join(format!(
            "quackgis-migrate-report-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let _ = std::fs::remove_dir_all(&directory);
        let path = directory.join("preflight.json");
        write_json_atomic(&path, &serde_json::json!({"state": "first"})).unwrap();
        write_json_atomic(&path, &serde_json::json!({"state": "second"})).unwrap();
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(value["state"], "second");
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 1);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn binds_input_to_exact_digest() {
        let directory = std::env::temp_dir().join(format!(
            "quackgis-migrate-read-report-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("report.json");
        std::fs::write(&path, b"{\"state\":\"verified\"}\n").unwrap();
        let digest = file_sha256(&path).unwrap();
        let value: serde_json::Value = read_bound_json(&path, &digest).unwrap();
        assert_eq!(value["state"], "verified");
        assert!(read_bound_json::<serde_json::Value>(&path, &"0".repeat(64)).is_err());
        std::fs::remove_dir_all(directory).unwrap();
    }
}
