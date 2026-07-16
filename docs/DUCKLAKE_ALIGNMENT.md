# Official DuckLake alignment

QuackGIS creates new storage only through DuckDB's official `ducklake` extension.
It does not implement or vendor a DuckLake metadata writer.

## Current profile

- local DuckLake catalog path;
- local Parquet data path;
- data inlining disabled for predictable external files;
- exact pinned DuckDB/extension versions;
- atomic QuackGIS storage-authority marker before attach; and
- ADBC write/query plus independent DuckDB reopen evidence.

Remote catalog URLs and URI data paths fail closed today.

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
