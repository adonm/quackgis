# DuckDB/ADBC evaluation plan

This plan records and validates the selected migration to DuckDB as QuackGIS's
query/storage engine and official DuckLake as its storage authority. Today
QuackGIS is still a Rust pgwire service over DataFusion, SedonaDB, and
`datafusion-ducklake`; the Rust pgwire/PostGIS/control-plane contract remains while
the center is replaced through the D0–D5 gates in `ROADMAP.md`.

The evaluation exists because storage compatibility is the weakest current design
boundary: the PostgreSQL/S3 DuckLake profile is QuackGIS/`datafusion-ducklake`-
specific until a standard implementation can read it or a tested migration/export
path exists.

## Decision principles

1. **DuckDB-authored official DuckLake is the target.** Compatibility is proven at
   each migration gate; legacy QuackGIS catalogs are comparison/export inputs, not
   the future write format.
2. **pgwire remains a QuackGIS responsibility.** QGIS, GeoServer, OGR, Martin,
   pgjdbc, and PostgreSQL drivers observe PostgreSQL wire/catalog behavior that
   DuckDB does not provide.
3. **PostGIS compatibility is not delegated blindly.** DuckDB spatial support or a
   QuackGIS DuckDB extension may implement functions, but QuackGIS must still
   preserve advertised OIDs, metadata views, error behavior, and client traces.
4. **A mutation still publishes once.** Any DuckDB-backed writer must preserve the
   one-visible-snapshot boundary for DML, COPY, compaction, and maintenance.
5. **The durable byte contract stays WKB/EWKB until a stronger interop contract is
   proven.** DuckDB `GEOMETRY` identity may be useful, but it cannot silently
   replace maintained client encodings or geography limits.

## Candidate architectures

### 1. Reference-reader gate

QuackGIS writes data through the current path. A separate DuckDB process, accessed
by CLI or ADBC, loads the official `ducklake` extension, attaches a copied catalog
and object prefix, and verifies tables.

This is the lowest-risk path and should become mandatory before stronger DuckLake
claims.

Required checks:

- table discovery and schema comparison;
- counts and representative samples;
- bbox/extent parity for spatial columns;
- `DELETE`, `UPDATE`, and compaction output after native mutation;
- snapshot/time-travel behavior where DuckDB exposes it;
- geometry/geography bytes and visible type identity;
- failure behavior for QuackGIS-only metadata/functions.

### 2. Experimental DuckDB/ADBC storage kernel

Introduce an internal storage-kernel experiment that sends DDL, Arrow ingestion,
mutation, compaction, and metadata inspection to DuckDB through ADBC while keeping
QuackGIS's pgwire/catalog/policy layer unchanged.

The experiment is acceptable only if it runs side-by-side with the current backend
and compares every result against both QuackGIS reads and the DuckDB reference gate.

Proof requirements:

- local SQLite/filesystem and PostgreSQL/S3 profiles;
- COPY/INSERT throughput with Arrow batches;
- concurrent writers and stale-writer conflicts;
- process-kill drills before and after DuckDB/DuckLake commit;
- orphan/prewrite behavior and cleanup evidence;
- exact PostGIS-client traces unchanged at pgwire;
- clear rollback to the current `datafusion-ducklake` backend.

### 3. QuackGIS DuckDB extension

Build a DuckDB extension only after the reference gate identifies concrete gaps.
Prefer SQL macros, views, and small scalar/table functions before custom storage or
planner behavior.

Likely responsibilities:

- missing PostGIS-compatible `ST_*` aliases or wrappers;
- `postgis_version()` and compatibility helper functions;
- `geometry_columns`, `geography_columns`, `spatial_ref_sys`, and snapshot helper
  views where DuckDB cannot expose enough metadata directly;
- MVT helper functions if DuckDB spatial output is insufficient;
- QuackGIS maintenance/introspection functions.

The extension should not own pgwire auth, RowDescription OIDs, PostgreSQL catalog
emulation, cursor/portal state, COPY protocol, RBAC, metrics, or release-evidence
policy. Those remain in the QuackGIS server.

### 4. Full DuckDB-backed engine migration

This is the selected target, not an assumption that parity already exists. DuckDB
becomes the default only after the ADBC/storage, query, spatial/client, operations,
and cutover gates preserve mutation safety, client compatibility, and release
evidence while reducing fork burden and improving standard DuckLake
interoperability.

The pivot decision must explicitly compare:

- DataFusion/SedonaDB spatial execution versus DuckDB spatial plus QuackGIS gaps;
- Rust-native dependency simplicity versus native DuckDB/extension packaging;
- crash isolation in-process versus subprocess/sidecar DuckDB;
- current fork maintenance versus DuckDB extension maintenance;
- managed-service concurrency and backup/restore behavior.

## Minimal proof ladder

0. **Engine smoke:** run DuckDB out-of-process with the `spatial` and `ducklake`
   extensions and prove WKB spatial round-trips through DuckDB before touching
   QuackGIS storage:

   ```sh
   mise install
   mise run duckdb-bootstrap
   just duckdb-engine-probe
   ```

   `mise.toml`/`mise.lock` pin the CLI. The bootstrap verifies the official
   `libduckdb` release checksums and preinstalls DuckDB-signed, version-matched
   extensions under ignored `.tmp/duckdb`.

   The CLI and native library remain development/evaluation dependencies rather
   than part of the default server runtime. The probe writes
   `.tmp/duckdb-engine/README.md`, uses `LOAD` only, and fails closed if the pinned
   CLI or preinstalled extensions are unavailable.
1. **DuckDB attach smoke:** copied local catalog/object prefix opens in DuckDB with
   official `ducklake`; record DuckDB version, extension version, command/ADBC path,
   and exact failure if it cannot attach.
2. **Read parity:** compare schemas, counts, samples, extents, and geometry bytes
   for untouched COPY/INSERT tables.
3. **Mutation parity:** repeat after QuackGIS native `DELETE`, `UPDATE`, and
   compaction; reject partial or stale reads.
4. **External profile parity:** run the same checks against the managed
   PostgreSQL/S3 copied-prefix Alpha packet.
5. **Writer migration:** promote DuckDB/ADBC writes behind an explicit feature flag;
   compare result catalogs with the current backend and prohibit mixed writers.
6. **Cutover record:** document capability parity/limits, legacy export/import,
   packaged versions, rollback evidence, and the gates permitting DuckDB to become
   the default.

## Evidence record template

```text
DuckDB version:
DuckLake extension version/source:
Connection path: CLI | ADBC | other
Storage profile: local-sqlite | duckdb-local-ducklake | postgresql-s3-compatible
Catalog/object source SHA:
Dataset rows/files/bytes:
Attach result:
Schema/count/sample parity:
Spatial byte/type result:
Mutation/compaction result:
Snapshot/time-travel result:
Unsupported QuackGIS metadata behavior:
Decision: reference-readable | non-standard | export-required | writer-candidate
Rollback/migration implication:
```

Before using a reference-reader result in a roadmap or release packet, write a
manifest and validate it:

```sh
just duckdb-reference-evidence-check \
  manifest=.tmp/duckdb-reference/manifest.json \
  out=.tmp/duckdb-reference/README.md
```

The checker is a static packet gate. It does not run DuckDB; it prevents a packet
from claiming `duckdb_reference_readable` unless DuckDB/extension versions,
connection path, dataset counts, required parity checks, and the final decision are
present and consistent.

Minimal manifest shape:

```json
{
  "source_sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  "claim": "duckdb_reference_readable",
  "storage_profile": "postgresql-s3-compatible",
  "reader": {
    "duckdb_version": "1.5.0",
    "ducklake_extension": "ducklake bundled 2026-07",
    "connection_path": "adbc",
    "command_evidence": "interop/duckdb-adbc.log"
  },
  "dataset": {
    "description": "copied external alpha catalog/prefix",
    "catalog_source": "postgres://user:<redacted>@db.example/quackgis_copy",
    "object_prefix": "s3://bucket/quackgis-alpha-copy",
    "row_count": 1000,
    "file_count": 12,
    "object_bytes": 1048576
  },
  "checks": {
    "attach": { "status": "pass", "evidence": "interop/attach.log" },
    "schema": { "status": "pass", "evidence": "interop/schema.log" },
    "read_samples": { "status": "pass", "evidence": "interop/read.log" },
    "spatial_bytes": { "status": "pass", "evidence": "interop/spatial.log" },
    "mutation_compaction": { "status": "pass", "evidence": "interop/mutation.log" },
    "unsupported_metadata": { "status": "pass", "evidence": "interop/metadata.log" },
    "snapshot_time_travel": { "status": "pass", "evidence": "interop/snapshot.log" }
  },
  "decision": "reference_readable"
}
```

## Current stance

The roadmap now selects DuckDB as the target query/storage engine and its official
DuckLake extension as the storage authority. The Rust pgwire/PostGIS/control-plane
edge remains. Migration proceeds side-by-side through the D0–D5 capability,
storage, query, spatial/client, operations, and cutover gates in `ROADMAP.md`; the
current backend remains the comparison/rollback oracle until parity is proven.

## Initial local result — 2026-07-10

The first local probe used a downloaded DuckDB CLI under ignored `.tmp/bin`:

- DuckDB version: `v1.5.2 (Variegata) 8a5851971f`.
- `just duckdb-engine-probe` passed when run with `DUCKDB_BIN=.tmp/bin/duckdb`:
  DuckDB loaded `spatial` and `ducklake`, wrote WKB from WKT, read it back with
  `ST_GeomFromWKB`, and evaluated `ST_Intersects`.
- A QuackGIS `just preview-smoke` local SQLite/filesystem DuckLake catalog did
  **not** attach through DuckDB's official DuckLake extension, even using the
  documented `ducklake:sqlite:` catalog URI. DuckDB failed while reading the latest
  snapshot metadata:

  ```text
  Binder Error: Failed to query most recent snapshot for DuckLake: Referenced column
  "next_catalog_id" not found in FROM clause!
  Candidate bindings: "snapshot_id", "snapshot_time"
  ```

Validated packet summary: `.tmp/duckdb-reference/README.md` reported
`decision=non_standard`, with attach failed before schema/read/spatial parity.

Interpretation: DuckDB's engine/spatial surface is viable enough for the next
probe, but QuackGIS-written DuckLake catalogs are not yet DuckDB-readable even in
the local profile. The first real improvement path is therefore storage-authority
compatibility: either make `datafusion-ducklake` write the allocator/catalog fields
DuckDB expects, provide a tested export/migration, or prototype DuckDB/ADBC as the
writer and keep QuackGIS as the pgwire/PostGIS compatibility layer.

## ADBC storage-kernel slice — 2026-07-11

The concrete allocator incompatibility above justified starting the side-by-side
writer experiment. The `duckdb-adbc` Cargo feature now exposes
`DuckDbAdbcStorage`, a synchronous internal boundary that:

- dynamically loads an operator-selected absolute `libduckdb` path using the
  `duckdb_adbc_init` entry point and ADBC 1.1;
- loads the official `ducklake` extension and attaches a caller-selected local
  catalog/data path (remote paths fail closed until the shared-profile authority
  and credential adapter exists);
- disables DuckLake data inlining so the existing DataFusion comparison reader is
  not silently presented with rows outside Parquet;
- ingests Arrow 58 `RecordBatch` streams using ADBC target catalog/schema/table
  options;
- executes query, DDL, and DML statements; and
- maps an ADBC transaction to one DuckLake snapshot, with rollback on callback
  failure, panic cleanup, fail-fast reentrant access, autocommit restoration, and
  connection quarantine when commit/rollback cleanup leaves the outcome unsafe.

The real-driver slice passes against project-pinned DuckDB/libduckdb 1.5.4:

```sh
mise install
mise run duckdb-bootstrap
mise exec -- just duckdb-adbc-storage-test
```

It creates an official local DuckLake through Arrow ingestion, preserves valid WKB,
runs an exact DuckDB spatial predicate, rejects reentrant connection use without a
deadlock, runs `UPDATE` and `DELETE` in one transaction, verifies exactly one new
snapshot, drops the connection, and reopens the catalog for another exact spatial
read. The feature-gated local server now promotes this kernel into the real
`--engine-backend=duckdb` CLI route for bounded structural pgwire reads/writes,
COPY, transactions, SCRAM/table policy, and restart. The default server still uses
`datafusion-ducklake`; shared PostgreSQL/S3 configuration, native cancellation/
streaming, catalog/PostGIS/client parity, concurrent process writers, crash drills,
spatial family metadata, and production multi-platform packaging remain blockers.

ADBC itself does **not** enforce DuckLake compatibility. It is the Arrow transport;
compatibility comes from routing all durable operations through DuckDB's official
`ducklake` extension and testing the resulting catalogs with independent readers.
The repository bootstrap verifies the official libduckdb archive and extracted
library checksums. DuckDB verifies official extension signatures, and the bootstrap
records the installed extension digests in `.tmp/duckdb/manifest.json`; all probes
then use fail-closed `LOAD` only. This closes the reproducible Linux x86_64 local
dependency gap, not D4: immutable production image artifacts, a supported platform
matrix, upgrade/mixed-version refusal, and clean-room deployment evidence remain.
The required pull-request CI bootstraps the same pinned artifacts and runs the real
ADBC storage, engine smoke, and independent authority probes after the fast gate.

### Engine-contract expansion

The D0 `engine_api` contract now uses direct Arrow 58 types rather than DataFusion
plans or catalogs for statement description, parameter batches, query schemas and
batches, table discovery, ingest disposition, snapshots, maintenance, and bounded
classified errors. The DuckDB ADBC implementation proves against the pinned native
runtime that it can:

- prepare and describe positional parameters and result schemas;
- bind a one-row Arrow parameter batch without SQL literal interpolation;
- preserve a result schema when a query returns zero rows;
- discover an official DuckLake table schema through ADBC;
- return typed official snapshot ids/timestamps;
- merge adjacent official DuckLake files without changing row results; and
- shut down ADBC before an independent DuckDB CLI reopens the same authored
  catalog and reproduces exact spatial counts.

The process-owned `ManagedDatabase` can also open independent connection/session
handles against its one attached official catalog. The native gate proves a second
session sees committed Arrow ingest, cannot see another session's uncommitted
mutation, and sees it immediately after commit. Each handle retains independent
busy/quarantine state. This is local same-process isolation, not shared-catalog or
multi-process writer evidence.

The native-code trust boundary now verifies the exact committed `libduckdb.so`
SHA-256 before loading it, then queries and requires exact DuckDB SQL runtime
version `v1.5.4` before claiming the data root or attaching DuckLake. A focused
unit oracle proves a modified library fails without creating the configured root.
This complements image-context provenance and prevents a path-only configuration
from silently selecting a different ABI/runtime.

This is now both a storage/query-kernel contract and a bounded local pgwire route,
not D2 completion. ADBC results remain materialized, while the server owns
independent per-client transactions and lazily encodes their Arrow batches through
the existing PostgreSQL OID layer. Native cancellation, broader parameter/COPY
types and options, catalog/PostGIS shims, maintained clients, and shared operation
remain open; the legacy branch still returns a DataFusion `SessionContext`.

The native gate also carries a first DuckDB layout oracle: WKB points and hidden
bbox columns are written through official DuckLake, a polygon-hole query proves
bbox candidates safely over-select exact results, `EXPLAIN` contains both hidden
filters and `ST_Intersects`, and reopen preserves the exact count. Time/Morton
maintenance and scale/scan budgets remain future D3 evidence.
