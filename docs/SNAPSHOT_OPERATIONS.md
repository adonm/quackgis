# Snapshot, rollback, and time-travel plan

DuckLake snapshots are the natural boundary for backup, rollback, staged imports,
and SQL time travel. QuackGIS exposes safe metadata inspection through
`ducklake_snapshots()`, `ducklake_table_info()`, and `ducklake_list_files()`. The
first pgwire-safe snapshot read path is implemented for simple single-table reads
using either `public.table(snapshot => <snapshot_id>)` or
`public.table(snapshot_id => <snapshot_id>)`. Parser-level SQL `AS OF`, protected
snapshots, timestamp resolution, positional table-function selectors, rollback
integration, and CDC row functions remain future work.

## Current supported surface

| Surface | Status | Evidence |
|---|---|---|
| Snapshot metadata inspection | ✅ available through pgwire | `ducklake_metadata_table_functions_roundtrip_through_wire` |
| Simple snapshot-pinned table read | ✅ available through pgwire with `snapshot`/`snapshot_id` named selectors and count/extent parity | `ducklake_snapshot_selector_reads_pinned_table` |
| Matched local backup/restore | ✅ local oracle | `ducklake_local_backup_restore_copy_roundtrip` |
| External backup/restore drill | ⏳ runbook, execution required | `docs/ALPHA_EXTERNAL_SERVICES.md` |
| SQL `AS OF` time travel | ⏳ pgwire PostgreSQL dialect rejects table-version syntax before hooks | current selector syntax plus future parser/query tests |
| Protected snapshots / retention | ❌ not implemented | future DuckLake API alignment |
| CDC row UDTFs | ❌ disabled | pgwire/Arrow projection must be fixed first |

## Current snapshot selector syntax

The current safe SQL path is intentionally narrow and parser-compatible with the
PostgreSQL wire parser. Snapshot ids must come from `ducklake_snapshots()`:

```sql
SELECT * FROM public.assets(snapshot => 12345) WHERE id = 7;
SELECT * FROM public.assets(snapshot_id => 12345) WHERE id = 7;
```

Current limits:

1. exactly one snapshot-qualified DuckLake table;
2. simple `SELECT` only;
3. no joins, DML, `COPY`, compaction, or writes;
4. snapshot id must be a literal numeric id from `ducklake_snapshots()`;
5. the table must exist at that snapshot or the query fails closed; and
6. the snapshot read uses a temporary snapshot-pinned DuckLake catalog for that
   query only.

This is a prototype time-travel read path, not protected retention. Operators
must still preserve the referenced catalog/object state through backups or future
protected snapshot APIs.

## Parser-level SQL `AS OF` target shape

QuackGIS should prefer a small PostGIS-like read syntax that is explicit and easy
to reject when unsupported. The upstream SQL parser has table-version AST forms,
but the PostgreSQL dialect used by the pgwire path currently rejects those forms
before QuackGIS hooks can route them:

```sql
SELECT * FROM public.assets AS OF SNAPSHOT 12345 WHERE id = 7;
SELECT * FROM public.assets AS OF TIMESTAMP '2026-07-09T12:00:00Z';
```

Future implementation requirements:

1. Parse only unambiguous single-table snapshot qualifiers first.
2. Resolve snapshot ids/timestamps through DuckLake metadata and fail closed if
   missing or ambiguous.
3. Register a snapshot-pinned DuckLake catalog for the query scope only.
4. Preserve exact SedonaDB spatial recheck and deny unsafe pruning rewrites that
   cannot be proven against the pinned snapshot.
5. Keep writes, DML, compaction, and `COPY` invalid inside `AS OF` statements.
6. Add pgwire tests for table discovery, count/bbox parity, and stale-snapshot
   behavior before documenting support.

## Rollback and restore guidance

Until protected snapshots and SQL rollback exist, rollback is operational:

1. Quiesce writers.
2. Identify the desired catalog/object-prefix backup set.
3. Restore PostgreSQL catalog and object prefix into an isolated environment.
4. Run read-only validation: table list, representative counts/bboxes, and spatial
   queries.
5. Promote by changing deployment secrets/DNS to the restored pair, not by editing
   live catalog rows manually.

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
