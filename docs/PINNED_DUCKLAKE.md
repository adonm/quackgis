# Pinned DuckLake identity extension

This document describes the current executable DuckLake-only source/patch lane.
N0 will absorb or delete it under the atomic native bundle contract in
[NATIVE_BUNDLE.md](./NATIVE_BUNDLE.md). Until N0 passes equivalent source,
lifecycle, package, recovery, and rollback gates, the commands and digests here
remain authoritative.

Local 1.0 owns one narrowly patched DuckLake build until the public
`ducklake_column_info(catalog)` API is available upstream. The patch is read-only:
it exposes durable schema, table, and column identity from DuckLake's committed
snapshot and does not replace or modify DuckLake's writer, metadata, snapshot, or
data-file behavior. QuackGIS accepts only the tracked source/patch/tool pins and
one exact extension digest.

Stock DuckDB does not trust project-owned signing keys. The selected release
strategy therefore enables DuckDB's unsigned-extension setting only when this
paired path/digest policy is configured, loads the exact verified path during
bootstrap, and denies client `LOAD`/`INSTALL` statements. The runtime image keeps
the artifact and its parent directories read-only to the unprivileged process.

## Verified source and artifact

`native/bundle.json` and `patches/ducklake/series.json` are the machine-readable
authority. The former records source/core/artifact/toolchain identity; the latter
records the ordered patch and exact base/result Git trees:

| Input | Pin |
|---|---|
| Upstream DuckLake base | `v1.5-variegata` commit `84ef2d14a0161f6f6197d6c8d2b4dbc45bf40375` |
| Tracked patch SHA-256 | `2ad65d0c8cb89e06b21866eb58fb2acabe8db320d2afb7c99fdf815a81a26d76` |
| DuckDB submodule pin | `v1.5.4` at `08e34c447bae34eaee3723cac61f2878b6bdf787` |
| Built extension SHA-256 | `046e73c864b4403e73beddc39addc72a370dfbe633e2287181a1c0cdd37b5b94` |

The accepted artifact is built below
`.tmp/ref/quackgis-ducklake/build/release/extension/ducklake/ducklake.duckdb_extension`.
It was built with GCC 15.2.1, CMake 4.3.3, Ninja 1.13.2, and workspace-local
vcpkg commit `f87344cac03158cbf1467264565f1fd36b382a24`. That legacy separate-build
provenance remains explicit beside N0's shared candidate toolchain and cannot be
promoted to an accepted N0 bundle. The binary digest is evidence
for this artifact, not a promise that builds in different absolute paths or
build environments are byte-reproducible.

## Build and verify

Validate the tracked authority without network access, then build only the
loadable extension and its test runner from workspace-local checkouts:

```sh
mise exec -- just ducklake-pinned-source-check
mise exec -- just ducklake-pinned-build
```

The verified run passed 59 focused assertions and 143 assertions in the complete
DuckLake function-test group. The loadable binary also loaded in the official
DuckDB 1.5.4 CLI and returned identity for a newly created DuckLake table.

Run the QuackGIS lifecycle contract with the supported paired path and digest:

```sh
export QUACKGIS_DUCKLAKE_EXTENSION="$PWD/.tmp/ref/quackgis-ducklake/build/release/extension/ducklake/ducklake.duckdb_extension"
export QUACKGIS_DUCKLAKE_EXTENSION_SHA256=046e73c864b4403e73beddc39addc72a370dfbe633e2287181a1c0cdd37b5b94
mise exec -- just duckdb-pinned-ducklake-test
```

That gate covers exact output schema, empty and nested columns, view exclusion,
committed-snapshot behavior during uncommitted DDL, rollback, table and column
rename identity, added columns, reopen, and drop/recreate identity. It also
exercises the C2 registry through autocommit and explicit commit:
fixed `public` namespace identity, allocated schema/relation OIDs, durable
attribute numbers, monotonic schema epoch, retained tombstones, restart, and a
non-public schema.

## Catalog identity registry

When and only when this extension is selected, QuackGIS creates protected
`quackgis._quackgis` DuckLake tables for registry state, namespace OIDs, relation
and reserved row-type OIDs, and attribute numbers. Dynamic OIDs start at 100000,
above maintained built-in/spatial reservations; DuckLake `main` maps to
PostgreSQL `public` OID 2200. Dropped mappings are retained so names and attribute numbers cannot be
silently reused.

The selected function intentionally sees the pinned committed snapshot, not
uncommitted DDL. QuackGIS therefore cannot discover a new DuckLake UUID inside
the user DDL transaction. After a successful DuckLake commit and before reporting
success, it reconciles the now-committed identities in one separate registry
transaction. One process-wide commit lock covers the user commit and registry
commit across all server sessions, preventing supported concurrent writers from
allocating against the same state. Every public write/create path
uses this boundary. Startup repeats reconciliation to close a process-crash gap.
A rollback never allocates identity. If post-commit reconciliation fails,
QuackGIS reports that the user commit succeeded, quarantines the session, and
fatally closes an explicit pgwire transaction instead of returning a false
aborted-transaction state.

DuckLake 1.5 does not support primary-key or unique constraints. The state row
pins control format version 1, and QuackGIS checks the equivalent state-row, key,
global-OID, attribute-number, range, reference, and committed-snapshot coverage
invariants before/after reconciliation and fails closed on inconsistency. The gate
injects a duplicate mapping, requires the post-commit session to quarantine, and
requires restart rejection. A SHA-256 fingerprint of current identity names/IDs
plus DuckDB-reported column types, defaults, comments, nullability, and constraint
names advances the schema epoch for create, rename, add, drop, recreate, constraint,
and metadata changes; direct
pgwire relation references to the control schema are rejected, as are dynamic
`query`/`query_table` indirection and direct `ducklake_column_info` calls.

The selected API emits no row for an empty schema, so a standalone empty schema
cannot receive a durable namespace OID or advance this identity epoch. Pgwire
does not support `CREATE SCHEMA`; non-public schema evidence creates a table in
the same transaction. QuackGIS will not substitute an unstable name-based OID:
empty-schema support requires an upstream durable schema-identity surface.

The identity lane projects current base tables into `pg_class`,
current columns into `pg_attribute`, and one reserved composite row type per
table into `pg_type`. Namespace/relation/type/attribute references are joined
from the registry, `main` remains PostgreSQL `public`, dropped mappings stay
private tombstones, and unsupported DuckDB column types fail closed. Direct qualified or unambiguous table-column and plain wildcard RowDescription
fields carry the same relation OID/attribute number across maintained base-table
joins; expressions do not. PostgreSQL-reserved schema aliases and public
geometry/geography type-name collisions fail closed. Prepared reads pin the
schema epoch and fail `0A000` after a committed schema change. A guarded current-
column view also fails rather than returning partial rows if a read lands in the
user-commit/registry-reconciliation gap.

The lane also resolves one-/two-part quoted or unquoted `regclass`, `regtype`,
`regnamespace`, and `regrole` input against the maintained search path, including
nullable `to_reg*`, strict PostgreSQL SQLSTATEs, bound text input, explicit text
output, `format_type`, aliases/arrays/typmods, and OID casts. The exact
`pg18-column-core-v1` catalog descriptions execute through the same pgwire test.

The first traced structural slice projects normalized DuckLake column defaults
through `pg_attrdef` and table/column comments through `pg_description`.
`pg_get_expr`, `col_description`, and `obj_description` return PostgreSQL `text`,
while `adbin` retains PostgreSQL `pg_node_tree` wire identity. Effective-role
visibility is structurally intersected with the session login's legacy read/write
ceiling. DuckDB reports the implicit no-default marker as the string `NULL`; the
projection normalizes it to SQL NULL. An explicit `DEFAULT NULL` is therefore
semantically preserved as no default rather than distinguished as a separate
catalog row. Comment/default DDL is not added to the bounded pgwire statement
surface by this discovery slice.

DuckLake's one native constraint type, `NOT NULL`, receives a separate durable
OID registry keyed to durable table/column identity. Current constraints project
through PostgreSQL 18 `pg_constraint`; dropped mappings become inactive
tombstones, and table/column rename preserves the OID. `pg_index` is an empty
typed catalog and `pg_get_indexdef` returns NULL because DuckLake exposes no
primary, unique, foreign-key, check, or index implementation. QuackGIS does not
synthesize those semantics from names or data scans.

The lane also projects recognized geometry-family columns through a role-bound
`geometry_columns` compatibility view and advertises it with `spatial_ref_sys`
through `information_schema.tables`; those two public relation names are reserved
and fail identity validation if user tables collide with them. Because DuckLake columns do not enforce one
subtype, dimension, or integer SRID, every row is deliberately generic
`GEOMETRY`, dimension 2, SRID 0. `spatial_ref_sys` is typed but empty because no
authoritative maintained CRS registry exists. Focused actual-pgwire coverage
executes empty-geometry `ST_SRID`, DuckDB-backed version probes, and textual
`ST_Extent`/`ST_3DExtent` over stored WKB. SRID assignment, PostGIS box wire
types, and inferred CRS/subtype claims remain unsupported.

C3 implementation is complete in this pinned lane. Durable empty-schema identity
still needs upstream API support. Role semantics and role-aware information
schema also execute without identity enabled; authoritative
spatial typemod/CRS metadata, generated-column semantics, REST cache consumers, and broader expression
provenance remain C5 or later M3 slices.

## Runtime trust boundary

Both configuration values are required. Before initializing DuckDB, QuackGIS requires
an absolute non-symlink regular-file path, a 64-character lowercase SHA-256, and
an exact file digest match. Only this explicit policy sets DuckDB's
`allow_unsigned_extensions=true`, and bootstrap loads that exact DuckLake path.
The official signed `spatial` extension remains load-only from the isolated
DuckDB home. Startup without the pair never permits unsigned extensions or
publishes durable user-object identity.

A DuckDB extension is native code in the QuackGIS process. Release artifacts copy
the accepted binary into an immutable image owned outside UID 999. Non-image
operators must provide the same protection: all parent directories must be
outside untrusted write access and the file must not be replaced after startup
validation. Client SQL cannot select the path or digest and cannot execute
`LOAD` or `INSTALL`.

## Upgrade and deletion plan

QuackGIS owns ABI, source, artifact, lifecycle, and upgrade qualification for this
patch at every supported DuckDB/DuckLake bundle. N0's first migration gate is to
represent this base and patch in the common manifest/patch queue, build it against
the same central DuckDB as Spatial and QuackGIS-native code, and reproduce all
current tests. A candidate cannot replace the pin until upstream function tests,
QuackGIS identity lifecycle, native storage/pgwire workflows, independent reopen,
runtime image, backup/restore, and rollback gates pass. Delete the patch,
unsigned-extension policy, and dedicated builder once a version-matched official
extension or N0 replacement exposes the same accepted API and passes those gates.
