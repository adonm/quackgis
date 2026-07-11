# Project direction

This document defines the durable product direction.

- [ROADMAP.md](../ROADMAP.md) owns ordered milestones and exit gates.
- [ARCHITECTURE.md](../ARCHITECTURE.md) owns current implementation boundaries.
- [ROADMAP_STATUS.md](./ROADMAP_STATUS.md) owns the implemented evidence floor.
- [COMPATIBILITY.md](./COMPATIBILITY.md) owns current client and SQL claims.

## Product thesis

QuackGIS is a thin, high-performance PostgreSQL/PostGIS compatibility and control
edge for DuckDB Spatial and official DuckLake.

It lets PostgreSQL-oriented spatial tools use DuckDB's analytical and spatial
execution without introducing another query engine, storage writer, distributed
database, or row-wise geometry runtime. The Rust service owns protocol
compatibility, authorization, resource control, and operational policy. DuckDB
owns planning, vectorized execution, exact spatial computation, and official
DuckLake persistence.

The product advantage is the combination of:

1. maintained pgwire/PostGIS workflows at the edge;
2. DuckDB-native analytical and spatial performance;
3. official DuckLake storage and snapshot semantics;
4. bounded, trace-driven compatibility rather than broad emulation; and
5. predictable streaming, cancellation, admission, and bulk-ingest behavior.

## Current reality

The repository proves a bounded local runtime:

- DuckDB and official DuckLake are the only query and storage path.
- ADBC transports Arrow between DuckDB and the Rust edge.
- Simple and extended pgwire support a deliberately narrow statement surface.
- Parameterized reads/mutations, transactions, text COPY, SCRAM, table policy,
  restart, and reopen have pinned native integration tests.
- Forty-two native, rewrite, or macro spatial cases execute through pgwire.
- DuckDB and extension artifacts are version- and checksum-pinned.

It is not yet a resource-bounded service:

- query results are materialized before pgwire delivery;
- COPY buffers a complete request and has a 16 MiB ceiling;
- pgwire cancellation does not cancel the native statement;
- blocking query work has no productized admission budget;
- DuckDB memory, thread, temporary-storage, and spill policy are not configurable
  as a stable server contract;
- PostgreSQL catalogs, geometry identity, and named GIS clients are incomplete;
- shared catalog/object-storage profiles fail closed; and
- current scale evidence is fixture-level, not a product performance claim.

Direction starts from these constraints, not from capabilities of retired engines.

## First release

The first release is a single-process, read-mostly spatial analytical service with
controlled bulk ingestion over local official DuckLake.

Release-required outcomes:

- bounded streaming query results and COPY;
- native cancellation, deadlines, admission, memory limits, and spill policy;
- `psql`, `psycopg`, GDAL/OGR read and COPY load, and QGIS read-only workflows;
- a focused, versioned PostGIS function/catalog/type surface;
- selective spatial reads and ordinary DuckDB OLAP with measured plans;
- restart, backup, restore, compaction, and upgrade procedures; and
- reproducible packages with no runtime extension downloads.

Shared PostgreSQL catalog/object-storage operation is a later capability. It must
not block a useful local release or be claimed before official DuckLake
concurrency, visibility, and recovery evidence exists.

## Ownership rules

- DuckDB is the only query planner and spatial execution engine.
- Official DuckLake is the only writer of new durable catalogs and data.
- Rust does not implement row-wise spatial kernels or pull arbitrary table rows
  out of DuckDB for fallback execution.
- Rust does not maintain an independent table catalog, optimizer, or data cache.
- PostgreSQL compatibility exists only at observable protocol, SQL, type, and
  catalog boundaries required by maintained workflows.
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
ingest stream under one transaction. Bound rows and bytes per batch; roll back on
parse failure, cancellation, disconnect, or timeout. COPY is the primary bulk path;
INSERT remains a compatibility path.

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
  official DuckLake with bulk ingest and maintained read clients.
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
