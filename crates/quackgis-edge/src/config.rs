// SPDX-License-Identifier: Apache-2.0
//! Bounded operator configuration for the I0 bootstrap, worker, and client.

use std::collections::HashSet;
use std::net::{SocketAddr, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context, Result, bail};
use iroh::{EndpointAddr, EndpointId, PublicKey, RelayUrl, SecretKey};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::{CompressionPolicy, MAX_BOOTSTRAP_REGISTRATIONS, MAX_LEASE_TTL_SECONDS, RelayPolicy};

const MAX_CONFIG_BYTES: u64 = 1024 * 1024;
const MAX_KEY_BYTES: u64 = 256;
const MAX_CONNECTIONS: usize = 4096;
const MAX_DIRECT_ROUTES: usize = 16;
const MAX_DIRECT_HOST_BYTES: usize = 512;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BootstrapConfig {
    pub secret_key_path: PathBuf,
    pub registrations: Vec<BootstrapRegistrationConfig>,
    pub worker: EndpointAddressConfig,
    pub assignment_generation: u64,
    pub lease_ttl_seconds: u64,
    pub relays: Option<Vec<String>>,
    #[serde(default)]
    pub disable_relays: bool,
    pub bind: Option<SocketAddr>,
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
        config.registrations()?;
        config.worker.parse()?;
        config.relay_policy()?;
        Ok(config)
    }

    pub fn secret_key(&self) -> Result<SecretKey> {
        load_secret_key(&self.secret_key_path)
    }

    pub fn registrations(&self) -> Result<Vec<(PublicKey, String)>> {
        if self.registrations.is_empty() || self.registrations.len() > MAX_BOOTSTRAP_REGISTRATIONS {
            bail!(
                "bootstrap registrations must contain between 1 and {MAX_BOOTSTRAP_REGISTRATIONS} entries"
            );
        }
        let mut credentials = HashSet::new();
        self.registrations
            .iter()
            .enumerate()
            .map(|(index, registration)| {
                let credential = parse_public_key(
                    &registration.credential,
                    &format!("registrations[{index}].credential"),
                )?;
                if !credentials.insert(credential) {
                    bail!("bootstrap registrations contain a duplicate credential");
                }
                Ok((credential, registration.login_role.clone()))
            })
            .collect()
    }

    pub fn relay_policy(&self) -> Result<RelayPolicy> {
        relay_policy(self.disable_relays, self.relays.clone())
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BootstrapRegistrationConfig {
    pub credential: String,
    pub login_role: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerConfig {
    pub secret_key_path: PathBuf,
    pub bootstrap_public_key: String,
    pub backend: SocketAddr,
    pub relays: Option<Vec<String>>,
    #[serde(default)]
    pub disable_relays: bool,
    pub bind: Option<SocketAddr>,
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
        relay_policy(self.disable_relays, self.relays.clone())
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientConfig {
    pub credential_secret_key_path: PathBuf,
    pub transport_secret_key_path: PathBuf,
    pub bootstrap: EndpointAddressConfig,
    pub listen: SocketAddr,
    pub local_tls: Option<LocalTlsConfig>,
    pub relays: Option<Vec<String>>,
    #[serde(default)]
    pub disable_relays: bool,
    pub bind: Option<SocketAddr>,
    #[serde(default)]
    pub compression: CompressionPolicy,
    #[serde(default = "default_max_connections")]
    pub max_connections: usize,
}

impl ClientConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let config: Self = load_json(path)?;
        validate_connections(config.max_connections)?;
        validate_client_listener(config.listen, config.local_tls.is_some())?;
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
        relay_policy(self.disable_relays, self.relays.clone())
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalTlsConfig {
    pub certificate_path: PathBuf,
    pub private_key_path: PathBuf,
    pub client_ca_path: PathBuf,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EndpointAddressConfig {
    pub endpoint_id: String,
    #[serde(default)]
    pub direct_addresses: Vec<SocketAddr>,
    #[serde(default)]
    pub direct_hosts: Vec<String>,
    pub relay_url: Option<String>,
}

impl EndpointAddressConfig {
    pub fn parse(&self) -> Result<EndpointAddr> {
        let endpoint_id = parse_public_key(&self.endpoint_id, "endpoint_id")?;
        if self.direct_addresses.len() + self.direct_hosts.len() > MAX_DIRECT_ROUTES {
            bail!("endpoint address exceeds the {MAX_DIRECT_ROUTES}-route limit");
        }
        if self.direct_addresses.is_empty()
            && self.direct_hosts.is_empty()
            && self.relay_url.is_none()
        {
            bail!("endpoint address needs at least one direct address or relay_url");
        }
        let mut address = EndpointAddr::new(endpoint_id);
        let mut direct_addresses = self.direct_addresses.clone();
        for direct_host in &self.direct_hosts {
            if direct_host.is_empty()
                || direct_host.len() > MAX_DIRECT_HOST_BYTES
                || direct_host.chars().any(char::is_whitespace)
            {
                bail!("endpoint direct host is empty, oversized, or contains whitespace");
            }
            let resolved = direct_host
                .to_socket_addrs()
                .with_context(|| format!("cannot resolve endpoint direct host {direct_host:?}"))?
                .collect::<Vec<_>>();
            if resolved.is_empty() {
                bail!("endpoint direct host {direct_host:?} resolved to no addresses");
            }
            direct_addresses.extend(resolved);
            if direct_addresses.len() > MAX_DIRECT_ROUTES {
                bail!("resolved endpoint address exceeds the {MAX_DIRECT_ROUTES}-route limit");
            }
        }
        let mut seen = HashSet::new();
        for direct in direct_addresses {
            if !seen.insert(direct) {
                continue;
            }
            if direct.ip().is_unspecified() {
                bail!("endpoint direct address cannot be unspecified");
            }
            address = address.with_ip_addr(direct);
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
            direct_hosts: vec![],
            relay_url: address.relay_urls().next().map(ToString::to_string),
        }
    }
}

fn relay_policy(disable_relays: bool, relays: Option<Vec<String>>) -> Result<RelayPolicy> {
    if disable_relays {
        if relays.is_some() {
            bail!("disable_relays cannot be combined with relay configuration");
        }
        Ok(RelayPolicy::Disabled)
    } else {
        RelayPolicy::from_config(relays).map_err(Into::into)
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

fn validate_client_listener(listen: SocketAddr, has_local_tls: bool) -> Result<()> {
    if !listen.ip().is_loopback() && !has_local_tls {
        bail!("a non-loopback tiny client listener requires local_tls");
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
            direct_hosts: vec![],
            relay_url: None,
        };
        assert!(empty.parse().is_err());

        let direct = EndpointAddressConfig {
            endpoint_id: key.public().to_string(),
            direct_addresses: vec!["127.0.0.1:1234".parse().unwrap()],
            direct_hosts: vec![],
            relay_url: None,
        };
        assert_eq!(direct.parse().unwrap().id, key.public());
    }

    #[test]
    fn worker_backend_and_unprotected_client_addresses_must_be_loopback() {
        assert!(!"192.0.2.1".parse::<IpAddr>().unwrap().is_loopback());
        assert!("127.0.0.1".parse::<IpAddr>().unwrap().is_loopback());
        assert!(validate_client_listener("127.0.0.1:5432".parse().unwrap(), false).is_ok());
        assert!(validate_client_listener("0.0.0.0:5432".parse().unwrap(), false).is_err());
        assert!(validate_client_listener("0.0.0.0:5432".parse().unwrap(), true).is_ok());
    }

    #[test]
    fn explicit_direct_only_policy_conflicts_with_relay_configuration() {
        assert_eq!(relay_policy(true, None).unwrap(), RelayPolicy::Disabled);
        assert!(relay_policy(true, Some(vec!["https://relay.example".into()])).is_err());
    }

    #[test]
    fn bootstrap_registrations_are_bounded_and_unique() {
        let first = SecretKey::from_bytes(&[11; 32]).public().to_string();
        let second = SecretKey::from_bytes(&[12; 32]).public().to_string();
        let mut config = BootstrapConfig {
            secret_key_path: "bootstrap.key".into(),
            registrations: vec![
                BootstrapRegistrationConfig {
                    credential: first.clone(),
                    login_role: "postgres".to_owned(),
                },
                BootstrapRegistrationConfig {
                    credential: second,
                    login_role: "authenticator".to_owned(),
                },
            ],
            worker: EndpointAddressConfig {
                endpoint_id: SecretKey::from_bytes(&[13; 32]).public().to_string(),
                direct_addresses: vec!["127.0.0.1:4242".parse().unwrap()],
                direct_hosts: Vec::new(),
                relay_url: None,
            },
            assignment_generation: 1,
            lease_ttl_seconds: 60,
            relays: None,
            disable_relays: true,
            bind: None,
            max_connections: 4,
        };
        let registrations = config.registrations().expect("two registrations");
        assert_eq!(registrations[0].1, "postgres");
        assert_eq!(registrations[1].1, "authenticator");
        config.registrations[1].credential = first;
        assert!(config.registrations().is_err());
        config.registrations.clear();
        assert!(config.registrations().is_err());
    }
}
