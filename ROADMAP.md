# Roadmap

This document owns the **forward product roadmap only**. Implemented contracts and
their current evidence live in [docs/ROADMAP_STATUS.md](./docs/ROADMAP_STATUS.md);
architecture and invariants live in [ARCHITECTURE.md](./ARCHITECTURE.md); fork
details live in [DIVERGENCE.md](./DIVERGENCE.md) and
[docs/DUCKLAKE_ALIGNMENT.md](./docs/DUCKLAKE_ALIGNMENT.md). Domain plans are
linked from milestones instead of repeated here.

A milestone is complete only when its user-visible outcome has run at the stated
scale and promotion ring, produced reviewable artifacts for the exact source SHA,
and updated the compatibility, operations, and release-evidence records. Code,
design documents, and runbooks are prerequisites—not completion evidence.

## Mission

**QuackGIS is the PostGIS-compatible SQL and control plane for an open spatial
lakehouse.** The roadmap moves the primary query and storage engine from
DataFusion/SedonaDB/`datafusion-ducklake` to DuckDB with its official `ducklake`
extension. The Rust service remains the pgwire, PostGIS-compatibility, security,
catalog-emulation, and operations boundary. Platform teams should keep city,
regional, and national spatial data in standard DuckLake/Parquet while QGIS,
GeoServer, Martin, GDAL/OGR, PostgreSQL drivers, APIs, and BI tools continue to see
the QuackGIS contract rather than a raw DuckDB connection.

The goal is not a smaller PostgreSQL. It is a horizontally readable, operationally
recoverable spatial data plane where:

- object-store data feels like PostGIS to existing clients;
- DuckDB performs selective reads, broad OLAP, and official DuckLake writes using
  mature, upstream-maintained execution and storage primitives;
- parallel ingest and edit writers publish coherent official DuckLake snapshots;
- datasets can be inspected, protected, released, restored, and upgraded;
- raster, point-cloud, 3D, CAD/BIM, imagery, and reality-capture collections are
  queryable through trustworthy footprint, provenance, and sidecar indexes; and
- storage remains open enough to migrate with DuckLake rather than trapping data
  behind a QuackGIS-only service.

## Engine and storage direction

DuckDB is the target engine because it provides a more mature and stable embedded
OLAP core, a broad analytical SQL surface, first-party Arrow interchange, an
extensible extension ecosystem, and the official DuckLake implementation. Making
DuckDB author the catalog and snapshots is a stronger compatibility strategy than
continuing to reproduce DuckDB/DuckLake behavior in a young forked Rust storage
stack.

This is an architectural migration, not a license to discard QuackGIS behavior.
The target layer model is:

```text
QGIS / GeoServer / Martin / GDAL / PostgreSQL drivers / APIs / BI
                              │
Rust pgwire + QuackGIS compatibility and control plane
auth/TLS · PostgreSQL protocol/catalogs · COPY/cursors · PostGIS surface
authorization · query policy · snapshot/maintenance APIs · audit/metrics
                              │ Arrow / ADBC
DuckDB engine + pinned official extensions
SQL/OLAP · spatial · object-store connectors · official DuckLake
                              │
official DuckLake catalog + Parquet/delete objects
local catalog/data │ PostgreSQL catalog + object storage
```

ADBC is the preferred in-process Arrow boundary, but ADBC alone does not guarantee
DuckLake compatibility. Compatibility comes from routing durable operations
through DuckDB's official `ducklake` extension, pinning DuckDB/extension versions,
and independently reopening the resulting catalogs.

### Capability-preservation contract

Every current capability must be classified and tested before the old engine path
is removed:

| Capability | Target design |
|---|---|
| pgwire, TLS, SCRAM, simple/extended protocol, parameters, COPY, cursors/portals | Keep in Rust; translate validated operations to DuckDB and encode Arrow results with existing PostgreSQL OIDs/formats. |
| PostgreSQL/PostGIS catalogs and client shims | Keep the QuackGIS compatibility layer; derive real table/type metadata from DuckDB where possible and retain trace-driven synthetic surfaces only where necessary. |
| PostGIS SQL and exact spatial behavior | Use DuckDB `spatial` first; preserve WKB/EWKB and maintained function semantics with SQL rewrites, macros, or a small pinned QuackGIS DuckDB extension. Keep SedonaDB only as a temporary oracle or measured auxiliary path for gaps that cannot yet be closed. |
| DuckLake DDL, DML, snapshots, transactions, compaction, retention | Replace fork-owned metadata/file publication with DuckDB's official `ducklake` operations while preserving one-visible-snapshot behavior and QuackGIS administrative APIs. |
| Arrow batch ingest and query results | Use ADBC streams without row-wise conversion; retain current schema/nullability/decimal/time and WKB compatibility tests. |
| Hidden bbox/time/Morton layout and exact recheck | Preserve the logical optimization contract, but redesign implementation around DuckDB columns, statistics, partitioning, macros, and plans rather than DataFusion-specific rules. Candidate filters must remain conservative. |
| Time travel and metadata UDTFs | Keep stable QuackGIS SQL wrappers; reimplement them over official DuckLake snapshot and metadata functions. |
| Local and PostgreSQL/object-storage profiles | Recreate them as official DuckLake profiles. Local DuckDB catalogs remain single-client; shared/multi-process deployments use supported shared catalog and object-storage configurations. |
| Authorization, audit, metrics, evidence, backup/restore, orphan handling | Keep at the QuackGIS control-plane boundary and adapt instrumentation/runbooks to DuckDB/DuckLake failure and cleanup semantics. |
| Multi-modal sidecar inventories and maintained clients | Preserve unchanged at the SQL contract; re-run every client and lifecycle gate against the DuckDB path. |

Capabilities are not preserved merely because equivalent SQL exists. Preservation
requires the same maintained client trace, result semantics, failure boundary, and
promotion-ring evidence. A capability that cannot be retained must be explicitly
bounded in compatibility documentation before cutover.

### Migration rules

1. **No big-bang rewrite.** Keep the current engine as a comparison/rollback path
   behind an explicit backend selection until the DuckDB path passes each gate.
2. **No mixed storage authority.** One catalog is written by one backend. DuckDB-
   authored catalogs and legacy preview catalogs use separate roots; movement is a
   tested export/import or migration, never alternating writers.
3. **Preserve the edge, replace the center.** Pgwire, client compatibility,
   authorization, and operational APIs stay stable while engine-specific planning,
   metadata writing, mutation, and maintenance are replaced.
4. **Prefer DuckDB extension points over QuackGIS kernels.** Use SQL/macros/views,
   then small extension functions, before custom planner or storage code.
5. **Retire forks only after parity.** `datafusion-ducklake` and engine-specific
   Sedona/DataFusion code remain temporary oracles until equivalent DuckDB gates
   pass, then leave the default runtime and dependency graph.
6. **Fail closed on semantic gaps.** Unsupported functions, transaction shapes,
   types, or client behaviors must return explicit errors rather than silently
   changing results during migration.

## Product horizons

### 1.0 — regional spatial lakehouse

A platform team can deploy the DuckDB-backed QuackGIS service over managed
catalog/object storage, ingest and edit a real regional vector dataset, serve
maintained GIS/API clients, run budgeted DuckDB OLAP and spatial queries, recover
from failures, and upgrade or roll back without reading the source tree.

Target evidence includes routine 100M-feature runs, scheduled 1B-feature runs,
multiple stateless readers, concurrent writers, and a release packet with hard
limits and raw evidence. The soak gate is either one continuous 72-hour run or
three 24-hour runs on the same release candidate/profile; each must include mixed
reads/writes/maintenance, at least one controlled service disruption across the
set, no catalog reset, and the same correctness/budget assertions.

### 1.x — dataset release and maintenance control plane

Datasets become reviewable and publishable products: staged import, validation,
protected release references, promotion, rollback, retention, and maintained
tile/coverage summaries. Branch/merge and materialized-view primitives should come
from DuckLake when they satisfy QuackGIS correctness and interoperability gates.

### 2.x — multi-modal and national-scale spatial lakehouse

QuackGIS becomes the SQL/control plane for mixed vector and spatial-asset lakes:
real COG/raster, COPC/LAZ, 3D Tiles, CAD/BIM, aerial, and reality-capture
inventories with CRS/epoch/provenance fidelity. Manual and scheduled stress paths
should cover 10TB+ object inventories, scheduled billion-row evidence, and generated
trillion-class index/catalog limits without regressing simple-feature clients.

## Non-negotiable product invariants

1. **PostGIS is the interface, not the engine.** PostgreSQL-compatible behavior is
   implemented only where maintained workflows need it; DuckDB and official
   DuckLake become the canonical query/storage plane.
2. **Exact spatial results are authoritative.** Layout and candidate filters may
   over-select but may never replace the maintained exact predicate. DuckDB spatial
   results must be compared with existing SedonaDB/PostGIS oracles during migration.
3. **A mutation publishes once.** A SQL mutation or maintenance operation may
   prewrite orphan candidates, but partial catalog visibility is never acceptable.
4. **Claims name their evidence ring.** Local, Kind, managed-service, real-data,
   and release evidence are different claims.
5. **Client traces define compatibility.** New shims start as protocol/catalog
   surface fixtures, not client-name conditionals.
6. **Storage compatibility is explicit.** Standard DuckLake behavior and
   QuackGIS/fork-specific behavior must never be conflated in release claims.
7. **Heavy assets stay out of the SQL hot path.** SQL indexes identity, footprint,
   quality, CRS/epoch, provenance, and URIs; applications fetch source artifacts
   after candidate narrowing.
8. **The boring base stays cheap.** Every expensive scale or failure drill has a
   deterministic local/static companion gate.
9. **Extensions are pinned product dependencies.** DuckDB, `ducklake`, `spatial`,
   and any QuackGIS extension are versioned, packaged, provenance-checked, and
   upgraded through compatibility and recovery gates.

See [ARCHITECTURE.md](./ARCHITECTURE.md) for the design consequences of these
invariants.

## Current baseline

The implemented floor is deliberately summarized rather than replayed here:

The default runtime is still DataFusion + SedonaDB + `datafusion-ducklake`. A
feature-gated local DuckDB CLI backend now proves official DuckLake attach, Arrow
ingestion/COPY, structural simple and extended routing, parameterized reads and
mutations, transactions, SCRAM/table policy, snapshot inspection, restart, and
reopen against a real `libduckdb`. This remains bounded migration evidence, not
the default or shared production path. See
[docs/DUCKDB_ADBC_EVALUATION.md](./docs/DUCKDB_ADBC_EVALUATION.md).

| Evidence area | Authoritative record |
|---|---|
| Runtime, pgwire, PostGIS/client surface | [docs/COMPATIBILITY.md](./docs/COMPATIBILITY.md) |
| Closed contracts and active frontiers | [docs/ROADMAP_STATUS.md](./docs/ROADMAP_STATUS.md) |
| PostGIS claim tiers and regress results | [docs/POSTGIS_CONFORMANCE.md](./docs/POSTGIS_CONFORMANCE.md) |
| Native mutation and failure boundaries | [docs/NATIVE_DML_FORK_PLAN.md](./docs/NATIVE_DML_FORK_PLAN.md), [docs/MUTATION_FAILURE_DRILLS.md](./docs/MUTATION_FAILURE_DRILLS.md) |
| Snapshot/time-travel surface | [docs/SNAPSHOT_OPERATIONS.md](./docs/SNAPSHOT_OPERATIONS.md) |
| Spatial layout and benchmark evidence | [docs/DUCKLAKE_SPATIAL_LAYOUT.md](./docs/DUCKLAKE_SPATIAL_LAYOUT.md), [docs/ANALYTICS_BENCHMARKS.md](./docs/ANALYTICS_BENCHMARKS.md) |
| Storage-spec and fork alignment | [docs/DUCKLAKE_ALIGNMENT.md](./docs/DUCKLAKE_ALIGNMENT.md), [DIVERGENCE.md](./DIVERGENCE.md) |
| Security and operations floor | [docs/SECURITY_RBAC.md](./docs/SECURITY_RBAC.md), [docs/OPERATIONS.md](./docs/OPERATIONS.md) |
| Release artifact contract | [docs/RELEASE_EVIDENCE.md](./docs/RELEASE_EVIDENCE.md) |

## Promotion rings

| Ring | What it proves | What it does not prove |
|---|---|---|
| Local/DuckDB + legacy SQLite oracle | deterministic SQL, official DuckLake storage, restart, failure-injection, migration, and exact-result semantics | shared-service or provider behavior |
| Kind+Linkerd | multi-pod service shape, PostgreSQL/S3-like profile, mTLS visibility, budget plumbing, repeatable smoke scale | managed PostgreSQL failover, provider object-store semantics, production DR |
| Managed-service Alpha | real PostgreSQL/object storage, credential rotation, latency/throttling, backup/restore, cleanup, and restart behavior | regional client/product readiness by itself |
| Copied real-data Beta | maintained clients and analytics on representative city/regional datasets and edit histories | release upgrade and long-soak guarantees |
| Release | reproducible build, all required rings, upgrade/rollback, known limits, and attached artifacts | untested national/multi-modal claims |

## Cross-cutting release gates

Every Beta or release milestone must report the relevant gates, not merely link to
a workflow:

- **Correctness:** exact-vs-pruned equality, one-snapshot mutation visibility,
  stale-writer conflicts, restart persistence, and fail-closed unsupported shapes.
- **Compatibility:** workflow-level QGIS, GeoServer, OGR/GDAL, Martin, pgwire
  driver/API, and PostGIS-regress evidence with explicit supported versions.
- **Scale:** rows, bytes, files, row groups, readers/writers, p50/p95/p99, scanned
  bytes, candidate rows, catalog roundtrips, and hardware/profile metadata.
- **Operations:** backup/restore RPO/RTO, secret rotation, provider failure modes,
  orphan inventory/cleanup, compaction, retention, and capacity guidance.
- **Security:** transport/auth failures, authorization consistency between data and
  catalog metadata, service identities, and redacted audit/metrics behavior.
- **Interoperability:** standard-reader result or an explicit non-standard storage
  declaration and migration trigger.
- **Lifecycle:** release-to-release catalog/object-prefix upgrade, rollback, and
  artifact retention for the exact source SHA.

## Blocking DuckDB migration program

The following phases are release work, not an optional experiment. They may
overlap, but their exit gates are ordered: storage authority before broad query
routing, query routing before compatibility cutover, and local correctness before
shared-service promotion.

### D0 — freeze contracts and establish the engine boundary

**Outcome:** QuackGIS code outside the engine adapter no longer assumes DataFusion
catalog, plan, or mutation types.

Deliver:

- inventory every maintained SQL, protocol, type, client, storage, spatial,
  security, metrics, and operations capability as preserve/replace/redesign/defer;
- define an internal engine/storage interface for query, Arrow ingest, DDL/DML,
  transaction ownership, schema discovery, snapshots, maintenance, cancellation,
  and error classification;
- capture current DataFusion/SedonaDB output and failure oracles for the curated
  PostGIS regress set, pgwire protocols, COPY, native mutation, time travel,
  spatial pruning, and maintained clients; and
- make backend selection explicit and prevent both backends from writing one
  catalog root.

**Exit gate:** the current backend runs through the boundary without observable
regression, and every maintained capability has an owner and DuckDB parity test.

### D1 — DuckDB owns new DuckLake storage

**Outcome:** all new-path durable metadata and files are authored by DuckDB's
official `ducklake` extension through ADBC.

Deliver:

- promote the existing ADBC slice into a lifecycle-managed connection/database
  service with pinned `libduckdb` and extension artifacts;
- support official local and PostgreSQL/object-storage DuckLake attachment,
  secrets/configuration, schema discovery, Arrow create/append/replace, DDL, DML,
  transactions, snapshots, compaction, and reopen;
- preserve WKB/EWKB bytes, scalar Arrow types, hidden layout columns, and data-
  inlining policy needed by migration/reference readers;
- define export/import from unreleased legacy SQLite and PostgreSQL multicatalog
  roots, including checksums, counts, snapshots retained or intentionally reset,
  and rollback; and
- run concurrent writer, stale conflict, process-kill, response-loss, object
  prewrite, cleanup, backup, and restore oracles.

**Exit gate:** DuckDB-authored local and shared-profile catalogs pass independent
official-DuckLake reopen, one-snapshot mutation, crash, and migration checks. No
new production claim depends on the forked writer.

### D2 — DuckDB becomes the primary query engine

**Outcome:** pgwire `SELECT`, DDL/DML, COPY, and transactions execute through
DuckDB while preserving Arrow/PostgreSQL result contracts.

Deliver:

- route simple and extended query execution, parameters, cancellation, COPY Arrow
  streams, transaction state, and errors through the engine boundary;
- map DuckDB result schemas to maintained PostgreSQL OIDs, text/binary encodings,
  row descriptions, nullability, decimal/time behavior, and cursor semantics;
- move selective filters, joins, grouped/window analytics, Parquet scans, and
  maintenance to DuckDB-native SQL/plans;
- compare result sets and plans against current DataFusion/SedonaDB oracles and
  PostGIS fixtures, including unsupported-shape failures; and
- benchmark memory, spill, concurrency, startup, scan bytes, p95/p99, and ingest
  throughput before selecting defaults.

**Exit gate:** the smallest maintained workflow—create, COPY, parameterized
select, spatial filter, update, delete, transaction rollback/commit, restart, and
snapshot read—passes entirely through DuckDB with no client-visible regression.

### D3 — reconstruct spatial and PostGIS compatibility

**Outcome:** DuckDB is hidden behind the same useful PostGIS-compatible surface
rather than exposed as a different SQL product.

Deliver:

- classify all maintained `ST_*`, geography, CRS/projection, MVT, aggregate, and
  metadata behavior as native DuckDB spatial, SQL rewrite/macro, QuackGIS
  extension function, bounded auxiliary execution, or explicit unsupported gap;
- preserve WKB/EWKB, geometry/geography OIDs, `geometry_columns`,
  `geography_columns`, `spatial_ref_sys`, subtype/SRID/dimension policy, and
  pgjdbc/OGR/QGIS discovery traces;
- redesign hidden bbox/time/Morton maintenance and pruning around DuckDB plans,
  partitioning/statistics, and exact spatial recheck;
- retain SedonaDB only where a measured maintained workload requires it and where
  Arrow handoff, transaction visibility, and operational cost are explicit; and
- run curated PostGIS regress plus QGIS, GeoServer, OGR, Martin, psycopg,
  SQLAlchemy/GeoPandas, API, and BI traces side-by-side.

**Exit gate:** all release-required clients and spatial workloads either match
their current contract or have an approved, measured compatibility limit. There
is no silent approximate spatial result.

### D4 — operationalize the DuckDB runtime

**Outcome:** the native engine and extensions are supportable in local, container,
Kubernetes, and managed-service deployments.

Deliver:

- choose and document in-process ADBC versus isolated worker/sidecar failure
  boundaries; test engine crash containment and restart behavior;
- package checksummed DuckDB and extension binaries without runtime extension
  downloads; publish supported platform/architecture and upgrade matrices;
- validate multi-process readers/writers against supported DuckLake shared catalog
  profiles, connection limits, retries, resource isolation, admission control, and
  graceful shutdown;
- adapt authorization, secret rotation, audit, metrics, health, cancellation,
  backup/restore, retention, and orphan cleanup to DuckDB/DuckLake; and
- run clean-room deployment, rolling upgrade/rollback, mixed-version refusal, and
  24-hour then release-soak drills.

The repository-local Linux x86_64 developer slice now pins the DuckDB CLI in
`mise.lock`, checksum-verifies official libduckdb, preinstalls signed
engine-version-matched extensions, records their digests, and runs probes with
`LOAD` only. That removes optional host setup from D0/D1 evaluation but does not
close this production packaging gate or its platform and deployment evidence.

**Exit gate:** the DuckDB backend passes the managed-service Alpha operational,
security, recovery, and budget gates with pinned artifacts and no online install.

### D5 — cut over and retire the old storage engine

**Outcome:** DuckDB is the default and only release storage authority; old engine
code remains only where an explicitly approved auxiliary workload needs it.

Deliver:

- switch default builds, deployment manifests, docs, examples, and evidence jobs
  to DuckDB;
- execute the tested legacy export/import path and retain a rollback release for
  the declared support window;
- remove `datafusion-ducklake` writer/provider forks and DataFusion-specific
  mutation/catalog code from the default runtime;
- remove SedonaDB/DataFusion execution from the default runtime unless D3 evidence
  justifies a narrow, isolated auxiliary engine; and
- publish the final capability ledger, known losses, migration guide, dependency/
  extension provenance, and comparative correctness/performance evidence.

**Exit gate:** two consecutive release candidates pass all required rings using
DuckDB by default, migration rollback is proven, and no supported catalog can be
accidentally opened for writes by both authorities.

## Forward milestones

### M6 — managed-service External Alpha

**Outcome:** the DuckDB-authored official DuckLake profile works on real platform-
managed services, not only local/Kind stand-ins.

Deliver:

- execute [docs/ALPHA_EXTERNAL_SERVICES.md](./docs/ALPHA_EXTERNAL_SERVICES.md)
  against managed PostgreSQL and S3-compatible object storage through DuckDB's
  official DuckLake path;
- exercise at least two QuackGIS readers plus concurrent writers through catalog
  restart, credential rotation, network interruption, latency, and throttling;
- restore a matched catalog/object-prefix backup into isolation and record RPO/RTO;
- inventory failed prewrites and prove cleanup/quarantine cannot delete live data;
- prove that an independently opened, version-matched official DuckLake reader can
  discover schemas and reproduce counts/samples after ingest, mutation, compaction,
  restart, and restore; legacy library-specific multicatalog roots remain migration
  inputs rather than the release format; and
- attach metrics, logs, configuration, and a provider/profile manifest to release
  evidence.

**Exit gate:** two repeatable external runs on the same release candidate with no
torn mutations, unexplained visibility lag, secret leakage, or undocumented
storage incompatibility.

### M7 — city-scale Client Beta

**Outcome:** a platform team can ingest, browse, serve, analyze, and edit a real
city dataset through maintained client workflows without those clients detecting
the engine migration except through documented performance or feature changes.

Deliver:

- run a copied 10M+ feature OSM/Overture/GeoParquet-derived matrix with points,
  lines, polygons, wide attributes, skew, null/empty geometries, and stable ids;
- prove QGIS browse/render/filter/identify/edit, OGR COPY/read, GeoServer WMS/WFS/
  WFS-T, real Martin MVT attributes, psql/psycopg, SQLAlchemy/GeoPandas, one API
  server, and one BI client;
- compare counts, bounds, filtered samples, edit results, and representative
  outputs against PostGIS and the previous DataFusion/SedonaDB oracle;
- carry the implemented explicit geometry/geography family identity through the
  city matrix, then define subtype/SRID/dimension and old-blob migration policy;
- promote the implemented `Execute.max_rows` portal suspension oracle to pgjdbc
  fetch-size traces at realistic page sizes and publish measured memory/latency
  limits.

**Exit gate:** the versioned city matrix passes in local, Kind, and managed-service
rings with budgeted query/ingest results and no source-tree-only setup steps.

### M8 — durable edits, maintenance, and dataset history

**Outcome:** edits, compaction, snapshots, retries, and restore are predictable
under process failure and concurrent writers.

Deliver:

- promote the local before/after-commit native DELETE, UPDATE, and bucket
  compaction process-kill matrix to equivalent DuckDB/DuckLake operations in Kind
  and managed services; prove cleanup/deletion safety and application-specific
  response-loss reconciliation;
- run real edit traces across fragmented files and report write amplification,
  delete generations, compaction benefit, conflicts, and latency;
- map explicit transactions to DuckDB/DuckLake transaction and snapshot semantics,
  including multi-statement and multi-table boundaries, or retain a documented hard
  limit with measured fallback cost;
- add timestamp-based `AS OF`, protected release snapshots, retention policy, and
  an operational rollback drill;
- re-enable CDC rows only after simple and extended pgwire projection, bounds,
  ordering, update/delete semantics, and failure behavior are deterministic; and
- use official deletion-vector/Puffin, protection, compaction, or branch primitives
  only when they preserve one visible snapshot and pass reference-reader gates.

**Exit gate:** failure matrices show no partial visibility or duplicate/lost rows,
and operators can identify, protect, query, restore, and retire a dataset version
through documented interfaces.

### M9 — production security and operability

**Outcome:** QuackGIS can be operated as a shared service without relying on
network trust or source familiarity.

Deliver:

- promote implemented schema/table write allowlists through external service
  identities, then add read isolation where real deployments require it;
- filter `pg_catalog`, `information_schema`, metadata UDTFs, snapshot operations,
  and maintenance entrypoints consistently with authorization;
- execute TLS/plaintext-denial, wrong-password, pgwire/catalog/object secret
  rotation, credential revoke, and least-privilege probes externally;
- define redacted structured audit events for auth failures, denied mutations,
  maintenance, snapshot protection/restore, and administrative changes;
- add object-store IO, catalog roundtrip, conflict/retry, compaction queue,
  retention, and cleanup metrics with profile budgets; and
- publish deployment packaging, capacity guidance, upgrade notes, health checks,
  and incident/restore runbooks; then execute one clean-room upgrade/rollback
  rehearsal on a disposable copied catalog/object prefix.

**Exit gate:** a clean-room operator deploys, rotates, restores, diagnoses, and
upgrades a release using published artifacts only; a 24-hour mixed workload stays
within declared budgets.

### M10 — regional spatial analytics Beta

**Outcome:** QuackGIS is a credible regional analytical spatial data plane, not
only a client-compatibility server.

Deliver:

- make 100M-feature runs routine and 1B-feature runs scheduled on at least one
  real or realistically distributed regional dataset;
- benchmark selective predicates, spatial joins, grouped/window analytics where
  supported, coverage/asset inventory, candidate narrowing, concurrent readers,
  and parallel ingest/edit writers;
- publish comparative DuckDB versus previous DataFusion/SedonaDB plans, throughput,
  latency, memory/spill, and scan-byte evidence; keep an auxiliary engine only for a
  measured workload where its total operational benefit is clear;
- add layout selectivity, metadata-scan, catalog-roundtrip, file/row-group, bytes,
  candidate, and p95/p99 budgets with plan trend assertions;
- adopt upstream Bloom/statistics/metadata-scan improvements when they outperform
  QuackGIS workarounds without weakening exact recheck; and
- grow the PostGIS regress matrix only from maintained client/workload needs, with
  explicit skip/fail reasons.

**Exit gate:** one managed-service regional run and two consecutive scheduled
runs satisfy correctness and performance budgets with reproducible artifacts.

### 1.0 — operational regional spatial lakehouse

**Outcome:** a stable public release for regional vector lakehouse workloads.

Required release decision points:

- DuckDB is the default query and storage engine, and official DuckLake is the only
  release write format; legacy QuackGIS catalogs have a tested migration/export and
  rollback path;
- the old `datafusion-ducklake` writer is absent from the default runtime, and any
  retained DataFusion/SedonaDB auxiliary path is narrow, documented, and justified
  by release evidence;
- geometry/geography family identity has durable metadata and documented sentinel-
  OID behavior across maintained clients, with subtype/SRID/dimension, migration,
  and geography reference-reader limits resolved or explicitly bounded;
- upgrade and rollback are proven on copied catalog/object prefixes;
- required local, Kind, managed-service, city, and regional gates pass for two
  consecutive release candidates;
- the defined 72-hour or three-by-24-hour mixed reader/writer/maintenance soak
  meets budgets; and
- the release packet includes compatibility, conformance, benchmark, security,
  operations, DuckLake alignment, known limits, images/binaries, and selected raw
  artifacts.

### 1.x — releaseable datasets and maintained summaries

**Outcome:** teams can stage, review, publish, supersede, and recover datasets as
products.

Deliver a staged-import → validate → promote → protect → serve → roll back flow;
stable release references and retention semantics; branch/merge when upstream-safe;
and maintained MVT, extent, coverage, or asset summaries with freshness, rebuild,
and failure budgets. Promotion must be atomic from the perspective of readers.

**Exit gate:** one copied regional dataset completes the full lifecycle, concurrent
readers observe either the old or new release but never partial promotion, rollback
restores the prior reference, and one maintained summary meets declared freshness,
rebuild-time, and failure-recovery budgets.

### 2.x — multi-modal spatial asset lakehouse

**Outcome:** mixed spatial assets are discoverable and analyzable without turning
the QuackGIS process into a heavyweight format decoder.

Deliver real inventories for COG/raster, COPC/LAZ/E57, 3D Tiles/mesh, CAD/BIM,
imagery/aerial, and reality-capture collections; validate identity, URI lifecycle,
CRS/vertical datum/epoch, transform provenance, quality, and derived-footprint
fidelity; benchmark coverage, gap, change-candidate, provenance, and mixed
vector/asset workloads; and preserve QGIS/GeoServer/GDAL simple-feature gates.

**Exit gate:** at least one real COG inventory and one real COPC/LAZ inventory pass
identity, CRS/epoch/provenance, query, lifecycle, restore, and regional scale gates;
two additional asset families pass through the real-inventory/workload ring; all
maintained vector client gates remain green.

### 2.x — national and trillion-class stress envelope

**Outcome:** the architecture has measured limits beyond routine regional use.

Run 10TB+ object inventories, billion-row scheduled workloads, and generated
trillion-class index/catalog stress. Report catalog growth, listing/refresh time,
partition/file counts, compaction and recovery bounds, cost, and the point where a
single catalog/prefix must shard. This is an evidence target, not a blanket promise
that every trillion-row query is interactive.

**Exit gate:** two scheduled exact billion-row profile runs and one manual exact
10TB-inventory profile run publish catalog/object growth, query/maintenance/
recovery budgets, cost, and a tested or simulated sharding decision with no
correctness regression.

## Immediate execution queue

1. Complete D0 beyond the implemented startup foundation: replace the remaining
   DataFusion-specific query/transaction/schema/snapshot/maintenance/cancellation/
   error types behind the engine adapter and turn every blocked row in the
   capability ledger into a DuckDB parity test. Explicit backend selection and
   atomic per-data-root authority markers are implemented.
2. Complete the D1 local vertical slice through real pgwire: `CREATE TABLE`, Arrow
   `COPY`, parameterized `SELECT`, `UPDATE`, `DELETE`, transaction rollback/commit,
   snapshot inspection, restart, and independent official-DuckLake reopen. A
   bounded local workflow now proves pgwire CREATE, text COPY→Arrow/WKB,
   parameterized exact spatial SELECT, UPDATE/DELETE, empty RowDescription,
   per-client transaction isolation/rollback/commit/disconnect cleanup, snapshot
   inspection, restart/reopen, structural statement classification, parameterized
   DML, SCRAM plus read/write allowlists, and core scalar COPY types through the
   real feature-gated CLI backend. Native cancellation, ADBC batch streaming,
   COPY options/escaping, catalog/PostGIS shims, shared storage, and maintained
   client compatibility remain before default-backend promotion.
3. Build the legacy export/import oracle from current SQLite/local catalogs into a
   separate DuckDB-authored official DuckLake root; verify schema, counts, WKB,
   hidden layout columns, mutations, snapshots policy, and rollback.
4. Route the curated PostGIS/spatial and catalog compatibility suites through the
   DuckDB backend; produce the native/macro/extension/auxiliary/unsupported gap
   ledger before widening function coverage. The 57-case spatial ledger is now
   complete and all 40 native/rewrite/macro cases now pass through the real DuckDB
   pgwire route with maintained scalar results. The 12 Rust-edge cases, five
   extension candidates, catalog surfaces, and named client routing remain.
5. Package pinned DuckDB 1.5.x-compatible engine and `ducklake`/`spatial` extension
   artifacts in the runtime image with load-only production policy and checksum/
   provenance verification. The Linux x86_64 evaluation image now passes this
   offline gate; production promotion and platform/upgrade matrices remain.
6. Run the first DuckDB-authored PostgreSQL/object-storage Kind and managed-service
   drill, including concurrent writers, process kill, response loss, credential
   rotation, backup/restore, and independent reopen.
7. Promote named QGIS, GeoServer, OGR, Martin, pgjdbc, Python/API, and BI traces over
   one copied city dataset on both backends until DuckDB reaches parity.
8. Re-baseline `layoutbench-regional-r100m-v1` for DuckDB plans, catalog calls,
   memory/spill, scan bytes, and OLAP throughput before making it the default.

## Risk register

| Risk | Product impact | Required response |
|---|---|---|
| DuckDB migration drops useful QuackGIS behavior | maintained clients or workflows regress | maintain the capability ledger, side-by-side traces, and explicit parity/limit approval before cutover |
| Native DuckDB or extension ABI/supply-chain drift | startup, security, or upgrades become fragile | pin and package engine/extensions, verify checksums/provenance, prohibit production downloads, and test mixed-version refusal |
| DuckDB local concurrency model is used for shared service claims | multi-process writers block or corrupt operational assumptions | use supported shared DuckLake catalogs for multi-user profiles and test conflicts, retries, admission, and resource limits |
| DuckDB spatial differs from maintained PostGIS/Sedona semantics | incorrect results or client incompatibility | preserve exact-result oracles and close gaps with rewrites/macros/small extensions; fail closed otherwise |
| Legacy preview catalogs cannot migrate cleanly | users lose data/history or rollback | never mix authorities; build copy-based export/import with checksums, parity, declared snapshot treatment, and rollback |
| Young DuckLake APIs or extension drift | upgrades and maintenance semantics break | pin versions, use official primitives, test reopen/migration at milestones, and upgrade only behind equivalent gates |
| Spatial identity exceeds the family metadata claim | wrong subtype/SRID/dimension or external-reader assumptions | keep the family-only boundary explicit; test migration and reference readers before widening it |
| Object/catalog metadata dominates selective reads | high-QPS scale collapses despite Parquet pruning | budget roundtrips/listings/refresh and adopt upstream metadata improvements |
| ADBC serialization or in-process failure limits throughput/isolation | latency, cancellation, or crash containment misses service goals | benchmark connection ownership and compare in-process, worker-pool, and sidecar designs before D4 |
| Compatibility branches sprawl | fragile behavior across clients | classify by protocol/catalog surface and require trace fixtures |
| Security/DR trails feature work | no credible production release | treat M9 and external restore evidence as release blockers |
| Multi-modal scope dilutes vector reliability | core PostGIS workflows regress | keep sidecar-first architecture and require vector gates for every asset milestone |
| Scale language outruns evidence | roadmap ambition becomes misleading | publish exact rows/bytes/profile/cost and distinguish routine, scheduled, and stress claims |

## Scope boundaries

QuackGIS is not a full PostgreSQL replacement, an OLTP application database, a
document store, a desktop GIS/map server, or a general-purpose heavyweight
raster/CAD/point-cloud decoder. PL/pgSQL, triggers, LISTEN/NOTIFY, logical
replication, topology, Tiger geocoder, SFCGAL, and complete PostgreSQL semantics
remain out of scope unless the product goal materially changes.
