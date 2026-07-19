# Official DuckLake alignment

QuackGIS creates new storage only through DuckDB's official `ducklake` extension.
It does not implement or vendor a DuckLake metadata writer.

N0 will select and build the exact DuckDB/DuckLake/Spatial/QuackGIS source set as
one bundle while preserving this authority. Patch queues may expose metadata or a
narrow reviewed lifecycle hook; they do not create a separate format or writer.
See [NATIVE_BUNDLE.md](./NATIVE_BUNDLE.md).

## Current profile

- local DuckLake catalog path;
- local Parquet data path;
- data inlining disabled for predictable external files;
- exact pinned DuckDB/extension versions;
- atomic QuackGIS storage-authority marker before attach; and
- ADBC write/query plus independent DuckDB reopen evidence.

Remote catalog URLs and URI data paths fail closed today.

## Read-only fan-out evidence

`deploy/quackgis/compose.multi.yaml` adds a separate fixture-scale validation
profile for immutable local publication. One DuckDB process seeds an official
DuckLake catalog and external Parquet data volume, exits, and two independent
DuckDB/Quack workers attach that same storage `READ_ONLY`. Two PostgreSQL edges
and four concurrent sessions prove equal snapshot, file-set, geometry, extent,
and bounded-viewport results while the worker mounts reject writes.

This does not change the current authority model and does not close the shared
profile gate below. There is still one writer authority, and it is stopped
before readers start.

## Interoperability contract

A storage claim requires a version-matched independent DuckDB process to discover
schemas and reproduce counts, representative values, WKB bytes, spatial results,
and snapshot state after write, mutation, maintenance, restart, backup/restore,
and upgrade as applicable.

QuackGIS APIs must not expose private DuckLake metadata-table layouts. Snapshot
and maintenance controls should wrap stable official functions or return explicit
unsupported errors.

`just duckdb-catalog-identity-test` proves that official DuckLake table IDs/UUIDs
and Parquet field IDs are durable across supported table/column rename and reopen,
while drop/recreate gets a new table identity. They are suitable keys for a
PostgreSQL compatibility registry. They cannot be exposed directly as PostgreSQL
OIDs.

QuackGIS has selected a public identity API instead of a private metadata-table
adapter. `ducklake_column_info(catalog)` returns qualified schema/table names, IDs
and UUIDs plus top-level logical column names and IDs from the pinned committed
DuckLake snapshot. This deliberately omits views, nested child fields, uncommitted
DDL, types, and nullability; existing public SQL metadata provides non-identity
attributes. The tracked 1.5 patch is pinned to DuckDB 1.5.4, passes the complete
DuckLake function-test group, loads against the accepted QuackGIS driver ABI, and
passes the QuackGIS lifecycle gate. `PINNED_DUCKLAKE.md` records exact source,
patch, build, artifact, and trust-boundary pins.

Local 1.0 packages that artifact as an explicit QuackGIS support obligation while
upstreaming remains the deletion path. The patch is read-only and the official
DuckLake code remains the only metadata/data writer. QuackGIS does not depend on
hidden metadata tables or expose the attachment name to clients.

N0 must reproduce this identity contract before replacing the dedicated build
lane. S0 then tests official CRS-aware geometry type fidelity through DuckLake;
only demonstrated metadata loss can justify another narrow patch. Q0 may request
a writer validation hook only after defining enforceable key semantics. DuckLake
metadata-only key declarations do not become PostgreSQL key claims.

## Authority and migration

One data root has one writer authority. `_quackgis/storage-authority-v1` prevents
accidental reuse by an incompatible writer. It is not a migration mechanism.

Historical QuackGIS catalogs are unsupported inputs unless an actual persistence
obligation is identified. Any future migration must copy into a separate
DuckDB-authored root and verify schema, counts, WKB, checksums, declared snapshot
treatment, and rollback. Alternating writers is prohibited.

## Shared profile gate

Using PostgreSQL as the shared DuckLake metadata catalog with object storage is a
post-Local-1.0 milestone. This is distinct from the PostgreSQL-compatible
`pg_catalog` client surface. Enabling it requires official DuckLake support plus
measured multi-process visibility,
conflict/indeterminate-commit handling, credentials, authority, throttling,
backup/restore, cleanup, and independent-reader evidence. Emulator wiring alone is
not a product claim.

## Upgrade policy

DuckDB and both official extensions upgrade as one tested bundle. Mixed or
unverified versions fail startup. Upgrade evidence must include old-catalog reopen,
new writes, exact spatial checks, backup/restore, and rollback before changing the
supported bundle.
