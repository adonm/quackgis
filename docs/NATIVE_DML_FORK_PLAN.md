# Native DuckLake DML design

QuackGIS now carries a vendored `datafusion-ducklake` fork snapshot with an
atomic table-mutation metadata API. Autocommit `DELETE` and `UPDATE` use that
path: they resolve live DuckLake row lineage `rowid`s for the SQL predicate,
write one cumulative positional delete file per affected data file, and publish
all metadata changes under one catalog snapshot. `UPDATE` also stages the
replacement rows as a pending data file before that metadata commit.

Explicit transactions still use the safe staged/full-table rewrite path until
QuackGIS can batch those workflows through the same native mutation model.
Bucket-scoped compaction is wired to the native mutation path when row-lineage
planning succeeds, with the previous full-table rewrite retained as a fallback.

## Fork primitives available and tested

The vendored fork is based on `adonm/datafusion-ducklake` `117c0c5` and adds the
QuackGIS atomic mutation surface:

- `DuckLakeTable::resolve_positions(...)` scans a data file and returns physical
  row positions matching a DataFusion physical predicate. Fork oracles exercise
  it; the current QuackGIS SQL path uses row-lineage `rowid` projection instead.
- `DuckLakeTableWriter::write_delete_file(...)` writes standard positional
  delete Parquet files with `(file_path, pos)` rows.
- `MetadataWriter::set_delete_file(...)` registers one cumulative delete file for
  one data file with compare-and-swap checks on the live data file and previous
  delete-file generation.
- `MetadataWriter::set_delete_files(...)` registers multiple cumulative delete
  files for one table in a single metadata transaction/snapshot. SQLite and
  multicatalog PostgreSQL both implement it. These lower-level registration APIs
  remain available/tested; QuackGIS runtime publication uses `TableMutation`.
- `TableMutation` plus `MetadataWriter::commit_table_mutation(...)` commits
  positional delete-file generations, appended data-file metadata, and selected
  data-file retirements under one snapshot. Stale target files/delete generations
  fail closed with a conflict; failed metadata commits leave only prewritten
  object files as cleanup orphans.
- `DuckLakeTableWriter::write_pending_data_file(...)` writes a Parquet data file
  with the table's DuckLake field ids and returns `DataFileInfo` without creating
  catalog metadata. It requires an existing same-schema table and the pending
  object becomes visible only if a later `commit_table_mutation(...)` registers
  it.
- Fork tests cover the primitive path over multi-row-group files, appended files,
  schema evolution, cumulative delete generations, stale conflicts, and
  delete/append plus retire/append table mutations and pending-file visibility
  (`tests/positional_delete_oracle_tests.rs`).

## QuackGIS adoption boundary

Do **not** implement native DML as a sequence of independent `set_delete_file`
and append commits. That would be faster on success but would expose partial
results if the process dies between per-file commits or between delete and
append. QuackGIS only enables the native SQL path where the fork provides one
visible snapshot boundary.

The current native autocommit DML and bucket-compaction paths publish this in one
atomic commit:

1. one or more positional delete-file generations, each with its expected prior
   delete-file id;
2. for `UPDATE` or bucket compaction, one pending replacement data file
   containing the updated/compacted rows;
3. a live-data-file / previous-delete-file conflict check for each affected file;
4. one resulting catalog head.

### Physical-position invariant

Positional delete correctness depends on physical file row order. Row-lineage
scans therefore run in a dedicated mode that disables repartitioning, filter
pruning, and sort/pushdown transformations that could renumber positions. This is
intentionally less optimized than an ordinary SELECT. Performance work must keep
that isolation explicit rather than reusing plans whose row order is only
logically equivalent.

## Current integration shape

The metadata commit and pending data-file primitives now exist. The current
QuackGIS runtime path is:

```text
read current snapshot + live files with row_id_start/max_row_count
register a snapshot-pinned DuckLake catalog with row lineage enabled
SELECT rowid ... WHERE <predicate>
map each rowid into (data_file_id, physical_position)
merge prior delete positions and write cumulative pending delete files
for UPDATE/compaction, write_pending_data_file(...)
build TableMutation with expected prior file/delete generations
commit_table_mutation(table_id, schema, table, base_snapshot, mutation)
```

The final commit covers new data files as well as delete files. Autocommit
`UPDATE` and bucket-local compaction are wired; the object-writing surface stays
inside the fork so QuackGIS does not duplicate DuckLake field-id or
catalog-layout rules.

## Current SQL adoption

1. Autocommit `DELETE` uses native delete files. Unsupported row-lineage planning
   fails closed to the previous full-table rewrite path; catalog conflicts remain
   errors so callers can retry.
2. `DELETE ... RETURNING` collects returning rows before the mutation and
   publishes the native delete only after the returning query succeeds.
3. Autocommit `UPDATE` collects replacement rows at the same snapshot as the
   row-lineage plan, writes them as one pending data file, and commits that file
   with the matching positional delete files under one snapshot. `UPDATE ...
   RETURNING` preserves the current pre-commit returning-row collection.
4. Explicit transactions stay on the existing staged-table path until the fork can
   batch multiple table mutations under one commit model.
5. Bucket-local compaction collects rows in one `(_qg_time_bucket,
   _qg_space_bucket)` group, writes one sorted replacement data file, and masks
   the old positions in the same mutation commit. If native row-lineage planning
   fails, it falls back to the prior whole-table replacement path.

## Remaining work

1. Run the local before/after-commit process-kill matrix in Kind and managed-service
   profiles.
2. Promote the explicit offline orphan quarantine flow to Kind and managed
   profiles, then add restore-point-backed permanent-deletion proof for live,
   scheduled, and history-referenced objects.
3. Batch explicit transactions through native mutations only if one visible
   snapshot, conflict behavior, `RETURNING`, and edit-client semantics remain
   equivalent to the staged fallback.
4. Measure real edit/compaction traces across fragmented files, cumulative delete
   generations, and concurrent writers.
5. Add reference-reader gates and migrate to upstream deletion-vector/Puffin
   primitives when they preserve the same atomic boundary.

## Evidence gates

- QuackGIS pgwire test `ducklake_delete_uses_atomic_native_delete_files_across_data_files`
  proves a SQL `DELETE` spanning two data files writes two delete files under one
  snapshot and preserves exact survivors.
- QuackGIS pgwire test `ducklake_update_uses_atomic_native_delete_and_append`
  proves SQL `UPDATE` masks old rows across two files and appends replacement
  rows in the same mutation snapshot.
- QuackGIS pgwire test `ducklake_compact_table_accepts_layout_bucket_scope`
  proves bucket compaction masks bucket rows across two source files, appends one
  replacement file, and retires no whole data files.
- Fork oracle test `atomic_delete_commits_multiple_files_in_one_snapshot` proves
  the backend API commits multiple affected files together.
- Fork oracle tests `atomic_mutation_deletes_and_appends_in_one_snapshot`,
  `atomic_mutation_retires_and_appends_in_one_snapshot`, and
  `stale_atomic_mutation_conflict_leaves_no_metadata_commit` prove mixed
  mutation atomicity and fail-closed stale-generation behavior on SQLite.
- Fork oracle tests `pending_data_file_becomes_visible_only_via_atomic_mutation`
  and `pending_data_file_requires_existing_table_without_metadata_leak` prove
  staged replacement rows have no catalog visibility until the atomic mutation
  snapshot publishes them, and failed staging setup creates no schema/table rows.
- The PostgreSQL/S3-like Kind oracle is integration evidence, not production-scale
  DML proof. Repeat native DELETE/UPDATE/COMPACT failure and performance gates on
  managed services before production claims.
- Local QuackGIS tests `ducklake_native_delete_failpoint_before_commit_leaves_catalog_unchanged`,
  `ducklake_native_update_failpoint_before_commit_leaves_catalog_unchanged`, and
  `ducklake_native_compact_failpoint_before_commit_leaves_catalog_unchanged`
  inject aborts after native object prewrites and before `commit_table_mutation`,
  proving no catalog data-file/delete-file rows become visible for those
  boundaries.
- Six Unix `process_lifecycle` cases kill the actual server at private filesystem
  barriers before and after commit for native `DELETE`, `UPDATE`, and explicit
  bucket compaction. Before commit, the generated Parquet set exactly equals the
  age-gated offline inventory, restart exposes old rows, and an explicit retry
  reaches the intended state. After commit, backdated generated paths are absent
  from inventory and restart exposes the exact new state without replay. This is a
  local SQLite oracle, not generic mutation idempotency or Kind/managed-service
  evidence.
- `orphan_inventory::quarantine_requires_explicit_apply_and_stays_outside_live_prefix`
  proves the local quarantine operator path is opt-in, age-gated, refuses
  destinations inside the live data prefix, copies candidates outside live data,
  and leaves referenced DuckLake Parquet files in place.
- Extend the same process-kill probes to Kind and managed-service profiles; no
  committed snapshot may expose partial deletes, duplicate update rows, or lost
  bucket-compaction rows. The drill ladder and evidence packet are documented in
  `docs/MUTATION_FAILURE_DRILLS.md`.
- Prove `RETURNING`, QGIS keyless `_quackgis_rowid`, GeoServer WFS-T, and OGR
  edit traces still match the current rewrite semantics.
