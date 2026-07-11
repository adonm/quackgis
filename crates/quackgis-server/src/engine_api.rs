// SPDX-License-Identifier: Apache-2.0
//! Arrow-native contracts between the DuckDB storage kernel and protocol edge.
//!
//! These types intentionally expose neither DuckDB ADBC details nor protocol
//! implementation details, keeping storage behavior directly testable.

use std::error::Error;
use std::fmt;

use arrow_array::RecordBatch;
use arrow_schema::SchemaRef;

pub type EngineResult<T> = Result<T, EngineError>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EngineErrorKind {
    InvalidQuery,
    Unsupported,
    NotFound,
    AlreadyExists,
    Constraint,
    Unauthorized,
    Cancelled,
    Timeout,
    Io,
    Busy,
    Internal,
    IndeterminateCommit,
    Quarantined,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransactionOutcome {
    NotApplicable,
    RolledBack,
    Committed,
    Indeterminate,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum EngineTransactionState {
    #[default]
    Idle,
    Active,
    Quarantined,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EngineError {
    pub kind: EngineErrorKind,
    pub sqlstate: Option<String>,
    pub vendor_code: i32,
    pub transaction_outcome: TransactionOutcome,
    message: String,
}

impl EngineError {
    pub fn new(kind: EngineErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            sqlstate: None,
            vendor_code: 0,
            transaction_outcome: TransactionOutcome::NotApplicable,
            message: bounded_message(message.into()),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for EngineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for EngineError {}

fn bounded_message(mut message: String) -> String {
    const MAX_CHARS: usize = 1024;
    if message.chars().count() <= MAX_CHARS {
        return message;
    }
    message = message.chars().take(MAX_CHARS).collect();
    message.push('…');
    message
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EngineTableRef {
    pub catalog: String,
    pub schema: String,
    pub table: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IngestDisposition {
    Create,
    Append,
    Replace,
}

#[derive(Clone, Debug)]
pub struct EngineStatementDescription {
    pub parameter_schema: SchemaRef,
    pub result_schema: SchemaRef,
}

#[derive(Clone, Debug)]
pub struct EngineQueryResult {
    /// Preserved even when the query returns no batches.
    pub schema: SchemaRef,
    pub batches: Vec<RecordBatch>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EngineSnapshot {
    pub id: i64,
    pub timestamp: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum EngineMaintenanceRequest {
    MergeAdjacentFiles {
        schema: String,
        table: String,
        max_compacted_files: Option<u64>,
        max_file_size: Option<u64>,
        min_file_size: Option<u64>,
    },
    RewriteDataFiles {
        schema: String,
        table: String,
        delete_threshold: f64,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EngineMaintenanceReport {
    pub affected_rows: Option<i64>,
}

/// Synchronous kernel contract. Async server adapters must invoke blocking
/// implementations on an owned worker rather than a Tokio executor thread.
pub trait EngineStorageKernel {
    fn describe(&self, sql: &str) -> EngineResult<EngineStatementDescription>;
    fn query_result(&self, sql: &str) -> EngineResult<EngineQueryResult>;
    fn query_bound(&self, sql: &str, parameters: RecordBatch) -> EngineResult<EngineQueryResult>;
    fn execute_update_contract(&self, sql: &str) -> EngineResult<Option<i64>>;
    fn execute_update_bound(&self, sql: &str, parameters: RecordBatch)
    -> EngineResult<Option<i64>>;
    fn table_schema(&self, table: &EngineTableRef) -> EngineResult<SchemaRef>;
    fn ingest_contract(
        &self,
        table: &EngineTableRef,
        batches: Vec<RecordBatch>,
        disposition: IngestDisposition,
    ) -> EngineResult<Option<i64>>;
    fn snapshots(&self) -> EngineResult<Vec<EngineSnapshot>>;
    fn maintain(&self, request: EngineMaintenanceRequest) -> EngineResult<EngineMaintenanceReport>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_errors_bound_untrusted_native_messages() {
        let error = EngineError::new(EngineErrorKind::Internal, "x".repeat(2048));
        assert_eq!(error.message().chars().count(), 1025);
        assert!(error.message().ends_with('…'));
    }
}
