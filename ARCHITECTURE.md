# Architecture

QuackGIS is a Rust PostgreSQL wire/control edge over DuckDB Spatial and official
DuckLake. DuckDB is the sole planner/executor. Official DuckLake is the sole writer
of new durable catalogs and Parquet data.

Forward outcomes belong in [ROADMAP.md](./ROADMAP.md). Current evidence belongs in
[docs/ROADMAP_STATUS.md](./docs/ROADMAP_STATUS.md). Product ownership and extension
rules belong in [docs/PROJECT_DIRECTION.md](./docs/PROJECT_DIRECTION.md).

## Layer model

```text
PostgreSQL / GIS / application clients
                  │ pgwire
                  ▼
┌──────────────────────────────────────────────────────────────┐
│ Rust protocol and control edge                               │
│ startup · TLS/SCRAM · simple/extended protocol · COPY        │
│ structural SQL policy · portals · Arrow↔PostgreSQL encoding  │
│ bounded PostGIS rewrites/macros · audit/metrics              │
└──────────────────────────────────────────────────────────────┘
                  │ Arrow / ADBC
                  ▼
┌──────────────────────────────────────────────────────────────┐
│ DuckDB + official extensions                                 │
│ SQL planning/execution · Spatial · memory/spill · DuckLake    │
└──────────────────────────────────────────────────────────────┘
                  │
                  ▼
       official DuckLake catalog + Parquet
       maintained now: local catalog and data path
       future: supported shared catalog/object storage
```

## Component ownership

| Component | Owns | Must not own |
|---|---|---|
| Rust pgwire edge | protocol state, TLS/SCRAM, parsed table policy, COPY framing, PostgreSQL types/errors, connection lifecycle | SQL planning, spatial kernels, table data, independent catalogs |
| DuckDB | SQL planning, vectorized execution, exact spatial operations, transactions, resource/spill behavior | PostgreSQL protocol or identity policy |
| official DuckLake | catalog, snapshots, Parquet publication, maintenance primitives | client compatibility or authorization |
| `vendor/arrow-pg` | Arrow field/row encoding and maintained WKB wire identity | planning, catalogs, DataFusion support |
| optional future QuackGIS DuckDB extension | measured vectorized functions unavailable through native SQL/macros | pgwire, auth, policy, COPY, catalogs, snapshots, DuckLake writes |

## Trust boundaries

### Native runtime

`QUACKGIS_DUCKDB_ADBC_DRIVER` points to native code loaded in-process. It is
operator configuration, never SQL/client input. Startup verifies the exact
committed library SHA-256 and DuckDB version before claiming storage. Production
uses preinstalled signed `spatial` and `ducklake` extensions with `LOAD` only.

### Network and identity

Pgwire startup terminates at the Rust edge. Trust mode is development-only;
password mode uses SCRAM-SHA-256. TLS certificate and key must be configured
together. Parsed read/write policy runs before ADBC prepare or schema lookup.

### SQL admission

Standalone `sqlparser` parses exactly one statement. The current allowlist admits
bounded query, create-table, insert, update, delete, and simple transaction shapes.
Unsupported shapes fail closed. COPY has a dedicated parser and protocol state.

### Storage authority

Startup atomically creates or validates `_quackgis/storage-authority-v1` below the
local data root before attach. A mismatched marker fails. Migration targets a
separate root; alternating writers is never supported.

## Query lifecycle

Current path:

```text
SQL → normalize/rewrite → PostgreSQL AST → authorization
    → per-client DuckDB ADBC session → describe/bind/execute
    → Vec<RecordBatch> → Arrow-to-pgwire encoder → client
```

The materialized `Vec<RecordBatch>` boundary is migration debt. The target owns a
live ADBC reader/statement/connection lease and transfers bounded batches to async
pgwire with backpressure. Portals consume the same stream. Cancellation targets
the active native statement, and uncertain cleanup quarantines the connection.

## Session and transaction ownership

Each pgwire client lazily opens an independent DuckDB session. Explicit
transactions remain session-affine. Reentrant use fails instead of deadlocking.
Native failures that make commit/rollback state uncertain quarantine the session.
Disconnect attempts rollback. Future pools may reuse only clean, idle sessions.

## COPY lifecycle

Current COPY parses a bounded complete request into Arrow and ingests through ADBC.
The target parser incrementally builds bounded Arrow batches from protocol chunks
under one transaction. Parse failure, disconnect, cancel, or timeout must publish
zero rows. COPY is the primary bulk path; repeated INSERT is compatibility only.

## Spatial compatibility

DuckDB Spatial owns exact execution. Compatibility follows the decision ladder:

1. native DuckDB function;
2. optimizer-visible SQL macro or quote-safe rewrite;
3. Rust protocol/catalog edge for inherently PostgreSQL behavior;
4. vectorized DuckDB extension for measured gaps.

`spatial_compat.rs` currently rewrites a bounded set of function identifiers while
preserving strings, quoted identifiers, dollar bodies, and comments. Startup
installs uniquely named compatibility macros. Authorization parses the resulting
statement and remains table-structural.

WKB/EWKB is the current transport/interchange format. A binary field may advertise
a maintained geometry/geography sentinel OID through explicit Arrow metadata or a
conservative name convention, but broad `pg_type` discovery and durable subtype/
SRID/dimension identity remain open.

## Spatial performance

Exact predicates remain inside DuckDB. Bbox/locality predicates are candidates
only and may over-select. Any automatic optimization must:

- recognize a structurally safe query shape;
- keep the original exact predicate in the DuckDB plan;
- prove equality against unpruned execution for holes, boundaries, null/empty,
  invalid, and skewed geometries; and
- publish scan bytes, candidate rows, exact rows, memory, spill, and plan evidence.

Layout columns should be computed in DuckDB during COPY or compaction. Rust must
not decode table geometry row-by-row. Native DuckDB/DuckLake statistics,
partitioning, and geometry representation are preferred when measurements pass.

## PostgreSQL compatibility

Compatibility is surface-oriented and trace-driven:

- preserve observed row labels, OIDs, parameter types, nullability, formats,
  SQLSTATEs, and transaction behavior;
- derive catalog data from DuckDB rather than maintain a second catalog;
- use synthetic rows only for PostgreSQL concepts that do not exist and are safe;
- never branch on a client name; and
- remove shims when DuckDB or pgwire provides the same contract.

Broad PostgreSQL emulation is not a goal. The first release targets only the
catalog/type/protocol queries required by psql, psycopg, GDAL/OGR, and QGIS
read-only workflows.

## Resource model

The release architecture requires explicit limits for:

- accepted connections;
- active readers, writers, and maintenance tasks;
- blocking ADBC workers;
- Arrow batch/queue bytes;
- DuckDB threads and memory;
- temporary directory and spill; and
- query/queue deadlines.

Overload queues or fails with a stable PostgreSQL error. Capacity is reserved for
cancellation, health, rollback, and shutdown. Metrics exclude SQL text,
parameters, credentials, and object paths.

## Deployment model

The only maintained runtime image is the verified DuckDB image containing the
server, exact `libduckdb.so`, signed extensions, and isolated DuckDB home. A bare
Rust binary is not a complete runtime distribution.

The current supported profile is one process over local official DuckLake. Shared
catalog/object storage, multiple processes, and managed-service recovery are
future milestones and fail closed today.

## Architectural invariants

1. PostGIS is an interface, not the engine.
2. DuckDB Spatial decides exact spatial truth.
3. Official DuckLake is the durable publication boundary.
4. WKB/EWKB is stable interchange until measured evidence changes it.
5. Protocol, authorization, engine execution, and storage publication are separate
   boundaries.
6. Unsupported behavior fails closed.
7. Memory scales with configured bounds, not result or ingest cardinality.
8. Client traces precede compatibility code.
9. Every optimization retains a deterministic correctness oracle.
10. Every large claim names data, hardware, artifact versions, and evidence ring.
