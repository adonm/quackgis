// SPDX-License-Identifier: Apache-2.0
//! Executable I0 bootstrap, worker, and tiny-client transport path.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::File;
use std::future::Future;
use std::io::{BufReader, Error as IoError, ErrorKind};
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use iroh::endpoint::{Connection, RecvStream, SendStream};
use iroh::{Endpoint, EndpointAddr, EndpointId, PublicKey, SecretKey, endpoint::presets};
use rustls_pemfile::{certs, private_key};
use rustls_pki_types::CertificateDer;
use serde::{Serialize, de::DeserializeOwned};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Semaphore, watch};
use tokio::task::JoinSet;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::{RootCertStore, ServerConfig, server::WebPkiClientVerifier};

use crate::{
    AccessLeaseClaims, ApplicationProtocol, CONTROL_ALPN, CompressionCodec, CompressionPolicy,
    EDGE_ALPN, EdgeAuthenticate, EdgeAuthenticated, EdgeChallenge, LeaseRequest,
    MAX_CONTROL_MESSAGE_BYTES, PROTOCOL_VERSION, RelayPolicy, SignedAccessLease, StreamPrelude,
    compression::{Direction, TransportMetrics, copy_application},
    decode_control, encode_control,
};

const NONCE_CACHE_CAPACITY: usize = 4096;
const MAX_INITIAL_PGWIRE_PACKET_BYTES: usize = 16 * 1024 * 1024;
const POSTGRESQL_PROTOCOL_3: u32 = 196_608;
const CANCEL_REQUEST_CODE: u32 = 80_877_102;
const SSL_REQUEST_CODE: u32 = 80_877_103;
const GSSENC_REQUEST_CODE: u32 = 80_877_104;
const LOCAL_NEGOTIATION_TIMEOUT: Duration = Duration::from_secs(10);
const EDGE_FIRST_RESPONSE_TIMEOUT: Duration = Duration::from_secs(3);
const SERVICE_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

pub async fn bind_endpoint(
    secret_key: SecretKey,
    alpns: Vec<Vec<u8>>,
    relay_policy: &RelayPolicy,
) -> Result<Endpoint> {
    bind_endpoint_at(secret_key, alpns, relay_policy, None).await
}

pub async fn bind_endpoint_at(
    secret_key: SecretKey,
    alpns: Vec<Vec<u8>>,
    relay_policy: &RelayPolicy,
    bind: Option<SocketAddr>,
) -> Result<Endpoint> {
    let builder = Endpoint::builder(presets::N0)
        .secret_key(secret_key)
        .alpns(alpns)
        .relay_mode(relay_policy.iroh_mode());
    let builder = if *relay_policy == RelayPolicy::Disabled {
        builder.clear_address_lookup()
    } else {
        builder
    };
    let builder = if let Some(bind) = bind {
        builder
            .clear_ip_transports()
            .bind_addr(bind)
            .map_err(|error| anyhow!("invalid iroh bind address: {error}"))?
    } else {
        builder
    };
    builder
        .bind()
        .await
        .map_err(|error| anyhow!("cannot bind iroh endpoint: {error}"))
}

#[derive(Clone)]
pub struct BootstrapAuthority {
    secret_key: SecretKey,
    registrations: Arc<HashMap<PublicKey, String>>,
    worker: EndpointAddr,
    assignment_generation: u64,
    lease_ttl_seconds: u64,
    seen_nonces: Arc<Mutex<NonceCache>>,
}

impl BootstrapAuthority {
    pub fn new(
        secret_key: SecretKey,
        registered_credential: PublicKey,
        login_role: impl Into<String>,
        worker: EndpointAddr,
        assignment_generation: u64,
        lease_ttl_seconds: u64,
    ) -> Result<Self> {
        Self::new_registered(
            secret_key,
            [(registered_credential, login_role.into())],
            worker.clone(),
            assignment_generation,
            lease_ttl_seconds,
        )
    }

    pub fn new_registered(
        secret_key: SecretKey,
        registrations: impl IntoIterator<Item = (PublicKey, String)>,
        worker: EndpointAddr,
        assignment_generation: u64,
        lease_ttl_seconds: u64,
    ) -> Result<Self> {
        let now = unix_seconds()?;
        let mut validated = HashMap::new();
        for (credential, login_role) in registrations {
            let claims = AccessLeaseClaims::new(
                credential,
                login_role,
                worker.clone(),
                assignment_generation,
                now,
                now.saturating_add(lease_ttl_seconds),
                vec![
                    ApplicationProtocol::Pgwire,
                    ApplicationProtocol::Cancellation,
                ],
            )
            .context("invalid bootstrap lease registration")?;
            if validated.insert(credential, claims.login_role).is_some() {
                bail!("bootstrap lease registrations contain a duplicate credential");
            }
            if validated.len() > crate::MAX_BOOTSTRAP_REGISTRATIONS {
                bail!(
                    "bootstrap lease registrations exceed the {}-entry limit",
                    crate::MAX_BOOTSTRAP_REGISTRATIONS
                );
            }
        }
        if validated.is_empty() {
            bail!("bootstrap lease registrations cannot be empty");
        }
        Ok(Self {
            secret_key,
            registrations: Arc::new(validated),
            worker,
            assignment_generation,
            lease_ttl_seconds,
            seen_nonces: Arc::new(Mutex::new(NonceCache::default())),
        })
    }

    fn issue(
        &self,
        request: &LeaseRequest,
        remote_transport: EndpointId,
    ) -> Result<SignedAccessLease> {
        request
            .verify(self.secret_key.public(), remote_transport)
            .context("lease request proof failed")?;
        let login_role = self
            .registrations
            .get(&request.credential)
            .ok_or_else(|| anyhow!("credential is not registered"))?;
        self.seen_nonces
            .lock()
            .map_err(|_| anyhow!("lease nonce cache is unavailable"))?
            .insert(request.credential, request.nonce)?;
        let now = unix_seconds()?;
        let claims = AccessLeaseClaims::new(
            request.credential,
            login_role.clone(),
            self.worker.clone(),
            self.assignment_generation,
            now,
            now.saturating_add(self.lease_ttl_seconds),
            vec![
                ApplicationProtocol::Pgwire,
                ApplicationProtocol::Cancellation,
            ],
        )?;
        SignedAccessLease::issue(claims, &self.secret_key).map_err(Into::into)
    }
}

#[derive(Default)]
struct NonceCache {
    values: HashSet<(PublicKey, [u8; 32])>,
    order: VecDeque<(PublicKey, [u8; 32])>,
}

impl NonceCache {
    fn insert(&mut self, credential: PublicKey, nonce: [u8; 32]) -> Result<()> {
        let entry = (credential, nonce);
        if !self.values.insert(entry) {
            bail!("lease request nonce was already used");
        }
        self.order.push_back(entry);
        if self.order.len() > NONCE_CACHE_CAPACITY
            && let Some(expired) = self.order.pop_front()
        {
            self.values.remove(&expired);
        }
        Ok(())
    }
}

pub async fn serve_bootstrap(
    endpoint: Endpoint,
    authority: BootstrapAuthority,
    max_connections: usize,
    mut shutdown: watch::Receiver<bool>,
) -> Result<()> {
    let limit = Arc::new(Semaphore::new(max_connections));
    let mut tasks = JoinSet::new();
    loop {
        tokio::select! {
            biased;
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    break;
                }
            }
            incoming = endpoint.accept() => {
                let Some(incoming) = incoming else { break };
                let connection = match incoming.await {
                    Ok(connection) => connection,
                    Err(error) => {
                        log::warn!("iroh control handshake rejected: {error}");
                        continue;
                    }
                };
                let Ok(permit) = Arc::clone(&limit).try_acquire_owned() else {
                    connection.close(1u32.into(), b"control connection limit");
                    continue;
                };
                let authority = authority.clone();
                tasks.spawn(async move {
                    let _permit = permit;
                    if let Err(error) = handle_bootstrap_connection(connection, authority).await {
                        log::warn!("iroh control request rejected: {error}");
                    }
                });
            }
        }
    }
    endpoint.close().await;
    while let Some(result) = tasks.join_next().await {
        if let Err(error) = result {
            log::warn!("iroh control task failed: {error}");
        }
    }
    Ok(())
}

async fn handle_bootstrap_connection(
    connection: Connection,
    authority: BootstrapAuthority,
) -> Result<()> {
    let remote_transport = connection.remote_id();
    let (mut send, mut recv) = connection
        .accept_bi()
        .await
        .map_err(|error| anyhow!("cannot accept control stream: {error}"))?;
    let request: LeaseRequest = read_control(&mut recv).await?;
    let lease = authority.issue(&request, remote_transport)?;
    write_control(&mut send, &lease).await?;
    send.finish()
        .map_err(|error| anyhow!("cannot finish control response: {error}"))?;
    connection.closed().await;
    Ok(())
}

#[derive(Clone)]
pub struct WorkerAuthority {
    pub bootstrap_public_key: PublicKey,
    pub backend: SocketAddr,
    pub max_streams_per_connection: usize,
    pub compression_policy: CompressionPolicy,
    pub metrics: TransportMetrics,
}

impl WorkerAuthority {
    pub fn new(
        bootstrap_public_key: PublicKey,
        backend: SocketAddr,
        max_streams_per_connection: usize,
    ) -> Self {
        Self {
            bootstrap_public_key,
            backend,
            max_streams_per_connection,
            compression_policy: CompressionPolicy::Off,
            metrics: TransportMetrics::default(),
        }
    }

    pub fn with_compression(
        mut self,
        compression_policy: CompressionPolicy,
        metrics: TransportMetrics,
    ) -> Self {
        self.compression_policy = compression_policy;
        self.metrics = metrics;
        self
    }
}

pub async fn serve_worker(
    endpoint: Endpoint,
    authority: WorkerAuthority,
    max_connections: usize,
    mut shutdown: watch::Receiver<bool>,
) -> Result<()> {
    let limit = Arc::new(Semaphore::new(max_connections));
    let mut tasks = JoinSet::new();
    loop {
        tokio::select! {
            biased;
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    break;
                }
            }
            incoming = endpoint.accept() => {
                let Some(incoming) = incoming else { break };
                let connection = match incoming.await {
                    Ok(connection) => connection,
                    Err(error) => {
                        log::warn!("iroh edge handshake rejected: {error}");
                        continue;
                    }
                };
                let Ok(permit) = Arc::clone(&limit).try_acquire_owned() else {
                    connection.close(1u32.into(), b"edge connection limit");
                    continue;
                };
                let endpoint_id = endpoint.id();
                let authority = authority.clone();
                tasks.spawn(async move {
                    let _permit = permit;
                    if let Err(error) = handle_worker_connection(connection, endpoint_id, authority).await {
                        log::warn!("iroh edge connection rejected: {error}");
                    }
                });
            }
        }
    }
    endpoint.close().await;
    while let Some(result) = tasks.join_next().await {
        if let Err(error) = result {
            log::warn!("iroh edge task failed: {error}");
        }
    }
    Ok(())
}

async fn handle_worker_connection(
    connection: Connection,
    worker_id: EndpointId,
    authority: WorkerAuthority,
) -> Result<()> {
    let remote_transport = connection.remote_id();
    let (mut auth_send, mut auth_recv) = connection
        .accept_bi()
        .await
        .map_err(|error| anyhow!("cannot accept edge authentication stream: {error}"))?;
    let mut kickoff = [0u8; 1];
    auth_recv
        .read_exact(&mut kickoff)
        .await
        .map_err(|error| anyhow!("cannot read edge authentication kickoff: {error}"))?;
    if kickoff[0] != PROTOCOL_VERSION {
        bail!("unsupported edge authentication kickoff");
    }
    let challenge = EdgeChallenge::new(random_nonce());
    write_control(&mut auth_send, &challenge).await?;
    let authentication: EdgeAuthenticate = read_control(&mut auth_recv).await?;
    authentication.verify(
        authority.bootstrap_public_key,
        unix_seconds()?,
        worker_id,
        remote_transport,
        &challenge,
    )?;
    let selected_compression = authority
        .compression_policy
        .select(&authentication.compression_offers)?;
    let authenticated =
        EdgeAuthenticated::select(selected_compression, &authentication.compression_offers)?;
    write_control(&mut auth_send, &authenticated).await?;
    auth_send
        .finish()
        .map_err(|error| anyhow!("cannot finish edge authentication: {error}"))?;

    let stream_limit = Arc::new(Semaphore::new(authority.max_streams_per_connection));
    let mut streams = JoinSet::new();
    loop {
        let accepted = connection.accept_bi().await;
        let (mut send, mut recv) = match accepted {
            Ok(streams) => streams,
            Err(_) => break,
        };
        let Ok(permit) = Arc::clone(&stream_limit).try_acquire_owned() else {
            let _ = send.reset(2u32.into());
            let _ = recv.stop(2u32.into());
            continue;
        };
        let lease = authentication.lease.clone();
        let authority = authority.clone();
        streams.spawn(async move {
            let _permit = permit;
            if let Err(error) = forward_application_stream(
                &mut send,
                &mut recv,
                lease,
                authenticated.compression,
                worker_id,
                authority,
            )
            .await
            {
                log::warn!("iroh application stream rejected: {error}");
                let _ = send.reset(3u32.into());
                let _ = recv.stop(3u32.into());
            }
        });
    }
    while let Some(result) = streams.join_next().await {
        if let Err(error) = result {
            log::warn!("iroh application stream task failed: {error}");
        }
    }
    Ok(())
}

async fn forward_application_stream(
    send: &mut SendStream,
    recv: &mut RecvStream,
    lease: SignedAccessLease,
    compression: CompressionCodec,
    worker_id: EndpointId,
    authority: WorkerAuthority,
) -> Result<()> {
    lease.verify(
        authority.bootstrap_public_key,
        unix_seconds()?,
        worker_id,
        lease.claims.credential,
    )?;
    let prelude: StreamPrelude = read_control(recv).await?;
    prelude.verify(&lease.claims, compression)?;
    if prelude.protocol == ApplicationProtocol::Http {
        bail!("HTTP streams are not enabled in the I0 bridge");
    }

    let initial = read_pgwire_packet(recv).await?;
    authority
        .metrics
        .record_latency_sensitive(Direction::Upstream, initial.len());
    let packet_kind = classify_initial_packet(&initial)?;
    let expected = match packet_kind {
        InitialPacketKind::Cancellation => ApplicationProtocol::Cancellation,
        InitialPacketKind::Ssl | InitialPacketKind::GssEnc | InitialPacketKind::Startup => {
            ApplicationProtocol::Pgwire
        }
    };
    if prelude.protocol != expected {
        bail!("typed stream prelude does not match pgwire startup packet");
    }

    let mut startup = initial;
    loop {
        match classify_initial_packet(&startup)? {
            InitialPacketKind::Ssl | InitialPacketKind::GssEnc => {
                send.write_all(b"N")
                    .await
                    .map_err(|error| anyhow!("cannot reject nested pgwire encryption: {error}"))?;
                authority
                    .metrics
                    .record_latency_sensitive(Direction::Downstream, 1);
                startup = read_pgwire_packet(recv).await?;
                authority
                    .metrics
                    .record_latency_sensitive(Direction::Upstream, startup.len());
            }
            InitialPacketKind::Startup => {
                let user = startup_user(&startup)?;
                if user != lease.claims.login_role {
                    bail!("pgwire startup role does not match the access lease");
                }
                break;
            }
            InitialPacketKind::Cancellation => {
                let mut backend = connect_backend(authority.backend).await?;
                backend.write_all(&startup).await?;
                backend.shutdown().await?;
                send.finish()
                    .map_err(|error| anyhow!("cannot finish cancellation stream: {error}"))?;
                return Ok(());
            }
        }
    }

    let mut backend = connect_backend(authority.backend).await?;
    backend.write_all(&startup).await?;
    let first_response = read_backend_frame(&mut backend).await?;
    validate_backend_authentication(&first_response)?;
    send.write_all(&first_response)
        .await
        .map_err(|error| anyhow!("cannot forward backend startup response: {error}"))?;
    authority
        .metrics
        .record_latency_sensitive(Direction::Downstream, first_response.len());

    let (mut backend_read, mut backend_write) = backend.into_split();
    let upstream_metrics = authority.metrics.clone();
    let upstream = async {
        copy_application(
            recv,
            &mut backend_write,
            compression,
            false,
            &upstream_metrics,
            Direction::Upstream,
        )
        .await
        .map_err(|error| anyhow!(error))?;
        backend_write
            .shutdown()
            .await
            .map_err(|error| anyhow!(error))
    };
    let downstream_metrics = authority.metrics.clone();
    let downstream = async {
        copy_application(
            &mut backend_read,
            send,
            compression,
            true,
            &downstream_metrics,
            Direction::Downstream,
        )
        .await
        .map_err(|error| anyhow!(error))?;
        send.finish()
            .map_err(|error| anyhow!("cannot finish edge result stream: {error}"))
    };
    tokio::try_join!(upstream, downstream)?;
    Ok(())
}

async fn connect_backend(address: SocketAddr) -> Result<TcpStream> {
    TcpStream::connect(address)
        .await
        .context("cannot connect to the complete worker's loopback pgwire boundary")
}

#[derive(Clone)]
pub struct ClientConnector {
    endpoint: Endpoint,
    credential_secret: SecretKey,
    bootstrap: EndpointAddr,
    compression_policy: CompressionPolicy,
    metrics: TransportMetrics,
}

impl ClientConnector {
    pub fn new(endpoint: Endpoint, credential_secret: SecretKey, bootstrap: EndpointAddr) -> Self {
        Self {
            endpoint,
            credential_secret,
            bootstrap,
            compression_policy: CompressionPolicy::Off,
            metrics: TransportMetrics::default(),
        }
    }

    pub fn with_compression(
        mut self,
        compression_policy: CompressionPolicy,
        metrics: TransportMetrics,
    ) -> Self {
        self.compression_policy = compression_policy;
        self.metrics = metrics;
        self
    }

    pub fn metrics(&self) -> TransportMetrics {
        self.metrics.clone()
    }

    pub async fn connect(&self) -> Result<EdgeSession> {
        let lease = request_access_lease(
            &self.endpoint,
            &self.credential_secret,
            self.bootstrap.clone(),
        )
        .await?;
        lease.verify(
            self.bootstrap.id,
            unix_seconds()?,
            lease.claims.worker.id,
            self.credential_secret.public(),
        )?;
        let connection = self
            .endpoint
            .connect(lease.claims.worker.clone(), EDGE_ALPN)
            .await
            .map_err(|error| anyhow!("cannot connect to leased worker: {error}"))?;
        let (mut send, mut recv) = connection
            .open_bi()
            .await
            .map_err(|error| anyhow!("cannot open edge authentication stream: {error}"))?;
        send.write_all(&[PROTOCOL_VERSION])
            .await
            .map_err(|error| anyhow!("cannot start edge authentication: {error}"))?;
        let challenge: EdgeChallenge = read_control(&mut recv).await?;
        let authentication = EdgeAuthenticate::sign(
            lease.clone(),
            &self.credential_secret,
            self.endpoint.id(),
            challenge,
            self.compression_policy.offers(),
        )?;
        write_control(&mut send, &authentication).await?;
        send.finish()
            .map_err(|error| anyhow!("cannot finish edge proof: {error}"))?;
        let authenticated: EdgeAuthenticated = read_control(&mut recv).await?;
        if authenticated.version != PROTOCOL_VERSION
            || !authentication
                .compression_offers
                .contains(&authenticated.compression)
        {
            bail!("worker selected an unsupported transport capability");
        }
        Ok(EdgeSession {
            connection,
            lease,
            compression: authenticated.compression,
            metrics: self.metrics.clone(),
        })
    }
}

#[derive(Clone)]
pub struct EdgeSession {
    connection: Connection,
    lease: SignedAccessLease,
    compression: CompressionCodec,
    metrics: TransportMetrics,
}

impl EdgeSession {
    pub fn is_usable(&self, minimum_remaining_seconds: u64) -> bool {
        self.connection.close_reason().is_none()
            && self.lease.claims.expires_at_unix_seconds
                > unix_seconds()
                    .unwrap_or(u64::MAX)
                    .saturating_add(minimum_remaining_seconds)
    }

    pub async fn open(&self, protocol: ApplicationProtocol) -> Result<(SendStream, RecvStream)> {
        if !self.is_usable(0) {
            bail!("edge session is closed or its access lease has expired");
        }
        if !self.lease.claims.permits(protocol) {
            bail!("application protocol is not permitted by the access lease");
        }
        let (mut send, recv) = self
            .connection
            .open_bi()
            .await
            .map_err(|error| anyhow!("cannot open edge stream: {error}"))?;
        let compression = match protocol {
            ApplicationProtocol::Cancellation => CompressionCodec::None,
            _ => self.compression,
        };
        write_control(&mut send, &StreamPrelude::new(protocol, compression)).await?;
        self.metrics.record_stream(protocol);
        Ok((send, recv))
    }
}

pub async fn request_access_lease(
    endpoint: &Endpoint,
    credential_secret: &SecretKey,
    bootstrap: EndpointAddr,
) -> Result<SignedAccessLease> {
    let connection = endpoint
        .connect(bootstrap.clone(), CONTROL_ALPN)
        .await
        .map_err(|error| anyhow!("cannot connect to bootstrap: {error}"))?;
    let (mut send, mut recv) = connection
        .open_bi()
        .await
        .map_err(|error| anyhow!("cannot open control stream: {error}"))?;
    let request = LeaseRequest::sign(
        credential_secret,
        endpoint.id(),
        bootstrap.id,
        random_nonce(),
    )?;
    write_control(&mut send, &request).await?;
    send.finish()
        .map_err(|error| anyhow!("cannot finish lease request: {error}"))?;
    let lease = read_control(&mut recv).await?;
    connection.close(0u32.into(), b"lease received");
    Ok(lease)
}

pub async fn serve_local_client(
    listener: TcpListener,
    connector: ClientConnector,
    max_connections: usize,
    shutdown: watch::Receiver<bool>,
) -> Result<()> {
    serve_local_client_with_tls(listener, connector, None, max_connections, shutdown).await
}

#[derive(Clone)]
pub struct LocalClientTls(TlsAcceptor);

impl LocalClientTls {
    pub fn load(
        certificate_path: &Path,
        private_key_path: &Path,
        client_ca_path: &Path,
    ) -> Result<Self> {
        let _ = tokio_rustls::rustls::crypto::ring::default_provider().install_default();
        let certificates = certs(&mut BufReader::new(File::open(certificate_path)?))
            .collect::<Result<Vec<CertificateDer<'static>>, IoError>>()?;
        if certificates.is_empty() {
            bail!("local TLS certificate file is empty");
        }
        let key =
            private_key(&mut BufReader::new(File::open(private_key_path)?))?.ok_or_else(|| {
                IoError::new(ErrorKind::InvalidInput, "local TLS private key is missing")
            })?;
        let client_roots = certs(&mut BufReader::new(File::open(client_ca_path)?))
            .collect::<Result<Vec<CertificateDer<'static>>, IoError>>()?;
        if client_roots.is_empty() {
            bail!("local TLS client CA file is empty");
        }
        let mut roots = RootCertStore::empty();
        for certificate in client_roots {
            roots
                .add(certificate)
                .context("invalid local TLS client CA certificate")?;
        }
        let verifier = WebPkiClientVerifier::builder(Arc::new(roots))
            .build()
            .context("invalid local TLS client verifier")?;
        let config = ServerConfig::builder()
            .with_client_cert_verifier(verifier)
            .with_single_cert(certificates, key)
            .context("invalid local TLS certificate or private key")?;
        Ok(Self(TlsAcceptor::from(Arc::new(config))))
    }
}

pub async fn serve_local_client_with_tls(
    listener: TcpListener,
    connector: ClientConnector,
    local_tls: Option<LocalClientTls>,
    max_connections: usize,
    mut shutdown: watch::Receiver<bool>,
) -> Result<()> {
    let session = Arc::new(tokio::sync::Mutex::new(None::<EdgeSession>));
    let limit = Arc::new(Semaphore::new(max_connections));
    let mut tasks = JoinSet::new();
    loop {
        tokio::select! {
            biased;
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    break;
                }
            }
            accepted = listener.accept() => {
                let (socket, _) = accepted?;
                let Ok(permit) = Arc::clone(&limit).try_acquire_owned() else {
                    drop(socket);
                    continue;
                };
                let connector = connector.clone();
                let session = Arc::clone(&session);
                let local_tls = local_tls.clone();
                tasks.spawn(async move {
                    let _permit = permit;
                    if let Err(error) = forward_local_connection(socket, connector, session, local_tls).await {
                        log::warn!("local pgwire bridge connection failed: {error}");
                    }
                });
            }
        }
    }
    while let Some(result) = tasks.join_next().await {
        if let Err(error) = result {
            log::warn!("local pgwire bridge task failed: {error}");
        }
    }
    Ok(())
}

async fn forward_local_connection(
    mut socket: TcpStream,
    connector: ClientConnector,
    session: Arc<tokio::sync::Mutex<Option<EdgeSession>>>,
    local_tls: Option<LocalClientTls>,
) -> Result<()> {
    if let Some(local_tls) = local_tls {
        let mut request = tokio::time::timeout(
            LOCAL_NEGOTIATION_TIMEOUT,
            read_encryption_request(&mut socket),
        )
        .await
        .context("local pgwire encryption negotiation timed out")??;
        if request == InitialPacketKind::GssEnc {
            socket.write_all(b"N").await?;
            request = tokio::time::timeout(
                LOCAL_NEGOTIATION_TIMEOUT,
                read_encryption_request(&mut socket),
            )
            .await
            .context("local pgwire encryption negotiation timed out")??;
        }
        if request != InitialPacketKind::Ssl {
            bail!("local pgwire bridge requires mutual TLS");
        }
        socket.write_all(b"S").await?;
        let mut socket =
            tokio::time::timeout(LOCAL_NEGOTIATION_TIMEOUT, local_tls.0.accept(socket))
                .await
                .context("local pgwire mutual TLS handshake timed out")?
                .context("local pgwire mutual TLS handshake failed")?;
        let initial =
            tokio::time::timeout(LOCAL_NEGOTIATION_TIMEOUT, read_pgwire_packet(&mut socket))
                .await
                .context("local pgwire startup timed out")??;
        if matches!(
            classify_initial_packet(&initial)?,
            InitialPacketKind::Ssl | InitialPacketKind::GssEnc
        ) {
            bail!("nested local encryption request is not permitted");
        }
        return forward_local_stream(socket, initial, connector, session).await;
    }
    let initial = tokio::time::timeout(LOCAL_NEGOTIATION_TIMEOUT, read_pgwire_packet(&mut socket))
        .await
        .context("local pgwire startup timed out")??;
    forward_local_stream(socket, initial, connector, session).await
}

async fn read_encryption_request(socket: &mut TcpStream) -> Result<InitialPacketKind> {
    let mut packet = [0_u8; 8];
    socket
        .read_exact(&mut packet)
        .await
        .context("cannot read local pgwire encryption request")?;
    let kind = classify_initial_packet(&packet)?;
    if !matches!(kind, InitialPacketKind::Ssl | InitialPacketKind::GssEnc) {
        bail!("local pgwire bridge requires an encryption request");
    }
    Ok(kind)
}

async fn forward_local_stream<S>(
    socket: S,
    initial: Vec<u8>,
    connector: ClientConnector,
    session: Arc<tokio::sync::Mutex<Option<EdgeSession>>>,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let result = forward_local_stream_once(socket, initial, connector, Arc::clone(&session)).await;
    if result.is_err() {
        session.lock().await.take();
    }
    result
}

async fn forward_local_stream_once<S>(
    mut socket: S,
    mut initial: Vec<u8>,
    connector: ClientConnector,
    session: Arc<tokio::sync::Mutex<Option<EdgeSession>>>,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let protocol = match classify_initial_packet(&initial)? {
        InitialPacketKind::Cancellation => ApplicationProtocol::Cancellation,
        _ => ApplicationProtocol::Pgwire,
    };
    let edge = {
        let mut current = session.lock().await;
        if !current.as_ref().is_some_and(|value| value.is_usable(10)) {
            *current = Some(connector.connect().await?);
        }
        current
            .as_ref()
            .cloned()
            .ok_or_else(|| anyhow!("edge session was not established"))?
    };
    let (mut send, mut recv) = edge.open(protocol).await?;
    loop {
        let packet_kind = classify_initial_packet(&initial)?;
        send.write_all(&initial)
            .await
            .map_err(|error| anyhow!("cannot forward pgwire startup: {error}"))?;
        edge.metrics
            .record_latency_sensitive(Direction::Upstream, initial.len());
        match packet_kind {
            InitialPacketKind::Ssl | InitialPacketKind::GssEnc => {
                let mut denied = [0_u8; 1];
                tokio::time::timeout(EDGE_FIRST_RESPONSE_TIMEOUT, recv.read_exact(&mut denied))
                    .await
                    .context("worker nested encryption response timed out")?
                    .map_err(|error| anyhow!("cannot read nested encryption denial: {error}"))?;
                if denied != [b'N'] {
                    bail!("worker returned an invalid nested encryption response");
                }
                edge.metrics
                    .record_latency_sensitive(Direction::Downstream, denied.len());
                socket.write_all(&denied).await?;
                initial = read_pgwire_packet(&mut socket).await?;
            }
            InitialPacketKind::Startup => {
                let first_response = tokio::time::timeout(
                    EDGE_FIRST_RESPONSE_TIMEOUT,
                    read_backend_frame(&mut recv),
                )
                .await
                .context("worker authentication response timed out")??;
                validate_backend_authentication(&first_response)?;
                edge.metrics
                    .record_latency_sensitive(Direction::Downstream, first_response.len());
                socket.write_all(&first_response).await?;
                break;
            }
            InitialPacketKind::Cancellation => {
                send.finish()
                    .map_err(|error| anyhow!("cannot finish cancellation stream: {error}"))?;
                let response =
                    tokio::time::timeout(EDGE_FIRST_RESPONSE_TIMEOUT, recv.read_to_end(1))
                        .await
                        .context("worker cancellation response timed out")?
                        .map_err(|error| anyhow!("cannot finish cancellation response: {error}"))?;
                if !response.is_empty() {
                    bail!("worker returned unexpected cancellation bytes");
                }
                socket.shutdown().await?;
                return Ok(());
            }
        }
    }
    let (mut local_read, mut local_write) = tokio::io::split(socket);
    let upstream_metrics = edge.metrics.clone();
    let upstream = async {
        copy_application(
            &mut local_read,
            &mut send,
            edge.compression,
            true,
            &upstream_metrics,
            Direction::Upstream,
        )
        .await
        .map_err(|error| anyhow!(error))?;
        send.finish()
            .map_err(|error| anyhow!("cannot finish local edge stream: {error}"))
    };
    let downstream_metrics = edge.metrics.clone();
    let downstream = async {
        copy_application(
            &mut recv,
            &mut local_write,
            edge.compression,
            false,
            &downstream_metrics,
            Direction::Downstream,
        )
        .await
        .map_err(|error| anyhow!(error))?;
        local_write.shutdown().await.map_err(|error| anyhow!(error))
    };
    tokio::try_join!(upstream, downstream)?;
    Ok(())
}

pub async fn write_control<T: Serialize>(send: &mut SendStream, value: &T) -> Result<()> {
    let encoded = encode_control(value)?;
    let length = u32::try_from(encoded.len()).context("control message length overflow")?;
    send.write_all(&length.to_be_bytes())
        .await
        .map_err(|error| anyhow!("cannot write control length: {error}"))?;
    send.write_all(&encoded)
        .await
        .map_err(|error| anyhow!("cannot write control message: {error}"))
}

pub async fn read_control<T: DeserializeOwned>(recv: &mut RecvStream) -> Result<T> {
    let mut header = [0u8; 4];
    recv.read_exact(&mut header)
        .await
        .map_err(|error| anyhow!("cannot read control length: {error}"))?;
    let length = u32::from_be_bytes(header) as usize;
    if length > MAX_CONTROL_MESSAGE_BYTES {
        bail!("control message exceeds the {MAX_CONTROL_MESSAGE_BYTES}-byte limit");
    }
    let mut encoded = vec![0; length];
    recv.read_exact(&mut encoded)
        .await
        .map_err(|error| anyhow!("cannot read control message: {error}"))?;
    decode_control(&encoded).map_err(Into::into)
}

async fn read_pgwire_packet<R>(reader: &mut R) -> Result<Vec<u8>>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut header = [0u8; 4];
    reader.read_exact(&mut header).await?;
    let length = u32::from_be_bytes(header) as usize;
    if !(8..=MAX_INITIAL_PGWIRE_PACKET_BYTES).contains(&length) {
        bail!("invalid or oversized initial pgwire packet length");
    }
    let mut packet = Vec::with_capacity(length);
    packet.extend_from_slice(&header);
    packet.resize(length, 0);
    reader.read_exact(&mut packet[4..]).await?;
    Ok(packet)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InitialPacketKind {
    Startup,
    Cancellation,
    Ssl,
    GssEnc,
}

fn classify_initial_packet(packet: &[u8]) -> Result<InitialPacketKind> {
    if packet.len() < 8 {
        bail!("initial pgwire packet is truncated");
    }
    let code = u32::from_be_bytes(packet[4..8].try_into().expect("checked packet length"));
    match code {
        POSTGRESQL_PROTOCOL_3 => Ok(InitialPacketKind::Startup),
        CANCEL_REQUEST_CODE if packet.len() == 16 => Ok(InitialPacketKind::Cancellation),
        SSL_REQUEST_CODE if packet.len() == 8 => Ok(InitialPacketKind::Ssl),
        GSSENC_REQUEST_CODE if packet.len() == 8 => Ok(InitialPacketKind::GssEnc),
        _ => bail!("unsupported initial pgwire request"),
    }
}

fn startup_user(packet: &[u8]) -> Result<String> {
    if classify_initial_packet(packet)? != InitialPacketKind::Startup
        || packet.last().copied() != Some(0)
    {
        bail!("invalid pgwire startup packet");
    }
    let mut fields = packet[8..packet.len() - 1]
        .split(|byte| *byte == 0)
        .collect::<Vec<_>>();
    if fields.last().is_some_and(|field| field.is_empty()) {
        fields.pop();
    }
    if fields.len() % 2 != 0 {
        bail!("pgwire startup fields are not key/value pairs");
    }
    let mut user = None;
    for pair in fields.chunks_exact(2) {
        if pair[0] == b"user" {
            if user.is_some() {
                bail!("pgwire startup contains duplicate user fields");
            }
            user = Some(
                std::str::from_utf8(pair[1])
                    .context("pgwire startup user is not UTF-8")?
                    .to_owned(),
            );
        }
    }
    user.filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("pgwire startup user is missing"))
}

async fn read_backend_frame<R>(backend: &mut R) -> Result<Vec<u8>>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut header = [0u8; 5];
    backend.read_exact(&mut header).await?;
    let length = u32::from_be_bytes(header[1..5].try_into().expect("fixed header")) as usize;
    if !(4..=MAX_CONTROL_MESSAGE_BYTES).contains(&length) {
        bail!("invalid or oversized first backend frame");
    }
    let mut frame = Vec::with_capacity(length + 1);
    frame.extend_from_slice(&header);
    frame.resize(length + 1, 0);
    backend.read_exact(&mut frame[5..]).await?;
    Ok(frame)
}

fn validate_backend_authentication(frame: &[u8]) -> Result<()> {
    if frame.first().copied() != Some(b'R') || frame.len() != 9 {
        bail!("loopback backend did not begin with PostgreSQL AuthenticationOk");
    }
    let auth_code = u32::from_be_bytes(frame[5..9].try_into().expect("checked auth frame"));
    if auth_code != 0 {
        bail!("loopback backend requested credentials across the iroh edge");
    }
    Ok(())
}

fn random_nonce() -> [u8; 32] {
    SecretKey::generate().to_bytes()
}

fn unix_seconds() -> Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before the Unix epoch")
        .map(|duration| duration.as_secs())
}

pub async fn run_until_signal<F>(
    endpoint: Endpoint,
    shutdown: watch::Sender<bool>,
    service: F,
) -> Result<()>
where
    F: Future<Output = Result<()>>,
{
    tokio::pin!(service);
    tokio::select! {
        result = &mut service => result,
        signal = shutdown_signal() => {
            signal?;
            let _ = shutdown.send(true);
            endpoint.close().await;
            tokio::time::timeout(SERVICE_SHUTDOWN_TIMEOUT, service)
                .await
                .context("edge service shutdown timed out")?
        },
    }
}

async fn shutdown_signal() -> Result<()> {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .context("cannot install termination signal handler")?;
        tokio::select! {
            signal = tokio::signal::ctrl_c() => signal.context("cannot install interrupt signal handler"),
            _ = terminate.recv() => Ok(()),
        }
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c()
        .await
        .context("cannot install interrupt signal handler")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_role_is_structural_and_exact() {
        let mut packet = Vec::new();
        packet.extend_from_slice(&0u32.to_be_bytes());
        packet.extend_from_slice(&POSTGRESQL_PROTOCOL_3.to_be_bytes());
        packet.extend_from_slice(b"user\0reader\0database\0quackgis\0\0");
        let length = u32::try_from(packet.len()).unwrap();
        packet[..4].copy_from_slice(&length.to_be_bytes());
        assert_eq!(startup_user(&packet).unwrap(), "reader");
        packet[13] = b'R';
        assert_eq!(startup_user(&packet).unwrap(), "Reader");
    }

    #[test]
    fn backend_must_use_authentication_ok() {
        let ok = [b'R', 0, 0, 0, 8, 0, 0, 0, 0];
        assert!(validate_backend_authentication(&ok).is_ok());
        let scram = [b'R', 0, 0, 0, 8, 0, 0, 0, 10];
        assert!(validate_backend_authentication(&scram).is_err());
    }

    #[test]
    fn replay_cache_is_bounded_and_rejects_reuse() {
        let credential = SecretKey::from_bytes(&[22; 32]).public();
        let mut cache = NonceCache::default();
        cache.insert(credential, [1; 32]).unwrap();
        assert!(cache.insert(credential, [1; 32]).is_err());
        for value in 0..=NONCE_CACHE_CAPACITY {
            let mut nonce = [0; 32];
            nonce[..8].copy_from_slice(&(value as u64).to_be_bytes());
            cache.insert(credential, nonce).ok();
        }
        assert!(cache.order.len() <= NONCE_CACHE_CAPACITY);
        assert!(cache.values.len() <= NONCE_CACHE_CAPACITY);
    }

    #[test]
    fn bootstrap_registration_selects_role_only_from_proven_credential() {
        let bootstrap = SecretKey::from_bytes(&[41; 32]);
        let postgres = SecretKey::from_bytes(&[42; 32]);
        let authenticator = SecretKey::from_bytes(&[43; 32]);
        let unknown = SecretKey::from_bytes(&[44; 32]);
        let transport = SecretKey::from_bytes(&[45; 32]).public();
        let worker = EndpointAddr::new(SecretKey::from_bytes(&[46; 32]).public());
        let authority = BootstrapAuthority::new_registered(
            bootstrap.clone(),
            [
                (postgres.public(), "postgres".to_owned()),
                (authenticator.public(), "authenticator".to_owned()),
            ],
            worker,
            1,
            60,
        )
        .expect("plural bootstrap authority");
        let postgres_request =
            LeaseRequest::sign(&postgres, transport, bootstrap.public(), [1; 32]).unwrap();
        let authenticator_request =
            LeaseRequest::sign(&authenticator, transport, bootstrap.public(), [2; 32]).unwrap();
        assert_eq!(
            authority
                .issue(&postgres_request, transport)
                .unwrap()
                .claims
                .login_role,
            "postgres"
        );
        assert_eq!(
            authority
                .issue(&authenticator_request, transport)
                .unwrap()
                .claims
                .login_role,
            "authenticator"
        );
        let unknown_request =
            LeaseRequest::sign(&unknown, transport, bootstrap.public(), [3; 32]).unwrap();
        assert!(authority.issue(&unknown_request, transport).is_err());
        assert!(
            BootstrapAuthority::new_registered(
                bootstrap,
                [
                    (postgres.public(), "postgres".to_owned()),
                    (postgres.public(), "authenticator".to_owned()),
                ],
                EndpointAddr::new(SecretKey::from_bytes(&[46; 32]).public()),
                1,
                60,
            )
            .is_err()
        );
    }
}
