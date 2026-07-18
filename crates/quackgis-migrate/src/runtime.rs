// SPDX-License-Identifier: Apache-2.0

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RuntimeIdentity {
    pub migrator_sha256: String,
    pub artifact_manifest_sha256: Option<String>,
    pub source_sha: Option<String>,
    pub source_dirty: Option<bool>,
    pub artifacts: BTreeMap<String, String>,
    pub target_runtime_image_sha256: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct RuntimeIdentityOptions {
    pub artifact_manifest: Option<PathBuf>,
    pub target_runtime_image: Option<String>,
}

#[derive(Deserialize)]
struct ArtifactManifest {
    source: ManifestSource,
    artifacts: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct ManifestSource {
    sha: String,
    dirty: bool,
}

pub fn collect_runtime_identity(options: &RuntimeIdentityOptions) -> Result<RuntimeIdentity> {
    let executable = std::env::current_exe().context("resolve running migrator executable")?;
    let migrator_sha256 =
        sha256_regular_file(&executable, None).context("hash running migrator executable")?;
    let target_runtime_image_sha256 = options
        .target_runtime_image
        .as_deref()
        .map(image_digest)
        .transpose()?;
    let Some(path) = &options.artifact_manifest else {
        return Ok(RuntimeIdentity {
            migrator_sha256,
            artifact_manifest_sha256: None,
            source_sha: None,
            source_dirty: None,
            artifacts: BTreeMap::new(),
            target_runtime_image_sha256,
        });
    };
    let raw = read_regular_file(path, MAX_MANIFEST_BYTES, "runtime artifact manifest")?;
    let artifact_manifest_sha256 = hex::encode(Sha256::digest(&raw));
    let manifest: ArtifactManifest =
        serde_json::from_slice(&raw).context("parse runtime artifact manifest")?;
    if !is_lower_hex(&manifest.source.sha, 40) {
        bail!("runtime artifact manifest source SHA is invalid");
    }
    for (name, digest) in &manifest.artifacts {
        if name.is_empty() || name.len() > 128 || !is_lower_hex(digest, 64) {
            bail!("runtime artifact manifest contains an invalid artifact digest");
        }
    }
    if manifest.artifacts.get("quackgis-migrate") != Some(&migrator_sha256) {
        bail!("running migrator does not match the runtime artifact manifest");
    }
    Ok(RuntimeIdentity {
        migrator_sha256,
        artifact_manifest_sha256: Some(artifact_manifest_sha256),
        source_sha: Some(manifest.source.sha),
        source_dirty: Some(manifest.source.dirty),
        artifacts: manifest.artifacts,
        target_runtime_image_sha256,
    })
}

fn image_digest(value: &str) -> Result<String> {
    let Some((name, digest)) = value.rsplit_once("@sha256:") else {
        bail!("target runtime image must be an immutable image@sha256 reference");
    };
    if name.is_empty() || name.chars().any(char::is_whitespace) || !is_lower_hex(digest, 64) {
        bail!("target runtime image must be an immutable image@sha256 reference");
    }
    Ok(digest.to_owned())
}

fn sha256_regular_file(path: &Path, maximum: Option<u64>) -> Result<String> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!("runtime evidence input must be a non-symlink regular file");
    }
    if maximum.is_some_and(|maximum| metadata.len() > maximum) {
        bail!("runtime evidence input exceeds its size limit");
    }
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn read_regular_file(path: &Path, maximum: u64, label: &str) -> Result<Vec<u8>> {
    let metadata = std::fs::symlink_metadata(path).with_context(|| format!("inspect {label}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > maximum {
        bail!("{label} must be a non-symlink regular file no larger than {maximum} bytes");
    }
    std::fs::read(path).with_context(|| format!("read {label}"))
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_immutable_runtime_images() {
        assert_eq!(
            image_digest(&format!(
                "registry.example/quackgis@sha256:{}",
                "a".repeat(64)
            ))
            .unwrap(),
            "a".repeat(64)
        );
        assert!(image_digest("registry.example/quackgis:latest").is_err());
        assert!(image_digest(&format!("image@sha256:{}", "A".repeat(64))).is_err());
    }
}
