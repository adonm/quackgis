// SPDX-License-Identifier: Apache-2.0
//! Executable I0 bootstrap, worker, and tiny-client transport path.

use std::collections::{HashSet, VecDeque};
use std::future::Future;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use iroh::endpoint::{Connection, RecvStream, SendStream};
use iroh::{Endpoint, EndpointAddr, EndpointId, PublicKey, SecretKey, endpoint::presets};
use serde::{Serialize, de::DeserializeOwned};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Semaphore, watch};
use tokio::task::JoinSet;

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

pub async fn bind_endpoint(
    secret_key: SecretKey,
    alpns: Vec<Vec<u8>>,
    relay_policy: &RelayPolicy,
) -> Result<Endpoint> {
    Endpoint::builder(presets::N0)
        .secret_key(secret_key)
        .alpns(alpns)
        .relay_mode(relay_policy.iroh_mode())
        .bind()
        .await
        .map_err(|error| anyhow!("cannot bind iroh endpoint: {error}"))
}

#[derive(Clone)]
pub struct BootstrapAuthority {
    secret_key: SecretKey,
    registered_credential: PublicKey,
    login_role: String,
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
        let now = unix_seconds()?;
        AccessLeaseClaims::new(
            registered_credential,
            login_role.into(),
            worker.clone(),
            assignment_generation,
            now,
            now.saturating_add(lease_ttl_seconds),
            vec![
                ApplicationProtocol::Pgwire,
                ApplicationProtocol::Cancellation,
            ],
        )
        .context("invalid bootstrap lease configuration")
        .map(|claims| Self {
            secret_key,
            registered_credential,
            login_role: claims.login_role,
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
        if request.credential != self.registered_credential {
            bail!("credential is not registered");
        }
        self.seen_nonces
            .lock()
            .map_err(|_| anyhow!("lease nonce cache is unavailable"))?
            .insert(request.credential, request.nonce)?;
        let now = unix_seconds()?;
        let claims = AccessLeaseClaims::new(
            request.credential,
            self.login_role.clone(),
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
                tasks.spawn(async move {
                    let _permit = permit;
                    if let Err(error) = forward_local_connection(socket, connector, session).await {
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
) -> Result<()> {
    let mut initial = read_pgwire_packet(&mut socket).await?;
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
                recv.read_exact(&mut denied)
                    .await
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
                let first_response = read_backend_frame(&mut recv).await?;
                validate_backend_authentication(&first_response)?;
                edge.metrics
                    .record_latency_sensitive(Direction::Downstream, first_response.len());
                socket.write_all(&first_response).await?;
                break;
            }
            InitialPacketKind::Cancellation => {
                send.finish()
                    .map_err(|error| anyhow!("cannot finish cancellation stream: {error}"))?;
                let response = recv
                    .read_to_end(1)
                    .await
                    .map_err(|error| anyhow!("cannot finish cancellation response: {error}"))?;
                if !response.is_empty() {
                    bail!("worker returned unexpected cancellation bytes");
                }
                socket.shutdown().await?;
                return Ok(());
            }
        }
    }
    let (mut local_read, mut local_write) = socket.into_split();
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

pub async fn run_until_signal<F>(endpoint: Endpoint, service: F) -> Result<()>
where
    F: Future<Output = Result<()>>,
{
    tokio::pin!(service);
    tokio::select! {
        result = &mut service => result,
        signal = tokio::signal::ctrl_c() => {
            signal.context("cannot install shutdown signal handler")?;
            endpoint.close().await;
            service.await
        }
    }
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
}
