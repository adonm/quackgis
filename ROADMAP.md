# Roadmap

This is the ordered forward roadmap for the DuckDB-only product. Current evidence
lives in [docs/ROADMAP_STATUS.md](./docs/ROADMAP_STATUS.md); durable direction and
the extension decision ladder live in
[docs/PROJECT_DIRECTION.md](./docs/PROJECT_DIRECTION.md).

A milestone closes only when:

- implementation runs through the DuckDB-only server;
- tests are registered and execute in the named gate;
- performance budgets name hardware, data, and native artifact versions;
- evidence records the exact source SHA; and
- status and compatibility documents are updated.

Retired-engine behavior, unregistered tests, design documents, and static profile
validation do not close milestones.

## Baseline

| Area | Current floor | Important limit |
|---|---|---|
| Engine/storage | pinned DuckDB 1.5.4 through ADBC and local official DuckLake | local paths only |
| Protocol | bounded simple/extended pgwire | narrow statements and parameter types |
| Results | one driver Arrow batch at a time through pgwire with fail-closed byte ceiling | native-allocation/RSS scale evidence open |
| COPY | incremental bounded text decoding to one ADBC stream and atomic DuckLake publication | 1 GiB/RSS/throughput evidence and pre-decode pgwire frame bound open; accepted wire chunks are bounded after dependency decoding |
| Transactions | independent sessions, commit/rollback/isolation and failed-transaction `25P02` enforcement | write/commit cancellation policy incomplete |
| Spatial | 42 native/rewrite/macro cases through pgwire | 10 edge gaps and 5 extension candidates |
| Security | SCRAM and read/write table allowlists | incomplete metadata filtering/TLS evidence |
| Operations | restart/reopen, snapshot inspection, adjacent-file merge, checksummed offline exact-path backup/restore | no online/relocated production recovery or shared profile |
| Performance | fixture-level bbox/exact-recheck oracle | no current scale or layout-maintenance claim |
| Metrics/status | policy, classed admission, lifecycle, cancellation, timeout, quarantine, COPY rows/bytes/batches/latency, sampled DuckDB memory/temporary storage, liveness, and DuckLake-probed readiness/drain state | probe is local/read-only; no write-capacity SLO |

## M0 — truthful, focused repository

**Outcome:** active documentation, commands, examples, workflows, and deployment
assets describe only the current DuckDB runtime and immediate release path.

Deliver:

- separate current DuckDB evidence from historical/oracle fixtures;
- delete commands and runbooks for absent tests, retired engines, and unsupported
  shared profiles;
- register every claimed test target explicitly;
- create one maintained documentation path for direction, architecture, roadmap,
  status, compatibility, operations, and benchmarks;
- add a deterministic DuckDB performance profile measuring direct DuckDB, ADBC,
  and pgwire; and
- make capability status mechanically checkable where practical.

Exit gates:

- `just ci` and every indexed quick-start command pass from a clean bootstrap;
- every status claim links to an executable command or says blocked/deferred;
- supported spatial counts equal cases executed through pgwire;
- no active claim depends on DataFusion, SedonaDB, fork-owned DuckLake, removed
  CLI flags, or an unregistered test; and
- unsupported shared deployment automation is absent from scheduled workflows.

## M1 — bounded execution plane

**Outcome:** large results and concurrent clients cannot exhaust the process by
construction.

Deliver:

- replace collected query results with incremental ADBC Arrow streams;
- connect portal paging to the live stream;
- propagate pgwire cancellation and deadlines to native statements;
- add fixed connection, reader, writer, maintenance, and blocking-worker limits;
- configure DuckDB threads, memory, temporary storage, and spill at startup;
- add query queue/lifecycle, memory, spill, timeout, cancellation, and quarantine
  metrics; and
- define disconnect, partial-delivery, and uncertain-cleanup behavior.

Exit gates:

- 1M- and 10M-row results remain within the configured stream budget plus 128 MiB;
- in-flight Arrow batch count is independent of result cardinality;
- time to first row occurs before full query completion;
- 100 long-query cancellations complete within 500 ms p95 on the reference host;
- cancelled connections are reusable or explicitly quarantined, with no active
  transaction left behind;
- a configured eight-query limit never executes nine under 32 clients; and
- pgwire overhead is at most 15% over direct ADBC for scans lasting at least one
  second on the same process and data.

## M2 — streaming bulk ingest

**Outcome:** GDAL/OGR-scale loads use bounded COPY and publish atomically.

Deliver:

- parse COPY chunks incrementally into bounded Arrow builders;
- stream ADBC ingest rather than collecting the request;
- support PostgreSQL escaping, NULL, WKB/EWKB, and release-required scalars;
- add cancellation, timeout, disconnect, malformed-row, and rollback tests;
- report rows, bytes, batches, throughput, and commit latency; and
- exercise official DuckLake compaction after fragmented loads.

Exit gates:

- a 10M-row or 1 GiB COPY has no request-size ceiling;
- peak COPY RSS remains within idle plus 256 MiB on the reference profile;
- no Arrow batch exceeds configured row/byte limits;
- pgwire text COPY reaches at least 50% of direct ADBC Arrow-ingest throughput;
- WKB bytes, NULLs, decimals, dates, and timestamps survive commit/reopen; and
- parse failure, cancellation, disconnect, and rollback add zero visible rows.

## M3 — focused compatibility product

**Outcome:** the first named client set works without DuckDB-specific setup.

Release-required clients:

- `psql`;
- `psycopg`;
- GDAL/OGR read and COPY load; and
- QGIS read-only discovery, filtering, identify, and render.

Deliver:

- derive the required `pg_catalog`/`information_schema` surfaces from DuckDB;
- stabilize geometry RowDescription OIDs and text/binary WKB behavior;
- define or explicitly bound family, subtype, SRID, and dimension metadata;
- assign each release spatial requirement to native, macro/rewrite, Rust edge,
  DuckDB extension, or unsupported;
- replace text signatures with reusable AST/catalog/protocol rules;
- add fuzz/property coverage for the Arrow-to-pgwire encoder; and
- keep GeoServer, editing, Martin, BI, and broad ORM compatibility deferred unless
  they fit without materially widening the first-release surface.

Exit gates:

- each required client has a version-pinned copied-data end-to-end test;
- required catalog queries are fixture-tested independently of client names;
- every supported spatial function has exactly one implementation disposition;
- all supported cases pass through pgwire with exact expected results;
- QGIS and OGR observe maintained geometry fields rather than generic `bytea`;
- unsupported functions/shapes return stable errors; and
- no release query uses row-wise Rust spatial fallback.

## M4 — spatial analytical performance

**Outcome:** QuackGIS earns selective spatial and OLAP performance rather than
providing protocol compatibility alone.

Deliver:

- inject safe, planner-visible bbox predicates for proven literal/bound shapes;
- retain the original exact DuckDB predicate;
- maintain bbox/locality columns during COPY and compaction in DuckDB;
- benchmark WKB storage against native geometry before changing representation;
- set file/row-group sizing from DuckDB evidence;
- cover selective scans, grouped aggregates, bounded spatial joins, wide
  projections, and fragmented-file compaction; and
- rebuild exact 10M and 100M profiles using DuckDB plans/profiling rather than
  retired provider counters.

Exit gates:

- every pruned result equals its unpruned exact result;
- holes, null/empty, invalid, and boundary geometries prove conservative behavior;
- representative selective queries scan at most 5% of table bytes or improve scan
  volume by at least 20x;
- exact recheck remains visible in `EXPLAIN`;
- compaction halves fragmented file count without result changes;
- two 10M runs pass before 100M promotion; and
- two consecutive 100M runs publish and satisfy committed load, first-row,
  p50/p95/p99, RSS, spill, scan-byte, file, row-group, and plan budgets.

## M5 — Local 1.0

**Outcome:** a user can deploy and operate the single-node product without
repository knowledge.

Deliver:

- immutable runtime artifacts with DuckDB/extension provenance and no online
  extension install;
- health, readiness, graceful shutdown, and transaction drain;
- backup, restore, compaction, capacity, spill, and incident procedures;
- supported DuckDB/extension upgrade and reopen tests;
- TLS and secret-rotation evidence; and
- mixed read/COPY/mutation/cancel/compaction/restart/restore workloads.

Exit gates:

- a clean environment starts from published artifacts only;
- backup/restore reproduces the declared committed snapshot and exact counts;
- controlled termination exposes no partial mutation;
- restart recovery completes within 60 seconds for the release catalog;
- a 24-hour mixed workload has no correctness failure, leaked transaction, or
  unbounded memory growth;
- required client and 10M performance gates remain green after packaging; and
- statement/type/transaction/concurrency limits are published.

## M6 — Shared DuckLake 1.x

**Outcome:** an official managed catalog/object-storage profile preserves the
local query and compatibility contract.

This begins only after Local 1.0.

Deliver:

- official PostgreSQL catalog and object-storage configuration;
- shared credentials and writer-authority validation;
- measured multi-process readers/writers using supported DuckLake behavior;
- reader visibility and writer consistency policy;
- deterministic conflict/indeterminate-commit classification; and
- throttling, interruption, rotation, backup, restore, cleanup, and independent
  reader tests.

Exit gates:

- two readers and one writer run for 24 hours with no loss, duplicates, or partial
  visibility;
- committed changes meet the declared visibility SLO;
- conflict/response-loss tests have deterministic reconciliation outcomes;
- independent version-matched DuckDB reproduces schemas, counts, samples, and
  snapshots;
- restored catalog/object storage reproduces the recovery point; and
- two managed-service runs pass on the same release candidate.

## M7 — dataset lifecycle 1.x

**Outcome:** operators can stage, validate, publish, protect, roll back, and retire
dataset versions using official DuckLake primitives.

Exit gates:

- readers see either old or new release, never partial promotion;
- rollback restores the prior exact result set;
- retention cannot remove protected release data; and
- one maintained extent/tile summary meets freshness, rebuild, and recovery
  budgets.

## Deferred until after Local 1.0

- GeoServer/WFS-T and broad JDBC catalog compatibility.
- QGIS transactional editing.
- Martin/MVT beyond a measured release need.
- Multi-modal COG, point-cloud, CAD/BIM, or reality-capture product claims.
- Billion-row scheduled, 10 TB, or trillion-class claims.
- Branch/merge and CDC row functions.
- A QuackGIS DuckDB extension without an accepted, benchmarked proposal.

## Risk controls

| Risk | Required response |
|---|---|
| native/extension supply-chain or ABI drift | pin artifacts, verify checksums, prohibit production downloads, test upgrades/mixed-version refusal |
| unbounded ADBC materialization or blocking work | M1 streaming, cancellation, admission, memory/spill budgets before broader clients |
| compatibility sprawl | require client traces, implementation disposition, stable errors, and delete shims replaced upstream |
| incorrect spatial pruning | conservative candidate oracle plus visible exact recheck for every optimized shape |
| DuckLake API/semantics drift | use official primitives, independent reopen, backup/restore, versioned upgrade gates |
| shared claims outrun local product | Local 1.0 is a hard prerequisite for M6 |
| scale language outruns evidence | publish exact rows/bytes/files/hardware/cost and distinguish routine from stress runs |

## Scope boundaries

QuackGIS is not a full PostgreSQL replacement, OLTP database, document store,
desktop GIS/map server, or heavyweight raster/CAD/point-cloud decoder. PL/pgSQL,
triggers, LISTEN/NOTIFY, logical replication, topology, Tiger geocoder, SFCGAL,
and complete PostgreSQL semantics remain out of scope unless product direction
materially changes.
