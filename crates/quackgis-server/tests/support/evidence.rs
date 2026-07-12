// SPDX-License-Identifier: Apache-2.0
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceLevel {
    Smoke,
    Local,
    Reference,
    External,
}

impl EvidenceLevel {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "smoke" => Ok(Self::Smoke),
            "local" => Ok(Self::Local),
            "reference" => Ok(Self::Reference),
            "external" => Ok(Self::External),
            _ => Err(format!("unknown evidence level {value:?}")),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Smoke => "smoke",
            Self::Local => "local",
            Self::Reference => "reference",
            Self::External => "external",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionEnvironment {
    HostProcess,
    ConstrainedContainer,
    Kind,
    ManagedService,
}

impl ExecutionEnvironment {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "host_process" => Ok(Self::HostProcess),
            "constrained_container" => Ok(Self::ConstrainedContainer),
            "kind" => Ok(Self::Kind),
            "managed_service" => Ok(Self::ManagedService),
            _ => Err(format!("unknown execution environment {value:?}")),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::HostProcess => "host_process",
            Self::ConstrainedContainer => "constrained_container",
            Self::Kind => "kind",
            Self::ManagedService => "managed_service",
        }
    }
}

#[derive(Debug, Serialize)]
pub struct SourceEvidence {
    pub sha: String,
    pub dirty: bool,
    pub status_sha256: Option<String>,
    pub diff_sha256: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct NativeEvidence {
    pub duckdb_version: String,
    pub platform: String,
    pub libduckdb_sha256: String,
    pub extensions: BTreeMap<String, String>,
}

#[derive(Debug, Serialize)]
pub struct HostEvidence {
    pub os: String,
    pub architecture: String,
    pub cpu_model: String,
    pub logical_cpus: usize,
    pub memory_bytes: Option<u64>,
    pub cgroup_memory_max_bytes: Option<u64>,
    pub cgroup_cpu_max: Option<String>,
    pub storage: String,
}

pub struct EvidenceProfile {
    pub id: String,
    pub level: EvidenceLevel,
    pub environment: ExecutionEnvironment,
    pub scope: String,
}

impl EvidenceProfile {
    pub fn new(
        id: impl Into<String>,
        level: EvidenceLevel,
        environment: ExecutionEnvironment,
        scope: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            level,
            environment,
            scope: scope.into(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct EvidenceEnvelope {
    pub schema_version: u8,
    pub profile_id: String,
    pub evidence_level: EvidenceLevel,
    pub execution_environment: ExecutionEnvironment,
    pub status: &'static str,
    pub source: SourceEvidence,
    pub runtime: NativeEvidence,
    pub host: HostEvidence,
    pub data: Value,
    pub correctness: Value,
    pub measurements: Value,
    pub budgets: Value,
    pub scope: String,
}

impl EvidenceEnvelope {
    pub fn collect(
        profile: EvidenceProfile,
        data: Value,
        correctness: Value,
        measurements: Value,
        budgets: Value,
    ) -> Result<Self, String> {
        let root = repository_root()?;
        let runtime_manifest = std::env::var_os("QUACKGIS_DUCKDB_MANIFEST")
            .map(PathBuf::from)
            .unwrap_or_else(|| root.join(".tmp/duckdb/manifest.json"));
        let envelope = Self {
            schema_version: 1,
            profile_id: profile.id,
            evidence_level: profile.level,
            execution_environment: profile.environment,
            status: "pass",
            source: source_evidence(&root)?,
            runtime: native_evidence(&runtime_manifest)?,
            host: host_evidence(),
            data,
            correctness,
            measurements,
            budgets,
            scope: profile.scope,
        };
        envelope.validate()?;
        Ok(envelope)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.profile_id.trim().is_empty() {
            return Err("evidence profile_id must not be empty".to_owned());
        }
        if !self.data.is_object()
            || !self.correctness.is_object()
            || !self.measurements.is_object()
            || !self.budgets.is_object()
        {
            return Err(
                "evidence data, correctness, measurements, and budgets must be objects".to_owned(),
            );
        }
        if matches!(
            self.evidence_level,
            EvidenceLevel::Reference | EvidenceLevel::External
        ) && self.source.dirty
        {
            return Err("reference/external evidence requires a clean source tree".to_owned());
        }
        if matches!(self.evidence_level, EvidenceLevel::Reference)
            && self.host.storage == "unspecified"
        {
            return Err("reference evidence requires QUACKGIS_PROFILE_STORAGE metadata".to_owned());
        }
        Ok(())
    }

    pub fn write(&self, path: &Path) -> Result<(), String> {
        self.validate()?;
        let parent = path
            .parent()
            .ok_or_else(|| "evidence output must have a parent directory".to_owned())?;
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        let temporary = parent.join(format!(
            ".{}.tmp-{}",
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("evidence"),
            std::process::id()
        ));
        let mut bytes = serde_json::to_vec_pretty(self).map_err(|error| error.to_string())?;
        bytes.push(b'\n');
        std::fs::write(&temporary, bytes).map_err(|error| error.to_string())?;
        std::fs::rename(&temporary, path).map_err(|error| error.to_string())
    }
}

fn repository_root() -> Result<PathBuf, String> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .map_err(|error| format!("resolving repository root: {error}"))
}

fn source_evidence(root: &Path) -> Result<SourceEvidence, String> {
    let sha = git_output(root, &["rev-parse", "HEAD"])?;
    let status = git_bytes(
        root,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?;
    let diff = git_bytes(root, &["diff", "--binary", "HEAD"])?;
    let dirty = !status.is_empty();
    Ok(SourceEvidence {
        sha: sha.trim().to_owned(),
        dirty,
        status_sha256: dirty.then(|| sha256(&status)),
        diff_sha256: dirty.then(|| sha256(&diff)),
    })
}

fn git_output(root: &Path, arguments: &[&str]) -> Result<String, String> {
    String::from_utf8(git_bytes(root, arguments)?)
        .map_err(|error| format!("git output is not UTF-8: {error}"))
}

fn git_bytes(root: &Path, arguments: &[&str]) -> Result<Vec<u8>, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(arguments)
        .output()
        .map_err(|error| format!("running git: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "git {} failed: {}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(output.stdout)
}

fn native_evidence(path: &Path) -> Result<NativeEvidence, String> {
    let value: Value = serde_json::from_slice(
        &std::fs::read(path)
            .map_err(|error| format!("reading native manifest {}: {error}", path.display()))?,
    )
    .map_err(|error| format!("parsing native manifest {}: {error}", path.display()))?;
    native_evidence_from_value(&value)
}

fn native_evidence_from_value(value: &Value) -> Result<NativeEvidence, String> {
    let text = |pointer: &str| {
        value
            .pointer(pointer)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| format!("native manifest is missing {pointer}"))
    };
    let mut extensions = BTreeMap::new();
    for extension in value
        .get("extensions")
        .and_then(Value::as_array)
        .ok_or_else(|| "native manifest is missing /extensions".to_owned())?
    {
        let name = extension
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| "native extension is missing name".to_owned())?;
        let digest = extension
            .get("sha256")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("native extension {name} is missing sha256"))?;
        extensions.insert(name.to_owned(), digest.to_owned());
    }
    for required in ["ducklake", "spatial"] {
        if !extensions.contains_key(required) {
            return Err(format!("native manifest is missing {required} extension"));
        }
    }
    Ok(NativeEvidence {
        duckdb_version: text("/duckdb_version")?,
        platform: text("/platform")?,
        libduckdb_sha256: text("/libduckdb/sha256")?,
        extensions,
    })
}

fn host_evidence() -> HostEvidence {
    HostEvidence {
        os: os_description(),
        architecture: std::env::consts::ARCH.to_owned(),
        cpu_model: proc_value("/proc/cpuinfo", "model name").unwrap_or_else(|| "unknown".into()),
        logical_cpus: std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1),
        memory_bytes: proc_memory_bytes(),
        cgroup_memory_max_bytes: read_optional_u64("/sys/fs/cgroup/memory.max"),
        cgroup_cpu_max: read_trimmed("/sys/fs/cgroup/cpu.max"),
        storage: std::env::var("QUACKGIS_PROFILE_STORAGE")
            .unwrap_or_else(|_| "unspecified".to_owned()),
    }
}

fn os_description() -> String {
    let Some(contents) = std::fs::read_to_string("/etc/os-release").ok() else {
        return std::env::consts::OS.to_owned();
    };
    contents
        .lines()
        .find_map(|line| line.strip_prefix("PRETTY_NAME="))
        .map(|value| value.trim_matches('"').to_owned())
        .unwrap_or_else(|| std::env::consts::OS.to_owned())
}

fn proc_value(path: &str, key: &str) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()?
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.trim().eq(key).then(|| value.trim().to_owned())
        })
}

fn proc_memory_bytes() -> Option<u64> {
    let kib = proc_value("/proc/meminfo", "MemTotal")?
        .split_whitespace()
        .next()?
        .parse::<u64>()
        .ok()?;
    kib.checked_mul(1024)
}

fn read_trimmed(path: &str) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().to_owned())
}

fn read_optional_u64(path: &str) -> Option<u64> {
    read_trimmed(path).and_then(|value| (value != "max").then(|| value.parse().ok()).flatten())
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn native_manifest_projection_excludes_paths_and_requires_extensions() {
        let evidence = native_evidence_from_value(&json!({
            "duckdb_version": "1.5.4",
            "platform": "linux-amd64",
            "libduckdb": {"path": "/secret/libduckdb.so", "sha256": "library"},
            "extensions": [
                {"name": "ducklake", "path": "/secret/ducklake", "sha256": "lake"},
                {"name": "spatial", "path": "/secret/spatial", "sha256": "spatial"}
            ]
        }))
        .expect("native projection");
        let rendered = serde_json::to_string(&evidence).expect("native JSON");
        assert!(!rendered.contains("/secret"));
        assert_eq!(evidence.extensions["ducklake"], "lake");
        assert_eq!(evidence.extensions["spatial"], "spatial");
    }

    #[test]
    fn host_fingerprint_has_bounded_required_fields() {
        let host = host_evidence();
        assert!(host.logical_cpus > 0);
        assert!(!host.os.is_empty());
        assert!(!host.architecture.is_empty());
    }

    #[test]
    fn evidence_levels_and_environments_are_strict() {
        assert!(matches!(
            EvidenceLevel::parse("reference"),
            Ok(EvidenceLevel::Reference)
        ));
        assert_eq!(EvidenceLevel::Local.as_str(), "local");
        assert!(EvidenceLevel::parse("benchmark").is_err());
        assert!(matches!(
            ExecutionEnvironment::parse("kind"),
            Ok(ExecutionEnvironment::Kind)
        ));
        assert_eq!(
            ExecutionEnvironment::ConstrainedContainer.as_str(),
            "constrained_container"
        );
        assert!(ExecutionEnvironment::parse("cluster").is_err());
    }
}
