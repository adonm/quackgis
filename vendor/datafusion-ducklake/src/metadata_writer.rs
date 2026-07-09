//! Metadata writer trait and common types for DuckLake catalog writes.
//!
//! This module provides the `MetadataWriter` trait for writing metadata to DuckLake catalogs,
//! along with helper types for column definitions and data file registration.

use crate::{DuckLakeError, Result};
use std::collections::HashSet;

/// Maximum allowed length for catalog entity names (schemas, tables, columns).
pub const MAX_NAME_LENGTH: usize = 1024;

/// Validate a catalog entity name (schema, table, or column).
///
/// Rejects names that are:
/// - Empty or whitespace-only
/// - Contain ASCII control characters (0x00-0x1F, 0x7F)
/// - Exceed [`MAX_NAME_LENGTH`] characters
pub fn validate_name(name: &str, kind: &str) -> Result<()> {
    if name.trim().is_empty() {
        return Err(DuckLakeError::InvalidConfig(format!(
            "{kind} name cannot be empty or whitespace-only"
        )));
    }
    if let Some(pos) = name.find(|c: char| c.is_ascii_control()) {
        let byte = name.as_bytes()[pos];
        return Err(DuckLakeError::InvalidConfig(format!(
            "{kind} name contains control character 0x{byte:02X} at position {pos}"
        )));
    }
    if name.len() > MAX_NAME_LENGTH {
        return Err(DuckLakeError::InvalidConfig(format!(
            "{kind} name exceeds maximum length of {MAX_NAME_LENGTH} characters (got {})",
            name.len()
        )));
    }
    Ok(())
}

/// Write mode for table operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteMode {
    /// Drop existing data and replace with new data
    Replace,
    /// Keep existing data and append new records
    Append,
}
use crate::types::{arrow_to_ducklake_type, ducklake_to_arrow_type};
use arrow::datatypes::DataType;

/// Column definition for creating or updating a table's schema.
///
/// Unlike `DuckLakeTableColumn` (used for reading), this struct doesn't have a `column_id`
/// field since IDs are assigned by the catalog during write operations.
#[derive(Debug, Clone)]
pub struct ColumnDef {
    /// Column name
    pub(crate) name: String,
    /// DuckLake type string (e.g., "varchar", "int64", "decimal(10,2)")
    pub(crate) ducklake_type: String,
    /// Whether this column allows NULL values
    pub(crate) is_nullable: bool,
}

impl ColumnDef {
    /// Returns the column name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the DuckLake type string.
    pub fn ducklake_type(&self) -> &str {
        &self.ducklake_type
    }

    /// Returns whether this column allows NULL values.
    pub fn is_nullable(&self) -> bool {
        self.is_nullable
    }

    /// Create a new column definition.
    ///
    /// Validates that `ducklake_type` is a recognized DuckLake type string by converting
    /// it to an Arrow DataType. Returns an error if the type is invalid or unsupported.
    pub fn new(
        name: impl Into<String>,
        ducklake_type: impl Into<String>,
        is_nullable: bool,
    ) -> Result<Self> {
        let name = name.into();
        validate_name(&name, "Column")?;
        let ducklake_type = ducklake_type.into();
        // Validate the type string by attempting to convert it to an Arrow type.
        // We discard the result; we only care that the conversion succeeds.
        ducklake_to_arrow_type(&ducklake_type)?;
        Ok(Self {
            name,
            ducklake_type,
            is_nullable,
        })
    }

    /// Create a column definition from an Arrow DataType.
    ///
    /// This is a convenience constructor that converts the Arrow type to a DuckLake type string.
    /// The resulting DuckLake type is guaranteed to be valid since it was derived from a known
    /// Arrow type.
    pub fn from_arrow(
        name: impl Into<String>,
        data_type: &DataType,
        is_nullable: bool,
    ) -> Result<Self> {
        let name = name.into();
        validate_name(&name, "Column")?;
        let ducklake_type = arrow_to_ducklake_type(data_type)?;
        // We use direct struct construction here since the ducklake_type was just
        // produced by arrow_to_ducklake_type, so it is guaranteed to be valid.
        Ok(Self {
            name,
            ducklake_type,
            is_nullable,
        })
    }
}

/// Whether `proposed` is a *schema change* relative to `existing` — i.e. whether a
/// commit carrying it is DDL (and must bump `schema_version`) rather than a pure
/// data write (which carries `schema_version` forward).
///
/// `existing` is the table's currently-live columns as `(name, ducklake_type,
/// nullable)`, ordered by `column_order`; `proposed` is the incoming schema. The
/// comparison is positional, mirroring upstream's per-column diff.
///
/// A same-name type difference is NOT treated as a change when it's the benign
/// Append-vs-promote race: a data write that PASSED the begin-time type reject (its
/// staged type matched the type AT BEGIN) but whose column a concurrent promote
/// widened before this commit. The staged (narrower) type losslessly widens to the
/// committed type and is served via cast-on-read, so it must NOT bump
/// `schema_version`. We accept canonical-equal OR staged-widens-to-committed;
/// anything else is real DDL. (Not `types_compatible`, which would also accept
/// committed-widens-to-staged and wrongly classify the race as DDL.)
///
/// Shared by the SQLite and Postgres writers so the DDL/DML classification can't
/// drift between backends.
pub(crate) fn columns_differ(existing: &[(String, String, bool)], proposed: &[ColumnDef]) -> bool {
    if existing.len() != proposed.len() {
        return true;
    }
    for ((ex_name, ex_type, ex_nullable), new_col) in existing.iter().zip(proposed.iter()) {
        if ex_name != &new_col.name {
            return true;
        }
        let same_type = crate::types::types_equal_canonical(ex_type, &new_col.ducklake_type)
            || crate::types::is_promotable(&new_col.ducklake_type, ex_type);
        if !same_type {
            return true;
        }
        if *ex_nullable != new_col.is_nullable {
            return true;
        }
    }
    false
}

/// Information about a data file to register in the catalog.
///
/// This struct contains the metadata needed to register a Parquet file in the DuckLake catalog.
#[derive(Debug, Clone)]
pub struct DataFileInfo {
    /// Path to the file (relative to table path or absolute)
    pub path: String,
    /// Whether the path is relative to the table's path
    pub path_is_relative: bool,
    /// Size of the file in bytes
    pub file_size_bytes: i64,
    /// Size of the Parquet footer in bytes (optimization hint for reads)
    pub footer_size: Option<i64>,
    /// Number of records in the file
    pub record_count: i64,
}

impl DataFileInfo {
    /// Create a new data file info with relative path.
    ///
    /// # Panics
    ///
    /// Panics if `record_count` is negative. Record counts originate from
    /// `RecordBatch::num_rows()` (always non-negative), so a negative value
    /// indicates a programming error.
    pub fn new(path: impl Into<String>, file_size_bytes: i64, record_count: i64) -> Self {
        assert!(
            record_count >= 0,
            "record_count must be non-negative, got {}",
            record_count
        );
        Self {
            path: path.into(),
            path_is_relative: true,
            file_size_bytes,
            footer_size: None,
            record_count,
        }
    }

    /// Set the footer size for read optimization.
    pub fn with_footer_size(mut self, footer_size: i64) -> Self {
        self.footer_size = Some(footer_size);
        self
    }

    /// Mark this file as having an absolute path.
    pub fn with_absolute_path(mut self) -> Self {
        self.path_is_relative = false;
        self
    }
}

/// A positional delete file to register via [`MetadataWriter::set_delete_file`].
/// Mirrors [`DataFileInfo`]; the parquet has the standard `(file_path, pos)`
/// schema. Must be cumulative for its data file (all still-deleted positions),
/// since at most one delete file is live per data file at a time.
#[derive(Debug, Clone)]
pub struct DeleteFileInfo {
    /// Path to the delete file (relative to the table path, or absolute).
    pub path: String,
    /// Whether the path is relative to the table's path.
    pub path_is_relative: bool,
    /// Size of the delete file in bytes.
    pub file_size_bytes: i64,
    /// Size of the Parquet footer in bytes (read optimization hint).
    pub footer_size: Option<i64>,
    /// Number of deleted positions in this file.
    pub delete_count: i64,
}

/// One positional delete-file registration in an atomic table mutation.
#[derive(Debug, Clone)]
pub struct DeleteFileMutation {
    /// Catalog id of the live data file being masked.
    pub data_file_id: i64,
    /// Delete-file id that was live for `data_file_id` when positions were
    /// resolved, or `None` when no delete file was live. Used as a CAS guard.
    pub expected_prev_delete_file: Option<i64>,
    /// New cumulative delete file to make live for `data_file_id`.
    pub delete: DeleteFileInfo,
}

impl DeleteFileMutation {
    pub fn new(
        data_file_id: i64,
        expected_prev_delete_file: Option<i64>,
        delete: DeleteFileInfo,
    ) -> Self {
        Self {
            data_file_id,
            expected_prev_delete_file,
            delete,
        }
    }
}

/// A table-scoped metadata mutation committed under one DuckLake snapshot.
///
/// This is the fork's low-level atomic DML primitive. Callers may combine:
/// - appended data files that have already been durably written to object storage,
/// - cumulative positional delete files for existing data files, and
/// - retirement of existing data files (for compaction / rewrite-style updates).
///
/// The metadata writer must make all entries visible together, or none of them.
/// It must fail with [`DuckLakeError::Conflict`] when the caller's base snapshot
/// is stale for any target file/delete generation.
#[derive(Debug, Clone, Default)]
pub struct TableMutation {
    /// New data files to make visible at the mutation snapshot.
    pub append_data_files: Vec<DataFileInfo>,
    /// Cumulative positional delete files to make live for existing data files.
    pub delete_files: Vec<DeleteFileMutation>,
    /// Existing data_file_id values to retire at the mutation snapshot.
    pub retire_data_file_ids: Vec<i64>,
}

impl TableMutation {
    /// Create an empty mutation. Add at least one operation before commit.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an already-written data file to be registered atomically.
    pub fn append_data_file(mut self, file: DataFileInfo) -> Self {
        self.append_data_files.push(file);
        self
    }

    /// Add a cumulative positional delete-file generation.
    pub fn set_delete_file(mut self, delete: DeleteFileMutation) -> Self {
        self.delete_files.push(delete);
        self
    }

    /// Retire an existing live data file.
    pub fn retire_data_file(mut self, data_file_id: i64) -> Self {
        self.retire_data_file_ids.push(data_file_id);
        self
    }

    /// Whether the mutation contains no work.
    pub fn is_empty(&self) -> bool {
        self.append_data_files.is_empty()
            && self.delete_files.is_empty()
            && self.retire_data_file_ids.is_empty()
    }

    /// Validate shape invariants before a backend opens its transaction.
    pub fn validate(&self) -> Result<()> {
        if self.is_empty() {
            return Err(DuckLakeError::InvalidConfig(
                "table mutation requires at least one operation".to_string(),
            ));
        }

        let mut deleted = HashSet::new();
        for delete in &self.delete_files {
            if delete.data_file_id <= 0 {
                return Err(DuckLakeError::InvalidConfig(format!(
                    "delete mutation references invalid data_file_id {}",
                    delete.data_file_id
                )));
            }
            if !deleted.insert(delete.data_file_id) {
                return Err(DuckLakeError::InvalidConfig(format!(
                    "duplicate delete mutation for data file {}",
                    delete.data_file_id
                )));
            }
            if delete.delete.file_size_bytes < 0 {
                return Err(DuckLakeError::InvalidConfig(format!(
                    "delete file for data file {} has negative file_size_bytes {}",
                    delete.data_file_id, delete.delete.file_size_bytes
                )));
            }
            if delete.delete.delete_count < 0 {
                return Err(DuckLakeError::InvalidConfig(format!(
                    "delete file for data file {} has negative delete_count {}",
                    delete.data_file_id, delete.delete.delete_count
                )));
            }
        }

        let mut retired = HashSet::new();
        for data_file_id in &self.retire_data_file_ids {
            if *data_file_id <= 0 {
                return Err(DuckLakeError::InvalidConfig(format!(
                    "retire mutation references invalid data_file_id {data_file_id}"
                )));
            }
            if !retired.insert(*data_file_id) {
                return Err(DuckLakeError::InvalidConfig(format!(
                    "duplicate retire mutation for data file {data_file_id}"
                )));
            }
        }

        for data_file_id in deleted.intersection(&retired) {
            return Err(DuckLakeError::InvalidConfig(format!(
                "data file {data_file_id} cannot be both deleted and retired in one mutation"
            )));
        }

        for file in &self.append_data_files {
            if file.file_size_bytes < 0 {
                return Err(DuckLakeError::InvalidConfig(format!(
                    "appended data file '{}' has negative file_size_bytes {}",
                    file.path, file.file_size_bytes
                )));
            }
            if file.record_count < 0 {
                return Err(DuckLakeError::InvalidConfig(format!(
                    "appended data file '{}' has negative record_count {}",
                    file.path, file.record_count
                )));
            }
        }

        Ok(())
    }
}

impl DeleteFileInfo {
    /// Create a new delete-file info with a relative path.
    ///
    /// # Panics
    /// Panics if `delete_count` is negative.
    pub fn new(path: impl Into<String>, file_size_bytes: i64, delete_count: i64) -> Self {
        assert!(
            delete_count >= 0,
            "delete_count must be non-negative, got {delete_count}"
        );
        Self {
            path: path.into(),
            path_is_relative: true,
            file_size_bytes,
            footer_size: None,
            delete_count,
        }
    }

    /// Set the footer size for read optimization.
    pub fn with_footer_size(mut self, footer_size: i64) -> Self {
        self.footer_size = Some(footer_size);
        self
    }

    /// Mark this delete file as having an absolute path.
    pub fn with_absolute_path(mut self) -> Self {
        self.path_is_relative = false;
        self
    }
}

/// Result of a write operation.
#[derive(Debug)]
pub struct WriteResult {
    /// Snapshot ID of the write operation
    pub snapshot_id: i64,
    /// Table ID (may be newly created)
    pub table_id: i64,
    /// Schema ID (may be newly created)
    pub schema_id: i64,
    /// Number of files written
    pub files_written: usize,
    /// Total records written
    pub records_written: i64,
}

/// The ids actually committed by `register_data_file` / `publish_snapshot`.
///
/// On multicatalog Postgres all metadata is written at the commit point, so the
/// committed `snapshot_id` is assigned there and the `schema_id`/`table_id` are
/// the real committed ids (which may differ from the begin-time reservations in
/// [`WriteSetupResult`] if a concurrent writer created the schema/table first).
/// Callers should use these for the authoritative result rather than the
/// begin-time reservations.
#[derive(Debug, Clone, Copy)]
pub struct CommitIds {
    /// Snapshot id assigned at commit (the new catalog head for this write).
    pub snapshot_id: i64,
    /// Committed schema id.
    pub schema_id: i64,
    /// Committed table id.
    pub table_id: i64,
}

/// Result of a transactional write setup operation.
#[derive(Debug)]
pub struct WriteSetupResult {
    /// Snapshot ID created for this write
    pub snapshot_id: i64,
    /// The catalog head observed at `begin_write_transaction` (the base for
    /// `Replace` conflict detection), threaded back to the commit step. If a
    /// concurrent writer committed a newer generation of the table since this base
    /// — i.e. any data file or column with `begin_snapshot`/`end_snapshot > base`
    /// — the commit aborts with [`crate::DuckLakeError::Conflict`]. Both backends
    /// now share this model: snapshot ids are assigned at *commit* (single-catalog
    /// SQLite `MAX(snapshot_id)+1`; multicatalog Postgres a plain `IDENTITY`
    /// insert), so per-catalog id order == commit order and the scalar
    /// `> base` test is exact.
    pub base_snapshot_id: i64,
    /// Schema ID (may be newly created)
    pub schema_id: i64,
    /// Table ID (may be newly created)
    pub table_id: i64,
    /// Column IDs in order
    pub column_ids: Vec<i64>,
}

/// Trait for writing metadata to DuckLake catalogs.
///
/// Implementations must be thread-safe (`Send + Sync`).
pub trait MetadataWriter: Send + Sync + std::fmt::Debug {
    /// Create a new snapshot and return its ID.
    fn create_snapshot(&self) -> Result<i64>;

    /// Get or create a schema, returning `(schema_id, was_created)`.
    fn get_or_create_schema(
        &self,
        name: &str,
        path: Option<&str>,
        snapshot_id: i64,
    ) -> Result<(i64, bool)>;

    /// Get or create a table, returning `(table_id, was_created)`.
    fn get_or_create_table(
        &self,
        schema_id: i64,
        name: &str,
        path: Option<&str>,
        snapshot_id: i64,
    ) -> Result<(i64, bool)>;

    /// Set columns for a table, returning assigned column IDs.
    /// Ends existing columns using end_snapshot pattern for time travel.
    fn set_columns(
        &self,
        table_id: i64,
        columns: &[ColumnDef],
        snapshot_id: i64,
    ) -> Result<Vec<i64>>;

    /// Promote (widen) an existing column's type in place — DuckLake schema
    /// evolution, distinct from a data write (which *rejects* type changes; see
    /// [`MetadataWriter::begin_write_transaction`]).
    ///
    /// In a single transaction: validate the change is a lossless widening
    /// ([`crate::types::is_promotable`]), create a new snapshot, retire the live
    /// `ducklake_column` row (set its `end_snapshot`), and insert a new row with
    /// the **same `column_id`**, the new `column_type`, and `begin_snapshot` = the
    /// new snapshot. The stable `column_id` keeps Parquet field-ids valid, so
    /// files written before and after both resolve to their snapshot's version
    /// (the read path casts old narrow values up to the widened type). Returns the
    /// new snapshot id.
    ///
    /// Default impl errors — backends that don't support promotion yet return
    /// [`crate::DuckLakeError::InvalidConfig`].
    fn promote_column_type(
        &self,
        _table_id: i64,
        _column_name: &str,
        _new_ducklake_type: &str,
    ) -> Result<i64> {
        Err(DuckLakeError::InvalidConfig(
            "promote_column_type is not supported on this metadata backend".to_string(),
        ))
    }

    /// Register a new data file and publish its snapshot as the catalog head,
    /// atomically. For `Replace`, retires the prior generation in the same
    /// transaction. Returns the committed snapshot id: assigned at this commit
    /// for SQLite (so it may differ from `WriteSetupResult::snapshot_id` under
    /// concurrency), reserved at begin for Postgres.
    ///
    /// `columns` / `column_ids` describe the snapshot's column generation (in
    /// `column_order`, ids matching `WriteSetupResult::column_ids`). Backends
    /// that finalize columns in `begin_write_transaction` (multicatalog
    /// Postgres) ignore them; single-catalog backends (SQLite) defer the
    /// column generation to this commit and use them to insert the column rows.
    ///
    /// `base_snapshot` is the catalog head observed at `begin_write_transaction`
    /// ([`WriteSetupResult::base_snapshot_id`]). For `Replace`, the commit aborts
    /// with [`crate::DuckLakeError::Conflict`] if any data file of the table has
    /// `begin_snapshot` or `end_snapshot` greater than `base_snapshot` — i.e.
    /// another writer published a newer generation since this write began — so
    /// concurrent replaces never silently union or clobber each other.
    ///
    /// `schema_name` / `table_name` identify the target. Multicatalog Postgres
    /// writes ALL metadata at this commit (the schema/table get-or-create happens
    /// here, keyed by these names) so it needs them; single-catalog SQLite already
    /// created the schema/table at begin and ignores them.
    /// Returns the [`CommitIds`] actually committed (the snapshot id assigned at
    /// commit, and the real schema/table ids — which may differ from the
    /// begin-time reservations if a concurrent writer created them first).
    #[allow(clippy::too_many_arguments)]
    fn register_data_file(
        &self,
        table_id: i64,
        schema_name: &str,
        table_name: &str,
        snapshot_id: i64,
        file: &DataFileInfo,
        mode: WriteMode,
        base_snapshot: i64,
        columns: &[ColumnDef],
        column_ids: &[i64],
    ) -> Result<CommitIds>;

    /// Commit a table-scoped DML metadata mutation under one visible snapshot.
    ///
    /// `base_snapshot` is the snapshot used to plan target data-file positions
    /// or selected retirements. Backends must fail closed with
    /// [`crate::DuckLakeError::Conflict`] if any target file/delete generation no
    /// longer matches that base. Appends of unrelated new data files commute, but
    /// appended file metadata included in this mutation must become visible in the
    /// same snapshot as the delete/retire operations.
    ///
    /// Object files are expected to be written before this metadata call. If this
    /// call fails, metadata must remain unchanged; prewritten objects are orphaned
    /// for cleanup rather than referenced by a partial snapshot.
    fn commit_table_mutation(
        &self,
        _table_id: i64,
        _schema_name: &str,
        _table_name: &str,
        _base_snapshot: i64,
        _mutation: &TableMutation,
    ) -> Result<CommitIds> {
        Err(DuckLakeError::InvalidConfig(
            "commit_table_mutation is not supported by this metadata writer".to_string(),
        ))
    }

    /// Register positional delete files for one table in a single catalog commit,
    /// superseding any prior live delete file for each affected data file (at
    /// most one is live per data file).
    ///
    /// In one transaction, abort with [`crate::DuckLakeError::Conflict`] if any
    /// target `data_file_id` is no longer live for this table (a concurrent
    /// Replace/compaction retired it since `base_snapshot`, invalidating the
    /// resolved positions) or if any currently-live delete file no longer matches
    /// its expected previous id (a concurrent delete on that file won the race).
    /// A concurrent *append* that only adds other files does NOT conflict — it
    /// never moves rows in the files being masked. Otherwise retire each prior
    /// delete file and insert each new cumulative delete file under one snapshot.
    ///
    /// Default implementation delegates to [`MetadataWriter::commit_table_mutation`].
    fn set_delete_files(
        &self,
        table_id: i64,
        schema_name: &str,
        table_name: &str,
        base_snapshot: i64,
        deletes: &[DeleteFileMutation],
    ) -> Result<CommitIds> {
        let mut mutation = TableMutation::new();
        mutation.delete_files.extend_from_slice(deletes);
        self.commit_table_mutation(table_id, schema_name, table_name, base_snapshot, &mutation)
    }

    /// Register one positional delete file. Backends implement this through
    /// [`MetadataWriter::set_delete_files`] so callers can migrate from the
    /// legacy single-file API without changing semantics.
    #[allow(clippy::too_many_arguments)]
    fn set_delete_file(
        &self,
        table_id: i64,
        schema_name: &str,
        table_name: &str,
        _snapshot_id: i64,
        data_file_id: i64,
        expected_prev_delete_file: Option<i64>,
        base_snapshot: i64,
        delete: &DeleteFileInfo,
    ) -> Result<CommitIds> {
        self.set_delete_files(
            table_id,
            schema_name,
            table_name,
            base_snapshot,
            &[DeleteFileMutation::new(data_file_id, expected_prev_delete_file, delete.clone())],
        )
    }

    /// Publish a write's snapshot as the catalog head with no data file (CREATE
    /// TABLE, zero-row Replace). For `Replace`, retires the prior generation.
    /// See [`MetadataWriter::register_data_file`] for the parameters.
    ///
    /// Default no-op. Backends that advance the head in
    /// `begin_write_transaction` could rely on it, but both shipped backends
    /// override: multicatalog Postgres writes the snapshot/schema/table/column
    /// metadata and inserts the `ducklake_catalog_snapshot_map` head row, and
    /// SQLite (which defers the `ducklake_snapshot` row insert out of
    /// `begin_write_transaction`) inserts the snapshot row + column generation here.
    #[allow(clippy::too_many_arguments)]
    fn publish_snapshot(
        &self,
        _table_id: i64,
        _schema_name: &str,
        _table_name: &str,
        _snapshot_id: i64,
        _mode: WriteMode,
        _base_snapshot: i64,
        _columns: &[ColumnDef],
        _column_ids: &[i64],
    ) -> Result<CommitIds> {
        Ok(CommitIds {
            snapshot_id: _snapshot_id,
            schema_id: 0,
            table_id: _table_id,
        })
    }

    /// End all existing data files for a table. Returns count of files ended.
    fn end_table_files(&self, table_id: i64, snapshot_id: i64) -> Result<u64>;

    /// Get the data path from catalog metadata.
    fn get_data_path(&self) -> Result<String>;

    /// Set the data path in catalog metadata.
    fn set_data_path(&self, path: &str) -> Result<()>;

    /// Initialize DuckLake schema tables if they don't exist.
    fn initialize_schema(&self) -> Result<()>;

    /// Atomically set up catalog metadata for a write operation.
    /// Creates snapshot, schema, table, columns in a single transaction.
    /// If mode is `WriteMode::Replace`, ends existing data files.
    fn begin_write_transaction(
        &self,
        schema_name: &str,
        table_name: &str,
        columns: &[ColumnDef],
        mode: WriteMode,
    ) -> Result<WriteSetupResult>;

    /// Prepare to write a data file for an already-existing table without
    /// publishing or creating any catalog metadata.
    ///
    /// This is the setup half of
    /// [`DuckLakeTableWriter::write_pending_data_file`](crate::DuckLakeTableWriter::write_pending_data_file):
    /// it returns the live table id, current conflict base, and stable column ids
    /// to bake into the staged parquet file. Unlike
    /// [`MetadataWriter::begin_write_transaction`], implementations must fail if
    /// the schema/table does not already exist and must reject schema changes;
    /// pending object files are later made visible only through
    /// [`MetadataWriter::commit_table_mutation`].
    fn begin_data_file_staging(
        &self,
        schema_name: &str,
        table_name: &str,
        columns: &[ColumnDef],
    ) -> Result<WriteSetupResult>;

    /// The catalog id this writer is scoped to, when the backend has a notion
    /// of catalogs (multicatalog Postgres). Single-catalog backends (SQLite)
    /// return `None`, which keeps `DuckLakeTableWriter` from inserting a
    /// per-catalog directory segment into newly-written file paths and so
    /// preserves today's `{data_path}/{schema}/{table}/…` layout.
    fn catalog_id(&self) -> Option<i64> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DuckLakeError;

    #[test]
    fn test_column_def_new() {
        let col = ColumnDef::new("test_col", "int32", true).unwrap();
        assert_eq!(col.name, "test_col");
        assert_eq!(col.ducklake_type, "int32");
        assert!(col.is_nullable);
    }

    #[test]
    fn test_column_def_new_valid_types() {
        // Various valid type strings should be accepted
        assert!(ColumnDef::new("a", "int32", true).is_ok());
        assert!(ColumnDef::new("b", "varchar", false).is_ok());
        assert!(ColumnDef::new("c", "boolean", true).is_ok());
        assert!(ColumnDef::new("d", "float64", true).is_ok());
        assert!(ColumnDef::new("e", "decimal(10,2)", true).is_ok());
        assert!(ColumnDef::new("f", "timestamp", true).is_ok());
        assert!(ColumnDef::new("g", "date", true).is_ok());
        assert!(ColumnDef::new("h", "bigint", true).is_ok());
        assert!(ColumnDef::new("i", "text", true).is_ok());
    }

    #[test]
    fn test_column_def_new_invalid_type_rejected() {
        let result = ColumnDef::new("col", "not_a_type", true);
        assert!(result.is_err());
        match result {
            Err(DuckLakeError::UnsupportedType(msg)) => {
                assert_eq!(msg, "not_a_type");
            },
            other => panic!("Expected UnsupportedType error, got {:?}", other),
        }
    }

    #[test]
    fn test_column_def_new_empty_type_rejected() {
        let result = ColumnDef::new("col", "", true);
        assert!(result.is_err());
        match result {
            Err(DuckLakeError::UnsupportedType(_)) => {},
            other => panic!("Expected UnsupportedType error, got {:?}", other),
        }
    }

    #[test]
    fn test_column_def_from_arrow() {
        let col = ColumnDef::from_arrow("id", &DataType::Int64, false).unwrap();
        assert_eq!(col.name, "id");
        assert_eq!(col.ducklake_type, "int64");
        assert!(!col.is_nullable);
    }

    #[test]
    fn test_data_file_info_new() {
        let file = DataFileInfo::new("test.parquet", 1024, 100);
        assert_eq!(file.path, "test.parquet");
        assert!(file.path_is_relative);
        assert_eq!(file.file_size_bytes, 1024);
        assert_eq!(file.record_count, 100);
        assert!(file.footer_size.is_none());
    }

    #[test]
    fn test_data_file_info_with_footer_size() {
        let file = DataFileInfo::new("test.parquet", 1024, 100).with_footer_size(256);
        assert_eq!(file.footer_size, Some(256));
    }

    #[test]
    fn test_data_file_info_with_absolute_path() {
        let file = DataFileInfo::new("/absolute/path.parquet", 1024, 100).with_absolute_path();
        assert!(!file.path_is_relative);
    }

    #[test]
    fn test_column_def_empty_name_rejected() {
        let result = ColumnDef::new("", "int32", true);
        assert!(result.is_err());
        match result {
            Err(DuckLakeError::InvalidConfig(msg)) => {
                assert!(msg.contains("empty"), "Expected 'empty' in: {msg}");
            },
            other => panic!("Expected InvalidConfig, got {:?}", other),
        }
    }

    #[test]
    fn test_column_def_control_char_name_rejected() {
        let result = ColumnDef::new("col\0name", "int32", true);
        assert!(result.is_err());
        match result {
            Err(DuckLakeError::InvalidConfig(msg)) => {
                assert!(
                    msg.contains("control character"),
                    "Expected 'control character' in: {msg}"
                );
            },
            other => panic!("Expected InvalidConfig, got {:?}", other),
        }
    }

    #[test]
    fn test_column_def_from_arrow_empty_name_rejected() {
        let result = ColumnDef::from_arrow("", &DataType::Int64, false);
        assert!(result.is_err());
        match result {
            Err(DuckLakeError::InvalidConfig(msg)) => {
                assert!(msg.contains("empty"), "Expected 'empty' in: {msg}");
            },
            other => panic!("Expected InvalidConfig, got {:?}", other),
        }
    }

    #[test]
    fn test_column_def_from_arrow_control_char_rejected() {
        let result = ColumnDef::from_arrow("col\nnewline", &DataType::Int64, false);
        assert!(result.is_err());
        match result {
            Err(DuckLakeError::InvalidConfig(msg)) => {
                assert!(
                    msg.contains("control character"),
                    "Expected 'control character' in: {msg}"
                );
            },
            other => panic!("Expected InvalidConfig, got {:?}", other),
        }
    }

    #[test]
    fn test_validate_name_valid() {
        assert!(validate_name("users", "Table").is_ok());
        assert!(validate_name("my_column", "Column").is_ok());
        assert!(validate_name("Schema123", "Schema").is_ok());
        assert!(validate_name("a", "Column").is_ok());
    }

    #[test]
    fn test_validate_name_empty() {
        let result = validate_name("", "Table");
        assert!(result.is_err());
        let result = validate_name("   ", "Table");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_name_control_chars() {
        // Null byte
        assert!(validate_name("col\0", "Column").is_err());
        // Newline
        assert!(validate_name("col\n", "Column").is_err());
        // Tab
        assert!(validate_name("col\t", "Column").is_err());
        // DEL (0x7F)
        assert!(validate_name("col\x7F", "Column").is_err());
    }

    #[test]
    fn test_validate_name_length_limit() {
        // Exactly at limit should succeed
        let at_limit = "a".repeat(MAX_NAME_LENGTH);
        assert!(validate_name(&at_limit, "Table").is_ok());

        // One over should fail
        let over_limit = "a".repeat(MAX_NAME_LENGTH + 1);
        assert!(validate_name(&over_limit, "Table").is_err());
    }

    #[test]
    fn test_column_def_long_name_rejected() {
        let long_name = "x".repeat(MAX_NAME_LENGTH + 1);
        let result = ColumnDef::new(long_name, "int32", true);
        assert!(result.is_err());
        match result {
            Err(DuckLakeError::InvalidConfig(msg)) => {
                assert!(
                    msg.contains("exceeds maximum length"),
                    "Expected 'exceeds maximum length' in: {msg}"
                );
            },
            other => panic!("Expected InvalidConfig, got {:?}", other),
        }
    }

    #[test]
    fn test_data_file_info_zero_record_count() {
        let file = DataFileInfo::new("empty.parquet", 0, 0);
        assert_eq!(file.record_count, 0);
    }

    #[test]
    #[should_panic(expected = "record_count must be non-negative")]
    fn test_data_file_info_negative_record_count_panics() {
        DataFileInfo::new("test.parquet", 1024, -1);
    }
}
