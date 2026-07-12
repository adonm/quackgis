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
bounded query, create-table, insert, update, delete, simple transaction, and strict
maintained SET/SHOW shapes. An AST relation visitor maps PostgreSQL `public` to
DuckLake `quackgis.main` before execution while policy sees the original target.
Unsupported shapes fail closed. COPY uses parsed one-/two-/three-part identifiers
and dedicated protocol state.

### Storage authority

Startup atomically creates or validates `_quackgis/storage-authority-v1` below the
local data root before attach. A mismatched marker fails. Migration targets a
separate root; alternating writers is never supported.

## Query lifecycle

Current path:

```text
SQL → normalize/rewrite → PostgreSQL AST → authorization/admission
    → per-client DuckDB ADBC session → describe/bind/execute
    → owned ADBC reader → one Arrow batch → pgwire rows → client
```

The live stream owns its ADBC reader, statement, connection lease, admission
permit, cancellation registration, and deadline. Pgwire requests another native
batch only after exhausting the current batch; portals consume the same stream.
Native interruption maps to SQLSTATE `57014`. Only a stream that reaches native
EOF returns its connection; cancellation, reader failure, or dropping a partially
delivered result quarantines the session. Driver batches larger than the
configured byte ceiling fail with SQLSTATE `54000` before pgwire encoding. This
bounds the protocol edge but does not
prevent a native driver from temporarily allocating that batch; full RSS evidence
remains open.

## Session and transaction ownership

Each pgwire client lazily opens an independent DuckDB session. Explicit
transactions remain session-affine. Reentrant use fails instead of deadlocking.
Native failures that make commit/rollback state uncertain quarantine the session.
Each first quarantine transition increments a path-free process counter.
Disconnect attempts rollback. Future pools may reuse only clean, idle sessions.

All runtime ADBC calls pass through a fixed process-owned blocking-worker pool.
Regular work is capped below the total by one slot so cancellation/control work
cannot queue behind every operation it may need to interrupt. Admission permits
bound complete operations globally and within reader/writer classes; a reserved
maintenance class defines the control-plane ceiling for future server-exposed
maintenance. Worker permits bound only the synchronous native call.

## COPY lifecycle

COPY incrementally decodes protocol chunks into row- and byte-bounded Arrow
batches. One bounded channel feeds one ADBC stream into a session-local temporary
DuckDB table. Clean EOF publishes to DuckLake with one atomic `INSERT`; parse
failure, disconnect, cancel, or timeout drops the staging input without touching
the target. Worker-owned admission/deadline guards release on cancellation; the
pinned pgwire callback API cannot deliver an asynchronous error to an idle COPY
socket until its next frame or disconnect. COPY is the primary bulk path;
repeated INSERT is compatibility only.

When a target declares all four `DOUBLE` columns `_qg_minx`, `_qg_miny`,
`_qg_maxx`, `_qg_maxy` and COPY supplies a recognized binary geometry column,
the publication statement computes bbox values with DuckDB Spatial. Rust never
decodes geometry rows. The reserved columns must be nullable, the table must have
exactly one recognized geometry field, and clients may not supply bbox values.
Partial, wrong-type, caller-supplied, or ambiguous layouts fail closed before
staging; tables with no reserved bbox columns are copied unchanged. Direct INSERT,
geometry assignments, and reserved bbox assignments fail closed until schema-aware
recomputation exists. UPDATEs touching only ordinary columns are permitted and
leave maintained geometry/bounds unchanged; COPY remains the supported spatial
write path.

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
conservative name convention. A narrow structural `pg_type` adapter resolves those
two sentinel OIDs, and the native workflow proves geometry RowDescription plus
text, binary, and NULL transport. Broad catalog discovery, named-client evidence,
and durable subtype/SRID/dimension identity remain open.

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
