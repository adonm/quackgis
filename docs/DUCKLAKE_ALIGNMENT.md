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
