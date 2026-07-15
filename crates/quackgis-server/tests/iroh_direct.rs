// SPDX-License-Identifier: Apache-2.0

use std::time::Duration;

use anyhow::{Result, anyhow};
use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use iroh::{Endpoint, SecretKey, endpoint::presets};
use quackgis_edge::runtime::{
    BootstrapAuthority, ClientConnector, WorkerAuthority, serve_bootstrap, serve_local_client,
    serve_worker,
};
use quackgis_edge::{CONTROL_ALPN, EDGE_ALPN};
use quackgis_server::pgwire_server::ServerOptions;
use tokio::net::TcpListener;
use tokio::sync::watch;

#[path = "support/runtime.rs"]
mod runtime;
use runtime::TestRuntime;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires the pinned DuckDB ADBC runtime"]
async fn duckdb_pgwire_oracles_pass_through_local_iroh() -> Result<()> {
    let runtime = TestRuntime::start(
        ServerOptions::new()
            .with_max_connections(8)
            .with_max_active_queries(4)
            .with_max_reader_queries(4)
            .with_max_blocking_workers(5)
            .with_statement_timeout(Duration::from_secs(30)),
    )
    .await;
    runtime.storage().readiness_probe()?;
    let (direct, direct_connection) = runtime.connect().await;
    let direct_connection_task = tokio::spawn(direct_connection);
    assert_eq!(
        direct
            .query_one("SELECT 41::BIGINT + 1", &[])
            .await?
            .get::<_, i64>(0),
        42
    );
    drop(direct);
    direct_connection_task.abort();

    let bootstrap_secret = SecretKey::from_bytes(&[41; 32]);
    let worker_secret = SecretKey::from_bytes(&[42; 32]);
    let credential_secret = SecretKey::from_bytes(&[43; 32]);
    let client_transport_secret = SecretKey::from_bytes(&[44; 32]);
    let worker = direct_endpoint(worker_secret, vec![EDGE_ALPN.to_vec()]).await?;
    let bootstrap = direct_endpoint(bootstrap_secret.clone(), vec![CONTROL_ALPN.to_vec()]).await?;
    let tiny_client = direct_endpoint(client_transport_secret, vec![]).await?;

    let authority = BootstrapAuthority::new(
        bootstrap_secret.clone(),
        credential_secret.public(),
        "postgres",
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
        WorkerAuthority {
            bootstrap_public_key: bootstrap_secret.public(),
            backend: format!("127.0.0.1:{}", runtime.port()).parse()?,
            max_streams_per_connection: 8,
        },
        4,
        worker_shutdown,
    ));
    let connector = ClientConnector::new(tiny_client.clone(), credential_secret, bootstrap.addr());
    let local_listener = TcpListener::bind("127.0.0.1:0").await?;
    let local_address = local_listener.local_addr()?;
    let (client_shutdown_tx, client_shutdown) = watch::channel(false);
    let client_task = tokio::spawn(serve_local_client(
        local_listener,
        connector,
        8,
        client_shutdown,
    ));

    let (mut client, connection) = connect(local_address).await?;
    let connection_task = tokio::spawn(connection);
    let typed = client
        .prepare_typed(
            "SELECT $1::BIGINT + 1",
            &[tokio_postgres::types::Type::INT8],
        )
        .await?;
    assert_eq!(
        client.query_one(&typed, &[&41_i64]).await?.get::<_, i64>(0),
        42
    );
    assert_eq!(
        client
            .query_one("SELECT ST_AsText(ST_GeomFromText('POINT(1 2)'))", &[],)
            .await?
            .get::<_, String>(0),
        "POINT (1 2)"
    );
    client
        .batch_execute("CREATE TABLE quackgis.main.iroh_copy(id BIGINT, name VARCHAR)")
        .await?;
    let copy = client
        .copy_in("COPY public.iroh_copy (id, name) FROM STDIN")
        .await?;
    let mut copy = std::pin::pin!(copy);
    copy.send(Bytes::from_static(b"1\tone\n2\ttwo\n")).await?;
    assert_eq!(copy.finish().await?, 2);
    let aggregate = client
        .query_one(
            "SELECT count(*)::BIGINT, sum(id)::BIGINT FROM public.iroh_copy",
            &[],
        )
        .await?;
    assert_eq!(aggregate.get::<_, i64>(0), 2);
    assert_eq!(aggregate.get::<_, i64>(1), 3);

    let transaction = client.transaction().await?;
    transaction
        .batch_execute("INSERT INTO public.iroh_copy VALUES (3, 'rolled back')")
        .await?;
    transaction.rollback().await?;
    assert_eq!(
        client
            .query_one("SELECT count(*)::BIGINT FROM public.iroh_copy", &[])
            .await?
            .get::<_, i64>(0),
        2
    );

    let cancel = client.cancel_token();
    let rows = client
        .query_raw(
            "SELECT i::BIGINT FROM range(1000000000) AS cancel_rows(i)",
            std::iter::empty::<&i32>(),
        )
        .await?;
    futures::pin_mut!(rows);
    rows.next()
        .await
        .ok_or_else(|| anyhow!("long query returned no first row"))??;
    cancel.cancel_query(tokio_postgres::NoTls).await?;
    let cancellation = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match rows.next().await {
                Some(Ok(_)) => continue,
                Some(Err(error)) => return Ok::<_, anyhow::Error>(error),
                None => return Err(anyhow!("long query completed instead of cancelling")),
            }
        }
    })
    .await??;
    assert_eq!(
        cancellation.code(),
        Some(&tokio_postgres::error::SqlState::QUERY_CANCELED)
    );
    let quarantine = client
        .query_one("SELECT 1::INTEGER", &[])
        .await
        .expect_err("cancelled native session must remain quarantined");
    assert_eq!(
        quarantine.code(),
        Some(&tokio_postgres::error::SqlState::INTERNAL_ERROR)
    );
    drop(client);
    connection_task.abort();

    let (fresh, fresh_connection) = connect(local_address).await?;
    let fresh_connection_task = tokio::spawn(fresh_connection);
    assert_eq!(
        fresh
            .query_one("SELECT count(*)::BIGINT FROM public.iroh_copy", &[])
            .await?
            .get::<_, i64>(0),
        2
    );
    drop(fresh);
    fresh_connection_task.abort();

    client_shutdown_tx.send(true).ok();
    bootstrap_shutdown_tx.send(true).ok();
    worker_shutdown_tx.send(true).ok();
    tiny_client.close().await;
    bootstrap.close().await;
    worker.close().await;
    client_task.await??;
    bootstrap_task.await??;
    worker_task.await??;
    println!("duckdb_iroh_direct_smoke_ok rows=2 sum=3 cancellation=57014");
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

async fn connect(
    address: std::net::SocketAddr,
) -> Result<(
    tokio_postgres::Client,
    tokio_postgres::Connection<tokio_postgres::Socket, tokio_postgres::tls::NoTlsStream>,
)> {
    tokio_postgres::connect(
        &format!(
            "host={} port={} user=postgres dbname=quackgis",
            address.ip(),
            address.port()
        ),
        tokio_postgres::NoTls,
    )
    .await
    .map_err(Into::into)
}
