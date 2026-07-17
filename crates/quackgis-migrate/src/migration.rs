// SPDX-License-Identifier: Apache-2.0

use std::time::Instant;

use anyhow::{Context, Result, anyhow, bail};
use bytes::Bytes;
use chrono::{NaiveDate, NaiveDateTime};
use futures::{SinkExt, StreamExt};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio_postgres::{Client, GenericClient, Transaction};

use crate::config::MigrationConfig;
use crate::connect::{ConnectionOptions, connect};
use crate::plan::{
    Action, ColumnPlan, PreflightReport, PreflightStatus, TablePlan, build_preflight,
};
use crate::source::{begin_source_snapshot, quote_identifier};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MigrationState {
    Rejected,
    FailedRolledBack,
    CommitIndeterminate,
    CommittedUnverified,
    Verified,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TargetIdentity {
    pub server_version_num: u32,
    pub server_version: String,
    pub database_name: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MigrationReport {
    pub format_version: u32,
    pub state: MigrationState,
    pub preflight: PreflightReport,
    pub target: Option<TargetIdentity>,
    pub tables: Vec<TableTransfer>,
    pub duration_ms: u64,
    pub errors: Vec<String>,
    pub final_decision: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TableTransfer {
    pub source_identity: String,
    pub target_identity: String,
    pub rows: u64,
    pub wire_bytes: u64,
    pub wire_sha256: String,
    pub table_checksum: String,
    pub columns: Vec<ColumnVerification>,
    pub duration_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ColumnVerification {
    pub source_name: String,
    pub target_name: String,
    pub null_count: u64,
    pub checksum: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CleanupReport {
    pub format_version: u32,
    pub target: TargetIdentity,
    pub dropped_configured_targets: Vec<String>,
    pub duration_ms: u64,
}

pub async fn cleanup_configured_targets(
    config: &MigrationConfig,
    target_options: &ConnectionOptions,
) -> Result<CleanupReport> {
    let started = Instant::now();
    let mut target_client = connect(target_options)
        .await
        .context("connect migration target for explicit cleanup")?;
    let target_identity = inspect_target_identity(&target_client).await?;
    let target = target_client
        .transaction()
        .await
        .context("begin configured-target cleanup transaction")?;
    let mut dropped = Vec::with_capacity(config.tables.len());
    for table in &config.tables {
        let identity = format!("{}.{}", table.target_schema, table.target_table);
        target
            .batch_execute(&format!(
                "DROP TABLE IF EXISTS {}.{}.{}",
                quote_identifier("quackgis"),
                quote_identifier(&table.target_schema),
                quote_identifier(&table.target_table)
            ))
            .await
            .with_context(|| format!("drop configured migration target {identity}"))?;
        dropped.push(identity);
    }
    target
        .commit()
        .await
        .context("commit configured-target cleanup")?;
    Ok(CleanupReport {
        format_version: 1,
        target: target_identity,
        dropped_configured_targets: dropped,
        duration_ms: millis(started.elapsed()),
    })
}

pub async fn run_migration(
    config: &MigrationConfig,
    source_options: &ConnectionOptions,
    target_options: &ConnectionOptions,
) -> Result<MigrationReport> {
    let started = Instant::now();
    let mut source_client = connect(source_options)
        .await
        .context("connect migration source")?;
    let (source, inventory) = begin_source_snapshot(&mut source_client, config).await?;
    let preflight = build_preflight(config, inventory);
    if preflight.status == PreflightStatus::Rejected {
        source.rollback().await?;
        return Ok(MigrationReport {
            format_version: 1,
            state: MigrationState::Rejected,
            preflight,
            target: None,
            tables: vec![],
            duration_ms: millis(started.elapsed()),
            errors: vec!["preflight rejected before target mutation".to_owned()],
            final_decision: "rejected_before_target_mutation".to_owned(),
        });
    }

    let mut target_client = connect(target_options)
        .await
        .context("connect migration target after successful preflight")?;
    let target_identity = inspect_target_identity(&target_client).await?;
    let target = target_client
        .transaction()
        .await
        .context("begin atomic target migration transaction")?;
    let prepared = prepare_target(&source, &target, &preflight).await;
    let transfers = match prepared {
        Ok(transfers) => transfers,
        Err(error) => {
            let rollback_error = target.rollback().await.err();
            source.rollback().await?;
            let mut errors = vec![format!("migration preparation failed: {error:#}")];
            if let Some(rollback_error) = rollback_error {
                errors.push(format!("target rollback failed: {rollback_error}"));
            }
            return Ok(MigrationReport {
                format_version: 1,
                state: MigrationState::FailedRolledBack,
                preflight,
                target: Some(target_identity),
                tables: vec![],
                duration_ms: millis(started.elapsed()),
                errors,
                final_decision: "not_published".to_owned(),
            });
        }
    };

    if let Err(error) = target.commit().await {
        source.rollback().await?;
        return Ok(MigrationReport {
            format_version: 1,
            state: MigrationState::CommitIndeterminate,
            preflight,
            target: Some(target_identity),
            tables: transfers,
            duration_ms: millis(started.elapsed()),
            errors: vec![format!(
                "target commit response was not successful: {error}"
            )],
            final_decision: "operator_reconciliation_required".to_owned(),
        });
    }

    drop(target_client);
    let fresh_target = match connect(target_options).await {
        Ok(target) => target,
        Err(error) => {
            source.rollback().await?;
            return Ok(MigrationReport {
                format_version: 1,
                state: MigrationState::CommittedUnverified,
                preflight,
                target: Some(target_identity),
                tables: transfers,
                duration_ms: millis(started.elapsed()),
                errors: vec![format!("fresh target connection failed: {error:#}")],
                final_decision: "committed_but_reconciliation_required".to_owned(),
            });
        }
    };
    let verification = verify_fresh_target(&fresh_target, &preflight, &transfers).await;
    source.rollback().await?;
    match verification {
        Ok(()) => Ok(MigrationReport {
            format_version: 1,
            state: MigrationState::Verified,
            preflight,
            target: Some(target_identity),
            tables: transfers,
            duration_ms: millis(started.elapsed()),
            errors: vec![],
            final_decision: "verified_snapshot_prepared_for_operator_cutover".to_owned(),
        }),
        Err(error) => Ok(MigrationReport {
            format_version: 1,
            state: MigrationState::CommittedUnverified,
            preflight,
            target: Some(target_identity),
            tables: transfers,
            duration_ms: millis(started.elapsed()),
            errors: vec![format!("fresh target verification failed: {error:#}")],
            final_decision: "committed_but_reconciliation_required".to_owned(),
        }),
    }
}

async fn inspect_target_identity(client: &Client) -> Result<TargetIdentity> {
    let row = client
        .query_one("SELECT version(), current_database()", &[])
        .await
        .context("read target identity")?;
    let server_version = row.get::<_, String>(0);
    Ok(TargetIdentity {
        server_version_num: postgres_version_num(&server_version)?,
        server_version,
        database_name: row.get(1),
    })
}

async fn prepare_target(
    source: &Transaction<'_>,
    target: &Transaction<'_>,
    preflight: &PreflightReport,
) -> Result<Vec<TableTransfer>> {
    let selected = selected_tables(preflight);
    for table in &selected {
        target
            .batch_execute(&create_table_sql(table)?)
            .await
            .with_context(|| format!("create fresh target table {}", target_identity(table)))?;
    }

    let mut transfers = Vec::with_capacity(selected.len());
    for table in selected {
        let started = Instant::now();
        let (rows, wire_bytes, wire_sha256) = copy_table(source, target, table).await?;
        if rows != table.row_count {
            bail!(
                "target COPY row count {rows} differs from snapshot inventory {} for {}",
                table.row_count,
                source_identity(table)
            );
        }
        apply_comments(target, table).await?;
        let source_checksums = checksum_table(source, table, ChecksumSide::Source).await?;
        let target_checksums = checksum_table(target, table, ChecksumSide::Target).await?;
        if source_checksums != target_checksums {
            bail!("canonical checksums differ for {}", source_identity(table));
        }
        transfers.push(TableTransfer {
            source_identity: source_identity(table),
            target_identity: target_identity(table),
            rows,
            wire_bytes,
            wire_sha256,
            table_checksum: source_checksums.table_checksum,
            columns: table
                .columns
                .iter()
                .zip(source_checksums.columns)
                .map(|(column, checksum)| ColumnVerification {
                    source_name: column.source_name.clone(),
                    target_name: column.target_name.clone().expect("ready target column"),
                    null_count: checksum.null_count,
                    checksum: checksum.checksum,
                })
                .collect(),
            duration_ms: millis(started.elapsed()),
        });
    }
    Ok(transfers)
}

async fn verify_fresh_target(
    target: &Client,
    preflight: &PreflightReport,
    expected: &[TableTransfer],
) -> Result<()> {
    let selected = selected_tables(preflight);
    if selected.len() != expected.len() {
        bail!("fresh target verification table count changed");
    }
    for (table, transfer) in selected.into_iter().zip(expected) {
        let actual = checksum_table(target, table, ChecksumSide::Target).await?;
        if actual.table_checksum != transfer.table_checksum
            || actual.columns.len() != transfer.columns.len()
            || actual
                .columns
                .iter()
                .zip(&transfer.columns)
                .any(|(actual, expected)| {
                    actual.null_count != expected.null_count || actual.checksum != expected.checksum
                })
        {
            bail!(
                "fresh target canonical checksums differ for {}",
                target_identity(table)
            );
        }
        let row = target
            .query_one(
                &format!("SELECT count(*)::BIGINT FROM {}", qualified_target(table)),
                &[],
            )
            .await?;
        if u64::try_from(row.get::<_, i64>(0))? != transfer.rows {
            bail!(
                "fresh target row count differs for {}",
                target_identity(table)
            );
        }
    }
    Ok(())
}

fn selected_tables(preflight: &PreflightReport) -> Vec<&TablePlan> {
    preflight
        .tables
        .iter()
        .filter(|table| {
            table.target_table.is_some()
                && table.disposition.action != Action::Reject
                && table.blockers.is_empty()
        })
        .collect()
}

fn create_table_sql(table: &TablePlan) -> Result<String> {
    let columns = table
        .columns
        .iter()
        .map(|column| {
            let target_name = column
                .target_name
                .as_deref()
                .ok_or_else(|| anyhow!("ready column has no target name"))?;
            let target_type = column
                .target_type
                .as_deref()
                .ok_or_else(|| anyhow!("ready column has no target type"))?;
            let mut definition = format!("{} {target_type}", quote_identifier(target_name));
            if !column.nullable {
                definition.push_str(" NOT NULL");
            }
            if let Some(default) = &column.default_expression {
                definition.push_str(" DEFAULT ");
                definition.push_str(default);
            }
            Ok(definition)
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(format!(
        "CREATE TABLE {} ({})",
        qualified_target(table),
        columns.join(", ")
    ))
}

async fn apply_comments(target: &Transaction<'_>, table: &TablePlan) -> Result<()> {
    if let Some(comment) = &table.comment {
        target
            .batch_execute(&format!(
                "COMMENT ON TABLE {} IS {}",
                qualified_target(table),
                quote_literal(comment)
            ))
            .await
            .context("apply target table comment")?;
    }
    for column in &table.columns {
        if let Some(comment) = &column.comment {
            target
                .batch_execute(&format!(
                    "COMMENT ON COLUMN {}.{} IS {}",
                    qualified_target(table),
                    quote_identifier(column.target_name.as_deref().expect("ready target column")),
                    quote_literal(comment)
                ))
                .await
                .context("apply target column comment")?;
        }
    }
    Ok(())
}

async fn copy_table(
    source: &Transaction<'_>,
    target: &Transaction<'_>,
    table: &TablePlan,
) -> Result<(u64, u64, String)> {
    let source_expressions = table
        .columns
        .iter()
        .map(|column| {
            let identifier = quote_identifier(&column.source_name);
            if column.source_type.starts_with("geometry(") || column.source_type == "geometry" {
                format!("encode(public.ST_AsBinary({identifier}, 'NDR'), 'hex')")
            } else {
                identifier
            }
        })
        .collect::<Vec<_>>();
    let target_columns = table
        .columns
        .iter()
        .map(|column| quote_identifier(column.target_name.as_deref().expect("ready target column")))
        .collect::<Vec<_>>();
    let source_sql = format!(
        "COPY (SELECT {} FROM {}.{}) TO STDOUT WITH (FORMAT TEXT)",
        source_expressions.join(", "),
        quote_identifier(&table.source_schema),
        quote_identifier(&table.source_table)
    );
    let target_sql = format!(
        "COPY {} ({}) FROM STDIN",
        qualified_target(table),
        target_columns.join(", ")
    );
    let output = source.copy_out(&source_sql).await?;
    let input = target.copy_in(&target_sql).await?;
    let mut output = std::pin::pin!(output);
    let mut input = std::pin::pin!(input);
    let mut bytes = 0_u64;
    let mut hasher = Sha256::new();
    while let Some(chunk) = output.next().await {
        let chunk: Bytes = chunk?;
        bytes = bytes
            .checked_add(u64::try_from(chunk.len())?)
            .ok_or_else(|| anyhow!("COPY byte count overflow"))?;
        hasher.update(&chunk);
        input.send(chunk).await?;
    }
    let rows = input.finish().await?;
    Ok((rows, bytes, hex::encode(hasher.finalize())))
}

#[derive(Clone, Copy)]
enum ChecksumSide {
    Source,
    Target,
}

#[derive(Debug, Eq, PartialEq)]
struct TableChecksums {
    table_checksum: String,
    columns: Vec<ChecksumValue>,
}

#[derive(Debug, Eq, PartialEq)]
struct ChecksumValue {
    null_count: u64,
    checksum: String,
}

async fn checksum_table<C>(
    client: &C,
    table: &TablePlan,
    side: ChecksumSide,
) -> Result<TableChecksums>
where
    C: GenericClient + Sync,
{
    let expressions = table
        .columns
        .iter()
        .map(|column| canonical_text_expression(column, side))
        .collect::<Vec<_>>();
    let relation = match side {
        ChecksumSide::Source => format!(
            "{}.{}",
            quote_identifier(&table.source_schema),
            quote_identifier(&table.source_table)
        ),
        ChecksumSide::Target => qualified_target(table),
    };
    let sql = format!("SELECT {} FROM {relation}", expressions.join(", "));
    let parameters: [&(dyn tokio_postgres::types::ToSql + Sync); 0] = [];
    let rows = client.query_raw(&sql, parameters).await?;
    let mut rows = std::pin::pin!(rows);
    let mut table_accumulator = MultisetChecksum::default();
    let mut column_accumulators = (0..table.columns.len())
        .map(|_| MultisetChecksum::default())
        .collect::<Vec<_>>();
    while let Some(row) = rows.next().await {
        let row = row?;
        let mut canonical_row = Vec::new();
        for (index, column) in table.columns.iter().enumerate() {
            let raw = row.get::<_, Option<String>>(index);
            let value = raw
                .as_deref()
                .map(|raw| canonicalize(raw, column))
                .transpose()?;
            column_accumulators[index].update(value.as_deref());
            append_canonical(&mut canonical_row, value.as_deref());
        }
        table_accumulator.update(Some(&canonical_row));
    }
    Ok(TableChecksums {
        table_checksum: table_accumulator.finish(),
        columns: column_accumulators
            .into_iter()
            .map(|accumulator| ChecksumValue {
                null_count: accumulator.null_count,
                checksum: accumulator.finish(),
            })
            .collect(),
    })
}

fn canonical_text_expression(column: &ColumnPlan, side: ChecksumSide) -> String {
    let name = match side {
        ChecksumSide::Source => &column.source_name,
        ChecksumSide::Target => column.target_name.as_ref().expect("ready target column"),
    };
    let identifier = quote_identifier(name);
    let binary = column.source_type == "bytea";
    let geometry = column.source_type.starts_with("geometry(") || column.source_type == "geometry";
    match (side, binary, geometry) {
        (ChecksumSide::Source, true, _) => format!("encode({identifier}, 'hex')"),
        (ChecksumSide::Source, _, true) => {
            format!("encode(public.ST_AsBinary({identifier}, 'NDR'), 'hex')")
        }
        (ChecksumSide::Target, true, _) | (ChecksumSide::Target, _, true) => {
            format!("lower(hex({identifier}))")
        }
        _ => format!("CAST({identifier} AS TEXT)"),
    }
}

fn canonicalize(raw: &str, column: &ColumnPlan) -> Result<Vec<u8>> {
    let canonical = match source_base_type(&column.source_type) {
        "bool" | "boolean" => match raw.to_ascii_lowercase().as_str() {
            "t" | "true" => "1".to_owned(),
            "f" | "false" => "0".to_owned(),
            _ => bail!("invalid canonical boolean in column {}", column.source_name),
        },
        "int2" | "smallint" | "int4" | "integer" | "int8" | "bigint" => raw
            .parse::<i64>()
            .with_context(|| format!("canonicalize integer column {}", column.source_name))?
            .to_string(),
        "float4" | "real" => format!(
            "{:08x}",
            raw.parse::<f32>()
                .with_context(|| format!("canonicalize real column {}", column.source_name))?
                .to_bits()
        ),
        "float8" | "double precision" => format!(
            "{:016x}",
            raw.parse::<f64>()
                .with_context(|| format!("canonicalize double column {}", column.source_name))?
                .to_bits()
        ),
        "numeric" | "decimal" => raw
            .parse::<Decimal>()
            .with_context(|| format!("canonicalize decimal column {}", column.source_name))?
            .normalize()
            .to_string(),
        "date" => NaiveDate::parse_from_str(raw, "%Y-%m-%d")
            .with_context(|| format!("canonicalize date column {}", column.source_name))?
            .format("%Y-%m-%d")
            .to_string(),
        value if value.starts_with("timestamp") => {
            NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S%.f")
                .with_context(|| format!("canonicalize timestamp column {}", column.source_name))?
                .and_utc()
                .timestamp_micros()
                .to_string()
        }
        "bytea" | "geometry" => {
            if !raw.len().is_multiple_of(2) || !raw.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                bail!(
                    "invalid canonical binary hex in column {}",
                    column.source_name
                );
            }
            raw.to_ascii_lowercase()
        }
        _ => raw.to_owned(),
    };
    Ok(canonical.into_bytes())
}

fn source_base_type(source_type: &str) -> &str {
    source_type
        .split_once('(')
        .map_or(source_type, |(base, _)| base)
}

#[derive(Default)]
struct MultisetChecksum {
    count: u64,
    null_count: u64,
    sum: [u8; 32],
    xor: [u8; 32],
}

impl MultisetChecksum {
    fn update(&mut self, value: Option<&[u8]>) {
        self.count = self.count.saturating_add(1);
        if value.is_none() {
            self.null_count = self.null_count.saturating_add(1);
        }
        let mut digest = Sha256::new();
        append_canonical_digest(&mut digest, value);
        let digest: [u8; 32] = digest.finalize().into();
        let mut carry = 0_u16;
        for index in (0..32).rev() {
            let sum = u16::from(self.sum[index]) + u16::from(digest[index]) + carry;
            self.sum[index] = sum as u8;
            carry = sum >> 8;
            self.xor[index] ^= digest[index];
        }
    }

    fn finish(&self) -> String {
        let mut digest = Sha256::new();
        digest.update(b"quackgis-canonical-multiset-v1\0");
        digest.update(self.count.to_be_bytes());
        digest.update(self.null_count.to_be_bytes());
        digest.update(self.sum);
        digest.update(self.xor);
        hex::encode(digest.finalize())
    }
}

fn append_canonical(output: &mut Vec<u8>, value: Option<&[u8]>) {
    match value {
        Some(value) => {
            output.push(1);
            output.extend_from_slice(&u64::try_from(value.len()).unwrap_or(u64::MAX).to_be_bytes());
            output.extend_from_slice(value);
        }
        None => output.push(0),
    }
}

fn append_canonical_digest(digest: &mut Sha256, value: Option<&[u8]>) {
    match value {
        Some(value) => {
            digest.update([1]);
            digest.update(u64::try_from(value.len()).unwrap_or(u64::MAX).to_be_bytes());
            digest.update(value);
        }
        None => digest.update([0]),
    }
}

fn qualified_target(table: &TablePlan) -> String {
    format!(
        "{}.{}.{}",
        quote_identifier("quackgis"),
        quote_identifier(table.target_schema.as_deref().expect("ready target schema")),
        quote_identifier(table.target_table.as_deref().expect("ready target table"))
    )
}

fn source_identity(table: &TablePlan) -> String {
    format!("{}.{}", table.source_schema, table.source_table)
}

fn target_identity(table: &TablePlan) -> String {
    format!(
        "{}.{}",
        table.target_schema.as_deref().expect("ready target schema"),
        table.target_table.as_deref().expect("ready target table")
    )
}

fn quote_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn postgres_version_num(version: &str) -> Result<u32> {
    let dotted = version
        .strip_prefix("PostgreSQL ")
        .and_then(|remainder| remainder.split_whitespace().next())
        .ok_or_else(|| anyhow!("target version() is not PostgreSQL-shaped"))?;
    let (major, minor) = dotted
        .split_once('.')
        .ok_or_else(|| anyhow!("target version() omits a minor version"))?;
    let major = major.parse::<u32>()?;
    let minor = minor.parse::<u32>()?;
    major
        .checked_mul(10_000)
        .and_then(|version| version.checked_add(minor))
        .ok_or_else(|| anyhow!("target version number overflows"))
}

fn millis(duration: std::time::Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn column(source_type: &str) -> ColumnPlan {
        ColumnPlan {
            source_name: "value".to_owned(),
            source_type: source_type.to_owned(),
            target_name: Some("value".to_owned()),
            target_type: Some("VARCHAR".to_owned()),
            nullable: true,
            default_expression: None,
            comment: None,
            disposition: crate::plan::Disposition {
                action: Action::Migrate,
                reason: "test".to_owned(),
            },
        }
    }

    #[test]
    fn canonicalizes_cross_engine_scalar_spellings() {
        assert_eq!(canonicalize("t", &column("boolean")).unwrap(), b"1");
        assert_eq!(canonicalize("true", &column("boolean")).unwrap(), b"1");
        assert_eq!(canonicalize("001", &column("integer")).unwrap(), b"1");
        assert_eq!(
            canonicalize("1.2500", &column("numeric(10,4)")).unwrap(),
            b"1.25"
        );
        assert_eq!(
            canonicalize(
                "2026-07-18 01:02:03.123456",
                &column("timestamp(6) without time zone")
            )
            .unwrap(),
            b"1784336523123456"
        );
        assert_eq!(canonicalize("00aF", &column("bytea")).unwrap(), b"00af");
    }

    #[test]
    fn multiset_checksum_is_order_independent_and_null_sensitive() {
        let mut left = MultisetChecksum::default();
        left.update(Some(b"one"));
        left.update(None);
        left.update(Some(b"two"));
        let mut right = MultisetChecksum::default();
        right.update(Some(b"two"));
        right.update(Some(b"one"));
        right.update(None);
        assert_eq!(left.finish(), right.finish());
        right.update(None);
        assert_ne!(left.finish(), right.finish());
    }

    #[test]
    fn quotes_comments_as_data() {
        assert_eq!(quote_literal("owner's table"), "'owner''s table'");
    }

    #[test]
    fn parses_postgresql_shaped_target_versions() {
        assert_eq!(
            postgres_version_num("PostgreSQL 18.4 (QuackGIS DuckDB compatibility profile)")
                .unwrap(),
            180_004
        );
        assert!(postgres_version_num("DuckDB v1.5.4").is_err());
    }
}
