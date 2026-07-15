// SPDX-License-Identifier: Apache-2.0
//! Bounded, independent application-data compression blocks and payload-free metrics.

use std::io::{Error, ErrorKind, Result};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use serde::Serialize;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::{ApplicationProtocol, CompressionCodec};

pub const MAX_UNCOMPRESSED_BLOCK_BYTES: usize = 64 * 1024;
pub const MAX_COMPRESSED_BLOCK_BYTES: usize = MAX_UNCOMPRESSED_BLOCK_BYTES;
pub const MAX_EXPANSION_RATIO: usize = 256;
pub const MIN_COMPRESSION_INPUT_BYTES: usize = 1024;
pub const MIN_COMPRESSION_SAVINGS_BYTES: usize = 64;
pub const MIN_COMPRESSION_SAVINGS_PERCENT: usize = 12;
pub const INCOMPRESSIBLE_PROBES_BEFORE_BACKOFF: usize = 2;
pub const INCOMPRESSIBLE_BACKOFF_BLOCKS: usize = 8;

const BLOCK_HEADER_BYTES: usize = 9;
const RAW_BLOCK: u8 = 0;
const LZ4_BLOCK: u8 = 1;

#[derive(Clone, Copy, Debug)]
pub(crate) enum Direction {
    Upstream,
    Downstream,
}

#[derive(Clone, Default)]
pub struct TransportMetrics {
    inner: Arc<MetricsInner>,
}

#[derive(Default)]
struct MetricsInner {
    upstream: DirectionMetrics,
    downstream: DirectionMetrics,
    pgwire_streams: AtomicU64,
    cancellation_streams: AtomicU64,
}

#[derive(Default)]
struct DirectionMetrics {
    uncompressed_bytes: AtomicU64,
    wire_bytes: AtomicU64,
    latency_sensitive_bytes: AtomicU64,
    blocks: AtomicU64,
    compressed_blocks: AtomicU64,
    raw_small_blocks: AtomicU64,
    raw_incompressible_blocks: AtomicU64,
    raw_backoff_blocks: AtomicU64,
    compression_cpu_nanos: AtomicU64,
    decompression_cpu_nanos: AtomicU64,
    decode_failures: AtomicU64,
}

#[derive(Clone, Debug, Serialize)]
pub struct TransportMetricsSnapshot {
    pub upstream: DirectionMetricsSnapshot,
    pub downstream: DirectionMetricsSnapshot,
    pub pgwire_streams: u64,
    pub cancellation_streams: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct DirectionMetricsSnapshot {
    pub uncompressed_bytes: u64,
    pub wire_bytes: u64,
    pub bytes_saved: i64,
    pub latency_sensitive_bytes: u64,
    pub blocks: u64,
    pub compressed_blocks: u64,
    pub raw_small_blocks: u64,
    pub raw_incompressible_blocks: u64,
    pub raw_backoff_blocks: u64,
    pub compression_cpu_nanos: u64,
    pub decompression_cpu_nanos: u64,
    pub decode_failures: u64,
}

impl TransportMetrics {
    pub fn record_stream(&self, protocol: ApplicationProtocol) {
        match protocol {
            ApplicationProtocol::Pgwire => &self.inner.pgwire_streams,
            ApplicationProtocol::Cancellation => &self.inner.cancellation_streams,
            ApplicationProtocol::Http => return,
        }
        .fetch_add(1, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> TransportMetricsSnapshot {
        TransportMetricsSnapshot {
            upstream: self.inner.upstream.snapshot(),
            downstream: self.inner.downstream.snapshot(),
            pgwire_streams: self.inner.pgwire_streams.load(Ordering::Relaxed),
            cancellation_streams: self.inner.cancellation_streams.load(Ordering::Relaxed),
        }
    }

    pub(crate) fn record_latency_sensitive(&self, direction: Direction, bytes: usize) {
        self.direction(direction)
            .latency_sensitive_bytes
            .fetch_add(bytes as u64, Ordering::Relaxed);
    }

    fn direction(&self, direction: Direction) -> &DirectionMetrics {
        match direction {
            Direction::Upstream => &self.inner.upstream,
            Direction::Downstream => &self.inner.downstream,
        }
    }
}

impl DirectionMetrics {
    fn snapshot(&self) -> DirectionMetricsSnapshot {
        let uncompressed_bytes = self.uncompressed_bytes.load(Ordering::Relaxed);
        let wire_bytes = self.wire_bytes.load(Ordering::Relaxed);
        DirectionMetricsSnapshot {
            uncompressed_bytes,
            wire_bytes,
            bytes_saved: i64::try_from(uncompressed_bytes).unwrap_or(i64::MAX)
                - i64::try_from(wire_bytes).unwrap_or(i64::MAX),
            latency_sensitive_bytes: self.latency_sensitive_bytes.load(Ordering::Relaxed),
            blocks: self.blocks.load(Ordering::Relaxed),
            compressed_blocks: self.compressed_blocks.load(Ordering::Relaxed),
            raw_small_blocks: self.raw_small_blocks.load(Ordering::Relaxed),
            raw_incompressible_blocks: self.raw_incompressible_blocks.load(Ordering::Relaxed),
            raw_backoff_blocks: self.raw_backoff_blocks.load(Ordering::Relaxed),
            compression_cpu_nanos: self.compression_cpu_nanos.load(Ordering::Relaxed),
            decompression_cpu_nanos: self.decompression_cpu_nanos.load(Ordering::Relaxed),
            decode_failures: self.decode_failures.load(Ordering::Relaxed),
        }
    }

    fn record_block(
        &self,
        uncompressed_bytes: usize,
        wire_bytes: usize,
        classification: BlockClassification,
    ) {
        self.uncompressed_bytes
            .fetch_add(uncompressed_bytes as u64, Ordering::Relaxed);
        self.wire_bytes
            .fetch_add(wire_bytes as u64, Ordering::Relaxed);
        self.blocks.fetch_add(1, Ordering::Relaxed);
        match classification {
            BlockClassification::Compressed => &self.compressed_blocks,
            BlockClassification::RawSmall => &self.raw_small_blocks,
            BlockClassification::RawIncompressible => &self.raw_incompressible_blocks,
            BlockClassification::RawBackoff => &self.raw_backoff_blocks,
        }
        .fetch_add(1, Ordering::Relaxed);
    }
}

#[derive(Clone, Copy)]
enum BlockClassification {
    Compressed,
    RawSmall,
    RawIncompressible,
    RawBackoff,
}

pub(crate) async fn copy_application<R, W>(
    reader: &mut R,
    writer: &mut W,
    codec: CompressionCodec,
    encode: bool,
    metrics: &TransportMetrics,
    direction: Direction,
) -> Result<u64>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    match (codec, encode) {
        (CompressionCodec::None, _) => copy_unframed(reader, writer, metrics, direction).await,
        (CompressionCodec::Lz4Block, true) => {
            copy_compressed(reader, writer, metrics, direction).await
        }
        (CompressionCodec::Lz4Block, false) => {
            copy_decompressed(reader, writer, metrics, direction).await
        }
    }
}

async fn copy_unframed<R, W>(
    reader: &mut R,
    writer: &mut W,
    metrics: &TransportMetrics,
    direction: Direction,
) -> Result<u64>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let counters = metrics.direction(direction);
    let mut total = 0_u64;
    let mut buffer = vec![0; MAX_UNCOMPRESSED_BLOCK_BYTES];
    loop {
        let read = reader.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        writer.write_all(&buffer[..read]).await?;
        counters.record_block(read, read, BlockClassification::RawIncompressible);
        total = total.saturating_add(read as u64);
    }
    Ok(total)
}

async fn copy_compressed<R, W>(
    reader: &mut R,
    writer: &mut W,
    metrics: &TransportMetrics,
    direction: Direction,
) -> Result<u64>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let counters = metrics.direction(direction);
    let mut total = 0_u64;
    let mut input = vec![0; MAX_UNCOMPRESSED_BLOCK_BYTES];
    let mut incompressible_probes = 0_usize;
    let mut backoff_blocks = 0_usize;
    loop {
        let read = reader.read(&mut input).await?;
        if read == 0 {
            break;
        }
        let input = &input[..read];
        let (kind, payload, classification) = if read < MIN_COMPRESSION_INPUT_BYTES {
            (RAW_BLOCK, input.to_vec(), BlockClassification::RawSmall)
        } else if backoff_blocks > 0 {
            backoff_blocks -= 1;
            (RAW_BLOCK, input.to_vec(), BlockClassification::RawBackoff)
        } else {
            let started = Instant::now();
            let compressed = lz4_flex::block::compress(input);
            counters
                .compression_cpu_nanos
                .fetch_add(elapsed_nanos(started), Ordering::Relaxed);
            let saved = read.saturating_sub(compressed.len());
            let saves_enough = saved >= MIN_COMPRESSION_SAVINGS_BYTES
                && saved.saturating_mul(100)
                    >= read.saturating_mul(MIN_COMPRESSION_SAVINGS_PERCENT);
            if saves_enough {
                incompressible_probes = 0;
                (LZ4_BLOCK, compressed, BlockClassification::Compressed)
            } else {
                incompressible_probes += 1;
                if incompressible_probes >= INCOMPRESSIBLE_PROBES_BEFORE_BACKOFF {
                    incompressible_probes = 0;
                    backoff_blocks = INCOMPRESSIBLE_BACKOFF_BLOCKS;
                }
                (
                    RAW_BLOCK,
                    input.to_vec(),
                    BlockClassification::RawIncompressible,
                )
            }
        };
        let uncompressed_length = u32::try_from(read).expect("block limit fits u32");
        let payload_length = u32::try_from(payload.len()).expect("block limit fits u32");
        writer.write_all(&[kind]).await?;
        writer.write_all(&uncompressed_length.to_be_bytes()).await?;
        writer.write_all(&payload_length.to_be_bytes()).await?;
        writer.write_all(&payload).await?;
        counters.record_block(read, BLOCK_HEADER_BYTES + payload.len(), classification);
        total = total.saturating_add(read as u64);
    }
    Ok(total)
}

async fn copy_decompressed<R, W>(
    reader: &mut R,
    writer: &mut W,
    metrics: &TransportMetrics,
    direction: Direction,
) -> Result<u64>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let counters = metrics.direction(direction);
    let result = async {
        let mut total = 0_u64;
        loop {
            let mut kind = [0_u8; 1];
            if reader.read(&mut kind).await? == 0 {
                break;
            }
            let mut lengths = [0_u8; 8];
            reader.read_exact(&mut lengths).await?;
            let uncompressed_length =
                u32::from_be_bytes(lengths[..4].try_into().expect("fixed length")) as usize;
            let payload_length =
                u32::from_be_bytes(lengths[4..].try_into().expect("fixed length")) as usize;
            validate_lengths(kind[0], uncompressed_length, payload_length)?;
            let mut payload = vec![0; payload_length];
            reader.read_exact(&mut payload).await?;
            let started = Instant::now();
            let (decoded, classification) = match kind[0] {
                RAW_BLOCK => (payload, classify_raw(uncompressed_length)),
                LZ4_BLOCK => (
                    lz4_flex::block::decompress(&payload, uncompressed_length).map_err(
                        |error| {
                            Error::new(
                                ErrorKind::InvalidData,
                                format!("invalid LZ4 block: {error}"),
                            )
                        },
                    )?,
                    BlockClassification::Compressed,
                ),
                _ => unreachable!("validated block kind"),
            };
            counters
                .decompression_cpu_nanos
                .fetch_add(elapsed_nanos(started), Ordering::Relaxed);
            if decoded.len() != uncompressed_length {
                return Err(Error::new(
                    ErrorKind::InvalidData,
                    "compression block decoded to the wrong length",
                ));
            }
            writer.write_all(&decoded).await?;
            counters.record_block(
                uncompressed_length,
                BLOCK_HEADER_BYTES + payload_length,
                classification,
            );
            total = total.saturating_add(uncompressed_length as u64);
        }
        Ok(total)
    }
    .await;
    if result.is_err() {
        counters.decode_failures.fetch_add(1, Ordering::Relaxed);
    }
    result
}

fn validate_lengths(kind: u8, uncompressed: usize, payload: usize) -> Result<()> {
    if !matches!(kind, RAW_BLOCK | LZ4_BLOCK) {
        return Err(Error::new(
            ErrorKind::InvalidData,
            "unknown compression block kind",
        ));
    }
    if !(1..=MAX_UNCOMPRESSED_BLOCK_BYTES).contains(&uncompressed)
        || !(1..=MAX_COMPRESSED_BLOCK_BYTES).contains(&payload)
    {
        return Err(Error::new(
            ErrorKind::InvalidData,
            "compression block exceeds a declared length limit",
        ));
    }
    if kind == RAW_BLOCK && payload != uncompressed {
        return Err(Error::new(
            ErrorKind::InvalidData,
            "raw compression block lengths differ",
        ));
    }
    if kind == LZ4_BLOCK
        && (payload >= uncompressed || uncompressed > payload.saturating_mul(MAX_EXPANSION_RATIO))
    {
        return Err(Error::new(
            ErrorKind::InvalidData,
            "compressed block violates the savings or expansion-ratio limit",
        ));
    }
    Ok(())
}

fn classify_raw(length: usize) -> BlockClassification {
    if length < MIN_COMPRESSION_INPUT_BYTES {
        BlockClassification::RawSmall
    } else {
        BlockClassification::RawIncompressible
    }
}

fn elapsed_nanos(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn adaptive_blocks_compress_repetition_and_skip_noise_and_small_input() {
        let mut input = vec![b'x'; MAX_UNCOMPRESSED_BLOCK_BYTES];
        let mut state = 0x9e37_79b9_u32;
        input.extend((0..MAX_UNCOMPRESSED_BLOCK_BYTES).map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state as u8
        }));
        let metrics = TransportMetrics::default();
        let (mut source, mut source_peer) = tokio::io::duplex(256 * 1024);
        let (mut framed, mut framed_peer) = tokio::io::duplex(256 * 1024);
        let expected = input.clone();
        let producer = tokio::spawn(async move {
            source_peer.write_all(&input).await.unwrap();
            source_peer.shutdown().await.unwrap();
        });
        copy_compressed(&mut source, &mut framed_peer, &metrics, Direction::Upstream)
            .await
            .unwrap();
        framed_peer.shutdown().await.unwrap();
        producer.await.unwrap();

        let decoded_metrics = TransportMetrics::default();
        let mut decoded = Vec::new();
        copy_decompressed(
            &mut framed,
            &mut decoded,
            &decoded_metrics,
            Direction::Upstream,
        )
        .await
        .unwrap();
        assert_eq!(decoded, expected);
        let encoded = metrics.snapshot().upstream;
        assert!(encoded.compressed_blocks > 0);
        assert!(encoded.raw_incompressible_blocks > 0);
        assert!(encoded.bytes_saved > 0);

        let small_metrics = TransportMetrics::default();
        let (mut small, mut small_peer) = tokio::io::duplex(1024);
        let (mut output, mut output_peer) = tokio::io::duplex(1024);
        small_peer.write_all(b"small").await.unwrap();
        small_peer.shutdown().await.unwrap();
        copy_compressed(
            &mut small,
            &mut output_peer,
            &small_metrics,
            Direction::Downstream,
        )
        .await
        .unwrap();
        output_peer.shutdown().await.unwrap();
        let mut framed_small = Vec::new();
        output.read_to_end(&mut framed_small).await.unwrap();
        assert_eq!(framed_small[0], RAW_BLOCK);
        assert_eq!(small_metrics.snapshot().downstream.raw_small_blocks, 1);
    }

    #[tokio::test]
    async fn decoder_rejects_corrupt_truncated_oversized_and_abusive_blocks() {
        for frame in [
            vec![9, 0, 0, 0, 1, 0, 0, 0, 1, 0],
            vec![RAW_BLOCK, 0, 0, 0, 2, 0, 0, 0, 2, 1],
            vec![RAW_BLOCK, 0, 1, 0, 1, 0, 0, 0, 1, 0],
            vec![LZ4_BLOCK, 0, 1, 0, 0, 0, 0, 0, 1, 0],
            vec![LZ4_BLOCK, 0, 0, 2, 0, 0, 0, 0, 2, 0, 0],
        ] {
            let metrics = TransportMetrics::default();
            let mut reader = std::io::Cursor::new(frame);
            let mut output = Vec::new();
            assert!(
                copy_decompressed(&mut reader, &mut output, &metrics, Direction::Downstream)
                    .await
                    .is_err()
            );
            assert_eq!(metrics.snapshot().downstream.decode_failures, 1);
            assert!(output.is_empty());
        }
    }

    #[test]
    fn metrics_are_payload_free() {
        let rendered = serde_json::to_string(&TransportMetrics::default().snapshot()).unwrap();
        for forbidden in ["sql", "parameter", "credential", "path", "payload"] {
            assert!(!rendered.contains(forbidden));
        }
    }
}
