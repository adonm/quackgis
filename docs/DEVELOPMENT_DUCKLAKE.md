# Development DuckLake identity extension

QuackGIS production and default development startup load the signed DuckLake
extension installed by `scripts/bootstrap_duckdb.py`. The override described here
exists only to unblock C2/C3 catalog development while the public
`ducklake_column_info(catalog)` API is upstreamed. It is not a release-supported
storage profile.

## Verified source and artifact

The current workspace-local fork is `.tmp/ref/ducklake` with these exact pins:

| Input | Pin |
|---|---|
| Upstream DuckLake branch/base | `v1.5-variegata` at `84ef2d14a0161f6f6197d6c8d2b4dbc45bf40375` |
| Original prototype | `adonm/ducklake` commit `b9648457e65991b2b4de1793ca077d9536af6fcd` |
| 1.5 port | `26ffcc91b3f91d51cc349eec1965a05dab2195d8` |
| DuckDB submodule pin | `v1.5.4` at `08e34c447bae34eaee3723cac61f2878b6bdf787` |
| 1.5 API fix/final source | `4096657cf9a16b853594e9eefb4b114a11a7c0c6` |
| Built extension SHA-256 | `046e73c864b4403e73beddc39addc72a370dfbe633e2287181a1c0cdd37b5b94` |

The recorded artifact is
`.tmp/ref/ducklake/build/release/extension/ducklake/ducklake.duckdb_extension`.
It was built with GCC 15.2.1, CMake 4.3.3, Ninja 1.13.2, and workspace-local
vcpkg commit `f87344cac03158cbf1467264565f1fd36b382a24`. The binary digest is evidence
for this artifact, not a promise that builds in different absolute paths or
build environments are byte-reproducible.

The local fork branch contains three focused commits and has not been pushed:

```text
4096657c fix: port column identity to DuckDB 1.5
5716167f build: pin DuckDB 1.5.4
26ffcc91 feat: expose DuckLake column identity
```

## Build and verify

Initialize the pinned submodules and a workspace-local vcpkg checkout, then build
only the loadable extension and its test runner:

```sh
git -C .tmp/ref/ducklake submodule update --init duckdb extension-ci-tools
git -C .tmp/ref/ducklake/duckdb checkout 08e34c447bae34eaee3723cac61f2878b6bdf787

git clone --depth 1 https://github.com/microsoft/vcpkg.git .tmp/ref/vcpkg
.tmp/ref/vcpkg/bootstrap-vcpkg.sh -disableMetrics

cd .tmp/ref/ducklake
mise x aqua:Kitware/CMake@4.3.3 aqua:ninja-build/ninja@1.13.2 -- \
  sh -c 'GEN=ninja VCPKG_TOOLCHAIN_PATH="$(realpath ../vcpkg/scripts/buildsystems/vcpkg.cmake)" make release'
build/release/test/unittest test/sql/functions/ducklake_column_info.test
build/release/test/unittest 'test/sql/functions/*'
sha256sum build/release/extension/ducklake/ducklake.duckdb_extension
```

The verified run passed 59 focused assertions and 143 assertions in the complete
DuckLake function-test group. The loadable binary also loaded in the official
DuckDB 1.5.4 CLI and returned identity for a newly created DuckLake table.

Run the QuackGIS lifecycle contract only with an explicit path and expected
digest:

```sh
export QUACKGIS_DEV_DUCKLAKE_EXTENSION="$PWD/.tmp/ref/ducklake/build/release/extension/ducklake/ducklake.duckdb_extension"
export QUACKGIS_DEV_DUCKLAKE_EXTENSION_SHA256=046e73c864b4403e73beddc39addc72a370dfbe633e2287181a1c0cdd37b5b94
mise exec -- just duckdb-development-ducklake-test
```

That gate covers exact output schema, empty and nested columns, view exclusion,
committed-snapshot behavior during uncommitted DDL, rollback, table and column
rename identity, added columns, reopen, and drop/recreate identity. It also
exercises the development C2 registry through autocommit and explicit commit:
fixed `public` namespace identity, allocated schema/relation OIDs, durable
attribute numbers, monotonic schema epoch, retained tombstones, restart, and a
non-public schema.

## Development C2 registry

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
allocating against the same state. Every public development write/create path
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
advances the schema epoch for create, rename, add, drop, and recreate; direct
pgwire relation references to the control schema are rejected, as are dynamic
`query`/`query_table` indirection and direct `ducklake_column_info` calls.

The selected API emits no row for an empty schema, so a standalone empty schema
cannot receive a durable namespace OID or advance this identity epoch. Pgwire
does not support `CREATE SCHEMA`; non-public schema evidence creates a table in
the same transaction. QuackGIS will not substitute an unstable name-based OID:
empty-schema support requires an upstream durable schema-identity surface.

The same development lane now projects current base tables into `pg_class`,
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

Broader expression provenance, durable empty-schema identity, `reg*` resolution,
constraints/indexes/defaults/comments, and REST cache consumers remain later C3
slices.

## Runtime trust boundary

Both override values are required. Before initializing DuckDB, QuackGIS requires
an absolute non-symlink regular-file path, a 64-character lowercase SHA-256, and
an exact file digest match. Only this explicit policy sets DuckDB's
`allow_unsigned_extensions=true`, and bootstrap loads that exact DuckLake path.
The official signed `spatial` extension remains load-only from the isolated
DuckDB home. Default startup never permits unsigned extensions.

A DuckDB extension is native code in the QuackGIS process. Keep the artifact and
all parent directories outside untrusted write access, do not replace it after
validation, and use a disposable development data root. Never use this override
in an image, deployment manifest, release profile, or production catalog.

## Exit and deletion plan

Delete the override, its integration test, and this document once an official
version-matched DuckLake bundle exposes the accepted public function and passes
the same lifecycle gate. Upstream acceptance remains a Local 1.0 release gate.
If upstream declines the API, continuing beyond development requires a separate
architecture and long-term fork-support decision; the temporary override does not
make that decision implicitly.
