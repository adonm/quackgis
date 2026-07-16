// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use iroh::{
    Endpoint, RelayMap, SecretKey, TransportAddr,
    endpoint::{TransportAddrUsage, presets},
};
use iroh_relay::tls::CaTlsConfig;
use quackgis_edge::compression::TransportMetrics;
use quackgis_edge::runtime::{
    BootstrapAuthority, ClientConnector, WorkerAuthority, serve_bootstrap, serve_local_client,
    serve_worker,
};
use quackgis_edge::{CONTROL_ALPN, CompressionPolicy, EDGE_ALPN, RelayPolicy};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;

#[tokio::test(flavor = "multi_thread")]
async fn custom_relay_forces_application_bytes_off_direct_paths() -> Result<()> {
    let (relay_map, relay_url, _relay) = iroh::test_utils::run_relay_server().await?;
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

    let bootstrap_secret = SecretKey::from_bytes(&[51; 32]);
    let worker_secret = SecretKey::from_bytes(&[52; 32]);
    let credential_secret = SecretKey::from_bytes(&[53; 32]);
    let client_transport_secret = SecretKey::from_bytes(&[54; 32]);
    let worker = relay_endpoint(
        worker_secret.clone(),
        vec![EDGE_ALPN.to_vec()],
        relay_map.clone(),
    )
    .await?;
    let bootstrap = relay_endpoint(
        bootstrap_secret.clone(),
        vec![CONTROL_ALPN.to_vec()],
        relay_map.clone(),
    )
    .await?;
    let client = relay_endpoint(client_transport_secret, vec![], relay_map.clone()).await?;
    for endpoint in [&worker, &bootstrap, &client] {
        assert!(endpoint.addr().ip_addrs().next().is_none());
        assert_eq!(endpoint.addr().relay_urls().next(), Some(&relay_url));
    }

    let authority = BootstrapAuthority::new(
        bootstrap_secret.clone(),
        credential_secret.public(),
        "reader",
        worker.addr().with_ip_addr("127.0.0.1:9".parse()?),
        1,
        60,
    )?;
    let (bootstrap_shutdown_tx, bootstrap_shutdown) = watch::channel(false);
    let bootstrap_task = tokio::spawn(serve_bootstrap(
        bootstrap.clone(),
        authority,
        4,
        bootstrap_shutdown,
    ));
    let worker_metrics = TransportMetrics::default();
    let (worker_shutdown_tx, worker_shutdown) = watch::channel(false);
    let worker_task = tokio::spawn(serve_worker(
        worker.clone(),
        WorkerAuthority::new(bootstrap_secret.public(), backend_address, 4)
            .with_compression(CompressionPolicy::Auto, worker_metrics.clone()),
        4,
        worker_shutdown,
    ));
    let client_metrics = TransportMetrics::default();
    let connector =
        ClientConnector::new(client.clone(), credential_secret.clone(), bootstrap.addr())
            .with_compression(CompressionPolicy::Auto, client_metrics.clone());
    let reconnect_connector = connector.clone();
    let local_listener = TcpListener::bind("127.0.0.1:0").await?;
    let local_address = local_listener.local_addr()?;
    let (local_shutdown_tx, local_shutdown) = watch::channel(false);
    let local_client_task = tokio::spawn(serve_local_client(
        local_listener,
        connector,
        4,
        local_shutdown,
    ));

    exercise_local_bridge(local_address, 64 * 1024).await?;
    exercise_local_bridge(local_address, 64 * 1024).await?;
    let remote = client
        .remote_info(worker.id())
        .await
        .context("client has no worker route information")?;
    assert!(remote.addrs().any(|address| {
        matches!(address.addr(), TransportAddr::Relay(url) if url == &relay_url)
            && matches!(address.usage(), TransportAddrUsage::Active)
    }));
    assert!(remote.addrs().all(|address| {
        !address.addr().is_ip() || !matches!(address.usage(), TransportAddrUsage::Active)
    }));
    assert_eq!(backend_connections.load(Ordering::SeqCst), 2);

    let client_snapshot = client_metrics.snapshot();
    let worker_snapshot = worker_metrics.snapshot();
    assert!(client_snapshot.upstream.compressed_blocks > 0);
    assert!(worker_snapshot.downstream.compressed_blocks > 0);
    assert_eq!(client_snapshot.downstream.decode_failures, 0);
    assert_eq!(worker_snapshot.upstream.decode_failures, 0);

    let mut cancellation = TcpStream::connect(local_address).await?;
    cancellation
        .write_all(&[0, 0, 0, 16, 4, 210, 22, 46, 0, 0, 0, 1, 0, 0, 0, 2])
        .await?;
    cancellation.shutdown().await?;
    cancellation.read_to_end(&mut Vec::new()).await?;
    wait_for_connections(&backend_connections, 3).await?;
    assert_eq!(client_metrics.snapshot().cancellation_streams, 1);

    local_shutdown_tx.send(true).ok();
    local_client_task.await??;
    worker_shutdown_tx.send(true).ok();
    worker_task.await??;
    worker.close().await;

    let restarted_worker =
        relay_endpoint(worker_secret, vec![EDGE_ALPN.to_vec()], relay_map.clone()).await?;
    assert_eq!(restarted_worker.id(), worker.id());
    let (restarted_worker_shutdown_tx, restarted_worker_shutdown) = watch::channel(false);
    let restarted_worker_task = tokio::spawn(serve_worker(
        restarted_worker.clone(),
        WorkerAuthority::new(bootstrap_secret.public(), backend_address, 4)
            .with_compression(CompressionPolicy::Auto, TransportMetrics::default()),
        4,
        restarted_worker_shutdown,
    ));
    let reconnect_listener = TcpListener::bind("127.0.0.1:0").await?;
    let reconnect_address = reconnect_listener.local_addr()?;
    let (reconnect_shutdown_tx, reconnect_shutdown) = watch::channel(false);
    let reconnect_task = tokio::spawn(serve_local_client(
        reconnect_listener,
        reconnect_connector,
        4,
        reconnect_shutdown,
    ));
    exercise_local_bridge(reconnect_address, 4096).await?;
    reconnect_shutdown_tx.send(true).ok();
    reconnect_task.await??;

    bootstrap_shutdown_tx.send(true).ok();
    bootstrap_task.await??;
    bootstrap.close().await;

    let rotated_credential = SecretKey::from_bytes(&[55; 32]);
    let restarted_bootstrap = relay_endpoint(
        bootstrap_secret.clone(),
        vec![CONTROL_ALPN.to_vec()],
        relay_map.clone(),
    )
    .await?;
    let rotated_authority = BootstrapAuthority::new(
        bootstrap_secret.clone(),
        rotated_credential.public(),
        "reader",
        restarted_worker.addr(),
        2,
        60,
    )?;
    let (rotated_bootstrap_shutdown_tx, rotated_bootstrap_shutdown) = watch::channel(false);
    let rotated_bootstrap_task = tokio::spawn(serve_bootstrap(
        restarted_bootstrap.clone(),
        rotated_authority,
        4,
        rotated_bootstrap_shutdown,
    ));
    let old_credential = ClientConnector::new(
        client.clone(),
        credential_secret,
        restarted_bootstrap.addr(),
    );
    assert!(
        tokio::time::timeout(Duration::from_secs(2), old_credential.connect())
            .await
            .context("old credential lease attempt timed out")?
            .is_err()
    );

    let rotated_client =
        relay_endpoint(SecretKey::from_bytes(&[56; 32]), vec![], relay_map).await?;
    let rotated_connector = ClientConnector::new(
        rotated_client.clone(),
        rotated_credential,
        restarted_bootstrap.addr(),
    );
    let rotated_listener = TcpListener::bind("127.0.0.1:0").await?;
    let rotated_address = rotated_listener.local_addr()?;
    let (rotated_client_shutdown_tx, rotated_client_shutdown) = watch::channel(false);
    let rotated_client_task = tokio::spawn(serve_local_client(
        rotated_listener,
        rotated_connector,
        4,
        rotated_client_shutdown,
    ));
    exercise_local_bridge(rotated_address, 4096).await?;

    rotated_client_shutdown_tx.send(true).ok();
    restarted_worker_shutdown_tx.send(true).ok();
    rotated_bootstrap_shutdown_tx.send(true).ok();
    rotated_client_task.await??;
    restarted_worker_task.await??;
    rotated_bootstrap_task.await??;
    rotated_client.close().await;
    client.close().await;
    restarted_bootstrap.close().await;
    restarted_worker.close().await;
    backend_task.abort();
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires outbound access to the public iroh relay preset"]
async fn omitted_configuration_uses_public_relay_for_reconnect() -> Result<()> {
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

    let bootstrap_secret = SecretKey::from_bytes(&[71; 32]);
    let worker_secret = SecretKey::from_bytes(&[72; 32]);
    let credential_secret = SecretKey::from_bytes(&[73; 32]);
    let worker = public_relay_endpoint(worker_secret, vec![EDGE_ALPN.to_vec()]).await?;
    let bootstrap =
        public_relay_endpoint(bootstrap_secret.clone(), vec![CONTROL_ALPN.to_vec()]).await?;
    let client = public_relay_endpoint(SecretKey::from_bytes(&[74; 32]), vec![]).await?;
    for endpoint in [&worker, &bootstrap, &client] {
        assert!(endpoint.addr().ip_addrs().next().is_none());
        assert!(endpoint.addr().relay_urls().next().is_some());
    }

    let authority = BootstrapAuthority::new(
        bootstrap_secret.clone(),
        credential_secret.public(),
        "reader",
        worker.addr(),
        1,
        60,
    )?;
    let (bootstrap_shutdown_tx, bootstrap_shutdown) = watch::channel(false);
    let bootstrap_task = tokio::spawn(serve_bootstrap(
        bootstrap.clone(),
        authority,
        4,
        bootstrap_shutdown,
    ));
    let (worker_shutdown_tx, worker_shutdown) = watch::channel(false);
    let worker_task = tokio::spawn(serve_worker(
        worker.clone(),
        WorkerAuthority::new(bootstrap_secret.public(), backend_address, 4),
        4,
        worker_shutdown,
    ));
    let connector = ClientConnector::new(client.clone(), credential_secret, bootstrap.addr());
    let local_listener = TcpListener::bind("127.0.0.1:0").await?;
    let local_address = local_listener.local_addr()?;
    let (local_shutdown_tx, local_shutdown) = watch::channel(false);
    let local_client_task = tokio::spawn(serve_local_client(
        local_listener,
        connector,
        4,
        local_shutdown,
    ));

    exercise_local_bridge(local_address, 4096).await?;
    exercise_local_bridge(local_address, 4096).await?;
    assert_eq!(backend_connections.load(Ordering::SeqCst), 2);
    let remote = client
        .remote_info(worker.id())
        .await
        .context("public client has no worker route information")?;
    assert!(remote.addrs().any(|address| address.addr().is_relay()));
    assert!(remote.addrs().all(|address| !address.addr().is_ip()));

    local_shutdown_tx.send(true).ok();
    worker_shutdown_tx.send(true).ok();
    bootstrap_shutdown_tx.send(true).ok();
    local_client_task.await??;
    worker_task.await??;
    bootstrap_task.await??;
    client.close().await;
    bootstrap.close().await;
    worker.close().await;
    backend_task.abort();
    Ok(())
}

async fn relay_endpoint(
    secret: SecretKey,
    alpns: Vec<Vec<u8>>,
    relay_map: RelayMap,
) -> Result<Endpoint> {
    let endpoint = Endpoint::builder(presets::Minimal)
        .secret_key(secret)
        .alpns(alpns)
        .relay_mode(iroh::RelayMode::Custom(relay_map))
        .ca_tls_config(CaTlsConfig::insecure_skip_verify())
        .clear_address_lookup()
        .clear_ip_transports()
        .bind()
        .await
        .map_err(|error| anyhow!("relay-only endpoint bind failed: {error}"))?;
    tokio::time::timeout(Duration::from_secs(10), endpoint.online())
        .await
        .context("relay-only endpoint did not become online")?;
    Ok(endpoint)
}

async fn public_relay_endpoint(secret: SecretKey, alpns: Vec<Vec<u8>>) -> Result<Endpoint> {
    let endpoint = Endpoint::builder(presets::N0)
        .secret_key(secret)
        .alpns(alpns)
        .relay_mode(RelayPolicy::from_config(None)?.iroh_mode())
        .clear_ip_transports()
        .bind()
        .await
        .map_err(|error| anyhow!("public-relay endpoint bind failed: {error}"))?;
    tokio::time::timeout(Duration::from_secs(45), endpoint.online())
        .await
        .context("public-relay endpoint did not become online")?;
    Ok(endpoint)
}

async fn exercise_local_bridge(address: std::net::SocketAddr, payload_bytes: usize) -> Result<()> {
    let mut local = TcpStream::connect(address).await?;
    local.write_all(&startup_packet("reader")).await?;
    let mut authentication_ok = [0; 9];
    local.read_exact(&mut authentication_ok).await?;
    assert_eq!(authentication_ok, [b'R', 0, 0, 0, 8, 0, 0, 0, 0]);
    let payload = vec![b'x'; payload_bytes];
    local.write_all(&payload).await?;
    let mut echoed = vec![0; payload.len()];
    local.read_exact(&mut echoed).await?;
    assert_eq!(echoed, payload);
    local.shutdown().await?;
    Ok(())
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
    let mut buffer = [0; 64 * 1024];
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
    tokio::time::timeout(Duration::from_secs(2), async {
        while counter.load(Ordering::SeqCst) < expected {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .context("backend connection count did not advance")
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
