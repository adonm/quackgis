# Snapshot, rollback, and time-travel plan

DuckLake snapshots are the natural boundary for backup, rollback, staged imports,
and SQL time travel. QuackGIS exposes safe metadata inspection through
`ducklake_snapshots()`, `ducklake_table_info()`, and `ducklake_list_files()`. The
first pgwire-safe snapshot read path is implemented for simple single-table reads
using `AS OF SNAPSHOT <snapshot_id>`, `public.table(snapshot => <snapshot_id>)`,
`public.table(snapshot_id => <snapshot_id>)`, `AS OF TIMESTAMP '<RFC3339>'`, or
`public.table(snapshot_at => '<RFC3339>')`. A local rollback oracle now proves
that a matched catalog/object backup restores the recorded prior head after the
source advances. Protected snapshots, live SQL rollback, positional table-function
selectors, and CDC row functions remain future work.

## Current supported surface

| Surface | Status | Evidence |
|---|---|---|
| Snapshot metadata inspection | ✅ available through pgwire | `ducklake_metadata_table_functions_roundtrip_through_wire` |
| Simple snapshot-pinned table read | ✅ available through pgwire with id/timestamp `AS OF` and named selectors, count/extent parity, unknown-id rejection, and Prometheus success/error counters | `ducklake_snapshot_selector_reads_pinned_table` |
| Matched local rollback validation | ✅ local oracle proves prior head/current rows/simple-protocol `AS OF`/file references after source advancement | `ducklake_local_rollback_to_matched_backup_restores_prior_head` |
| External backup/restore drill | ⏳ runbook, execution required | `docs/ALPHA_EXTERNAL_SERVICES.md` |
| SQL `AS OF` time travel | ✅ snapshot-id and RFC3339 timestamp forms available for the same narrow simple-read path | `ducklake_snapshot_selector_reads_pinned_table` |
| Protected snapshots / retention | ❌ not implemented | future DuckLake API alignment |
| CDC row UDTFs | ❌ disabled | pgwire/Arrow projection must be fixed first |

## Current snapshot selector syntax

The current safe SQL path is intentionally narrow. Table-function forms are
PostgreSQL-parser compatible; both `AS OF` forms are tokenized and lowered before
the PostgreSQL dialect parser runs. Snapshot ids must come from
`ducklake_snapshots()`; timestamps resolve to the latest catalog snapshot at or
before the requested instant:

```sql
SELECT * FROM public.assets AS OF SNAPSHOT 12345 WHERE id = 7;
SELECT * FROM public.assets(snapshot => 12345) WHERE id = 7;
SELECT * FROM public.assets(snapshot_id => 12345) WHERE id = 7;
SELECT * FROM public.assets AS OF TIMESTAMP '2026-07-09T12:00:00Z';
SELECT * FROM public.assets(snapshot_at => '2026-07-09T12:00:00+00:00');
```

The PostgreSQL parser may accept positional `public.assets(12345)` as a generic
table function, but QuackGIS does not route it as a snapshot selector; planning
fails closed. It is not part of the supported surface.

Current limits:

1. exactly one snapshot-qualified DuckLake table;
2. simple `SELECT` only;
3. no joins, DML, `COPY`, compaction, or writes;
4. snapshot id must be a literal numeric id from `ducklake_snapshots()`, or the
   timestamp must be a literal RFC3339 instant with an explicit offset;
5. unknown ids, times before retained history, null/malformed catalog timestamps,
   and tables absent at the resolved snapshot fail closed;
6. equal catalog timestamps resolve deterministically to the greatest snapshot id;
7. catalog timestamps without offsets are interpreted as UTC because both
   maintained backends write `CURRENT_TIMESTAMP` into timezone-naive columns; and
8. the snapshot read uses a temporary snapshot-pinned DuckLake catalog for that
   query only.

This is a prototype time-travel read path, not protected retention. Successful
snapshot-catalog registrations increment `quackgis_snapshot_reads_total`; rejected
snapshot selectors increment `quackgis_snapshot_read_errors_total`. Operators must
still preserve the referenced catalog/object state through backups or future
protected snapshot APIs.

## SQL `AS OF` resolution

QuackGIS uses a small PostGIS-like read syntax that is explicit and easy to reject
when unsupported. The pgwire layer rewrites both literal forms before sqlparser
runs:

```sql
SELECT * FROM public.assets AS OF SNAPSHOT 12345 WHERE id = 7;
```

```sql
SELECT * FROM public.assets AS OF TIMESTAMP '2026-07-09T12:00:00Z';
```

Implementation requirements:

1. Parse only unambiguous literal single-table snapshot qualifiers. ✅
2. Resolve snapshot ids/timestamps through DuckLake metadata and fail closed on
   unknown ids or unusable timestamp metadata. ✅
3. Register a snapshot-pinned DuckLake catalog for the query scope only.
4. Preserve exact SedonaDB spatial recheck and deny unsafe pruning rewrites that
   cannot be proven against the pinned snapshot.
5. Keep writes, DML, compaction, and `COPY` invalid inside `AS OF` statements.
6. Cover count/bbox parity, unknown/future ids, timestamp tie-breaking, and times
   before retained history through pgwire. ✅

## Rollback and restore guidance

Until protected snapshots and SQL rollback exist, rollback is operational:

1. Quiesce writers.
2. Identify the desired catalog/object-prefix backup set.
3. Restore PostgreSQL catalog and object prefix into an isolated environment.
4. Run read-only validation: table list, representative counts/bboxes, and spatial
   queries.
5. Promote by changing deployment secrets/DNS to the restored pair, not by editing
   live catalog rows manually.

The local `ducklake_local_rollback_to_matched_backup_restores_prior_head` oracle
implements this validation shape with a SQLite catalog and filesystem data prefix:
it records a release snapshot, copies both durability boundaries, advances the
source, then proves the isolated restore has the prior head, current release rows,
simple-protocol `AS OF` parity, and referenced data files. This is deterministic
rollback integration evidence, not proof of provider backup coordination.

Do not delete current snapshots or objects as a rollback mechanism. Treat manual
catalog/object edits as unsupported unless performed in a copied environment.

## Protected snapshot target

Protected snapshots should cover:

- release cut points;
- backup start/end points;
- pre-compaction restore points;
- staged-import review points;
- external-service failure drills.

When DuckLake exposes stable protected snapshot/retention APIs, QuackGIS should
wrap them with explicit operations docs and a pgwire-visible metadata surface
rather than inventing separate retention metadata.

## CDC row table-function policy

CDC row UDTFs remain disabled. A previous attempt to expose row-level functions
through pgwire/Arrow projection failed with a projection-shape panic. Re-enable
only when all of these are true:

1. projection through pgwire is safe for the table-function schema;
2. row order, delete/update semantics, and snapshot bounds are documented;
3. tests cover simple and extended protocol projection;
4. clients can request a bounded snapshot range and receive deterministic rows;
5. unsupported shapes fail closed instead of truncating or misprojecting rows.

## Branch/merge and staged imports

DuckLake branch/merge should be the preferred foundation for future staged
imports, edit-review workflows, and release-style dataset publication. Until that
is stable, use ordinary staging tables and explicit copy/swap workflows documented
in `docs/OSM_POSTGIS_PARITY.md`.

## Completion criteria

Snapshot operations move from plan to claim only after the docs name the syntax,
gates, retention semantics, restore drill, and unsupported cases. Metadata UDTFs
alone are inspection evidence, not SQL time-travel support.
