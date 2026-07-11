// SPDX-License-Identifier: Apache-2.0
//! Engine-neutral pgwire/TLS/SCRAM edge and DuckDB handler assembly.

use std::fs::File;
use std::io::{BufReader, Error as IoError, ErrorKind};
use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use pgwire::api::auth::noop::NoopStartupHandler;
use pgwire::api::auth::sasl::SASLAuthStartupHandler;
use pgwire::api::auth::sasl::scram::ScramAuth;
use pgwire::api::auth::{
    AuthSource, DefaultServerParameterProvider, LoginInfo, Password, ServerParameterProvider,
    StartupHandler,
};
use pgwire::api::{ClientInfo, ConnectionManager, ErrorHandler, PgWireServerHandlers};
use pgwire::tokio::process_socket;
use rustls_pemfile::{certs, pkcs8_private_keys};
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use tokio::net::TcpListener;
use tokio::sync::Semaphore;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::{self, ServerConfig};

use crate::auth::{AccessRole, AuthConfig};

mod duckdb;
pub use duckdb::{serve_duckdb, serve_duckdb_on_listener};

#[derive(Clone, Debug)]
pub struct ServerOptions {
    host: String,
    port: u16,
    tls_cert_path: Option<String>,
    tls_key_path: Option<String>,
    max_connections: usize,
}

impl Default for ServerOptions {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 5432,
            tls_cert_path: None,
            tls_key_path: None,
            max_connections: 0,
        }
    }
}

impl ServerOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_host(mut self, host: String) -> Self {
        self.host = host;
        self
    }

    pub fn with_port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    pub fn with_tls_cert_path(mut self, path: Option<String>) -> Self {
        self.tls_cert_path = path;
        self
    }

    pub fn with_tls_key_path(mut self, path: Option<String>) -> Self {
        self.tls_key_path = path;
        self
    }

    pub fn with_max_connections(mut self, max_connections: usize) -> Self {
        self.max_connections = max_connections;
        self
    }
}

fn setup_tls(cert_path: &str, key_path: &str) -> Result<TlsAcceptor, IoError> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let certificates = certs(&mut BufReader::new(File::open(cert_path)?))
        .collect::<Result<Vec<CertificateDer<'static>>, IoError>>()?;
    let key = pkcs8_private_keys(&mut BufReader::new(File::open(key_path)?))
        .map(|key| key.map(PrivateKeyDer::from))
        .collect::<Result<Vec<_>, IoError>>()?
        .into_iter()
        .next()
        .ok_or_else(|| IoError::new(ErrorKind::InvalidInput, "No private key found"))?;
    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certificates, key)
        .map_err(|error| IoError::new(ErrorKind::InvalidInput, error))?;
    Ok(TlsAcceptor::from(Arc::new(config)))
}

pub async fn serve_with_handlers<H>(
    handlers: Arc<H>,
    options: &ServerOptions,
) -> Result<(), IoError>
where
    H: PgWireServerHandlers + Send + Sync + 'static,
{
    let address = format!("{}:{}", options.host, options.port);
    let listener = TcpListener::bind(&address).await?;
    serve_with_handlers_on_listener(handlers, listener, options).await
}

pub async fn serve_with_handlers_on_listener<H>(
    handlers: Arc<H>,
    listener: TcpListener,
    options: &ServerOptions,
) -> Result<(), IoError>
where
    H: PgWireServerHandlers + Send + Sync + 'static,
{
    let tls_acceptor = match (
        options.tls_cert_path.as_deref(),
        options.tls_key_path.as_deref(),
    ) {
        (Some(cert), Some(key)) => Some(setup_tls(cert, key).map_err(|error| {
            IoError::new(
                error.kind(),
                format!("failed to configure requested TLS: {error}"),
            )
        })?),
        (None, None) => None,
        _ => {
            return Err(IoError::new(
                ErrorKind::InvalidInput,
                "TLS certificate and key must be configured together",
            ));
        }
    };
    log::info!("quackgis pgwire listening on {}", listener.local_addr()?);
    let limiter =
        (options.max_connections > 0).then(|| Arc::new(Semaphore::new(options.max_connections)));
    loop {
        match listener.accept().await {
            Ok((socket, peer)) => {
                let handlers = Arc::clone(&handlers);
                let tls_acceptor = tls_acceptor.clone();
                let limiter = limiter.clone();
                tokio::spawn(async move {
                    let _permit = match limiter {
                        Some(limiter) => match limiter.try_acquire_owned() {
                            Ok(permit) => Some(permit),
                            Err(_) => {
                                log::warn!("pgwire connection rejected from {peer}: limit reached");
                                return;
                            }
                        },
                        None => None,
                    };
                    if let Err(error) = process_socket(socket, tls_acceptor, handlers).await {
                        log::warn!("pgwire socket error from {peer}: {error}");
                    }
                });
            }
            Err(error) => log::warn!("pgwire accept error: {error}"),
        }
    }
}

#[derive(Debug, Clone)]
struct StaticPasswordAuthSource {
    auth: AuthConfig,
}

#[async_trait]
impl AuthSource for StaticPasswordAuthSource {
    async fn get_password(&self, login: &LoginInfo) -> pgwire::error::PgWireResult<Password> {
        let Some(username) = login.user() else {
            return Err(pgwire::error::PgWireError::InvalidPassword(String::new()));
        };
        let Some(user) = self.auth.user(username) else {
            return Err(pgwire::error::PgWireError::InvalidPassword(
                username.to_string(),
            ));
        };
        Ok(Password::new(
            Some(user.scram_salt.clone()),
            user.scram_salted_password.clone(),
        ))
    }
}

#[derive(Debug)]
struct QuackGisServerParameterProvider {
    auth: AuthConfig,
}

impl QuackGisServerParameterProvider {
    fn new(auth: AuthConfig) -> Self {
        Self { auth }
    }
}

impl ServerParameterProvider for QuackGisServerParameterProvider {
    fn server_parameters<C>(&self, client: &C) -> Option<HashMap<String, String>>
    where
        C: ClientInfo,
    {
        let mut params = DefaultServerParameterProvider::default().server_parameters(client)?;
        let role = self.auth.role_for_client(client);
        params.insert("is_superuser".to_string(), "off".to_string());
        params.insert(
            "default_transaction_read_only".to_string(),
            match role {
                AccessRole::ReadWrite => "off",
                AccessRole::ReadOnly => "on",
            }
            .to_string(),
        );
        Some(params)
    }
}

struct ScramStartupSession {
    handler: SASLAuthStartupHandler<QuackGisServerParameterProvider>,
}

#[derive(Debug)]
struct PerConnectionScramStartupHandler {
    auth_source: Arc<StaticPasswordAuthSource>,
    parameter_provider: Arc<QuackGisServerParameterProvider>,
    connection_manager: Arc<ConnectionManager>,
}

impl PerConnectionScramStartupHandler {
    fn new(auth: AuthConfig, connection_manager: Arc<ConnectionManager>) -> Self {
        Self {
            auth_source: Arc::new(StaticPasswordAuthSource { auth: auth.clone() }),
            parameter_provider: Arc::new(QuackGisServerParameterProvider::new(auth)),
            connection_manager,
        }
    }

    fn startup_session(&self) -> ScramStartupSession {
        let auth_source: Arc<dyn AuthSource> = self.auth_source.clone();
        ScramStartupSession {
            handler: SASLAuthStartupHandler::new(Arc::clone(&self.parameter_provider))
                .with_scram(ScramAuth::new(auth_source))
                .with_connection_manager(Arc::clone(&self.connection_manager)),
        }
    }
}

#[async_trait]
impl StartupHandler for PerConnectionScramStartupHandler {
    async fn on_startup<C>(
        &self,
        client: &mut C,
        message: pgwire::messages::PgWireFrontendMessage,
    ) -> pgwire::error::PgWireResult<()>
    where
        C: ClientInfo + futures::Sink<pgwire::messages::PgWireBackendMessage> + Unpin + Send + Sync,
        C::Error: std::fmt::Debug,
        pgwire::error::PgWireError:
            From<<C as futures::Sink<pgwire::messages::PgWireBackendMessage>>::Error>,
    {
        let session = client
            .session_extensions()
            .get_or_insert_with(|| self.startup_session());
        session.handler.on_startup(client, message).await
    }
}

enum QuackGisStartupHandler {
    Trust(SimpleStartupHandler),
    Password(Box<PerConnectionScramStartupHandler>),
}

#[async_trait]
impl StartupHandler for QuackGisStartupHandler {
    async fn on_startup<C>(
        &self,
        client: &mut C,
        message: pgwire::messages::PgWireFrontendMessage,
    ) -> pgwire::error::PgWireResult<()>
    where
        C: ClientInfo + futures::Sink<pgwire::messages::PgWireBackendMessage> + Unpin + Send + Sync,
        C::Error: std::fmt::Debug,
        pgwire::error::PgWireError:
            From<<C as futures::Sink<pgwire::messages::PgWireBackendMessage>>::Error>,
    {
        match self {
            Self::Trust(handler) => handler.on_startup(client, message).await,
            Self::Password(handler) => handler.on_startup(client, message).await,
        }
    }
}

struct SimpleStartupHandler {
    connection_manager: Arc<ConnectionManager>,
}

#[async_trait]
impl NoopStartupHandler for SimpleStartupHandler {
    fn connection_manager(&self) -> Option<Arc<ConnectionManager>> {
        Some(Arc::clone(&self.connection_manager))
    }
}

struct LoggingErrorHandler;

impl ErrorHandler for LoggingErrorHandler {
    fn on_error<C>(&self, client: &C, error: &mut pgwire::error::PgWireError)
    where
        C: ClientInfo,
    {
        let kind = match error {
            pgwire::error::PgWireError::InvalidPassword(_)
            | pgwire::error::PgWireError::UserNameRequired => "auth_failure",
            pgwire::error::PgWireError::UserError(_) => "user_error",
            pgwire::error::PgWireError::ApiError(_) => "api_error",
            _ => "protocol_error",
        };
        let user = client
            .metadata()
            .get("user")
            .map(String::as_str)
            .unwrap_or("unknown");
        log::info!("quackgis_pgwire_error kind={kind} user={user}");
        if kind == "auth_failure" {
            crate::audit::log_auth_failure(user, kind);
        }
    }
}
