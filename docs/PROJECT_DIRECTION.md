# Project direction

This document defines the durable product direction.

- [ROADMAP.md](../ROADMAP.md) owns ordered milestones and exit gates.
- [ARCHITECTURE.md](../ARCHITECTURE.md) owns current implementation boundaries.
- [ROADMAP_STATUS.md](./ROADMAP_STATUS.md) owns the implemented evidence floor.
- [COMPATIBILITY.md](./COMPATIBILITY.md) owns current client and SQL claims.
- [POSTGRESQL_COMPATIBILITY.md](./POSTGRESQL_COMPATIBILITY.md) owns the target
  catalog, role, privilege, and REST delivery contract.
- [DUCKDB_ROADMAP_ALIGNMENT.md](./DUCKDB_ROADMAP_ALIGNMENT.md) owns conditional
  adoption and deletion gates for upstream DuckDB/DuckLake roadmap features.

## Product thesis

QuackGIS is a thin, high-performance PostgreSQL/PostGIS compatibility and control
edge for DuckDB Spatial and official DuckLake. One complete worker owns the Rust
policy edge, ADBC, and its in-process DuckDB execution engine.

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
   a load-balanceable PostgREST-style HTTP edge;
6. predictable streaming, cancellation, admission, and bulk-ingest behavior;
7. a tiny iroh client that is the sole application ingress, pairs once, supports
   desktop and serverless runtimes, and forwards pgwire or HTTP without owning SQL
   policy, worker selection, or storage credentials;
8. adaptive, low-overhead transport compression that reduces relayed bytes when
   measured savings justify its CPU and latency cost without compressing
   authentication/control traffic; and
9. horizontally replaceable complete workers over one managed control/catalog
   database and object store, without a central SQL router.

## Current reality

The repository proves a bounded local runtime:

- DuckDB and official DuckLake are the only query and storage path.
- ADBC transports Arrow between DuckDB and the Rust edge.
- Simple and extended pgwire support a deliberately narrow statement surface.
- Parameterized reads/mutations, transactions, text COPY, SCRAM, table policy,
  maintained session settings/search path, `public` mapping, quoted COPY, restart,
  and reopen have pinned native integration tests.
- Forty-three native, rewrite, or macro spatial cases execute through pgwire.
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

The first release has one state-owning, read-mostly spatial analytical worker with
controlled bulk ingestion over local official DuckLake. Maintained PostgreSQL/GIS
clients and optional stateless HTTP read replicas enter through the packaged tiny
client. Direct TCP remains a current/development correctness and performance
baseline rather than a supported application ingress. The early
one-client/one-worker iroh profile reaches the same complete Rust/ADBC/DuckDB
worker and does not claim shared storage or clustered authorization.

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
- a packaged tiny client that obtains a signed one-worker access lease from a
  minimal config-backed bootstrap and carries pgwire to that worker over iroh,
  with direct, public-relay-default, and explicitly configured relay performance,
  cancellation, COPY, reconnect, and resource evidence;
- no application-facing worker TCP/HTTP listener in the release profile; standard
  clients use the tiny client's owner-protected local or embedded adapter;
- restart, backup, restore, compaction, and upgrade procedures; and
- reproducible packages with no runtime extension downloads.

Local 1.0 accepts one source-pinned DuckLake divergence: a read-only table
function exposing durable schema/table/column identity from the committed
snapshot. QuackGIS owns its tracked patch, exact artifact digest, immutable
packaging, ABI/lifecycle/upgrade gates, and upstream deletion path. DuckLake's
official code remains the only metadata and data writer.

Using PostgreSQL as a shared DuckLake metadata catalog and object storage is a
later storage capability. It is distinct from QuackGIS's PostgreSQL-compatible
`pg_catalog` surface and must not block a useful local release or be claimed
before official DuckLake concurrency, visibility, and recovery evidence exists.

Iroh transport work is deliberately earlier than the shared-cluster milestone.
The one-client/one-complete-worker path must expose connection setup, direct-path
establishment, relay fallback, throughput, time-to-first-row, COPY, cancellation,
CPU, memory, stream-multiplexing, and adaptive-compression costs while the local
execution baseline is still small. I0 includes the control/edge protocol split,
registered-key proof, and one bootstrap-issued worker lease so later work cannot
move passwords or assignment into the client or worker. Durable PostgreSQL pairing
state, multi-bootstrap gossip reconciliation, and remote DuckLake remain later
layers built on that measured path.

## Shared iroh cluster target

After Local 1.0, the scale-out target is an iroh-native cluster of complete
QuackGIS workers over one managed PostgreSQL and object-storage stack:

```text
desktop / server / serverless application
             │ local socket or embedded adapter
             ▼
      tiny paired QuackGIS client
             │ iroh through configured relays
             ▼
       one assigned full worker
             │
             ├── protected QuackGIS control/user database
             ├── PostgreSQL-backed official DuckLake catalog
             └── shared object storage
```

The durable cluster contract is:

- SCRAM is used by bootstrap to pair or recover a client, not by workers and not
  stored as a runtime client password;
- pairing registers a generated credential public key in the control database;
  the credential private key and handle are stored in the desktop keystore or
  serverless secret store and remain usable while the role and credential row are
  enabled;
- bootstrap issues one short-lived signed access lease binding the credential
  public key, LOGIN role, permitted protocols, assigned worker, assignment
  generation, and security/configuration epochs. The lease is the runtime
  certificate and assignment proof; there is no second long-lived signed client
  certificate to renew or reconcile;
- the credential identity is distinct from the iroh transport `EndpointId`, so a
  service deployment may share one credential while each ephemeral instance uses
  its own iroh endpoint key;
- bootstrap, not the client, assigns one credential to one worker at a time and
  issues the lease. All pgwire and HTTP sessions use that worker until bootstrap
  fences the generation during an explicit drain or failure transition;
- workers enforce signed assignment generations; clients do not route individual
  statements or simultaneously fan one credential out across workers;
- bootstrap nodes and workers form one bounded gossip pool for liveness,
  readiness, drain, and coarse capacity; clients never join that gossip topic;
- PostgreSQL control metadata is authoritative for users, SCRAM verifiers, client
  credentials, roles, grants, policy, worker registration, pool configuration,
  assignments, revocation, and schema/security/configuration epochs; gossip is
  never an authorization or configuration authority;
- one authenticated `quackgis/edge/1` connection carries typed pgwire, HTTP, and
  cancellation streams under the same access lease, relay setup, compression
  negotiation, and connection lifecycle;
- the client contains no DuckDB, ADBC, SQL authorization, object-store
  credentials, gossip, worker list, scoring algorithm, or failover decision; it
  only obtains, caches, and follows a bootstrap-issued access lease;
- bootstrap, worker, and client relay lists are independently configurable. An
  omitted relay configuration selects iroh's public relay preset. A configured
  non-empty list selects exactly those relays; an explicitly configured empty list
  is invalid;
  and
- transport compression is negotiated per protocol connection with `none` as a
  mandatory fallback and an `auto` policy that emits independent bounded raw or
  compressed blocks according to measured gain, path cost, and available CPU.

Public relays make development and first use work without infrastructure setup.
Production deployments may select hosted relays without changing pairing,
identity, assignment, pgwire, HTTP, or storage semantics. Relay API secrets and
client/worker private keys remain in their respective secret stores and never
enter gossip or DuckLake metadata.

The native client is delivered as a small binary and reusable library. Desktop
mode exposes an owner-protected local socket, using a generated installation-local
credential only where the local transport cannot authenticate the OS user.
Serverless mode loads the registered service credential key and handle from the
platform secret store and uses an embedded adapter or process-local listener.
Multiple instances may share that credential while each uses an independent iroh
transport key and the same bootstrap assignment. Direct browser/WASM access is a
separate security and transport capability.

Client, bootstrap, and worker consume one shared transport implementation for
relay policy, ALPN and stream preludes, access-lease/proof types, framing,
compression, limits, errors, and metrics. Bootstrap/control uses
`quackgis/control/1` for SCRAM pairing and lease refresh. Workers accept
application data only through `quackgis/edge/1`; they do not handle pairing,
passwords, worker selection, credential registration/access-lease issuance, or
local-client secrets.

Shared operator recovery also uses authenticated iroh, but it is not pgwire data
or a tiny-client file transfer. A separately authorized, versioned control
capability starts and monitors a backend operation. The backend fences workers,
coordinates one recovery point across the official DuckLake PostgreSQL catalog,
the separate QuackGIS control database, and versioned object storage, and moves
bytes through provider-native snapshot/copy APIs. Restore stays available while
workers are fenced, advances assignment/security epochs, and requires independent
DuckDB reopen before service resumes. Ordinary LOGIN roles and application access
leases never grant this operator capability.

## Ownership rules

- DuckDB is the only query planner and spatial execution engine.
- DuckLake is the only writer of new durable user-schema catalogs and table data.
  QuackGIS's source patch adds one read-only identity function and does not alter
  that writer authority. The separate Shared 1.x PostgreSQL control database writes only
  QuackGIS identity, policy, configuration, and operational state.
- Rust does not implement row-wise spatial kernels or pull arbitrary table rows
  out of DuckDB for fallback execution.
- Rust does not maintain an independent table catalog, optimizer, or data cache.
- Local compatibility identity and catalog epochs may use protected metadata
  written through the supported DuckDB/DuckLake path. Shared users, SCRAM
  verifiers, client credentials, roles, memberships, grants, policy, worker
  pools, assignments, revocation, and security/configuration epochs live in a
  transactional PostgreSQL control database. Both remain separate from official
  DuckLake metadata and may project authoritative user schema but may not become
  a second user-table authority.
- Iroh authenticates transport endpoints. A bootstrap-signed access lease,
  credential-key proof, and the control database determine database identity and
  worker assignment; relay admission, client assertions, and gossip membership
  never grant a LOGIN role.
- Every worker owns a distinct iroh private key. A service client credential may
  intentionally be shared by serverless instances, but worker endpoint keys are
  never shared across replicas.
- Worker affinity reduces cross-worker visibility transitions but never replaces
  DuckLake conflict, visibility, backup, restore, or independent-reader evidence.
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
- Speculative upstream features may intentionally defer overlapping QuackGIS
  work when they have a plausible deletion path, but production adopts only a
  released pinned feature that passes the maintained gate.

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

### Connected sessions and native leases

The clustered worker separates cheap authenticated client sessions from native
DuckDB connection leases and active-query permits. Thousands of mostly idle
pgwire sessions may share one worker without allocating one native connection or
blocking thread per session. Autocommit statements lease a clean engine
connection; explicit transactions, COPY, and active or suspended result streams
pin one within separate limits. Idle transactions, portals, COPY input, queues,
and per-credential fan-out have bounded time and count limits. A reused native
connection is reset or quarantined before another pgwire session can acquire it.

### Iroh edge and worker pools

The tiny client obtains one signed access lease from a small bootstrap pool, then
maps each local pgwire, HTTP, or cancellation session to a typed stream on that
lease's worker. It never receives a pool to score. Worker addition does not
rebalance established clients. Graceful drain completes old sessions before
bootstrap advances the assignment; abrupt failure drops sessions, bootstrap
fences the old generation, and a refreshed lease permits reconnect to one
caught-up replacement. No transaction or statement is replayed transparently.

Bootstrap and worker gossip is soft state with bounded members, message size,
frequency, and expiry. PostgreSQL remains the source of truth for desired pool
state and assignment generations. Pool scale improves aggregate execution and
availability; one worker must still support the declared mostly-idle client
population with bounded memory and native leases.

### Adaptive transport compression

Compression runs in QuackGIS's framed tunnel before QUIC encryption and after the
remote client or worker connection is authenticated. Enrollment, SCRAM,
grant/access proofs, assignment/control messages, cancellation, and other small
latency-sensitive frames remain uncompressed. Compression state is isolated per
direction and pgwire/HTTP stream; dictionaries are never shared across clients,
credentials, requests, or sessions.

The negotiated protocol always supports `none`. The default `auto` policy samples
bounded application blocks, skips small or already incompressible data, and uses a
selected low-latency codec only when expected byte savings exceed its measured
CPU and latency cost. Relay use, direct-path bandwidth, worker CPU pressure, and
configured operator preference may influence the decision but never correctness.
Every block declares bounded compressed and decompressed lengths; invalid,
oversized, truncated, or expansion-ratio-violating input fails before allocation
or pgwire/HTTP delivery. Idle sessions retain no compression buffer.

Metrics expose aggregate input/output bytes, ratio, codec, CPU time, and
compressed/skipped/error counts without payloads, SQL, parameters, credentials,
or object paths. I0 selects the codec and thresholds from direct and relayed
compressible/incompressible profiles; the design does not commit to a codec before
that evidence.

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

Local 1.0 retains WKB plus the opt-in maintained bbox layout as its stored spatial
contract. Two clean 10M mixed point/line/polygon references show native
`GEOMETRY` files are about 45% smaller, but both layouts meet selective scan and
resource budgets while only the maintained WKB path has proven COPY, mutation,
pgwire, and catalog behavior. Re-evaluate this decision for each pinned DuckDB
candidate and delete the bbox machinery when native geometry also passes those
write/client contracts. M4's complete v5 workload proves both layouts through
selective scans, grouped aggregates, bounded joins, wide projections, and
compaction at 10M/100M; native geometry passes those analytical gates but not the
write/client lifecycle boundary.

### Observability

Measure queue/execution/time-to-first-row latency, Arrow batches, result/COPY
bytes, memory, spill, cancellation, files and bytes scanned, candidate/exact rows,
ingest throughput, transaction/conflict outcomes, and quarantined connections.
Never expose SQL text, parameters, credentials, or object paths in metrics.

## PostGIS migration path

Migration has two products with different dependency floors:

- **G0 offline snapshot** follows bounded COPY and catalog/type preflight. A small
  migrator holds source PostgreSQL credentials, reads one repeatable-read PostGIS
  snapshot, and writes only through the packaged iroh tiny client. It has no
  DuckDB, ADBC, object-store credentials, or DuckLake metadata access. Every
  source schema/type/geometry/security feature is migrated, explicitly mapped, or
  rejected before publication; exact counts/checksums/spatial summaries and named
  client reads gate cutover.
- **G1 online catch-up** waits for M6 durable control metadata, shared DuckLake,
  and worker fencing. One PostgreSQL logical slot exports the initial snapshot and
  start LSN. Durable source/LSN batch identity makes at-least-once decoding
  idempotent across target commit response loss; it is not described as
  distributed exactly-once commit. Supported UPDATE/DELETE tables require a real
  source replica identity, DDL stops apply, and cutover freezes PostGIS writes only
  long enough to drain to a declared final LSN and rerun G0 verification.

PostGIS remains the read-only rollback source for a timed window. The first
release has no dual-write, reverse replication, physical PostgreSQL replication,
or implicit migration of RLS, triggers, functions, roles/passwords, unsupported
types, or unsupported CRS/dimension semantics.

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
  maintained read clients, packaged role-aware HTTP read/OpenAPI replicas, and a
  measured tiny-client/minimal-bootstrap/one-complete-worker iroh transport
  profile using registered-key proof and a signed one-worker access lease.
- **Shared 1.x:** official shared DuckLake using managed catalog/object storage,
  protected PostgreSQL control state, a tiny paired iroh client, a bounded
  bootstrap/worker gossip pool, credential-to-worker affinity, and common pgwire
  and HTTP delivery. It is enabled only after concurrency, visibility, identity,
  revocation, relay, backup, and restore gates.
- **Dataset lifecycle 1.x:** protected versions, promotion, rollback, retention,
  and maintained summaries using official primitives.
- **Later research:** multi-modal inventories and national-scale stress after the
  10M and 100M vector gates are routine.

Iroh is the client and cluster-control transport; it does not split DuckDB from a
complete QuackGIS worker or bypass the Rust policy edge. Official DuckLake
protected snapshots, RBAC, UDTs, and materialized views are preferred future
primitives where they can delete QuackGIS control or summary machinery without
weakening PostgreSQL-facing semantics.

## Explicit non-goals

- Full PostgreSQL or PostGIS compatibility.
- OLTP/high-contention row locking.
- A custom DuckLake writer, catalog, or snapshot implementation.
- DataFusion, SedonaDB, PostgreSQL, or another auxiliary query engine.
- Row-wise spatial computation in Rust.
- Client-name-specific SQL branches.
- A central SQL router or manager in the query data path.
- Direct application connections to a worker that bypass the tiny client and its
  bootstrap-issued access lease.
- Per-statement worker routing or simultaneous multi-worker use by one client
  credential.
- Client-side worker discovery, scoring, assignment, or failover decisions.
- Client participation in the worker/bootstrap gossip pool.
- Transparent replay of failed statements or transactions on another worker.
- Reusing one iroh worker private key across autoscaled replicas.
- Treating relay access, a gossip topic, or a client-supplied role header as
  database authorization.
- Compressing enrollment/authentication/control traffic or sharing compression
  dictionaries across clients, credentials, requests, or sessions.
- PL/pgSQL, triggers, LISTEN/NOTIFY, logical replication, or `pg_dump` fidelity.
- Client-side `pg_dump` or iroh file streaming as the DuckLake backup mechanism;
  shared backup bytes move between backend PostgreSQL/object-storage systems.
- PostGIS topology, Tiger geocoder, SFCGAL, or raster pixel algebra.
- Multi-writer/horizontal-scale claims based only on emulators.
- Billion-row, 10 TB, trillion-class, or multi-modal release claims before the
  local vector product is proven.
