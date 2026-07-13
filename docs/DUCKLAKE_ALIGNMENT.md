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

QuackGIS has selected an upstream public identity API instead of a private or
version-pinned metadata adapter. The proposed `ducklake_column_info(catalog)`
returns qualified schema/table names, IDs and UUIDs plus top-level logical column
names and IDs from the pinned committed DuckLake snapshot. This deliberately
omits views, nested child fields, uncommitted DDL, types, and nullability; existing
public SQL metadata can provide non-identity attributes. A prototype based on
upstream commit `d4a23e83cab5ff81d239a40c7891141c19c611cb` passes transaction,
rename, reopen, empty-column, nested-column, view, drop/recreate, and output-schema
tests. Catalog projection remains blocked until that contract is accepted
upstream and available in QuackGIS's pinned official extension bundle. QuackGIS
will not carry the prototype as a runtime patch or depend on hidden attachment
names.

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
