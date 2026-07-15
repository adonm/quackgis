// SPDX-License-Identifier: Apache-2.0

use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use iroh::{Endpoint, RelayMap, SecretKey, endpoint::presets};
use iroh_relay::tls::CaTlsConfig;
use quackgis_edge::compression::TransportMetrics;
use quackgis_edge::runtime::{
    BootstrapAuthority, ClientConnector, WorkerAuthority, serve_bootstrap, serve_local_client,
    serve_worker,
};
use quackgis_edge::{CONTROL_ALPN, CompressionPolicy, EDGE_ALPN};
use quackgis_server::pgwire_server::ServerOptions;
use tokio::net::TcpListener;
use tokio::sync::watch;

#[path = "support/runtime.rs"]
#[allow(dead_code)]
mod runtime;
use runtime::TestRuntime;

#[derive(Debug, Eq, PartialEq)]
struct PgwireOracle {
    scalar_types: Vec<(String, u32)>,
    scalar_values: (bool, i64, u64, String, Option<i32>),
    compressible_result_bytes: usize,
    parameter: i64,
    portal_rows: Vec<i32>,
    unsupported_sqlstate: String,
    committed_rows: i64,
    committed_sum: i64,
    malformed_copy_sqlstate: String,
    cancellation_sqlstate: String,
    quarantine_sqlstate: String,
    reconnect_rows: i64,
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires the pinned DuckDB ADBC runtime"]
async fn duckdb_pgwire_oracles_pass_through_local_iroh() -> Result<()> {
    let runtime = TestRuntime::start(
        ServerOptions::new()
            .with_max_connections(12)
            .with_max_active_queries(4)
            .with_max_reader_queries(4)
            .with_max_blocking_workers(5)
            .with_statement_timeout(Duration::from_secs(30)),
    )
    .await;
    runtime.storage().readiness_probe()?;
    let backend_address = format!("127.0.0.1:{}", runtime.port()).parse()?;

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
    let worker_metrics = TransportMetrics::default();
    let worker_task = tokio::spawn(serve_worker(
        worker.clone(),
        WorkerAuthority::new(bootstrap_secret.public(), backend_address, 8)
            .with_compression(CompressionPolicy::Auto, worker_metrics.clone()),
        4,
        worker_shutdown,
    ));
    let client_metrics = TransportMetrics::default();
    let connector = ClientConnector::new(tiny_client.clone(), credential_secret, bootstrap.addr())
        .with_compression(CompressionPolicy::Auto, client_metrics.clone());
    let local_listener = TcpListener::bind("127.0.0.1:0").await?;
    let local_address = local_listener.local_addr()?;
    let (client_shutdown_tx, client_shutdown) = watch::channel(false);
    let client_task = tokio::spawn(serve_local_client(
        local_listener,
        connector,
        8,
        client_shutdown,
    ));

    let direct_address = format!("127.0.0.1:{}", runtime.port()).parse()?;
    let direct = run_pgwire_oracle(direct_address, "iroh_direct_oracle").await?;
    let tunneled = run_pgwire_oracle(local_address, "iroh_tunnel_oracle").await?;
    assert_eq!(tunneled, direct);

    let first = query_scalar(local_address, 20);
    let second = query_scalar(local_address, 40);
    assert_eq!(tokio::try_join!(first, second)?, (21, 41));
    let client_compression = client_metrics.snapshot();
    let worker_compression = worker_metrics.snapshot();
    assert!(client_compression.upstream.compressed_blocks > 0);
    assert!(worker_compression.downstream.compressed_blocks > 0);
    assert_eq!(client_compression.downstream.decode_failures, 0);
    assert_eq!(worker_compression.upstream.decode_failures, 0);

    client_shutdown_tx.send(true).ok();
    bootstrap_shutdown_tx.send(true).ok();
    worker_shutdown_tx.send(true).ok();
    tiny_client.close().await;
    bootstrap.close().await;
    worker.close().await;
    client_task.await??;
    bootstrap_task.await??;
    worker_task.await??;
    println!(
        "duckdb_iroh_direct_smoke_ok parity=true rows={} sum={} cancellation={}",
        tunneled.committed_rows, tunneled.committed_sum, tunneled.cancellation_sqlstate
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires the pinned DuckDB ADBC runtime"]
async fn duckdb_pgwire_oracles_pass_through_forced_custom_relay() -> Result<()> {
    let runtime = TestRuntime::start(
        ServerOptions::new()
            .with_max_connections(12)
            .with_max_active_queries(4)
            .with_max_reader_queries(4)
            .with_max_blocking_workers(5)
            .with_statement_timeout(Duration::from_secs(30)),
    )
    .await;
    runtime.storage().readiness_probe()?;
    let backend_address = format!("127.0.0.1:{}", runtime.port()).parse()?;
    let (relay_map, relay_url, _relay) = iroh::test_utils::run_relay_server().await?;

    let bootstrap_secret = SecretKey::from_bytes(&[61; 32]);
    let worker_secret = SecretKey::from_bytes(&[62; 32]);
    let credential_secret = SecretKey::from_bytes(&[63; 32]);
    let client_transport_secret = SecretKey::from_bytes(&[64; 32]);
    let worker = relay_endpoint(worker_secret, vec![EDGE_ALPN.to_vec()], relay_map.clone()).await?;
    let bootstrap = relay_endpoint(
        bootstrap_secret.clone(),
        vec![CONTROL_ALPN.to_vec()],
        relay_map.clone(),
    )
    .await?;
    let tiny_client = relay_endpoint(client_transport_secret, vec![], relay_map).await?;
    for endpoint in [&worker, &bootstrap, &tiny_client] {
        assert!(endpoint.addr().ip_addrs().next().is_none());
        assert_eq!(endpoint.addr().relay_urls().next(), Some(&relay_url));
    }

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
    let worker_metrics = TransportMetrics::default();
    let (worker_shutdown_tx, worker_shutdown) = watch::channel(false);
    let worker_task = tokio::spawn(serve_worker(
        worker.clone(),
        WorkerAuthority::new(bootstrap_secret.public(), backend_address, 8)
            .with_compression(CompressionPolicy::Auto, worker_metrics.clone()),
        4,
        worker_shutdown,
    ));
    let client_metrics = TransportMetrics::default();
    let connector = ClientConnector::new(tiny_client.clone(), credential_secret, bootstrap.addr())
        .with_compression(CompressionPolicy::Auto, client_metrics.clone());
    let local_listener = TcpListener::bind("127.0.0.1:0").await?;
    let local_address = local_listener.local_addr()?;
    let (client_shutdown_tx, client_shutdown) = watch::channel(false);
    let client_task = tokio::spawn(serve_local_client(
        local_listener,
        connector,
        8,
        client_shutdown,
    ));

    let direct = run_pgwire_oracle(backend_address, "relay_direct_oracle").await?;
    let relayed = run_pgwire_oracle(local_address, "relay_tunnel_oracle").await?;
    assert_eq!(relayed, direct);
    assert!(client_metrics.snapshot().upstream.compressed_blocks > 0);
    assert!(worker_metrics.snapshot().downstream.compressed_blocks > 0);
    let remote = tiny_client
        .remote_info(worker.id())
        .await
        .context("tiny client has no worker route information")?;
    assert!(remote.addrs().all(|address| !address.addr().is_ip()));

    client_shutdown_tx.send(true).ok();
    bootstrap_shutdown_tx.send(true).ok();
    worker_shutdown_tx.send(true).ok();
    tiny_client.close().await;
    bootstrap.close().await;
    worker.close().await;
    client_task.await??;
    bootstrap_task.await??;
    worker_task.await??;
    println!(
        "duckdb_iroh_custom_relay_smoke_ok parity=true rows={} sum={} cancellation={}",
        relayed.committed_rows, relayed.committed_sum, relayed.cancellation_sqlstate
    );
    Ok(())
}

async fn run_pgwire_oracle(address: std::net::SocketAddr, table: &str) -> Result<PgwireOracle> {
    let (mut client, connection) = connect(address).await?;
    let connection_task = tokio::spawn(connection);

    let scalar = client
        .prepare(
            "SELECT true::BOOLEAN AS enabled, 7::BIGINT AS big_id, \
             1.5::DOUBLE AS ratio, 'edge'::VARCHAR AS label, \
             NULL::INTEGER AS optional_id",
        )
        .await?;
    let scalar_types = scalar
        .columns()
        .iter()
        .map(|column| (column.type_().name().to_owned(), column.type_().oid()))
        .collect();
    let row = client.query_one(&scalar, &[]).await?;
    let scalar_values = (
        row.get::<_, bool>(0),
        row.get::<_, i64>(1),
        row.get::<_, f64>(2).to_bits(),
        row.get::<_, String>(3),
        row.get::<_, Option<i32>>(4),
    );
    let compressible = client
        .query_one("SELECT repeat('x', 16384)::VARCHAR", &[])
        .await?
        .get::<_, String>(0);
    assert!(compressible.bytes().all(|byte| byte == b'x'));
    let compressible_result_bytes = compressible.len();

    let typed = client
        .prepare_typed(
            "SELECT $1::BIGINT + 1",
            &[tokio_postgres::types::Type::INT8],
        )
        .await?;
    let parameter = client.query_one(&typed, &[&41_i64]).await?.get::<_, i64>(0);
    assert_eq!(
        client
            .query_one("SELECT ST_AsText(ST_GeomFromText('POINT(1 2)'))", &[])
            .await?
            .get::<_, String>(0),
        "POINT (1 2)"
    );

    let unsupported = client
        .query_one("SELECT ST_GeometryN(ST_Point(1, 2), 1)", &[])
        .await
        .expect_err("unsupported spatial function must fail closed");
    let unsupported_sqlstate = sqlstate(&unsupported)?;
    assert_eq!(unsupported_sqlstate, "0A000");

    let transaction = client.transaction().await?;
    let portal_statement = transaction
        .prepare("SELECT i::INTEGER FROM range(3) AS portal_rows(i) ORDER BY i")
        .await?;
    let portal = transaction.bind(&portal_statement, &[]).await?;
    let mut portal_rows = Vec::new();
    loop {
        let page = transaction.query_portal(&portal, 1).await?;
        if page.is_empty() {
            break;
        }
        portal_rows.push(page[0].get::<_, i32>(0));
    }
    transaction.commit().await?;

    client
        .batch_execute(&format!(
            "CREATE TABLE quackgis.main.{table}(id BIGINT, name VARCHAR)"
        ))
        .await?;
    let copy = client
        .copy_in(&format!("COPY public.{table} (id, name) FROM STDIN"))
        .await?;
    let mut copy = std::pin::pin!(copy);
    copy.send(Bytes::from(format!(
        "1\t{}\n2\ttwo\n",
        "compressible".repeat(1024)
    )))
    .await?;
    assert_eq!(copy.finish().await?, 2);

    let transaction = client.transaction().await?;
    transaction
        .batch_execute(&format!(
            "INSERT INTO public.{table} VALUES (3, 'rolled back')"
        ))
        .await?;
    transaction.rollback().await?;

    let malformed = client
        .copy_in(&format!("COPY public.{table} (id, name) FROM STDIN"))
        .await?;
    let mut malformed = std::pin::pin!(malformed);
    let malformed = match malformed
        .send(Bytes::from_static(b"not-an-integer\tbad\n"))
        .await
    {
        Err(error) => error,
        Ok(()) => malformed
            .finish()
            .await
            .expect_err("malformed COPY must fail atomically"),
    };
    let malformed_copy_sqlstate = sqlstate(&malformed)?;

    let aggregate = client
        .query_one(
            &format!("SELECT count(*)::BIGINT, sum(id)::BIGINT FROM public.{table}"),
            &[],
        )
        .await?;
    let committed_rows = aggregate.get::<_, i64>(0);
    let committed_sum = aggregate.get::<_, i64>(1);

    client.batch_execute("BEGIN").await?;
    client
        .batch_execute(&format!(
            "INSERT INTO public.{table} VALUES (4, 'disconnected')"
        ))
        .await?;
    drop(client);
    connection_task.abort();

    let (client, connection) = connect(address).await?;
    let connection_task = tokio::spawn(connection);
    assert_eq!(
        client
            .query_one(&format!("SELECT count(*)::BIGINT FROM public.{table}"), &[],)
            .await?
            .get::<_, i64>(0),
        committed_rows
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
    let cancellation_sqlstate = sqlstate(&cancellation)?;
    let quarantine = client
        .query_one("SELECT 1::INTEGER", &[])
        .await
        .expect_err("cancelled native session must remain quarantined");
    let quarantine_sqlstate = sqlstate(&quarantine)?;
    drop(client);
    connection_task.abort();

    let (fresh, fresh_connection) = connect(address).await?;
    let fresh_connection_task = tokio::spawn(fresh_connection);
    let reconnect_rows = fresh
        .query_one(&format!("SELECT count(*)::BIGINT FROM public.{table}"), &[])
        .await?
        .get::<_, i64>(0);
    drop(fresh);
    fresh_connection_task.abort();

    Ok(PgwireOracle {
        scalar_types,
        scalar_values,
        compressible_result_bytes,
        parameter,
        portal_rows,
        unsupported_sqlstate,
        committed_rows,
        committed_sum,
        malformed_copy_sqlstate,
        cancellation_sqlstate,
        quarantine_sqlstate,
        reconnect_rows,
    })
}

async fn query_scalar(address: std::net::SocketAddr, value: i64) -> Result<i64> {
    let (client, connection) = connect(address).await?;
    let connection_task = tokio::spawn(connection);
    let statement = client
        .prepare_typed(
            "SELECT $1::BIGINT + 1",
            &[tokio_postgres::types::Type::INT8],
        )
        .await?;
    let row = client
        .query_one(&statement, &[&value])
        .await?
        .get::<_, i64>(0);
    drop(client);
    connection_task.abort();
    Ok(row)
}

fn sqlstate(error: &tokio_postgres::Error) -> Result<String> {
    error
        .code()
        .map(|code| code.code().to_owned())
        .ok_or_else(|| anyhow!("pgwire error has no SQLSTATE: {error}"))
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
