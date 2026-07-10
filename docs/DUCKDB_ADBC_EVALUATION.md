# DuckDB/ADBC evaluation plan

This plan records the unreleased architecture pivot to DuckDB as the DuckLake
compatibility anchor and storage authority. Today QuackGIS still contains a Rust
pgwire service over DataFusion, SedonaDB, and `datafusion-ducklake`, but there is
no released storage contract to preserve. The decision is to iterate faster by
making DuckDB + official DuckLake the canonical writer/query substrate and keeping
QuackGIS focused on pgwire/PostGIS compatibility.

The evaluation exists because storage compatibility is the weakest current design
boundary: the PostgreSQL/S3 DuckLake profile is QuackGIS/`datafusion-ducklake`-
specific until a standard implementation can read it or a tested migration/export
path exists.

## Decision principles

1. **DuckDB-authored storage is the target.** Do not spend release-grade effort
   making old preview catalogs readable unless it directly informs export or
   compatibility tests.
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

Legacy QuackGIS writes data through the current path only as a compatibility
oracle. A separate DuckDB process, accessed by CLI or ADBC, loads the official
`ducklake` extension, attaches a copied catalog and object prefix, and verifies
tables.

This gate remains useful for proving that old QuackGIS-written catalogs are not the
future format. New positive storage claims should be earned from DuckDB-authored
catalogs.

Required checks:

- table discovery and schema comparison;
- counts and representative samples;
- bbox/extent parity for spatial columns;
- `DELETE`, `UPDATE`, and compaction output after native mutation;
- snapshot/time-travel behavior where DuckDB exposes it;
- geometry/geography bytes and visible type identity;
- failure behavior for QuackGIS-only metadata/functions.

### 2. DuckDB storage-authority kernel

Introduce a storage-authority path that sends DDL, Arrow/batch ingestion, mutation,
compaction, and metadata inspection to DuckDB while keeping QuackGIS's
pgwire/catalog/policy layer unchanged. Start out-of-process for speed and crash
isolation; move to ADBC or embedded DuckDB after semantics are proven.

The experiment is acceptable only if every step can be verified through DuckDB
reopen/reference reads and maintained pgwire client traces. Side-by-side comparison
against the old backend is useful, but not a release blocker because old catalogs
are unreleased preview artifacts.

Proof requirements:

- local SQLite/filesystem and PostgreSQL/S3 profiles;
- COPY/INSERT throughput with Arrow batches;
- concurrent writers and stale-writer conflicts;
- process-kill drills before and after DuckDB/DuckLake commit;
- orphan/prewrite behavior and cleanup evidence;
- exact PostGIS-client traces unchanged at pgwire;
- clear rollback to the current preview backend during the spike, followed by a
  delete/demote decision once DuckDB-backed storage passes the local and client
  gates.

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

### 4. Full DuckDB-backed engine pivot

This is now the preferred unreleased target. It becomes the only supported storage
path once the DuckDB authority probe, minimal pgwire route, maintained clients, and
external PostgreSQL/S3 profile pass without weakening mutation safety.

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
   just duckdb-engine-probe
   ```

   Set `DUCKDB_BIN=/path/to/duckdb` to use a locally downloaded CLI without
   changing the QuackGIS runtime or dependency graph.

   This command is optional because the repo does not pin DuckDB as a QuackGIS
   runtime dependency. It writes `.tmp/duckdb-engine/README.md` and fails closed if
   the DuckDB CLI is unavailable or the extensions cannot load.
1. **DuckDB attach smoke:** copied local catalog/object prefix opens in DuckDB with
   official `ducklake`; record DuckDB version, extension version, command/ADBC path,
   and exact failure if it cannot attach.
2. **DuckDB authority vertical slice:** run DuckDB as the writer against an
   official DuckLake catalog and prove create/insert/update/delete/reopen/snapshot
   behavior:

   ```sh
   just duckdb-authority-probe
   ```

   This writes `.tmp/duckdb-authority/README.md` and
   `.tmp/duckdb-authority/manifest.json`.
3. **Read parity:** compare schemas, counts, samples, extents, and geometry bytes
   for untouched COPY/INSERT tables.
4. **Mutation parity:** repeat after QuackGIS native `DELETE`, `UPDATE`, and
   compaction; reject partial or stale reads.
5. **External profile parity:** run the same checks against the managed
   PostgreSQL/S3 copied-prefix Alpha packet.
6. **Writer experiment:** prototype DuckDB/ADBC writes behind an explicit feature
   flag or separate binary; compare result catalogs with the current backend.
7. **Decision record:** choose one of: keep current backend with DuckDB reference
   gate, add DuckDB as optional writer, migrate storage authority to DuckDB, or
   document export/migration as the compatibility path.

## Evidence record template

```text
DuckDB version:
DuckLake extension version/source:
Connection path: CLI | ADBC | other
Storage profile: local-sqlite | postgresql-s3-compatible
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
present and consistent. It accepts both legacy `local-sqlite`/`postgresql-s3-
compatible` profiles and the new DuckDB-authored `duckdb-local-ducklake` profile.

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

The current recommendation is a direct DuckDB storage-authority pivot. DuckDB's
official DuckLake extension is both the writer target and the named reference gate
for QuackGIS storage claims. The old `datafusion-ducklake` writer remains only as a
preview/comparison path while QuackGIS has no release contract to migrate. A
QuackGIS DuckDB extension should wait until the DuckDB-backed pgwire route exposes
specific missing functions or metadata helpers.

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

## DuckDB-authority local result — 2026-07-10

The first DuckDB-authored official DuckLake vertical slice passed with DuckDB
`v1.5.2 (Variegata) 8a5851971f`:

```sh
DUCKDB_BIN=.tmp/bin/duckdb-v1.5.2/duckdb just duckdb-authority-probe
```

Validated artifact: `.tmp/duckdb-authority/README.md`.

The probe created a new official DuckLake catalog, disabled data inlining, created
`public.points` with WKB and `_qg_*` layout columns, inserted three rows, updated
one row, deleted one row, inspected DuckLake table metadata, reopened the catalog,
ran an exact WKB spatial predicate, and listed snapshots. All six checks passed:

- `create_insert`;
- `mutation`;
- `metadata`;
- `reopen`;
- `spatial_wkb`;
- `snapshot_metadata`.

Interpretation: the fastest ideal-architecture path is now validated at the storage
authority layer. Next work should route a minimal QuackGIS pgwire workflow to this
DuckDB-authored path instead of investing in the unreleased `datafusion-ducklake`
writer.

The same artifact was also accepted by the reference-reader packet checker under
the `duckdb-local-ducklake` profile:

```sh
just duckdb-reference-evidence-check \
  manifest=.tmp/duckdb-reference/authority-manifest.json \
  out=.tmp/duckdb-reference/authority-README.md
```

That packet reported `duckdb_reference_readable` with 7/7 checks passed. This is
the first positive storage-interoperability claim, and it comes from DuckDB-authored
storage rather than QuackGIS-authored preview catalogs.
