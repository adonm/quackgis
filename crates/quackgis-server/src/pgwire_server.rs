// SPDX-License-Identifier: Apache-2.0
//! Engine-neutral pgwire/TLS/SCRAM edge and DuckDB handler assembly.

use std::fs::File;
use std::io::{BufReader, Error as IoError, ErrorKind};
use std::time::Duration;
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
use pgwire::error::ErrorInfo;
use pgwire::tokio::process_socket_with_frontend_limit;
use rustls_pemfile::{certs, pkcs8_private_keys};
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use tokio::net::TcpListener;
use tokio::sync::{Semaphore, watch};
use tokio::task::JoinSet;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::{self, ServerConfig};

use crate::auth::{AccessRole, AuthConfig};
use crate::postgres_compat::POSTGRESQL_COMPATIBILITY_VERSION;

mod copy_text;
mod duckdb;
pub use duckdb::{
    serve_duckdb, serve_duckdb_on_listener, serve_duckdb_on_listener_until, serve_duckdb_until,
};
pub const MAX_COPY_BATCHES_PER_CHUNK: usize = copy_text::MAX_BATCHES_PER_COPY_CHUNK;
pub const DEFAULT_PGWIRE_MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct ServerOptions {
    host: String,
    port: u16,
    tls_cert_path: Option<String>,
    tls_key_path: Option<String>,
    tls_required: bool,
    max_connections: usize,
    max_active_queries: usize,
    max_reader_queries: usize,
    max_writer_queries: usize,
    max_maintenance_queries: usize,
    max_queued_queries: usize,
    max_blocking_workers: usize,
    queue_timeout: Duration,
    statement_timeout: Duration,
    result_batch_bytes: usize,
    pgwire_max_frame_bytes: usize,
    copy_batch_rows: usize,
    copy_batch_bytes: usize,
    copy_max_row_bytes: usize,
}

impl Default for ServerOptions {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 5432,
            tls_cert_path: None,
            tls_key_path: None,
            tls_required: false,
            max_connections: 64,
            max_active_queries: 8,
            max_reader_queries: 8,
            max_writer_queries: 2,
            max_maintenance_queries: 1,
            max_queued_queries: 64,
            max_blocking_workers: 9,
            queue_timeout: Duration::from_secs(30),
            statement_timeout: Duration::from_secs(300),
            result_batch_bytes: 8 * 1024 * 1024,
            pgwire_max_frame_bytes: DEFAULT_PGWIRE_MAX_FRAME_BYTES,
            copy_batch_rows: 65_536,
            copy_batch_bytes: 8 * 1024 * 1024,
            copy_max_row_bytes: 1024 * 1024,
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

    pub fn with_tls_required(mut self, required: bool) -> Self {
        self.tls_required = required;
        self
    }

    pub fn with_max_connections(mut self, max_connections: usize) -> Self {
        self.max_connections = max_connections;
        self
    }

    pub fn with_max_active_queries(mut self, max_active_queries: usize) -> Self {
        self.max_active_queries = max_active_queries;
        self
    }

    pub fn with_max_reader_queries(mut self, max_reader_queries: usize) -> Self {
        self.max_reader_queries = max_reader_queries;
        self
    }

    pub fn with_max_writer_queries(mut self, max_writer_queries: usize) -> Self {
        self.max_writer_queries = max_writer_queries;
        self
    }

    pub fn with_max_maintenance_queries(mut self, max_maintenance_queries: usize) -> Self {
        self.max_maintenance_queries = max_maintenance_queries;
        self
    }

    pub fn with_max_queued_queries(mut self, max_queued_queries: usize) -> Self {
        self.max_queued_queries = max_queued_queries;
        self
    }

    pub fn with_max_blocking_workers(mut self, max_blocking_workers: usize) -> Self {
        self.max_blocking_workers = max_blocking_workers;
        self
    }

    pub fn with_queue_timeout(mut self, queue_timeout: Duration) -> Self {
        self.queue_timeout = queue_timeout;
        self
    }

    pub fn with_statement_timeout(mut self, statement_timeout: Duration) -> Self {
        self.statement_timeout = statement_timeout;
        self
    }

    pub fn with_result_batch_bytes(mut self, result_batch_bytes: usize) -> Self {
        self.result_batch_bytes = result_batch_bytes;
        self
    }

    pub fn with_pgwire_max_frame_bytes(mut self, pgwire_max_frame_bytes: usize) -> Self {
        self.pgwire_max_frame_bytes = pgwire_max_frame_bytes;
        self
    }

    pub fn with_copy_batch_rows(mut self, copy_batch_rows: usize) -> Self {
        self.copy_batch_rows = copy_batch_rows;
        self
    }

    pub fn with_copy_batch_bytes(mut self, copy_batch_bytes: usize) -> Self {
        self.copy_batch_bytes = copy_batch_bytes;
        self
    }

    pub fn with_copy_max_row_bytes(mut self, copy_max_row_bytes: usize) -> Self {
        self.copy_max_row_bytes = copy_max_row_bytes;
        self
    }

    fn max_active_queries(&self) -> usize {
        self.max_active_queries
    }

    fn max_queued_queries(&self) -> usize {
        self.max_queued_queries
    }

    fn max_reader_queries(&self) -> usize {
        self.max_reader_queries
    }

    fn max_writer_queries(&self) -> usize {
        self.max_writer_queries
    }

    fn max_maintenance_queries(&self) -> usize {
        self.max_maintenance_queries
    }

    fn max_blocking_workers(&self) -> usize {
        self.max_blocking_workers
    }

    fn queue_timeout(&self) -> Duration {
        self.queue_timeout
    }

    fn statement_timeout(&self) -> Duration {
        self.statement_timeout
    }

    fn result_batch_bytes(&self) -> usize {
        self.result_batch_bytes
    }

    fn pgwire_max_frame_bytes(&self) -> usize {
        self.pgwire_max_frame_bytes
    }

    fn copy_batch_rows(&self) -> usize {
        self.copy_batch_rows
    }

    fn copy_batch_bytes(&self) -> usize {
        self.copy_batch_bytes
    }

    fn copy_max_row_bytes(&self) -> usize {
        self.copy_max_row_bytes
    }

    fn tls_required(&self) -> bool {
        self.tls_required
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

fn configure_tls(options: &ServerOptions) -> Result<Option<TlsAcceptor>, IoError> {
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
    if options.tls_required && tls_acceptor.is_none() {
        return Err(IoError::new(
            ErrorKind::InvalidInput,
            "TLS-required mode needs a certificate and private key",
        ));
    }
    Ok(tls_acceptor)
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
    let (_shutdown_guard, shutdown) = watch::channel(false);
    serve_with_handlers_on_listener_until(handlers, listener, options, shutdown).await
}

pub async fn serve_with_handlers_on_listener_until<H>(
    handlers: Arc<H>,
    listener: TcpListener,
    options: &ServerOptions,
    mut shutdown: watch::Receiver<bool>,
) -> Result<(), IoError>
where
    H: PgWireServerHandlers + Send + Sync + 'static,
{
    let tls_acceptor = configure_tls(options)?;
    let pgwire_max_frame_bytes = options.pgwire_max_frame_bytes();
    log::info!("quackgis pgwire listening on {}", listener.local_addr()?);
    let limiter =
        (options.max_connections > 0).then(|| Arc::new(Semaphore::new(options.max_connections)));
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            biased;
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    break;
                }
            }
            accepted = listener.accept() => match accepted {
                Ok((socket, peer)) => {
                    let handlers = Arc::clone(&handlers);
                    let tls_acceptor = tls_acceptor.clone();
                    let limiter = limiter.clone();
                    connections.spawn(async move {
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
                        if let Err(error) = process_socket_with_frontend_limit(
                            socket,
                            tls_acceptor,
                            handlers,
                            pgwire_max_frame_bytes,
                        ).await {
                            log::warn!("pgwire socket error from {peer}: {error}");
                        }
                    });
                }
                Err(error) => log::warn!("pgwire accept error: {error}"),
            },
        }
    }
    while let Some(result) = connections.join_next().await {
        if let Err(error) = result {
            log::warn!("pgwire connection task failed during drain: {error}");
        }
    }
    Ok(())
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
        let mut params = postgresql_server_parameter_provider().server_parameters(client)?;
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

fn postgresql_server_parameter_provider() -> DefaultServerParameterProvider {
    let mut provider = DefaultServerParameterProvider::default();
    provider.server_version = POSTGRESQL_COMPATIBILITY_VERSION.to_owned();
    provider.in_hot_standby = false;
    provider.is_superuser = false;
    provider
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

enum StartupAuthHandler {
    Trust(SimpleStartupHandler),
    Password(Box<PerConnectionScramStartupHandler>),
    EdgePreauthenticated {
        handler: SimpleStartupHandler,
        auth: AuthConfig,
    },
}

struct QuackGisStartupHandler {
    auth: StartupAuthHandler,
    tls_required: bool,
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
        if self.tls_required && !client.is_secure() {
            return Err(pgwire::error::PgWireError::UserError(Box::new(
                ErrorInfo::new(
                    "FATAL".to_owned(),
                    "28000".to_owned(),
                    "TLS is required for this QuackGIS endpoint".to_owned(),
                ),
            )));
        }
        match &self.auth {
            StartupAuthHandler::Trust(handler) => handler.on_startup(client, message).await,
            StartupAuthHandler::Password(handler) => handler.on_startup(client, message).await,
            StartupAuthHandler::EdgePreauthenticated { handler, auth } => {
                if let pgwire::messages::PgWireFrontendMessage::Startup(startup) = &message {
                    let user = startup.parameters.get("user").map(String::as_str);
                    if !user.is_some_and(|user| auth.allows_preauthenticated_login(user)) {
                        return Err(pgwire::error::PgWireError::UserError(Box::new(
                            ErrorInfo::new(
                                "FATAL".to_owned(),
                                "28000".to_owned(),
                                "edge-preauthenticated user is not a configured LOGIN role"
                                    .to_owned(),
                            ),
                        )));
                    }
                }
                handler.on_startup(client, message).await
            }
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

    fn server_parameter_provider(&self) -> DefaultServerParameterProvider {
        postgresql_server_parameter_provider()
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

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::{BufMut, BytesMut};

    #[test]
    fn startup_parameters_advertise_the_postgresql_18_profile() {
        let provider = postgresql_server_parameter_provider();
        assert_eq!(
            provider.server_version,
            crate::postgres_compat::POSTGRESQL_COMPATIBILITY_VERSION
        );
        assert!(!provider.in_hot_standby);
        assert!(!provider.is_superuser);
    }

    #[test]
    fn tls_required_mode_fails_without_material() {
        let error = configure_tls(&ServerOptions::new().with_tls_required(true))
            .err()
            .expect("required TLS without material");
        assert_eq!(error.kind(), ErrorKind::InvalidInput);
        assert!(error.to_string().contains("TLS-required"));
    }

    #[test]
    fn tls_material_must_be_paired() {
        let error =
            configure_tls(&ServerOptions::new().with_tls_cert_path(Some("server.pem".to_owned())))
                .err()
                .expect("certificate without key");
        assert_eq!(error.kind(), ErrorKind::InvalidInput);
        assert!(error.to_string().contains("configured together"));
    }

    #[test]
    fn frontend_frame_limit_rejects_header_before_body_allocation() {
        let mut frame = BytesMut::with_capacity(5);
        frame.put_u8(b'd');
        frame.put_i32(65_537);
        let capacity = frame.capacity();

        let error = pgwire::tokio::validate_frontend_message_length(&frame, 1, 65_536)
            .expect_err("oversized declared frame");
        assert!(error.to_string().contains("65537"));
        assert_eq!(frame.len(), 5);
        assert_eq!(frame.capacity(), capacity);
        assert!(pgwire::tokio::validate_frontend_message_length(&frame, 1, 65_537).is_ok());

        let mut invalid = BytesMut::new();
        invalid.put_u8(b'd');
        invalid.put_i32(-1);
        assert!(pgwire::tokio::validate_frontend_message_length(&invalid, 1, 65_536).is_err());
    }
}
