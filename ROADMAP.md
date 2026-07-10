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
lakehouse.** Because QuackGIS has not released, the roadmap now pivots directly to
DuckDB as the query/storage authority and official DuckLake as the durable storage
contract. The Rust pgwire service should let platform teams keep city, regional,
and national spatial data in DuckLake/Parquet while familiar QGIS, GeoServer,
Martin, GDAL/OGR, PostgreSQL-driver, API, and BI workflows connect through the
QuackGIS compatibility edge.

The goal is not a smaller PostgreSQL. It is a horizontally readable, operationally
recoverable spatial data plane where:

- object-store data feels like PostGIS to existing clients;
- DuckDB performs selective spatial reads, broad OLAP, and DuckLake writes through
  upstream-maintained primitives;
- parallel ingest and edit writers publish coherent official DuckLake snapshots;
- datasets can be inspected, protected, released, restored, and upgraded;
- raster, point-cloud, 3D, CAD/BIM, imagery, and reality-capture collections are
  queryable through trustworthy footprint, provenance, and sidecar indexes; and
- storage remains open enough to migrate with DuckLake rather than trapping data
  behind a QuackGIS-only service.

## Product horizons

### 1.0 — regional spatial lakehouse

A platform team can deploy QuackGIS over managed catalog/object storage, ingest
and edit a real regional vector dataset, serve maintained GIS/API clients, run
budgeted analytical queries, recover from failures, and upgrade or roll back
without reading the source tree.

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
   implemented only where maintained workflows need it; DuckDB + official DuckLake
   become the canonical query/storage plane unless a measured workload forces a
   narrower auxiliary engine.
2. **Exact spatial results are authoritative.** Layout and candidate filters may
   over-select but may never replace the exact DuckDB spatial/PostGIS-compatible
   predicate used for the maintained workload.
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

See [ARCHITECTURE.md](./ARCHITECTURE.md) for the design consequences of these
invariants.

## Current baseline

The implemented floor is deliberately summarized rather than replayed here:

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
| Local/SQLite | deterministic SQL, storage, restart, failure-injection, and exact-result semantics | shared-service or provider behavior |
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

## Forward milestones

### M6 — managed-service External Alpha

**Outcome:** the maintained lake profile works on real platform-managed services,
not only local/Kind stand-ins.

Deliver:

- execute [docs/ALPHA_EXTERNAL_SERVICES.md](./docs/ALPHA_EXTERNAL_SERVICES.md)
  against managed PostgreSQL and S3-compatible object storage;
- exercise at least two QuackGIS readers plus concurrent writers through catalog
  restart, credential rotation, network interruption, latency, and throttling;
- restore a matched catalog/object-prefix backup into isolation and record RPO/RTO;
- inventory failed prewrites and prove cleanup/quarantine cannot delete live data;
- record whether the PostgreSQL catalog backend is readable by a standard DuckLake
  implementation. DuckDB with the official DuckLake extension is the preferred
  named reference reader, driven through CLI or ADBC. The current library-specific
  multicatalog layout must be called non-standard until that result changes; and
- attach metrics, logs, configuration, and a provider/profile manifest to release
  evidence.

**Exit gate:** two repeatable external runs on the same release candidate with no
torn mutations, unexplained visibility lag, secret leakage, or undocumented
storage incompatibility.

### M7 — city-scale Client Beta

**Outcome:** a platform team can ingest, browse, serve, analyze, and edit a real
city dataset through maintained client workflows.

Deliver:

- run a copied 10M+ feature OSM/Overture/GeoParquet-derived matrix with points,
  lines, polygons, wide attributes, skew, null/empty geometries, and stable ids;
- prove QGIS browse/render/filter/identify/edit, OGR COPY/read, GeoServer WMS/WFS/
  WFS-T, real Martin MVT attributes, psql/psycopg, SQLAlchemy/GeoPandas, one API
  server, and one BI client;
- compare counts, bounds, filtered samples, edit results, and representative
  outputs against a PostGIS/reference oracle;
- carry the implemented explicit geometry/geography family identity through the
  city matrix, then define subtype/SRID/dimension and old-blob migration policy;
- probe pgjdbc fetch-size/portal suspension at realistic page sizes and either
  implement it or publish a measured client limit.

**Exit gate:** the versioned city matrix passes in local, Kind, and managed-service
rings with budgeted query/ingest results and no source-tree-only setup steps.

### M8 — durable edits, maintenance, and dataset history

**Outcome:** edits, compaction, snapshots, retries, and restore are predictable
under process failure and concurrent writers.

Deliver:

- promote the local before/after-commit native DELETE, UPDATE, and bucket
  compaction process-kill matrix to Kind and managed services; prove quarantine/
  deletion safety and application-specific response-loss reconciliation;
- run real edit traces across fragmented files and report write amplification,
  delete generations, compaction benefit, conflicts, and latency;
- either batch explicit transactions through native mutation primitives or retain
  a documented hard limit with measured fallback cost;
- add timestamp-based `AS OF`, protected release snapshots, retention policy, and
  an operational rollback drill;
- re-enable CDC rows only after simple and extended pgwire projection, bounds,
  ordering, update/delete semantics, and failure behavior are deterministic; and
- migrate to upstream deletion-vector/Puffin, protection, or branch primitives
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

- PostgreSQL catalog storage is either standard DuckLake-compatible or plainly
  declared a QuackGIS-specific backend with tested export/migration;
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

1. Complete the DuckDB storage-authority vertical slice and treat DuckDB-authored
   official DuckLake as the canonical new storage path.
2. Route the smallest pgwire/PostGIS workflow through DuckDB-backed storage:
   `CREATE TABLE`, `COPY`, `SELECT`, `UPDATE`, `DELETE`, restart, and reference
   readability.
3. Run the first managed PostgreSQL/object-storage drill against the DuckDB-authored
   path and publish its standard/non-standard catalog interoperability result.
4. Promote the existing SQL-surface probes into named client containers over one
   copied city dataset, including real Martin attributes and GeoServer.
5. Promote the deterministic local process-kill mutation matrix, real prewrite
   inventory evidence, and explicit offline quarantine flow to Kind and managed
   services; add restore-point-backed permanent-deletion proof without claiming
   generic replay idempotency.
6. Promote the implemented durable family-identity contract to PostgreSQL/S3 and
   reference-reader evidence; decide subtype/SRID/dimension and old-blob migration
   without changing WKB/EWKB or maintained client behavior.
7. Execute `layoutbench-regional-r100m-v1` with the bounded Kind phase runner and
   publish the first exact-process catalog provider-call measurements against its
   committed budgets; the process counter, snapshot-fresh 7-call execution path,
   profile/report contract, runner integration, and unambiguous naming gate are
   implemented, but wire-level roundtrips and 100M evidence remain open.
8. Promote the implemented valid-raster/point-cloud local companion gate to copied
   COG and COPC/LAZ inventories with object-store URI lifecycle, restore, and
   workload evidence packets that pass the copied-inventory manifest checker before
   expanding asset families.

## Risk register

| Risk | Product impact | Required response |
|---|---|---|
| PostgreSQL catalog backend remains non-standard | Open-storage and reference-reader claims fail | make the 1.0 backend/export decision explicit; test migration rather than hiding divergence |
| Young DuckLake APIs or fork drift | upgrades and maintenance semantics break | pin, test, record divergence, rebase at milestones, and migrate only behind equivalent gates |
| Spatial identity exceeds the family metadata claim | wrong subtype/SRID/dimension or external-reader assumptions | keep the family-only boundary explicit; test migration and reference readers before widening it |
| Object/catalog metadata dominates selective reads | high-QPS scale collapses despite Parquet pruning | budget roundtrips/listings/refresh and adopt upstream metadata improvements |
| Positional DML conflicts with scan optimization | edit correctness or performance regresses | keep physical-position scans isolated, exact, and separately benchmarked |
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
