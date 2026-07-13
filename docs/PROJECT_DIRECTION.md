# Project direction

This document defines the durable product direction.

- [ROADMAP.md](../ROADMAP.md) owns ordered milestones and exit gates.
- [ARCHITECTURE.md](../ARCHITECTURE.md) owns current implementation boundaries.
- [ROADMAP_STATUS.md](./ROADMAP_STATUS.md) owns the implemented evidence floor.
- [COMPATIBILITY.md](./COMPATIBILITY.md) owns current client and SQL claims.
- [POSTGRESQL_COMPATIBILITY.md](./POSTGRESQL_COMPATIBILITY.md) owns the target
  catalog, role, privilege, and REST delivery contract.

## Product thesis

QuackGIS is a thin, high-performance PostgreSQL/PostGIS compatibility and control
edge for DuckDB Spatial and official DuckLake.

It lets PostgreSQL-oriented spatial tools use DuckDB's analytical and spatial
execution without introducing another query engine, storage writer, distributed
database, or row-wise geometry runtime. The Rust service owns protocol
compatibility, PostgreSQL-facing catalog and session semantics, authorization,
resource control, and operational policy. DuckDB owns planning, vectorized
execution, exact spatial computation, and official DuckLake persistence.

The intended product advantage is the combination of:

1. maintained pgwire/PostGIS workflows at the edge;
2. DuckDB-native analytical and spatial performance;
3. official DuckLake storage and snapshot semantics;
4. bounded, trace-driven compatibility rather than broad emulation;
5. one role, privilege, and metadata contract shared by pgwire, GIS clients, and
   a load-balanceable PostgREST-style HTTP edge; and
6. predictable streaming, cancellation, admission, and bulk-ingest behavior.

## Current reality

The repository proves a bounded local runtime:

- DuckDB and official DuckLake are the only query and storage path.
- ADBC transports Arrow between DuckDB and the Rust edge.
- Simple and extended pgwire support a deliberately narrow statement surface.
- Parameterized reads/mutations, transactions, text COPY, SCRAM, table policy,
  maintained session settings/search path, `public` mapping, quoted COPY, restart,
  and reopen have pinned native integration tests.
- Forty-two native, rewrite, or macro spatial cases execute through pgwire.
- DuckDB and extension artifacts are version- and checksum-pinned.

M1 bounded execution and M2 streaming ingest now have reference evidence:

- clean 1M/10M BIGINT and 1M nullable VARCHAR/BLOB result profiles stay within
  their RSS/batch gates;
- a clean 10M COPY profile passes RSS, throughput, exact publication, and atomic
  abort gates;
- active native query/COPY cancellation and cancellable pre-commit writes have
  explicit rollback/reuse/quarantine outcomes, and the 100-cancel reference passes
  its latency budget; an idle COPY client still receives cancellation only when it
  sends another frame or disconnects;
- connection, queue, global active-query, reader/writer/maintenance admission,
  native worker, DuckDB memory/thread/temp/spill controls, and sampled resource
  metrics are implemented;
- maximum native-batch and additional type/shape resource profiles remain open;
- PostgreSQL catalogs, geometry identity, and named GIS clients are incomplete;
- the REST preview has a separate read-only schema cache and bearer identity; it
  does not yet use database role switching or role-aware OpenAPI;
- shared catalog/object-storage profiles fail closed; and
- current scale evidence is fixture-level, not a product performance claim.

Direction starts from these constraints, not from capabilities of retired engines.

## First release

The first release has one state-owning, read-mostly spatial analytical server with
controlled bulk ingestion over local official DuckLake, plus optional stateless
HTTP read replicas that reach it only through pgwire.

Release-required outcomes:

- bounded streaming query results and COPY;
- native cancellation, deadlines, admission, memory limits, and spill policy;
- `psql`, `psycopg`, GDAL/OGR read and COPY load, and QGIS read-only workflows;
- a focused PostgreSQL 18 catalog/session profile with configuration-backed
  roles, memberships, table/operation privileges, and stable object/type identity;
- a focused, versioned PostGIS function/catalog/type surface;
- a packaged stateless HTTP read edge whose JWT role mapping, schema discovery,
  authorization, and role-aware OpenAPI run through the same pgwire contract;
- selective spatial reads and ordinary DuckDB OLAP with measured plans;
- restart, backup, restore, compaction, and upgrade procedures; and
- reproducible packages with no runtime extension downloads.

Using PostgreSQL as a shared DuckLake metadata catalog and object storage is a
later storage capability. It is distinct from QuackGIS's PostgreSQL-compatible
`pg_catalog` surface and must not block a useful local release or be claimed
before official DuckLake concurrency, visibility, and recovery evidence exists.

## Ownership rules

- DuckDB is the only query planner and spatial execution engine.
- Official DuckLake is the only writer of new durable catalogs and data.
- Rust does not implement row-wise spatial kernels or pull arbitrary table rows
  out of DuckDB for fallback execution.
- Rust does not maintain an independent table catalog, optimizer, or data cache.
- QuackGIS may persist protected control metadata for roles, memberships, grants,
  policy, catalog epochs, and compatibility OID identity through the supported
  DuckDB/DuckLake transaction path. This metadata may project authoritative user
  schema but may not become a second user-table authority.
- PostgreSQL compatibility exists only at observable protocol, SQL, type, and
  catalog boundaries in the declared versioned compatibility profile.
- PostgreSQL catalog visibility, information-schema filtering, privilege inquiry,
  statement authorization, and role-aware OpenAPI must agree through one
  authorization implementation.
- WKB/EWKB remains the wire/interchange contract until a measured native geometry
  representation proves better interoperability and performance.
- Candidate filters may over-select but never replace exact DuckDB predicates.
- Unsupported behavior fails with stable SQLSTATEs rather than changing semantics.

## Extension decision ladder

Every missing requirement follows this order:

| Level | Use when | Required evidence |
|---|---|---|
| **1. DuckDB native** | DuckDB or an official extension already provides the semantics | pgwire fixture, direct DuckDB comparison, stable result/type/error behavior, acceptable plan |
| **2. SQL macro or rewrite** | Behavior composes from DuckDB operations and stays optimizer-visible | quote-safe/AST rewrite, NULL/empty/overload fixtures, `EXPLAIN`, no Rust materialization |
| **3. Rust edge** | Behavior is inherently PostgreSQL-facing or control-plane work | protocol/catalog trace, bounded memory, stable SQLSTATE/OID behavior, no row-wise spatial fallback |
| **4. DuckDB extension** | A maintained row, aggregate, or table operation cannot be efficient at earlier levels | real workload demand, semantic oracle, vectorized benchmark, fuzz/property, ABI/package/upgrade gates |

Additional rules:

- Function-count coverage alone never justifies an extension.
- Extension candidates require a maintained client or workload.
- Extension code may not own pgwire, auth, policy, catalogs, COPY protocol,
  snapshots, or DuckLake writes.
- Every DuckDB upgrade reruns the ladder; compatibility code is deleted when
  native behavior satisfies the contract.

## Performance direction

### Streaming query boundary

Move from collected `Vec<RecordBatch>` results to a stream owning the ADBC
statement, reader, schema, cancellation handle, and connection lease. Pull one
Arrow batch at a time and apply bounded backpressure into async pgwire. Portal
paging must consume the same stream. Memory must scale with configured batch and
queue limits, not result cardinality.

### Cancellation and admission

Register active native statements against pgwire cancel keys. Add statement and
queue deadlines, separate reader/writer/maintenance limits, and a fixed blocking
worker budget. Quarantine uncertain connections. Reserve capacity for cancel,
health, and transaction cleanup.

### Streaming ingestion

Parse COPY chunks incrementally into bounded Arrow builders and feed one ADBC
stream into session-local staging. Bound rows and bytes per batch, then publish to
DuckLake with one atomic statement only after clean EOF. Parse failure,
cancellation, disconnect, or timeout must leave the target unchanged. COPY is the
primary bulk path; INSERT remains a compatibility path.

For the explicit hidden bbox layout, derive bounds in the DuckDB publication SQL.
Do not decode WKB row-by-row in Rust. Automatic predicate injection must remain a
separate structurally proven rule that always retains the exact predicate.

### Spatial execution and layout

Keep exact operations inside DuckDB. Add planner-visible bbox predicates only for
structurally proven-safe shapes. Compute layout/locality columns with DuckDB SQL or
vectorized extension functions during bulk load and compaction. Prefer native
statistics, partitioning, and geometry improvements when measurements justify
them. Do not add a correctness-critical side index or spatial service.

### Observability

Measure queue/execution/time-to-first-row latency, Arrow batches, result/COPY
bytes, memory, spill, cancellation, files and bytes scanned, candidate/exact rows,
ingest throughput, transaction/conflict outcomes, and quarantined connections.
Never expose SQL text, parameters, credentials, or object paths in metrics.

## Capability and claim policy

A capability is supported only when it:

- runs through the current DuckDB-only server;
- is registered in an executed test/client gate;
- has one implementation level from the decision ladder;
- asserts result, type, error, and transaction behavior; and
- passes relevant resource/performance budgets.

Imported PostGIS and retired-engine fixtures are oracle pools, not product claims.
Coverage grows from maintained clients and workloads.

Evidence rings are ordered:

1. unit/static contract;
2. pinned native DuckDB integration;
3. local pgwire workflow;
4. named client workflow;
5. scale/resource budget;
6. managed shared-profile operation;
7. release soak and upgrade.

Passing an earlier ring does not imply a later claim.

## Product horizons

- **Local 1.0:** resource-bounded single-process vector analytics over local
  official DuckLake with bulk ingest, a PostgreSQL 18 catalog/RBAC profile,
  maintained read clients, and packaged role-aware HTTP read/OpenAPI replicas.
- **Shared 1.x:** official shared DuckLake using managed catalog/object storage,
  enabled only after concurrency, visibility, backup, and restore gates.
- **Dataset lifecycle 1.x:** protected versions, promotion, rollback, retention,
  and maintained summaries using official primitives.
- **Later research:** multi-modal inventories and national-scale stress after the
  10M and 100M vector gates are routine.

## Explicit non-goals

- Full PostgreSQL or PostGIS compatibility.
- OLTP/high-contention row locking.
- A custom DuckLake writer, catalog, or snapshot implementation.
- DataFusion, SedonaDB, PostgreSQL, or another auxiliary query engine.
- Row-wise spatial computation in Rust.
- Client-name-specific SQL branches.
- PL/pgSQL, triggers, LISTEN/NOTIFY, logical replication, or `pg_dump` fidelity.
- PostGIS topology, Tiger geocoder, SFCGAL, or raster pixel algebra.
- Multi-writer/horizontal-scale claims based only on emulators.
- Billion-row, 10 TB, trillion-class, or multi-modal release claims before the
  local vector product is proven.
