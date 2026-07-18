# Architecture

QuackGIS is a Rust PostgreSQL wire/control edge over DuckDB Spatial and official
DuckLake. DuckDB is the sole planner/executor. Official DuckLake is the sole writer
of new durable catalogs and Parquet data.

Forward outcomes belong in [ROADMAP.md](./ROADMAP.md). Current evidence belongs in
[docs/ROADMAP_STATUS.md](./docs/ROADMAP_STATUS.md). Product ownership and extension
rules belong in [docs/PROJECT_DIRECTION.md](./docs/PROJECT_DIRECTION.md). The
target PostgreSQL catalog/RBAC design belongs in
[docs/POSTGRESQL_COMPATIBILITY.md](./docs/POSTGRESQL_COMPATIBILITY.md). Conditional
adoption of upstream DuckDB/DuckLake roadmap work belongs in
[docs/DUCKDB_ROADMAP_ALIGNMENT.md](./docs/DUCKDB_ROADMAP_ALIGNMENT.md).

## Layer model

```text
PostgreSQL / GIS / application clients
                  │ pgwire
                  ▼
┌──────────────────────────────────────────────────────────────┐
│ Rust protocol and control edge                               │
│ startup · TLS/SCRAM · simple/extended protocol · COPY        │
│ target: roles/session · catalogs/privileges · shared policy  │
│ portals · Arrow↔PostgreSQL encoding · PostGIS compatibility  │
│ bounded request context · audit/metrics                      │
└──────────────────────────────────────────────────────────────┘
                  │ Arrow / ADBC inside one complete worker
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

The table includes target ownership explicitly; a target owner does not imply the
capability is implemented in the current preview.

| Component | Owns | Must not own |
|---|---|---|
| Rust pgwire edge | protocol state, TLS/SCRAM, target PostgreSQL-facing roles/session/catalog projection, parsed policy, COPY framing, PostgreSQL types/errors, connection lifecycle | SQL planning, spatial kernels, table data, an independent user-schema authority |
| DuckDB | SQL planning, vectorized execution, exact spatial operations, transactions, resource/spill behavior | PostgreSQL protocol or identity policy |
| official DuckLake | catalog, snapshots, Parquet publication, maintenance primitives | client compatibility or authorization |
| `quackgis-migrate` preview | pinned PostGIS source identity/inventory, fail-closed type/object disposition, one repeatable-read source snapshot, bounded pgwire COPY forwarding, canonical verification, path-free report | source credentials in workers, DuckDB/ADBC/object-store access, private DuckLake metadata, implicit DDL/security conversion, online replication or cutover authority |
| planned QuackGIS control metadata | local compatibility identity through supported DuckDB/DuckLake transactions; shared users, credentials, roles, policy, pools, assignments, and security/configuration epochs in a protected PostgreSQL control database | user table definitions/data, SQL planning, independent snapshot publication |
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

The I0 ingress treats an authenticated, end-to-end encrypted iroh connection as
an already secure pgwire channel; direct TCP continues to require configured TLS
outside development, but the iroh cluster leg does not nest pgwire TLS inside
QUIC. The shared protocol and executable seam now provide a config-backed
registered credential, bootstrap-signed one-worker lease, challenged key proof,
bounded loopback tiny client, and typed pgwire/cancellation worker streams. The
worker validates pgwire startup `user` against the lease, answers SSL/GSS requests
without nested encryption, and accepts only `AuthenticationOk` from the loopback
backend before forwarding any client authentication traffic. The tiny client recognizes initial cancellation framing but never parses SQL.
Current direct, forced-custom-relay, and opt-in public-default-relay evidence runs
one differential DuckDB/DuckLake result/type/error/transaction/COPY/cancellation
oracle. Mandatory `none` plus optional bounded adaptive LZ4 starts only after
`AuthenticationOk`; each stream direction has independent 64 KiB blocks and no
shared dictionary. Clean 8/32/64 MiB host profiles commit direct/relay latency,
CPU, RSS, throughput, cancellation, byte-saving, and codec budgets. K0 packages
the direct path in one ordered Pod: backend pgwire stays loopback-only, the
readiness-gated application Service exposes only a mutual-TLS tiny client, and
per-process key volumes plus denial/rotation Jobs protect the local boundary.
Packaged resource budgets and hosted-relay reruns remain open.

The release application path always enters through the tiny client. Bootstrap
registers the client-generated credential public key, selects one worker, and
signs a short-lived access lease; the client follows that lease and never receives
a pool to score. One
`quackgis/edge/1` connection carries typed pgwire, HTTP, and cancellation streams.
Workers verify the lease and credential-key proof before attaching `session_user`;
they do not handle pairing passwords, assignment, or local-client authentication.
Direct TCP remains a current/development test oracle rather than release ingress.

The target identity model distinguishes authenticated `session_user` from the
effective `current_user`. Configuration-backed LOGIN/NOLOGIN roles, memberships,
object grants, `SET ROLE`, transaction-local role/context, and catalog privilege
queries will be implemented at this boundary before mutable role DDL or RLS is
considered. Cleanup on commit, rollback, cancellation, disconnect, and native
connection reuse is part of the target security contract.

### Offline migration

The G0 migrator is an operator-side client, not a worker capability. It alone
opens the pinned PostgreSQL/PostGIS source and holds source credentials. A
read-only repeatable-read transaction starts before identity/inventory and remains
open through source COPY and checksum verification; the worker sees only ordinary
pgwire target DDL/COPY/verification through the configured target listener. The
migrator has no DuckDB, ADBC, object-store, or private DuckLake metadata access.

Source catalogs, identifiers, defaults, and rows are untrusted migration input.
Configuration and inventory counts are bounded, identifiers are quoted, and only
classified release types plus literal defaults can produce target SQL. Every
item returned by the maintained table/column/constraint/object/security inventory
gets an explicit migrate/map/reject disposition. One target transaction contains all table creation, COPY, and
comments, so a pre-commit conversion failure rolls back the whole prepared set.
Commit response loss is indeterminate rather than retried; after successful commit
a fresh target connection must reproduce count, NULL, complete-row, and
per-column canonical multiset checksums.

The direct actual-process smoke reaches QuackGIS on development loopback. The
separate packaged gate executes the provenance-pinned migrator as a non-root Job,
uses a distinct `migration_operator` credential/lease and migration-only client
CA, traverses the mutual-TLS tiny client and iroh worker, and rejects the ordinary
K0 client certificate. The source PostGIS sidecar exists only in that Job's network
namespace. Isolated staging-root promotion, runtime identity in the report,
role/grant application, restart verification, and named post-migration clients
remain G0 gates. A `verified` report means snapshot data is prepared for an
operator decision, not
that clients were atomically cut over or PostGIS can be retired. See
[docs/POSTGIS_MIGRATION.md](./docs/POSTGIS_MIGRATION.md).

### SQL admission

Standalone `sqlparser` parses exactly one general statement. The only batch path
accepts at most eight simple-protocol statements and requires every member to be a
strict maintained session `SET`; it emits one completion per member and never
reaches ADBC. The general allowlist admits bounded query, create-table, owner-only
table/column comment and single-table drop, insert, update, delete, simple
transaction, and maintained SET/SHOW shapes. An AST relation
visitor maps PostgreSQL `public` to DuckLake `quackgis.main` before execution while
policy sees the original target. Unsupported shapes fail closed. COPY uses parsed
one-/two-/three-part identifiers and dedicated protocol state.

DuckDB's planned default PEG parser does not replace this authorization boundary
unless a released stable API can provide an equivalent restricted AST contract.
Engine upgrades must run accepted and denied SQL under both parser modes while
both exist. Runtime grammar extension is operator-controlled and may never widen
client SQL implicitly.

### Storage authority

Startup atomically creates or validates `_quackgis/storage-authority-v1` below the
local data root before attach. A mismatched marker fails. Migration targets a
separate root; alternating writers is never supported.

### Backup and recovery control

The M5 exact-path backup tool is a local-singleton backend primitive. It copies a
stopped local DuckLake catalog and data root and is not the Shared 1.x backup
transport.

Shared recovery is an operator-only control operation reached over authenticated
iroh. Iroh carries a bounded request, operation identifier, status, and audit
result; it never carries catalog dumps, object payloads, cloud credentials, or
private keys. The backend fences writers and new assignments, records one declared
DuckLake/control recovery point, snapshots the managed PostgreSQL DuckLake and
QuackGIS control catalogs, protects the referenced versioned object-store set,
and verifies the result through an independent version-matched DuckDB reopen.
Provider-native snapshot/copy APIs move the durable bytes directly between
backend storage systems.

Restore runs while worker generations remain fenced. It restores the catalog and
object versions to the declared recovery point, advances assignment/security
epochs so stale leases cannot return, verifies schemas/counts/samples/snapshots
independently, and only then admits workers. A normal LOGIN role, pgwire SQL,
HTTP request, tiny-client filesystem, or relay credential cannot authorize this
operation. `pg_dump` compatibility is neither a complete DuckLake backup nor the
shared recovery mechanism.

## Query lifecycle

Current path:

```text
SQL → normalize/rewrite → PostgreSQL AST → authorization/admission
    → per-client DuckDB ADBC session → describe/bind/execute
    → owned ADBC reader → one Arrow batch → pgwire rows → client
```

ADBC remains inside each complete worker. The planned iroh transport terminates at
the same Rust pgwire/HTTP/catalog/authorization edge and does not replace this
engine boundary or expose DuckDB directly. An out-of-process engine would require
an explicit direction change and evidence that it can serve the attached official
DuckLake data plane while preserving streaming, parameters, transaction outcomes,
cancellation, extension pins, and resource budgets. Async I/O is adopted only
when exposed by a supported cancellable client API, at which point it replaces the
matching blocking path rather than creating a second execution path.

The live stream owns its ADBC reader, statement, connection lease, admission
permit, cancellation registration, and deadline. Pgwire requests another native
batch only after exhausting the current batch; portals consume the same stream.
Native interruption maps to SQLSTATE `57014`. Only a stream that reaches native
EOF returns its connection; cancellation, reader failure, or dropping a partially
delivered result quarantines the session. Driver batches larger than the
configured byte ceiling fail with SQLSTATE `54000` before pgwire encoding. This
bounds the protocol edge but does not prevent a native driver from temporarily
allocating that batch. Clean 1M/10M generated-BIGINT profiles prove cardinality-
independent process RSS for BIGINT, and the 1M nullable VARCHAR/BLOB profile
crosses hundreds of native batches within budget. Additional type shapes and the
maximum driver-produced batch remain open.

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
reserved bbox assignments, tuple geometry assignment, and arbitrary geometry
expressions fail closed. A numbered-bound parameter or NULL geometry UPDATE
recomputes all four bounds in the same DuckDB statement. UPDATEs touching only
ordinary columns preserve maintained geometry/bounds; COPY remains the primary
spatial write path.

At the storage trust boundary, before native describe/prepare/execute, a narrow
AST rule may add planner-visible bbox candidates to one-table reads over that
maintained layout. It accepts one mandatory `AND` conjunct shaped as
`ST_Intersects(ST_GeomFromWKB(the maintained column), probe)`, where `probe` is a
bounded literal envelope/text geometry or numbered-bound WKB. The generated
four-axis overlap test is conjoined with the original exact predicate; it never
replaces the exact DuckDB Spatial call. Joins, OR/NOT placement, subqueries,
multiple exact predicates, arbitrary/oversized probe expressions, and
non-maintained layouts are left unchanged; malformed or ambiguous reserved
layouts fail closed. Describe and execution use the same rewrite.

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
conservative name convention. Process-local relational namespace/type/range,
collation, and owner-role views include the PostgreSQL 18 profile and QGIS-required
built-ins, every referenced array partner, and both sentinel OIDs. Structural
rewriting honors explicit and implicit `pg_catalog` lookup. Proven source
projections receive explicit Arrow type hints; output aliases alone never select
PostgreSQL OID/`name`/internal-`char` encoding. Restricted identities cannot bypass
metadata policy through either private or unqualified names, and traced but
unimplemented `pg_catalog`/unqualified `pg_*` relations fail explicit `0A000`
rather than falling through to DuckDB or a user object. Catalog CTE shadowing,
wildcards, nested/set/derived type-preserving expressions, implicit-column joins,
and cross-database qualification are also rejected until provenance can preserve
PostgreSQL wire identity. Clients cannot address the private rewrite schema, and
the `TABLE` query form is rejected before authorization because its parser shape
does not retain sufficient structural identity. User-object catalogs,
RowDescription relation origins, named-client execution, and durable
subtype/SRID/dimension identity remain open.

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
For Local 1.0, WKB plus the opt-in maintained bbox columns remains the stored
contract. M4 proves selective scans, grouped aggregates, bounded spatial joins,
wide projections, compaction, and resources twice at both 10M and 100M. Native
`GEOMETRY` is a smaller measured candidate that passes the same analytical gates
but has not yet passed the maintained path's write/client lifecycle contract.

## PostgreSQL compatibility

Compatibility is surface-oriented and trace-driven:

- target a declared PostgreSQL 18 profile for maintained clients and REST;
- preserve observed row labels, OIDs, parameter types, nullability, formats,
  SQLSTATEs, source relation/attribute identity, and transaction behavior;
- derive user schema from DuckDB/DuckLake rather than maintain a second table
  catalog;
- maintain only protected role/grant/policy/epoch and compatibility identity
  control metadata required for coherent PostgreSQL behavior;
- keep `pg_catalog` visibility, privilege-filtered `information_schema`, privilege
  inquiry functions, execution authorization, and OpenAPI mutually consistent;
- use synthetic rows only for PostgreSQL concepts that do not exist and are safe;
- never branch on a client name; and
- remove shims when DuckDB or pgwire provides the same contract.

Catalog queries must execute relationally over maintained rows rather than match
one complete SQL string. Standard built-in OIDs are fixed by the profile;
installation-local compatibility and user-object OIDs need only be internally
stable and self-consistent. DuckDB's transient object OIDs are not suitable
because they change across rename/reopen. Broad PostgreSQL emulation remains a
non-goal.

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

## Optional REST edge

`quackgis-rest` is currently a separate, stateless, read-only HTTP process. It
reuses an immutable revision of `pg-rest-server`'s URL parser/query engine but reaches data
only through the maintained QuackGIS pgwire boundary. It does not link ADBC,
publish DuckLake state, or become a second catalog/security authority. The
current preview validates bounded HS256 JWT signature/issuer/audience/time/role,
uses one authenticator pgwire identity, and wraps each read in transaction-local
role/context. Role-filtered PostgreSQL catalog results drive per-role schema and
OpenAPI caches; shared epochs are consumed with the pinned identity artifact and
signed-only startup uses exact revision fallback. K0 packages two replicas. Each uses
a passwordless loopback tiny client whose separately proven credential is mapped
to an exact `authenticator` lease; the core loopback listener validates the role
catalog before `AuthenticationOk`. Replica readiness, denial, failover, reconnect,
and replacement credential/JWT rotation are executable.
Shared 1.x moves HTTP to the same assigned complete worker as pgwire over the
measured iroh edge. Unsupported PostgREST behavior fails closed until a maintained
compatibility case exists.

## Deployment model

The only maintained runtime image is the verified DuckDB image containing the
server, REST and edge binaries, exact `libduckdb.so`, signed Spatial, the exact
source/artifact-pinned read-only DuckLake identity patch, and an isolated DuckDB
home. A bare
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
