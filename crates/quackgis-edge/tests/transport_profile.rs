// SPDX-License-Identifier: Apache-2.0

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use iroh::{Endpoint, RelayMap, SecretKey, endpoint::presets};
use iroh_relay::tls::CaTlsConfig;
use quackgis_edge::compression::{DirectionMetricsSnapshot, TransportMetrics};
use quackgis_edge::runtime::{
    BootstrapAuthority, ClientConnector, WorkerAuthority, serve_bootstrap, serve_local_client,
    serve_worker,
};
use quackgis_edge::{CONTROL_ALPN, CompressionPolicy, EDGE_ALPN};
use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio::task::JoinHandle;

const SMOKE_PAYLOAD_BYTES: usize = 8 * 1024 * 1024;
const LOCAL_PAYLOAD_BYTES: usize = 32 * 1024 * 1024;
const REFERENCE_PAYLOAD_BYTES: usize = 64 * 1024 * 1024;
const MAX_PROFILE_PAYLOAD_BYTES: usize = 256 * 1024 * 1024;
const MAX_CONNECTION_MILLIS: f64 = 5_000.0;
const MAX_FIRST_BYTE_MILLIS: f64 = 2_000.0;
const MAX_CANCELLATION_MILLIS: f64 = 1_000.0;
const MAX_TRANSPORT_RSS_BYTES: u64 = 64 * 1024 * 1024;
const MIN_DIRECT_IROH_TCP_THROUGHPUT_RATIO: f64 = 0.05;
const MIN_RELAY_TCP_THROUGHPUT_RATIO: f64 = 0.02;
const MIN_INCOMPRESSIBLE_AUTO_RAW_THROUGHPUT_RATIO: f64 = 0.50;
const MIN_COMPRESSIBLE_SAVINGS_PERCENT: f64 = 50.0;

#[derive(Clone, Copy)]
enum ProfilePath {
    Direct,
    Relay,
}

#[derive(Serialize)]
struct ProfileEvidence {
    schema: &'static str,
    status: &'static str,
    source_sha: String,
    source_dirty: bool,
    runtime: RuntimeEvidence,
    host: HostEvidence,
    profile: String,
    payload_bytes: usize,
    modes: Vec<ModeEvidence>,
    budgets: Budgets,
}

#[derive(Serialize)]
struct RuntimeEvidence {
    quackgis_version: &'static str,
    iroh_version: &'static str,
    lz4_flex_version: &'static str,
    build_profile: &'static str,
}

#[derive(Serialize)]
struct HostEvidence {
    os: String,
    architecture: &'static str,
    cpu_model: String,
    logical_cpus: usize,
    memory_bytes: Option<u64>,
    cgroup_memory_max_bytes: Option<u64>,
    cgroup_cpu_max: Option<String>,
}

#[derive(Serialize)]
struct Budgets {
    max_connection_millis: f64,
    max_first_byte_millis: f64,
    max_cancellation_millis: f64,
    max_transport_rss_bytes: u64,
    min_direct_iroh_tcp_throughput_ratio: f64,
    min_relay_tcp_throughput_ratio: f64,
    min_incompressible_auto_raw_throughput_ratio: f64,
    min_compressible_savings_percent: f64,
}

#[derive(Serialize)]
struct ModeEvidence {
    name: &'static str,
    path: &'static str,
    compression: &'static str,
    setup_millis: f64,
    wall_millis: f64,
    cpu_millis: f64,
    idle_rss_bytes: u64,
    peak_rss_bytes: u64,
    rss_delta_bytes: u64,
    cancellation_millis: f64,
    concurrent_streams: usize,
    shapes: Vec<ShapeEvidence>,
}

#[derive(Serialize)]
struct ShapeEvidence {
    name: &'static str,
    payload_bytes: usize,
    connection_millis: Vec<f64>,
    first_byte_millis: Vec<f64>,
    transfer_millis: Vec<f64>,
    throughput_mib_per_second: Vec<f64>,
    throughput_p50_mib_per_second: f64,
    codec: Option<CodecDelta>,
}

#[derive(Clone, Debug, Serialize)]
struct CodecDelta {
    uncompressed_bytes: u64,
    wire_bytes: u64,
    bytes_saved: i64,
    blocks: u64,
    compressed_blocks: u64,
    raw_small_blocks: u64,
    raw_incompressible_blocks: u64,
    raw_backoff_blocks: u64,
    compression_cpu_nanos: u64,
    decompression_cpu_nanos: u64,
    decode_failures: u64,
}

struct Payload {
    name: &'static str,
    bytes: Arc<Vec<u8>>,
}

struct Tunnel {
    address: std::net::SocketAddr,
    setup_millis: f64,
    client_metrics: TransportMetrics,
    worker_metrics: TransportMetrics,
    local_shutdown: watch::Sender<bool>,
    worker_shutdown: watch::Sender<bool>,
    bootstrap_shutdown: watch::Sender<bool>,
    local_task: JoinHandle<Result<()>>,
    worker_task: JoinHandle<Result<()>>,
    bootstrap_task: JoinHandle<Result<()>>,
    client: Endpoint,
    worker: Endpoint,
    bootstrap: Endpoint,
}

impl Tunnel {
    async fn start(
        backend: std::net::SocketAddr,
        path: ProfilePath,
        relay_map: Option<RelayMap>,
        compression: CompressionPolicy,
        seed: u8,
    ) -> Result<Self> {
        let started = Instant::now();
        let bootstrap_secret = SecretKey::from_bytes(&[seed; 32]);
        let worker_secret = SecretKey::from_bytes(&[seed.wrapping_add(1); 32]);
        let credential_secret = SecretKey::from_bytes(&[seed.wrapping_add(2); 32]);
        let client_secret = SecretKey::from_bytes(&[seed.wrapping_add(3); 32]);
        let worker = profile_endpoint(
            worker_secret,
            vec![EDGE_ALPN.to_vec()],
            path,
            relay_map.clone(),
        )
        .await?;
        let bootstrap = profile_endpoint(
            bootstrap_secret.clone(),
            vec![CONTROL_ALPN.to_vec()],
            path,
            relay_map.clone(),
        )
        .await?;
        let client = profile_endpoint(client_secret, vec![], path, relay_map).await?;
        let authority = BootstrapAuthority::new(
            bootstrap_secret.clone(),
            credential_secret.public(),
            "reader",
            worker.addr(),
            1,
            60,
        )?;
        let (bootstrap_shutdown, bootstrap_rx) = watch::channel(false);
        let bootstrap_task = tokio::spawn(serve_bootstrap(
            bootstrap.clone(),
            authority,
            8,
            bootstrap_rx,
        ));
        let worker_metrics = TransportMetrics::default();
        let (worker_shutdown, worker_rx) = watch::channel(false);
        let worker_task = tokio::spawn(serve_worker(
            worker.clone(),
            WorkerAuthority::new(bootstrap_secret.public(), backend, 16)
                .with_compression(compression, worker_metrics.clone()),
            8,
            worker_rx,
        ));
        let client_metrics = TransportMetrics::default();
        let connector = ClientConnector::new(client.clone(), credential_secret, bootstrap.addr())
            .with_compression(compression, client_metrics.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let (local_shutdown, local_rx) = watch::channel(false);
        let local_task = tokio::spawn(serve_local_client(listener, connector, 16, local_rx));
        Ok(Self {
            address,
            setup_millis: millis(started.elapsed()),
            client_metrics,
            worker_metrics,
            local_shutdown,
            worker_shutdown,
            bootstrap_shutdown,
            local_task,
            worker_task,
            bootstrap_task,
            client,
            worker,
            bootstrap,
        })
    }

    fn codec_snapshot(&self) -> CodecSnapshot {
        let client = self.client_metrics.snapshot();
        let worker = self.worker_metrics.snapshot();
        CodecSnapshot {
            upstream: client.upstream,
            downstream: worker.downstream,
            upstream_decoder: worker.upstream,
            downstream_decoder: client.downstream,
        }
    }

    async fn shutdown(self) -> Result<()> {
        self.local_shutdown.send(true).ok();
        self.worker_shutdown.send(true).ok();
        self.bootstrap_shutdown.send(true).ok();
        self.client.close().await;
        self.worker.close().await;
        self.bootstrap.close().await;
        self.local_task.await??;
        self.worker_task.await??;
        self.bootstrap_task.await??;
        Ok(())
    }
}

struct CodecSnapshot {
    upstream: DirectionMetricsSnapshot,
    downstream: DirectionMetricsSnapshot,
    upstream_decoder: DirectionMetricsSnapshot,
    downstream_decoder: DirectionMetricsSnapshot,
}

impl CodecSnapshot {
    fn delta(&self, before: &Self) -> CodecDelta {
        let uncompressed_bytes = difference(
            self.upstream.uncompressed_bytes,
            before.upstream.uncompressed_bytes,
        ) + difference(
            self.downstream.uncompressed_bytes,
            before.downstream.uncompressed_bytes,
        );
        let wire_bytes = difference(self.upstream.wire_bytes, before.upstream.wire_bytes)
            + difference(self.downstream.wire_bytes, before.downstream.wire_bytes);
        CodecDelta {
            uncompressed_bytes,
            wire_bytes,
            bytes_saved: i64::try_from(uncompressed_bytes).unwrap_or(i64::MAX)
                - i64::try_from(wire_bytes).unwrap_or(i64::MAX),
            blocks: difference(self.upstream.blocks, before.upstream.blocks)
                + difference(self.downstream.blocks, before.downstream.blocks),
            compressed_blocks: difference(
                self.upstream.compressed_blocks,
                before.upstream.compressed_blocks,
            ) + difference(
                self.downstream.compressed_blocks,
                before.downstream.compressed_blocks,
            ),
            raw_small_blocks: difference(
                self.upstream.raw_small_blocks,
                before.upstream.raw_small_blocks,
            ) + difference(
                self.downstream.raw_small_blocks,
                before.downstream.raw_small_blocks,
            ),
            raw_incompressible_blocks: difference(
                self.upstream.raw_incompressible_blocks,
                before.upstream.raw_incompressible_blocks,
            ) + difference(
                self.downstream.raw_incompressible_blocks,
                before.downstream.raw_incompressible_blocks,
            ),
            raw_backoff_blocks: difference(
                self.upstream.raw_backoff_blocks,
                before.upstream.raw_backoff_blocks,
            ) + difference(
                self.downstream.raw_backoff_blocks,
                before.downstream.raw_backoff_blocks,
            ),
            compression_cpu_nanos: difference(
                self.upstream.compression_cpu_nanos,
                before.upstream.compression_cpu_nanos,
            ) + difference(
                self.downstream.compression_cpu_nanos,
                before.downstream.compression_cpu_nanos,
            ),
            decompression_cpu_nanos: difference(
                self.upstream_decoder.decompression_cpu_nanos,
                before.upstream_decoder.decompression_cpu_nanos,
            ) + difference(
                self.downstream_decoder.decompression_cpu_nanos,
                before.downstream_decoder.decompression_cpu_nanos,
            ),
            decode_failures: difference(
                self.upstream_decoder.decode_failures,
                before.upstream_decoder.decode_failures,
            ) + difference(
                self.downstream_decoder.decode_failures,
                before.downstream_decoder.decode_failures,
            ),
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "registered release-mode transport resource profile"]
async fn direct_and_relay_transport_profile() -> Result<()> {
    let (profile, payload_bytes) = profile_configuration()?;
    if profile == "reference" && source_state()?.1 {
        bail!("reference iroh evidence requires a clean source tree");
    }
    let payloads = profile_payloads(payload_bytes);
    let backend = TcpListener::bind("127.0.0.1:0").await?;
    let backend_address = backend.local_addr()?;
    let backend_task = tokio::spawn(async move {
        loop {
            let (socket, _) = backend.accept().await?;
            tokio::spawn(fake_trust_pgwire(socket));
        }
        #[allow(unreachable_code)]
        Ok::<(), anyhow::Error>(())
    });
    let (relay_map, _, _relay) = iroh::test_utils::run_relay_server().await?;

    let mut modes = Vec::new();
    modes.push(run_mode("tcp_raw", "tcp", "off", backend_address, None, &payloads).await?);

    for (name, path_name, path, policy, seed) in [
        (
            "iroh_direct_raw",
            "iroh_direct",
            ProfilePath::Direct,
            CompressionPolicy::Off,
            81,
        ),
        (
            "iroh_direct_auto",
            "iroh_direct",
            ProfilePath::Direct,
            CompressionPolicy::Auto,
            85,
        ),
        (
            "iroh_relay_raw",
            "iroh_forced_relay",
            ProfilePath::Relay,
            CompressionPolicy::Off,
            89,
        ),
        (
            "iroh_relay_auto",
            "iroh_forced_relay",
            ProfilePath::Relay,
            CompressionPolicy::Auto,
            93,
        ),
    ] {
        let tunnel = Tunnel::start(
            backend_address,
            path,
            matches!(path, ProfilePath::Relay).then(|| relay_map.clone()),
            policy,
            seed,
        )
        .await?;
        let setup_millis = tunnel.setup_millis;
        let evidence = run_mode(
            name,
            path_name,
            match policy {
                CompressionPolicy::Off => "off",
                CompressionPolicy::Auto => "auto_lz4",
            },
            tunnel.address,
            Some(&tunnel),
            &payloads,
        )
        .await?;
        assert_eq!(evidence.setup_millis, setup_millis);
        modes.push(evidence);
        tunnel.shutdown().await?;
    }
    backend_task.abort();

    let (source_sha, source_dirty) = source_state()?;
    if profile == "reference" && source_dirty {
        bail!("source tree changed during reference iroh evidence collection");
    }
    let evidence = ProfileEvidence {
        schema: "quackgis-iroh-transport-evidence-v1",
        status: "pass",
        source_sha,
        source_dirty,
        runtime: RuntimeEvidence {
            quackgis_version: env!("CARGO_PKG_VERSION"),
            iroh_version: "1.0.2",
            lz4_flex_version: "0.13.1",
            build_profile: "release",
        },
        host: host_evidence(),
        profile,
        payload_bytes,
        modes,
        budgets: Budgets {
            max_connection_millis: MAX_CONNECTION_MILLIS,
            max_first_byte_millis: MAX_FIRST_BYTE_MILLIS,
            max_cancellation_millis: MAX_CANCELLATION_MILLIS,
            max_transport_rss_bytes: MAX_TRANSPORT_RSS_BYTES,
            min_direct_iroh_tcp_throughput_ratio: MIN_DIRECT_IROH_TCP_THROUGHPUT_RATIO,
            min_relay_tcp_throughput_ratio: MIN_RELAY_TCP_THROUGHPUT_RATIO,
            min_incompressible_auto_raw_throughput_ratio:
                MIN_INCOMPRESSIBLE_AUTO_RAW_THROUGHPUT_RATIO,
            min_compressible_savings_percent: MIN_COMPRESSIBLE_SAVINGS_PERCENT,
        },
    };
    let output = std::env::var_os("QUACKGIS_IROH_PROFILE_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .join(".tmp/iroh-transport-profile/smoke.json")
        });
    enforce_budgets(&evidence.modes)?;
    write_evidence(&output, &evidence)?;
    println!("iroh_transport_profile_ok out={}", output.display());
    Ok(())
}

async fn run_mode(
    name: &'static str,
    path: &'static str,
    compression: &'static str,
    address: std::net::SocketAddr,
    tunnel: Option<&Tunnel>,
    payloads: &[Payload],
) -> Result<ModeEvidence> {
    let sampler = RssSampler::start()?;
    let cpu_start = process_cpu_nanos()?;
    let started = Instant::now();
    let mut shapes = Vec::new();
    for payload in payloads {
        let before = tunnel.map(Tunnel::codec_snapshot);
        let mut transfers = Vec::with_capacity(3);
        for _ in 0..3 {
            transfers.push(echo_round_trip(address, &payload.bytes).await?);
        }
        let codec = tunnel.map(|tunnel| {
            tunnel
                .codec_snapshot()
                .delta(before.as_ref().expect("snapshot exists"))
        });
        shapes.push(ShapeEvidence {
            name: payload.name,
            payload_bytes: payload.bytes.len(),
            connection_millis: transfers
                .iter()
                .map(|sample| sample.connection_millis)
                .collect(),
            first_byte_millis: transfers
                .iter()
                .map(|sample| sample.first_byte_millis)
                .collect(),
            transfer_millis: transfers
                .iter()
                .map(|sample| sample.transfer_millis)
                .collect(),
            throughput_mib_per_second: transfers
                .iter()
                .map(|sample| sample.throughput_mib_per_second)
                .collect(),
            throughput_p50_mib_per_second: median(
                &transfers
                    .iter()
                    .map(|sample| sample.throughput_mib_per_second)
                    .collect::<Vec<_>>(),
            ),
            codec,
        });
    }
    let cancellation_millis = cancellation_round_trip(address).await?;
    let concurrent_payload = Arc::new(vec![b'c'; 512 * 1024]);
    let (first, second) = tokio::try_join!(
        echo_round_trip(address, &concurrent_payload),
        echo_round_trip(address, &concurrent_payload)
    )?;
    assert_eq!(first.payload_bytes, concurrent_payload.len());
    assert_eq!(second.payload_bytes, concurrent_payload.len());
    let wall_millis = millis(started.elapsed());
    let cpu_millis = (process_cpu_nanos()?.saturating_sub(cpu_start)) as f64 / 1_000_000.0;
    let rss = sampler.finish().await?;
    Ok(ModeEvidence {
        name,
        path,
        compression,
        setup_millis: tunnel.map_or(0.0, |tunnel| tunnel.setup_millis),
        wall_millis,
        cpu_millis,
        idle_rss_bytes: rss.idle,
        peak_rss_bytes: rss.peak,
        rss_delta_bytes: rss.delta,
        cancellation_millis,
        concurrent_streams: 2,
        shapes,
    })
}

struct TransferMeasurement {
    payload_bytes: usize,
    connection_millis: f64,
    first_byte_millis: f64,
    transfer_millis: f64,
    throughput_mib_per_second: f64,
}

async fn echo_round_trip(
    address: std::net::SocketAddr,
    payload: &Arc<Vec<u8>>,
) -> Result<TransferMeasurement> {
    let connection_started = Instant::now();
    let mut socket = TcpStream::connect(address).await?;
    socket.write_all(&startup_packet("reader")).await?;
    let mut authentication_ok = [0; 9];
    socket.read_exact(&mut authentication_ok).await?;
    if authentication_ok != [b'R', 0, 0, 0, 8, 0, 0, 0, 0] {
        bail!("echo backend did not return AuthenticationOk");
    }
    let connection_millis = millis(connection_started.elapsed());
    let transfer_started = Instant::now();
    let (mut read, mut write) = socket.into_split();
    let expected = Arc::clone(payload);
    let writer = async move {
        write.write_all(&expected).await?;
        write.shutdown().await?;
        Ok::<_, anyhow::Error>(())
    };
    let reader = async {
        let mut offset = 0_usize;
        let mut first_byte_millis = None;
        let mut buffer = [0_u8; 64 * 1024];
        while offset < payload.len() {
            let count = read.read(&mut buffer).await?;
            if count == 0 {
                bail!(
                    "echo response ended after {offset} of {} bytes",
                    payload.len()
                );
            }
            if first_byte_millis.is_none() {
                first_byte_millis = Some(millis(transfer_started.elapsed()));
            }
            if buffer[..count] != payload[offset..offset + count] {
                bail!("echo response differs at offset {offset}");
            }
            offset += count;
        }
        Ok::<_, anyhow::Error>(first_byte_millis.unwrap_or_default())
    };
    let (_, first_byte_millis) = tokio::try_join!(writer, reader)?;
    let elapsed = transfer_started.elapsed();
    let transfer_millis = millis(elapsed);
    let total_bytes = payload.len().saturating_mul(2) as f64;
    let throughput_mib_per_second = total_bytes / (1024.0 * 1024.0) / elapsed.as_secs_f64();
    Ok(TransferMeasurement {
        payload_bytes: payload.len(),
        connection_millis,
        first_byte_millis,
        transfer_millis,
        throughput_mib_per_second,
    })
}

async fn cancellation_round_trip(address: std::net::SocketAddr) -> Result<f64> {
    let started = Instant::now();
    let mut socket = TcpStream::connect(address).await?;
    socket
        .write_all(&[0, 0, 0, 16, 4, 210, 22, 46, 0, 0, 0, 1, 0, 0, 0, 2])
        .await?;
    socket.shutdown().await?;
    let mut response = Vec::new();
    socket.read_to_end(&mut response).await?;
    if !response.is_empty() {
        bail!("cancellation returned unexpected application bytes");
    }
    Ok(millis(started.elapsed()))
}

async fn fake_trust_pgwire(mut socket: TcpStream) -> Result<()> {
    let mut length = [0; 4];
    socket.read_exact(&mut length).await?;
    let length = u32::from_be_bytes(length) as usize;
    if !(8..=16 * 1024 * 1024).contains(&length) {
        bail!("invalid fake-backend startup length");
    }
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

async fn profile_endpoint(
    secret: SecretKey,
    alpns: Vec<Vec<u8>>,
    path: ProfilePath,
    relay_map: Option<RelayMap>,
) -> Result<Endpoint> {
    let builder = Endpoint::builder(presets::Minimal)
        .secret_key(secret)
        .alpns(alpns)
        .clear_address_lookup()
        .clear_ip_transports();
    let endpoint = match path {
        ProfilePath::Direct => {
            builder
                .relay_mode(iroh::RelayMode::Disabled)
                .bind_addr("127.0.0.1:0")?
                .bind()
                .await
        }
        ProfilePath::Relay => {
            builder
                .relay_mode(iroh::RelayMode::Custom(
                    relay_map.context("relay profile needs a relay map")?,
                ))
                .ca_tls_config(CaTlsConfig::insecure_skip_verify())
                .bind()
                .await
        }
    }
    .map_err(|error| anyhow!("profile endpoint bind failed: {error}"))?;
    if matches!(path, ProfilePath::Relay) {
        tokio::time::timeout(Duration::from_secs(10), endpoint.online())
            .await
            .context("profile relay endpoint did not become online")?;
    }
    Ok(endpoint)
}

fn profile_payloads(payload_bytes: usize) -> Vec<Payload> {
    let small = vec![b's'; 128];
    let compressible = vec![b'x'; payload_bytes];
    let mut state = 0x243f_6a88_u32;
    let incompressible = (0..payload_bytes)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state as u8
        })
        .collect();
    let point_wkb = [
        1_u8, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xf0, 0x3f, 0, 0, 0, 0, 0, 0, 0x40,
    ];
    let wkb = point_wkb.into_iter().cycle().take(payload_bytes).collect();
    let copy_row = b"123456\tcompressible-copy-value-compressible-copy-value\n";
    let copy = copy_row
        .iter()
        .copied()
        .cycle()
        .take(payload_bytes)
        .collect();
    let result_row = b"D\0\0\0\x20\0\x02\0\0\0\x08\0\0\0\0\0\0\0\x01\0\0\0\x08resultxx";
    let result = result_row
        .iter()
        .copied()
        .cycle()
        .take(payload_bytes)
        .collect();
    vec![
        Payload {
            name: "small",
            bytes: Arc::new(small),
        },
        Payload {
            name: "compressible",
            bytes: Arc::new(compressible),
        },
        Payload {
            name: "incompressible",
            bytes: Arc::new(incompressible),
        },
        Payload {
            name: "wkb",
            bytes: Arc::new(wkb),
        },
        Payload {
            name: "copy",
            bytes: Arc::new(copy),
        },
        Payload {
            name: "result",
            bytes: Arc::new(result),
        },
    ]
}

fn profile_configuration() -> Result<(String, usize)> {
    let profile =
        std::env::var("QUACKGIS_IROH_PROFILE_LEVEL").unwrap_or_else(|_| "smoke".to_owned());
    let default_bytes = match profile.as_str() {
        "smoke" => SMOKE_PAYLOAD_BYTES,
        "local" => LOCAL_PAYLOAD_BYTES,
        "reference" => REFERENCE_PAYLOAD_BYTES,
        _ => bail!("unknown iroh evidence level {profile:?}"),
    };
    let payload_bytes = std::env::var("QUACKGIS_IROH_PROFILE_BYTES")
        .ok()
        .map(|value| value.parse::<usize>())
        .transpose()?
        .unwrap_or(default_bytes);
    if !(1024 * 1024..=MAX_PROFILE_PAYLOAD_BYTES).contains(&payload_bytes) {
        bail!(
            "iroh profile payload must be between 1 MiB and {} MiB",
            MAX_PROFILE_PAYLOAD_BYTES / (1024 * 1024)
        );
    }
    Ok((profile, payload_bytes))
}

fn enforce_budgets(modes: &[ModeEvidence]) -> Result<()> {
    for mode in modes {
        if mode.rss_delta_bytes > MAX_TRANSPORT_RSS_BYTES {
            bail!(
                "{} RSS delta {} exceeds {} bytes",
                mode.name,
                mode.rss_delta_bytes,
                MAX_TRANSPORT_RSS_BYTES
            );
        }
        if mode.cancellation_millis > MAX_CANCELLATION_MILLIS {
            bail!("{} cancellation exceeds its budget", mode.name);
        }
        for shape in &mode.shapes {
            if shape
                .connection_millis
                .iter()
                .any(|sample| *sample > MAX_CONNECTION_MILLIS)
            {
                bail!("{} {} connection exceeds its budget", mode.name, shape.name);
            }
            if shape
                .first_byte_millis
                .iter()
                .any(|sample| *sample > MAX_FIRST_BYTE_MILLIS)
            {
                bail!("{} {} first byte exceeds its budget", mode.name, shape.name);
            }
            if shape
                .codec
                .as_ref()
                .is_some_and(|codec| codec.decode_failures > 0)
            {
                bail!("{} {} recorded a decode failure", mode.name, shape.name);
            }
        }
    }
    let tcp = mode(modes, "tcp_raw")?;
    let direct_raw = mode(modes, "iroh_direct_raw")?;
    let relay_raw = mode(modes, "iroh_relay_raw")?;
    for shape_name in ["compressible", "incompressible", "wkb", "copy", "result"] {
        let tcp_throughput = shape(tcp, shape_name)?.throughput_p50_mib_per_second;
        if shape(direct_raw, shape_name)?.throughput_p50_mib_per_second / tcp_throughput
            < MIN_DIRECT_IROH_TCP_THROUGHPUT_RATIO
        {
            bail!("direct iroh {shape_name} throughput ratio is below budget");
        }
        if shape(relay_raw, shape_name)?.throughput_p50_mib_per_second / tcp_throughput
            < MIN_RELAY_TCP_THROUGHPUT_RATIO
        {
            bail!("relay iroh {shape_name} throughput ratio is below budget");
        }
    }
    for name in ["iroh_direct_auto", "iroh_relay_auto"] {
        let auto = mode(modes, name)?;
        let compressible = codec(shape(auto, "compressible")?)?;
        let savings_percent = compressible.bytes_saved.max(0) as f64 * 100.0
            / compressible.uncompressed_bytes.max(1) as f64;
        if savings_percent < MIN_COMPRESSIBLE_SAVINGS_PERCENT {
            bail!("{name} compressible savings {savings_percent:.2}% are below budget");
        }
        if codec(shape(auto, "small")?)?.compressed_blocks != 0 {
            bail!("{name} compressed a small payload");
        }
        let incompressible = codec(shape(auto, "incompressible")?)?;
        if incompressible.compressed_blocks != 0 {
            bail!("{name} compressed an incompressible payload");
        }
        let raw = mode(
            modes,
            if name == "iroh_direct_auto" {
                "iroh_direct_raw"
            } else {
                "iroh_relay_raw"
            },
        )?;
        if shape(auto, "incompressible")?.throughput_p50_mib_per_second
            / shape(raw, "incompressible")?.throughput_p50_mib_per_second
            < MIN_INCOMPRESSIBLE_AUTO_RAW_THROUGHPUT_RATIO
        {
            bail!("{name} incompressible overhead exceeds its budget");
        }
    }
    Ok(())
}

fn mode<'a>(modes: &'a [ModeEvidence], name: &str) -> Result<&'a ModeEvidence> {
    modes
        .iter()
        .find(|mode| mode.name == name)
        .ok_or_else(|| anyhow!("missing mode {name}"))
}

fn shape<'a>(mode: &'a ModeEvidence, name: &str) -> Result<&'a ShapeEvidence> {
    mode.shapes
        .iter()
        .find(|shape| shape.name == name)
        .ok_or_else(|| anyhow!("missing {} shape {name}", mode.name))
}

fn codec(shape: &ShapeEvidence) -> Result<&CodecDelta> {
    shape
        .codec
        .as_ref()
        .ok_or_else(|| anyhow!("{} has no codec metrics", shape.name))
}

fn difference(after: u64, before: u64) -> u64 {
    after.saturating_sub(before)
}

fn median(samples: &[f64]) -> f64 {
    let mut samples = samples.to_vec();
    samples.sort_by(f64::total_cmp);
    samples[samples.len() / 2]
}

fn millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

fn startup_packet(user: &str) -> Vec<u8> {
    let mut packet = Vec::new();
    packet.extend_from_slice(&0u32.to_be_bytes());
    packet.extend_from_slice(&196_608u32.to_be_bytes());
    packet.extend_from_slice(b"user\0");
    packet.extend_from_slice(user.as_bytes());
    packet.extend_from_slice(b"\0database\0quackgis\0\0");
    let length = u32::try_from(packet.len()).expect("bounded startup packet");
    packet[..4].copy_from_slice(&length.to_be_bytes());
    packet
}

struct RssSampler {
    idle: u64,
    peak: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    task: JoinHandle<()>,
}

struct RssMeasurement {
    idle: u64,
    peak: u64,
    delta: u64,
}

impl RssSampler {
    fn start() -> Result<Self> {
        let idle = process_rss_bytes().context("Linux process RSS is unavailable")?;
        let peak = Arc::new(AtomicU64::new(idle));
        let stop = Arc::new(AtomicBool::new(false));
        let task_peak = Arc::clone(&peak);
        let task_stop = Arc::clone(&stop);
        let task = tokio::spawn(async move {
            while !task_stop.load(Ordering::Relaxed) {
                if let Some(rss) = process_rss_bytes() {
                    task_peak.fetch_max(rss, Ordering::Relaxed);
                }
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        });
        Ok(Self {
            idle,
            peak,
            stop,
            task,
        })
    }

    async fn finish(self) -> Result<RssMeasurement> {
        self.stop.store(true, Ordering::Relaxed);
        self.task.await?;
        let final_rss = process_rss_bytes().unwrap_or(self.idle);
        let peak = self.peak.load(Ordering::Relaxed).max(final_rss);
        Ok(RssMeasurement {
            idle: self.idle,
            peak,
            delta: peak.saturating_sub(self.idle),
        })
    }
}

fn process_rss_bytes() -> Option<u64> {
    let contents = std::fs::read_to_string("/proc/self/status").ok()?;
    let kib = contents.lines().find_map(|line| {
        line.strip_prefix("VmRSS:")?
            .split_whitespace()
            .next()?
            .parse::<u64>()
            .ok()
    })?;
    kib.checked_mul(1024)
}

fn process_cpu_nanos() -> Result<u64> {
    let stat = std::fs::read_to_string("/proc/self/stat")?;
    let fields = stat
        .rsplit_once(')')
        .context("invalid /proc/self/stat command field")?
        .1
        .split_whitespace()
        .collect::<Vec<_>>();
    let user_ticks = fields
        .get(11)
        .context("missing process user ticks")?
        .parse::<u64>()?;
    let system_ticks = fields
        .get(12)
        .context("missing process system ticks")?
        .parse::<u64>()?;
    let ticks_per_second = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if ticks_per_second <= 0 {
        bail!("cannot determine process clock tick rate");
    }
    Ok(user_ticks
        .saturating_add(system_ticks)
        .saturating_mul(1_000_000_000)
        / ticks_per_second as u64)
}

fn source_state() -> Result<(String, bool)> {
    let sha = command_output(&["rev-parse", "HEAD"])?;
    let status = command_output(&["status", "--porcelain=v1", "--untracked-files=all"])?;
    Ok((sha.trim().to_owned(), !status.is_empty()))
}

fn host_evidence() -> HostEvidence {
    HostEvidence {
        os: os_description(),
        architecture: std::env::consts::ARCH,
        cpu_model: proc_value("/proc/cpuinfo", "model name").unwrap_or_else(|| "unknown".into()),
        logical_cpus: std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1),
        memory_bytes: proc_value("/proc/meminfo", "MemTotal")
            .and_then(|value| value.split_whitespace().next()?.parse::<u64>().ok())
            .and_then(|kib| kib.checked_mul(1024)),
        cgroup_memory_max_bytes: read_trimmed("/sys/fs/cgroup/memory.max")
            .and_then(|value| (value != "max").then(|| value.parse().ok()).flatten()),
        cgroup_cpu_max: read_trimmed("/sys/fs/cgroup/cpu.max"),
    }
}

fn os_description() -> String {
    std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|contents| {
            contents
                .lines()
                .find_map(|line| line.strip_prefix("PRETTY_NAME="))
                .map(|value| value.trim_matches('"').to_owned())
        })
        .unwrap_or_else(|| std::env::consts::OS.to_owned())
}

fn proc_value(path: &str, key: &str) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()?
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            (name.trim() == key).then(|| value.trim().to_owned())
        })
}

fn read_trimmed(path: &str) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().to_owned())
}

fn command_output(arguments: &[&str]) -> Result<String> {
    let output = Command::new("git").args(arguments).output()?;
    if !output.status.success() {
        bail!("git {} failed", arguments.join(" "));
    }
    String::from_utf8(output.stdout).context("git output is not UTF-8")
}

fn write_evidence(path: &Path, evidence: &ProfileEvidence) -> Result<()> {
    let parent = path.parent().context("evidence path has no parent")?;
    std::fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(
        ".{}.tmp-{}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("evidence"),
        std::process::id()
    ));
    let mut bytes = serde_json::to_vec_pretty(evidence)?;
    bytes.push(b'\n');
    std::fs::write(&temporary, bytes)?;
    std::fs::rename(temporary, path)?;
    Ok(())
}
