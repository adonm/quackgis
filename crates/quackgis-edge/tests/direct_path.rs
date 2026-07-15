// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::{Context, Result, anyhow};
use iroh::{Endpoint, SecretKey, endpoint::presets};
use quackgis_edge::runtime::{
    BootstrapAuthority, ClientConnector, WorkerAuthority, serve_bootstrap, serve_local_client,
    serve_worker,
};
use quackgis_edge::{ApplicationProtocol, CONTROL_ALPN, EDGE_ALPN};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;

#[tokio::test(flavor = "multi_thread")]
async fn one_lease_multiplexes_independent_pgwire_sessions() -> Result<()> {
    let _ = env_logger::builder().is_test(true).try_init();
    let backend = TcpListener::bind("127.0.0.1:0").await?;
    let backend_address = backend.local_addr()?;
    let backend_connections = Arc::new(AtomicUsize::new(0));
    let backend_task = tokio::spawn({
        let backend_connections = Arc::clone(&backend_connections);
        async move {
            loop {
                let (socket, _) = backend.accept().await?;
                backend_connections.fetch_add(1, Ordering::SeqCst);
                tokio::spawn(fake_trust_pgwire(socket));
            }
            #[allow(unreachable_code)]
            Ok::<(), anyhow::Error>(())
        }
    });

    let bootstrap_secret = SecretKey::from_bytes(&[31; 32]);
    let worker_secret = SecretKey::from_bytes(&[32; 32]);
    let credential_secret = SecretKey::from_bytes(&[33; 32]);
    let client_transport_secret = SecretKey::from_bytes(&[34; 32]);

    let worker = direct_endpoint(worker_secret, vec![EDGE_ALPN.to_vec()]).await?;
    let bootstrap = direct_endpoint(bootstrap_secret.clone(), vec![CONTROL_ALPN.to_vec()]).await?;
    let client = direct_endpoint(client_transport_secret, vec![]).await?;

    let authority = BootstrapAuthority::new(
        bootstrap_secret.clone(),
        credential_secret.public(),
        "reader",
        worker.addr(),
        1,
        60,
    )?;
    let (_bootstrap_shutdown_guard, bootstrap_shutdown) = watch::channel(false);
    let bootstrap_task = tokio::spawn(serve_bootstrap(
        bootstrap.clone(),
        authority,
        4,
        bootstrap_shutdown,
    ));
    let (_worker_shutdown_guard, worker_shutdown) = watch::channel(false);
    let worker_task = tokio::spawn(serve_worker(
        worker.clone(),
        WorkerAuthority {
            bootstrap_public_key: bootstrap_secret.public(),
            backend: backend_address,
            max_streams_per_connection: 4,
        },
        4,
        worker_shutdown,
    ));

    let connector = ClientConnector::new(client.clone(), credential_secret, bootstrap.addr());
    let session = connector.connect().await?;
    let first = exercise_session(session.clone(), b"first");
    let second = exercise_session(session, b"second");
    let (first, second) = tokio::try_join!(first, second)?;
    assert_eq!(first, b"first");
    assert_eq!(second, b"second");
    assert_eq!(backend_connections.load(Ordering::SeqCst), 2);

    let local_listener = TcpListener::bind("127.0.0.1:0").await?;
    let local_address = local_listener.local_addr()?;
    let (local_shutdown_tx, local_shutdown) = watch::channel(false);
    let local_client_task = tokio::spawn(serve_local_client(
        local_listener,
        connector,
        4,
        local_shutdown,
    ));
    let mut local = TcpStream::connect(local_address).await?;
    local.write_all(&[0, 0, 0, 8, 4, 210, 22, 47]).await?;
    let mut encryption_denied = [0; 1];
    local.read_exact(&mut encryption_denied).await?;
    assert_eq!(encryption_denied, [b'N']);
    local.write_all(&startup_packet("reader")).await?;
    let mut authentication_ok = [0; 9];
    local.read_exact(&mut authentication_ok).await?;
    assert_eq!(authentication_ok, [b'R', 0, 0, 0, 8, 0, 0, 0, 0]);
    local.write_all(b"through-local-client").await?;
    let mut echoed = [0; 20];
    local.read_exact(&mut echoed).await?;
    assert_eq!(&echoed, b"through-local-client");
    local.shutdown().await?;
    wait_for_connections(&backend_connections, 3).await?;

    let mut cancellation = TcpStream::connect(local_address).await?;
    cancellation
        .write_all(&[0, 0, 0, 16, 4, 210, 22, 46, 0, 0, 0, 1, 0, 0, 0, 2])
        .await?;
    cancellation.shutdown().await?;
    cancellation.read_to_end(&mut Vec::new()).await?;
    wait_for_connections(&backend_connections, 4).await?;
    local_shutdown_tx.send(true).ok();
    local_client_task.await??;

    client.close().await;
    bootstrap.close().await;
    worker.close().await;
    backend_task.abort();
    let _ = bootstrap_task.await;
    let _ = worker_task.await;
    Ok(())
}

async fn direct_endpoint(secret: SecretKey, alpns: Vec<Vec<u8>>) -> Result<Endpoint> {
    Endpoint::builder(presets::N0)
        .secret_key(secret)
        .alpns(alpns)
        .relay_mode(iroh::RelayMode::Disabled)
        .clear_address_lookup()
        .clear_ip_transports()
        .bind_addr("127.0.0.1:0")?
        .bind()
        .await
        .map_err(|error| anyhow!("direct endpoint bind failed: {error}"))
}

async fn fake_trust_pgwire(mut socket: TcpStream) -> Result<()> {
    let mut length = [0; 4];
    socket.read_exact(&mut length).await?;
    let length = u32::from_be_bytes(length) as usize;
    let mut startup = vec![0; length - 4];
    socket.read_exact(&mut startup).await?;
    if length == 16 && startup[..4] == [4, 210, 22, 46] {
        return Ok(());
    }
    socket.write_all(&[b'R', 0, 0, 0, 8, 0, 0, 0, 0]).await?;
    let mut buffer = [0; 1024];
    loop {
        let read = socket.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        socket.write_all(&buffer[..read]).await?;
    }
    Ok(())
}

async fn wait_for_connections(counter: &AtomicUsize, expected: usize) -> Result<()> {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while counter.load(Ordering::SeqCst) < expected {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .context("backend connection count did not advance")
}

async fn exercise_session(
    session: quackgis_edge::runtime::EdgeSession,
    payload: &'static [u8],
) -> Result<Vec<u8>> {
    let (mut send, mut recv) = session.open(ApplicationProtocol::Pgwire).await?;
    let startup = startup_packet("reader");
    send.write_all(&startup)
        .await
        .map_err(|error| anyhow!("startup write failed: {error}"))?;
    let mut authentication_ok = [0; 9];
    recv.read_exact(&mut authentication_ok)
        .await
        .map_err(|error| anyhow!("auth read failed: {error}"))?;
    assert_eq!(authentication_ok, [b'R', 0, 0, 0, 8, 0, 0, 0, 0]);
    send.write_all(payload)
        .await
        .map_err(|error| anyhow!("payload write failed: {error}"))?;
    let mut echoed = vec![0; payload.len()];
    recv.read_exact(&mut echoed)
        .await
        .map_err(|error| anyhow!("payload read failed: {error}"))?;
    send.finish()
        .map_err(|error| anyhow!("stream finish failed: {error}"))?;
    Ok(echoed)
}

fn startup_packet(user: &str) -> Vec<u8> {
    let mut packet = Vec::new();
    packet.extend_from_slice(&0u32.to_be_bytes());
    packet.extend_from_slice(&196_608u32.to_be_bytes());
    packet.extend_from_slice(b"user\0");
    packet.extend_from_slice(user.as_bytes());
    packet.extend_from_slice(b"\0database\0quackgis\0\0");
    let length = u32::try_from(packet.len()).unwrap();
    packet[..4].copy_from_slice(&length.to_be_bytes());
    packet
}
