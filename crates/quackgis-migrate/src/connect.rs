// SPDX-License-Identifier: Apache-2.0

use std::io::BufReader;
use std::net::IpAddr;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use rustls::RootCertStore;
use rustls_pemfile::{certs, private_key};
use tokio_postgres::config::{Host, SslMode};
use tokio_postgres::{Client, Config, NoTls};
use tokio_postgres_rustls::MakeRustlsConnect;

const MAX_PASSWORD_BYTES: u64 = 4096;

#[derive(Clone, Debug)]
pub struct ConnectionOptions {
    pub url: String,
    pub password_file: Option<PathBuf>,
    pub ca_certificate: Option<PathBuf>,
    pub client_certificate: Option<PathBuf>,
    pub client_private_key: Option<PathBuf>,
    pub allow_plaintext_loopback: bool,
}

pub async fn connect(options: &ConnectionOptions) -> Result<Client> {
    let mut config: Config = options
        .url
        .parse()
        .context("parse PostgreSQL connection URL")?;
    if config.get_password().is_some() {
        bail!("PostgreSQL connection URL must not contain a password; use a password file");
    }
    if let Some(path) = &options.password_file {
        config.password(read_owner_only_secret(path)?);
    }
    match &options.ca_certificate {
        Some(ca) => connect_tls(config, ca, options).await,
        None => connect_plaintext(config, options).await,
    }
}

async fn connect_plaintext(mut config: Config, options: &ConnectionOptions) -> Result<Client> {
    if !options.allow_plaintext_loopback {
        bail!("a CA certificate is required unless plaintext loopback is explicitly enabled");
    }
    if options.client_certificate.is_some() || options.client_private_key.is_some() {
        bail!("client certificate settings require a CA certificate");
    }
    if config.get_hosts().is_empty()
        || config.get_hosts().iter().any(|host| match host {
            Host::Tcp(host) => !host
                .parse::<IpAddr>()
                .is_ok_and(|address| address.is_loopback()),
            #[cfg(unix)]
            Host::Unix(_) => false,
        })
    {
        bail!("plaintext migration connections require literal loopback TCP or Unix-socket hosts");
    }
    config.ssl_mode(SslMode::Disable);
    let (client, connection) = config.connect(NoTls).await?;
    tokio::spawn(async move {
        if let Err(error) = connection.await {
            eprintln!("quackgis_migrate_connection_error error={error}");
        }
    });
    Ok(client)
}

async fn connect_tls(mut config: Config, ca: &Path, options: &ConnectionOptions) -> Result<Client> {
    if options.client_certificate.is_some() != options.client_private_key.is_some() {
        bail!("client certificate and private key must be configured together");
    }
    let _ = rustls::crypto::ring::default_provider().install_default();
    let mut roots = RootCertStore::empty();
    for certificate in load_certificates(ca, "CA")? {
        roots.add(certificate)?;
    }
    let builder = rustls::ClientConfig::builder().with_root_certificates(roots);
    let tls = match (
        options.client_certificate.as_deref(),
        options.client_private_key.as_deref(),
    ) {
        (Some(certificate), Some(key)) => builder.with_client_auth_cert(
            load_certificates(certificate, "client certificate")?,
            load_private_key(key)?,
        )?,
        (None, None) => builder.with_no_client_auth(),
        _ => unreachable!("client certificate pair was validated"),
    };
    config.ssl_mode(SslMode::Require);
    let (client, connection) = config.connect(MakeRustlsConnect::new(tls)).await?;
    tokio::spawn(async move {
        if let Err(error) = connection.await {
            eprintln!("quackgis_migrate_connection_error error={error}");
        }
    });
    Ok(client)
}

fn load_certificates(
    path: &Path,
    label: &str,
) -> Result<Vec<rustls::pki_types::CertificateDer<'static>>> {
    let file = checked_regular_file(path, 1024 * 1024, label)?;
    let certificates = certs(&mut BufReader::new(file)).collect::<Result<Vec<_>, _>>()?;
    if certificates.is_empty() {
        bail!("{label} contains no certificates");
    }
    Ok(certificates)
}

fn load_private_key(path: &Path) -> Result<rustls::pki_types::PrivateKeyDer<'static>> {
    let file = checked_regular_file(path, 1024 * 1024, "client private key")?;
    private_key(&mut BufReader::new(file))?
        .ok_or_else(|| anyhow::anyhow!("client private key contains no supported key"))
}

fn read_owner_only_secret(path: &Path) -> Result<Vec<u8>> {
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("cannot inspect password file {}", path.display()))?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() == 0
        || metadata.len() > MAX_PASSWORD_BYTES
    {
        bail!(
            "password file must be a non-empty, non-symlink regular file no larger than {MAX_PASSWORD_BYTES} bytes"
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.mode() & 0o077 != 0 {
            bail!("password file must not grant group or other permissions");
        }
    }
    let mut password = std::fs::read(path)
        .with_context(|| format!("cannot read password file {}", path.display()))?;
    if password.last() == Some(&b'\n') {
        password.pop();
        if password.last() == Some(&b'\r') {
            password.pop();
        }
    }
    if password.is_empty() || password.contains(&0) || password.contains(&b'\n') {
        bail!("password file must contain one non-empty NUL-free line");
    }
    Ok(password)
}

fn checked_regular_file(path: &Path, max_bytes: u64, label: &str) -> Result<std::fs::File> {
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("cannot inspect {label} {}", path.display()))?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() == 0
        || metadata.len() > max_bytes
    {
        bail!("{label} must be a bounded non-symlink regular file");
    }
    std::fs::File::open(path).with_context(|| format!("cannot open {label} {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rejects_passwords_in_urls_and_non_loopback_plaintext_before_connecting() {
        let password_url = ConnectionOptions {
            url: "postgresql://user:secret@127.0.0.1:1/database".to_owned(),
            password_file: None,
            ca_certificate: None,
            client_certificate: None,
            client_private_key: None,
            allow_plaintext_loopback: true,
        };
        assert!(connect(&password_url).await.is_err());

        let remote = ConnectionOptions {
            url: "postgresql://user@192.0.2.1:5432/database".to_owned(),
            ..password_url
        };
        assert!(connect(&remote).await.is_err());
    }
}
