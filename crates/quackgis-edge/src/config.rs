// SPDX-License-Identifier: Apache-2.0
//! Bounded operator configuration for the I0 bootstrap, worker, and client.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context, Result, bail};
use iroh::{EndpointAddr, EndpointId, PublicKey, RelayUrl, SecretKey};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::{CompressionPolicy, MAX_LEASE_TTL_SECONDS, RelayPolicy};

const MAX_CONFIG_BYTES: u64 = 1024 * 1024;
const MAX_KEY_BYTES: u64 = 256;
const MAX_CONNECTIONS: usize = 4096;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BootstrapConfig {
    pub secret_key_path: PathBuf,
    pub registered_credential: String,
    pub login_role: String,
    pub worker: EndpointAddressConfig,
    pub assignment_generation: u64,
    pub lease_ttl_seconds: u64,
    pub relays: Option<Vec<String>>,
    #[serde(default = "default_max_connections")]
    pub max_connections: usize,
}

impl BootstrapConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let config: Self = load_json(path)?;
        validate_connections(config.max_connections)?;
        if config.lease_ttl_seconds == 0 || config.lease_ttl_seconds > MAX_LEASE_TTL_SECONDS {
            bail!("lease_ttl_seconds must be between 1 and {MAX_LEASE_TTL_SECONDS}");
        }
        config.registered_credential()?;
        config.worker.parse()?;
        config.relay_policy()?;
        Ok(config)
    }

    pub fn secret_key(&self) -> Result<SecretKey> {
        load_secret_key(&self.secret_key_path)
    }

    pub fn registered_credential(&self) -> Result<PublicKey> {
        parse_public_key(&self.registered_credential, "registered_credential")
    }

    pub fn relay_policy(&self) -> Result<RelayPolicy> {
        RelayPolicy::from_config(self.relays.clone()).map_err(Into::into)
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerConfig {
    pub secret_key_path: PathBuf,
    pub bootstrap_public_key: String,
    pub backend: SocketAddr,
    pub relays: Option<Vec<String>>,
    #[serde(default)]
    pub compression: CompressionPolicy,
    #[serde(default = "default_max_connections")]
    pub max_connections: usize,
    #[serde(default = "default_max_streams")]
    pub max_streams_per_connection: usize,
}

impl WorkerConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let config: Self = load_json(path)?;
        validate_connections(config.max_connections)?;
        validate_connections(config.max_streams_per_connection)?;
        if !config.backend.ip().is_loopback() {
            bail!("worker backend must be a loopback address in the I0 profile");
        }
        config.bootstrap_public_key()?;
        config.relay_policy()?;
        Ok(config)
    }

    pub fn secret_key(&self) -> Result<SecretKey> {
        load_secret_key(&self.secret_key_path)
    }

    pub fn bootstrap_public_key(&self) -> Result<PublicKey> {
        parse_public_key(&self.bootstrap_public_key, "bootstrap_public_key")
    }

    pub fn relay_policy(&self) -> Result<RelayPolicy> {
        RelayPolicy::from_config(self.relays.clone()).map_err(Into::into)
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientConfig {
    pub credential_secret_key_path: PathBuf,
    pub transport_secret_key_path: PathBuf,
    pub bootstrap: EndpointAddressConfig,
    pub listen: SocketAddr,
    pub relays: Option<Vec<String>>,
    #[serde(default)]
    pub compression: CompressionPolicy,
    #[serde(default = "default_max_connections")]
    pub max_connections: usize,
}

impl ClientConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let config: Self = load_json(path)?;
        validate_connections(config.max_connections)?;
        if !config.listen.ip().is_loopback() {
            bail!("tiny client listener must use a loopback address in the I0 profile");
        }
        config.bootstrap.parse()?;
        config.relay_policy()?;
        Ok(config)
    }

    pub fn credential_secret_key(&self) -> Result<SecretKey> {
        load_secret_key(&self.credential_secret_key_path)
    }

    pub fn transport_secret_key(&self) -> Result<SecretKey> {
        load_secret_key(&self.transport_secret_key_path)
    }

    pub fn relay_policy(&self) -> Result<RelayPolicy> {
        RelayPolicy::from_config(self.relays.clone()).map_err(Into::into)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EndpointAddressConfig {
    pub endpoint_id: String,
    #[serde(default)]
    pub direct_addresses: Vec<SocketAddr>,
    pub relay_url: Option<String>,
}

impl EndpointAddressConfig {
    pub fn parse(&self) -> Result<EndpointAddr> {
        let endpoint_id = parse_public_key(&self.endpoint_id, "endpoint_id")?;
        if self.direct_addresses.is_empty() && self.relay_url.is_none() {
            bail!("endpoint address needs at least one direct address or relay_url");
        }
        let mut address = EndpointAddr::new(endpoint_id);
        for direct in &self.direct_addresses {
            if direct.ip().is_unspecified() {
                bail!("endpoint direct address cannot be unspecified");
            }
            address = address.with_ip_addr(*direct);
        }
        if let Some(raw) = &self.relay_url {
            let relay = raw
                .parse::<RelayUrl>()
                .with_context(|| format!("invalid endpoint relay_url {raw:?}"))?;
            address = address.with_relay_url(relay);
        }
        Ok(address)
    }

    pub fn from_endpoint_addr(address: &EndpointAddr) -> Self {
        Self {
            endpoint_id: address.id.to_string(),
            direct_addresses: address.ip_addrs().copied().collect(),
            relay_url: address.relay_urls().next().map(ToString::to_string),
        }
    }
}

pub fn load_secret_key(path: &Path) -> Result<SecretKey> {
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("cannot inspect key file {}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > MAX_KEY_BYTES {
        bail!("key path must be a non-symlink regular file no larger than {MAX_KEY_BYTES} bytes");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.mode() & 0o077 != 0 {
            bail!("key file must not grant group or other permissions");
        }
    }
    let encoded = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read key file {}", path.display()))?;
    SecretKey::from_str(encoded.trim()).context("invalid iroh secret key")
}

fn load_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let metadata = std::fs::metadata(path)
        .with_context(|| format!("cannot inspect configuration {}", path.display()))?;
    if !metadata.is_file() || metadata.len() > MAX_CONFIG_BYTES {
        bail!("configuration must be a regular JSON file no larger than {MAX_CONFIG_BYTES} bytes");
    }
    let raw = std::fs::read(path)
        .with_context(|| format!("cannot read configuration {}", path.display()))?;
    serde_json::from_slice(&raw)
        .with_context(|| format!("invalid configuration JSON in {}", path.display()))
}

fn parse_public_key(raw: &str, field: &str) -> Result<PublicKey> {
    PublicKey::from_str(raw).with_context(|| format!("invalid {field}"))
}

fn validate_connections(value: usize) -> Result<()> {
    if value == 0 || value > MAX_CONNECTIONS {
        bail!("connection and stream limits must be between 1 and {MAX_CONNECTIONS}");
    }
    Ok(())
}

fn default_max_connections() -> usize {
    64
}

fn default_max_streams() -> usize {
    64
}

pub fn endpoint_document(endpoint_id: EndpointId, address: &EndpointAddr) -> Result<String> {
    if endpoint_id != address.id {
        bail!("endpoint identity does not match advertised address");
    }
    serde_json::to_string(&EndpointAddressConfig::from_endpoint_addr(address))
        .context("cannot encode endpoint address")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::IpAddr;

    #[test]
    fn endpoint_config_requires_a_route() {
        let key = SecretKey::from_bytes(&[21; 32]);
        let empty = EndpointAddressConfig {
            endpoint_id: key.public().to_string(),
            direct_addresses: vec![],
            relay_url: None,
        };
        assert!(empty.parse().is_err());

        let direct = EndpointAddressConfig {
            endpoint_id: key.public().to_string(),
            direct_addresses: vec!["127.0.0.1:1234".parse().unwrap()],
            relay_url: None,
        };
        assert_eq!(direct.parse().unwrap().id, key.public());
    }

    #[test]
    fn worker_and_client_addresses_must_be_loopback() {
        assert!(!"192.0.2.1".parse::<IpAddr>().unwrap().is_loopback());
        assert!("127.0.0.1".parse::<IpAddr>().unwrap().is_loopback());
    }
}
