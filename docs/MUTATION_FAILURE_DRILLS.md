# Native mutation failure drills

Native DuckLake DML and bucket compaction must have one visible snapshot boundary:
either the entire delete/update/compaction mutation is visible, or none of it is.
Prewritten Parquet/delete objects may become cleanup candidates after a crash, but
catalog visibility must never show partial deletes, duplicate update rows, or lost
bucket rows.

This runbook defines the failure evidence still needed before QuackGIS can claim
production-grade native mutation hardening.

## Mutation boundary under test

| Operation | Current publish shape | Existing cheap evidence |
|---|---|---|
| Autocommit `DELETE` | positional delete files committed under one snapshot | `ducklake_delete_uses_atomic_native_delete_files_across_data_files` |
| Autocommit `DELETE` failpoint before commit + retry | prewritten delete objects, then injected abort before metadata commit; abort counter increments once; retry publishes intended delete | `ducklake_native_delete_failpoint_before_commit_leaves_catalog_unchanged` |
| Autocommit `UPDATE` | positional delete files + replacement data file committed under one snapshot | `ducklake_update_uses_atomic_native_delete_and_append` |
| Autocommit `UPDATE` failpoint before commit + retry | replacement data + delete objects prewritten, then injected abort before metadata commit; abort counter increments once; retry publishes intended update | `ducklake_native_update_failpoint_before_commit_leaves_catalog_unchanged` |
| Bucket compaction | old bucket rows masked + replacement data file committed under one snapshot | `ducklake_compact_table_accepts_layout_bucket_scope` |
| Bucket compaction failpoint before commit + retry | compacted data + delete objects prewritten, then injected abort before metadata commit; abort counter increments once; retry publishes compaction metadata without changing visible rows | `ducklake_native_compact_failpoint_before_commit_leaves_catalog_unchanged` |
| Full-table compaction | replacement table snapshot | `ducklake_compact_table_rewrites_without_changing_results` |

## Failure drill ladder

Run drills from local/fork oracle to external services. Keep object inventories,
catalog snapshots, QuackGIS logs, and `metrics.json` for each run.

| Drill | Fault injection point | Required invariant |
|---|---|---|
| Pending data write failure | before replacement file is registered | table is unchanged; no new catalog file rows |
| Delete-file write failure | after some delete files are written, before metadata commit | table is unchanged; written objects are orphan candidates only |
| Stale generation conflict | concurrent writer changes a target file/delete generation before commit | mutation fails closed; caller can retry; no partial metadata commit |
| Process kill before commit | after pending object writes, before `commit_table_mutation` returns | table is old state or new state, never mixed |
| Process kill after commit response loss | commit succeeds but client connection dies before observing success | retry is safe/idempotence is documented for the client workflow |
| Compaction fallback | native bucket planning fails and full replacement path is used | exact row count/bbox/query results unchanged |
| Cleanup quarantine | suspected orphans are moved out of prefix after restore point | representative reads still match before permanent deletion |

## Evidence commands

Cheap local gates that must stay green before destructive drills:

```sh
just check-fast
cargo test -p quackgis-server --test ducklake_persistence -- --nocapture
cargo test -p quackgis-server --test ducklake_persistence ducklake_native_delete_failpoint_before_commit_leaves_catalog_unchanged -- --nocapture
cargo test -p quackgis-server --test ducklake_persistence ducklake_native_update_failpoint_before_commit_leaves_catalog_unchanged -- --nocapture
cargo test -p quackgis-server --test ducklake_persistence ducklake_native_compact_failpoint_before_commit_leaves_catalog_unchanged -- --nocapture
```

The failpoint tests use in-process, one-shot native mutation failpoints at
`delete:before_commit:<schema.table>`, `update:before_commit:<schema.table>`, and
`compact:before_commit:<schema.table>`. They prove the local crash-boundary and
retry oracle: pending replacement data and delete files can be written before
`commit_table_mutation`, but no DuckLake catalog data-file/delete-file rows become
visible if the commit boundary aborts; rerunning the same mutation after the
one-shot fault publishes exactly the intended delete/update/compaction metadata.
Each injected abort increments `quackgis_native_mutation_aborts_total` exactly once;
the retry does not increment the abort counter again.

External-service drills must follow
[ALPHA_EXTERNAL_SERVICES.md](./ALPHA_EXTERNAL_SERVICES.md), use a disposable
catalog/object prefix, and include a restore point before cleanup.

## Acceptance packet

A mutation failure evidence packet should include:

- source SHA and QuackGIS image digest;
- storage profile: SQLite/local, Kind PostgreSQL/s3s-fs, or real PostgreSQL/S3;
- operation and injected fault point;
- catalog snapshot id before and after;
- object inventory diff before and after;
- representative row counts/bboxes/spatial query outputs;
- `quackgis_native_*_mutations_total`,
  `quackgis_native_mutation_aborts_total`, and `quackgis_compactions_total`
  metrics;
- cleanup/quarantine action and validation reads;
- explicit statement of whether the drill is local oracle evidence, Kind smoke, or
  real external-service evidence.

## Cleanup rule

Never delete suspected orphan objects from a live prefix without a restore point.
First quiesce writers, capture logs and inventory, copy/quarantine objects outside
the live prefix, rerun representative reads, and only then delete under the
platform retention policy.

## Future automation

The local before-commit failpoints are the first automated harness around the fork
`commit_table_mutation` boundary for native `DELETE`, `UPDATE`, and bucket
compaction, including one-shot retry after the injected failure. Extend them to
process-kill/retry, Kind PostgreSQL/s3s-fs, orphan quarantine, and finally real
external services before making production hardening claims. Until then, this
runbook keeps manual and external-service drills comparable and prevents
accidental production claims from ordinary successful DML tests.
