# Roadmap

This is the ordered forward roadmap for the DuckDB-only product. Current evidence
lives in [docs/ROADMAP_STATUS.md](./docs/ROADMAP_STATUS.md); durable direction and
the extension decision ladder live in
[docs/PROJECT_DIRECTION.md](./docs/PROJECT_DIRECTION.md).
The detailed PostgreSQL catalog, RBAC, and REST dependency plan lives in
[docs/POSTGRESQL_COMPATIBILITY.md](./docs/POSTGRESQL_COMPATIBILITY.md).

A milestone closes only when:

- implementation runs through the DuckDB-only server;
- tests are registered and execute in the named gate;
- performance budgets name hardware, data, and native artifact versions;
- evidence records the exact source SHA; and
- status and compatibility documents are updated.

Retired-engine behavior, unregistered tests, design documents, and static profile
validation do not close milestones.

## Local-first execution model

Most functional M1–M5 work must be reproducible on one developer workstation.
The same scenario and correctness oracle advance through four evidence levels;
only scale, duration, topology, and budget strictness change:

| Level | Typical duration | Purpose | Claim boundary |
|---|---:|---|---|
| smoke | seconds | code and contract regression | no scale or operational claim |
| local | minutes | full scenario/oracle at reduced scale | functional evidence only |
| reference | minutes to 24 hours | exact roadmap scale/budgets on a named host | may close local performance/soak gates |
| external | scheduled | publication or managed-service proof | required only where the gate explicitly says published or managed |

Environment ownership is deliberate:

- direct host processes or one resource-constrained container own RSS, latency,
  throughput, spill, and scan-byte budgets;
- a minimal DuckDB-only Kind cluster owns immutable-image startup, pinned client
  jobs, TLS/rotation, drain/restart, backup/restore, upgrade, and mixed-workload
  topology evidence;
- Kind PostgreSQL/MinIO services may rehearse M6 behavior after Local 1.0, but do
  not replace the required managed catalog, object-storage, iroh-relay, and
  serverless runs; and
- the old DataFusion/Sedona/Linkerd/shared-profile deployment tree remains retired.

Every profile emits one common evidence envelope containing source/dirty state,
profile ID and level, native artifact digests, host/cgroup capacity, data shape,
budgets, exact-result oracles, measurements, and pass/fail status. A reduced local
run must use the same implementation and oracle as its reference counterpart.

## Closure workstreams

The identifiers below preserve the established work-package names. Completed
foundations remain listed. The immediate native/compatibility dependency is now
**N0 → {S0, Q0}**: establish one clean DuckDB/DuckLake/Spatial/QuackGIS bundle,
then qualify authoritative CRS behavior and decide/implement a truthful validated-
key contract from that common base. S0 is the first adoption target; Q0's client-
scope decision can proceed in parallel. Key and CRS work must not grow separate
native fork/build paths. Neither expands the default Local 1.0 client contract
until its own product decision and gates pass. M6/G1 remain behind Local 1.0.

1. **E0 — evidence harness:** split reusable runtime/client/fixture/oracle/evidence
   support from monolithic native scenarios; add smoke/local/reference entrypoints.
2. **E1 — M1/M2 local profiles:** result RSS/first-row, 100 cancellations,
   mixed-class admission, ADBC/pgwire overhead, and COPY RSS/throughput/atomicity.
3. **I0 — early iroh transport foundation:** one tiny client obtains a signed
   one-worker lease from a minimal config-backed bootstrap and reaches one complete
   local-DuckLake worker, carrying opaque pgwire sessions through direct and
   relayed iroh paths with measured connection, stream, result, COPY, adaptive
   compression, cancellation, CPU, RSS, and throughput behavior. The shared
   control/edge framing precedes gossip or shared storage.
4. **K0 — minimal Local 1.0 Kind topology:** one bootstrap, one QuackGIS worker,
   one tiny client bridge, local durable volume, required credential configuration,
   optional relay configuration, and pinned psql/psycopg/GDAL jobs entering
   through the bridge; no service mesh or deferred clients.
5. **N0 — pinned native bundle:** resolve one exact compatible DuckDB, DuckLake,
   Spatial, and QuackGIS-native source set; apply ordered digest-pinned patches;
   build and test every extension against one DuckDB checkout; package immutable
   artifacts, source provenance, licenses, and an SBOM; and own one upgrade and
   rollback matrix.
6. **C0 — PostgreSQL compatibility program:** freeze a PostgreSQL 18 profile,
   implement stable catalog identity and role/privilege/session semantics, then
   qualify psql, psycopg, OGR, and headless QGIS against copied data.
7. **S0 — authoritative CRS:** qualify released CRS-aware native geometry and
   DuckLake persistence before projecting PostgreSQL/PostGIS CRS metadata or
   widening migration beyond SRID 0.
8. **Q0 — validated keys:** decide the required direct-table/creation client
   surface and expose only primary/unique semantics enforced across every
   supported write, restart, restore, and upgrade path; metadata-only keys do not
   qualify.
9. **H0 — role-aware HTTP:** migrate schema discovery and OpenAPI to the common
   catalog/authorization boundary, route the sidecar through the tiny client, then
   package multiple stateless replicas.
10. **G0 — offline PostGIS migration:** inventory and fail-closed schema preflight,
   one consistent read-only source snapshot, bounded COPY through the packaged
   tiny client, exact verification, and an auditable cutover report.
11. **P0 — M4 host profiles:** conservative predicate/layout work followed by two
   10M runs; introduce 100M only after those runs pass.
12. **K1 — operations and shared rehearsal:** termination, rotation, upgrade,
   recovery, mixed workload and soak locally; PostgreSQL/MinIO shared-profile
   rehearsal begins only after Local 1.0 closes.
13. **I1 — shared iroh cluster:** durable user/control metadata, one-time client
   pairing, a tiny desktop/serverless client, one-credential/one-worker affinity,
   bounded bootstrap/worker gossip, complete shared-DuckLake workers, and common
   pgwire/HTTP delivery. It builds on I0 and begins after Local 1.0 closes.
14. **G1 — online PostGIS catch-up/cutover:** after shared control/storage is
    proven, pair one replication-slot snapshot with ordered logical changes,
    durable source-LSN checkpoints, idempotent DuckLake microbatches, lag and
    response-loss reconciliation, and an explicit source-freeze cutover.

## Baseline

| Area | Current floor | Important limit |
|---|---|---|
| Engine/storage | pinned DuckDB 1.5.4 through ADBC and local DuckLake with one tracked read-only identity patch | local paths only; QuackGIS owns patch/ABI/upgrade qualification |
| Native bundle | one validated authority exact-pins common sources, trees, core targets, patches, executable toolchain, selected artifacts, tests, outputs, and upstream adoption decisions; authority `c6d896e…` emits dynamically loaded candidates and passes 143 DuckLake-function, 59 explicit overlapping patch, and 1,607 complete Spatial assertions; all consumers use linked authority and deterministic source/binary/patch SPDX | candidates remain `upstream-tested-unaccepted` and current runtime bytes retain legacy provenance; Spatial varies across clean builds, all seven candidate QuackGIS/package/lifecycle groups, pristine differential, license conclusions, upgrade, recovery, and rollback remain open before S0/Q0 |
| Protocol | bounded simple/extended pgwire | narrow statements and parameter types |
| Results | one driver Arrow batch at a time through pgwire with fail-closed byte ceiling; clean 1M/10M BIGINT and 1M nullable VARCHAR/BLOB reference profiles pass RSS and exact-value gates | maximum native-batch and additional type/shape RSS profiles open |
| COPY | pre-body bounded pgwire frames, incremental bounded text decoding to one ADBC stream, atomic DuckLake publication, and a clean passing 10M RSS/throughput reference | total COPY remains unbounded while each frame/chunk/row/Arrow batch is bounded; idle clients observe cancellation when they resume or disconnect |
| Transactions | independent sessions, commit/rollback/isolation, failed-transaction `25P02` precedence, harmless idle transaction end, QGIS-style failed COMMIT/ROLLBACK cleanup, storage-enforced `BEGIN READ ONLY` with `25006` at DML/DDL and COPY seams, cancellable pre-commit writes, and a non-cancellable indeterminate-failure commit boundary | commit response-loss reconciliation remains a Local 1.0 operations gate |
| Spatial | 44 native/rewrite/macro cases through pgwire | 8 edge gaps and 5 extension candidates |
| Security | SCRAM, outer read/write table allowlists, immutable table/operation RBAC, role-filtered maintained metadata, HS256 JWT role mapping with transaction-local claims and atomic direct key-file rotation, owner-only direct authenticator-password rotation, loopback-only role-catalog edge preauthentication, exact credential-to-role leases, role-aware OpenAPI, actual-process required-TLS/restart rotation, and packaged mTLS/edge/authenticator/JWT replacement with old-credential denial | no RLS, mutable administration, zero-downtime multi-key overlap, durable revocation, or production revocation drill |
| PostgreSQL catalogs/RBAC | PostgreSQL 18.4 startup/version/recovery identity, relational core catalogs, pinned stable user-object/default/comment/NOT-NULL identity, truthfully empty index projection, bounded role-aware spatial metadata, immutable roles/sessions/grants/inquiry, role-aware information schema, packaged psycopg COPY/reopen, complete psql 18.3 `\d+`, OGR SQL-result/direct Point/NULL discovery plus COPY to a predeclared target, and pinned QGIS 3.44.11 query-layer discovery/filter/extent/viewport-identify/render reads | S0 must qualify authoritative native CRS persistence/projection; Q0 must choose and prove validated key semantics before OGR-created tables or direct-table QGIS are claimed |
| REST | signed-JWT read-only PostgREST-style subset through an exact authenticator lease, transaction-local effective role/context, and automatically revalidated per-role catalog/OpenAPI cache; consumes shared monotonic schema/security epochs where durable identity exists with an exact revision fallback; two packaged replicas pass readiness, role denial, balancing, failover, core reconnect, and old authenticator/JWT denial | package pinned epochs with the owned DuckLake artifact; no full PostgREST parity, public HTTP edge, multi-key overlap, or RLS |
| Operations | restart/reopen, snapshot inspection, adjacent-file merge, checksummed offline exact-path backup/restore | no online/relocated production recovery or shared profile |
| Performance | M4-complete mixed-shape selective scan, grouped aggregate, bounded spatial join, wide projection, compaction, and exact 10M/100M profiles | single-node maintained workloads only; no general spatial-index or clustered-performance claim |
| Metrics/status | policy, classed admission, lifecycle, cancellation, timeout, quarantine, COPY rows/bytes/batches/latency, sampled DuckDB memory/temporary storage, liveness, and local DuckLake read/write-capacity readiness with drain state | write probe is non-publishing and singleton-local; remote dependency SLOs remain open |
| Iroh cluster edge | shared `control/1`/`edge/1` protocol plus executable config-backed bootstrap, bounded credential-to-role registrations, challenged worker authentication, typed pgwire/cancellation streams, and bounded tiny clients differentially pass direct TCP plus direct, forced-custom-relay, and public-default-relay oracles. K0 packages separate mutual-TLS `postgres` and migration ingresses plus two loopback `authenticator` REST sidecars, exact `postgres`/`authenticator`/`migration_operator` leases, ordered replacement/reconnect, denial, and old-key evidence; the migration lane passes report-bound staging/promotion, restart, and promoted-data psql/psycopg/OGR/QGIS | durable pairing/control state, gossip, worker pools, typed HTTP transport, packaged resource/hosted-relay operations, and serverless client remain open |
| Transport compression | authenticated negotiation requires `none` and optionally selects evidence-qualified LZ4; post-`AuthenticationOk` application bytes use independent adaptive blocks with 64 KiB ceilings, ratio/corruption checks, incompressible backoff, and payload-free metrics. Clean 8/32/64 MiB TCP/direct/forced-relay `off`/`auto` profiles pass committed connection, first-byte, cancellation, RSS, throughput, savings, and raw-overhead budgets | packaged/hosted-relay reruns and workload-specific WAN SLOs remain open; no dictionary or cross-stream context is supported |

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
- no active claim depends on DataFusion, SedonaDB, a fork-owned DuckLake writer,
  a private DuckLake metadata layout, removed CLI flags, or an unregistered test;
  and
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
- define disconnect, partial-delivery, and uncertain-cleanup behavior; and
- register smoke/local/reference result, cancellation, concurrency, and transport
  profiles using the common evidence envelope.

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
- exercise official DuckLake compaction after fragmented loads; and
- run the same COPY correctness oracle at smoke, local, and exact 10M/1 GiB
  reference scales.

Exit gates:

- a 10M-row or 1 GiB COPY has no request-size ceiling;
- peak COPY RSS remains within idle plus 256 MiB on the reference profile;
- no Arrow batch exceeds configured row/byte limits;
- pgwire text COPY reaches at least 50% of direct ADBC Arrow-ingest throughput;
- WKB bytes, NULLs, decimals, dates, and timestamps survive commit/reopen; and
- parse failure, cancellation, disconnect, and rollback add zero visible rows.

## I0 — early iroh transport foundation

**Outcome:** transport cost and behavior are known before compatibility, storage,
and durable cluster-control designs depend on iroh. One tiny client is the only
application ingress, obtains one signed worker lease from a minimal bootstrap, and
carries many pgwire sessions to one complete QuackGIS worker over the same Rust
policy, ADBC, DuckDB, and local official-DuckLake path used by the direct TCP test
baseline.

This work may proceed as soon as the M1/M2 streaming and cancellation primitives
it carries have stable smoke profiles. It does not wait for Local 1.0 or M6.

Deliver:

- a minimal native client/library and worker ingress using one versioned
  `quackgis/edge/1` ALPN, one connection-level access lease/key proof and
  compression negotiation, and a typed prelude that maps each local pgwire or
  cancellation connection to one bidirectional stream on the leased worker;
- a minimal `quackgis/control/1` bootstrap using explicit operator configuration
  for the registered client public key and one eligible worker, issuing a bounded
  signed lease after key proof without proxying SQL, handling the user's SCRAM
  password, or returning a pool to the client;
- one shared transport crate/module used by client, bootstrap, and worker for
  relay policy, ALPN/prelude types, leases/proofs, framing, compression, limits,
  errors, and metrics rather than parallel implementations;
- versioned tunnel capability negotiation with mandatory `none` support and an
  `auto` compression policy using independent bounded blocks per stream direction;
  compression starts only after access-lease/key-proof authentication and never
  covers control, grant/access proof, assignment, cancellation, or other
  designated latency-sensitive frames;
- an ingress security contract where direct TCP still requires configured TLS,
  while an authenticated, encrypted iroh connection is marked as an already
  secure pgwire channel and does not carry nested TLS on the cluster leg. The
  local socket/process boundary is protected independently; the worker validates
  the lease, attaches the preauthenticated LOGIN role, and signals compression
  after pgwire startup without receiving SCRAM material or requiring the tiny
  client to parse SQL;
- bounded sampling and candidate low-latency codec profiles that choose compressed
  blocks only when expected transfer savings exceed measured CPU/latency cost,
  skip small or incompressible input, retain no buffer for idle sessions, and use
  no dictionary shared across clients, credentials, requests, or sessions;
- declared compressed/decompressed block ceilings, expansion-ratio limits, and
  deterministic rejection of corrupt, truncated, oversized, or allocation-abusive
  input before pgwire delivery;
- bootstrap, worker, and client configuration accepting an ordered non-empty
  relay list, using iroh's public relay preset when relay configuration is omitted,
  and rejecting an explicitly configured empty list;
- direct TCP pgwire retained only as a current/development correctness and
  performance baseline, with the release application path going through the tiny
  client;
- deterministic tests for access-lease/key-proof authentication, startup under the
  leased LOGIN role, simple and extended queries, parameters, portals,
  transactions, COPY, cancellation, reconnect, disconnect, frame limits,
  backpressure, and graceful worker shutdown through the iroh path; direct TCP
  SCRAM remains a baseline-only oracle;
- smoke/local/reference profiles comparing direct TCP, an iroh direct path, and a
  forced relay path for cold/warm connection latency, time to first row, sustained
  result and COPY throughput, CPU, RSS, bytes transferred, cancellation latency,
  and concurrent stream behavior, each with compression disabled and automatic
  compression over representative compressible, incompressible, small, WKB, and
  COPY/result shapes;
- payload-free compression metrics for input/output bytes, achieved ratio, selected
  codec, CPU time, compressed/skipped reasons, and decode failures, without SQL,
  parameters, payload samples, credentials, or object paths;
- public-relay-default smoke plus an explicit configured hosted/custom relay
  profile, with relay credentials kept out of logs, metrics, tickets, and public
  configuration; and
- a written release budget and keep/change decision for every measured transport
  dimension before M5 packaging, rather than selecting thresholds after the
  clustered implementation exists.

Exit gates:

- direct TCP and iroh execute the same registered pgwire result, type, error,
  transaction, COPY atomicity, cancellation, and reconnect oracles;
- one client keeps multiple independent pgwire sessions on one worker without SQL
  parsing, a worker list, or per-session worker selection in the client;
- the client cannot connect to a worker absent a valid bootstrap-issued lease, the
  worker receives no pairing password/SCRAM verifier, and bootstrap cannot proxy
  application data;
- result and COPY memory remain bounded by the existing M1/M2 limits plus a
  measured, documented transport allowance independent of result/request
  cardinality;
- authentication/control bytes are never compressed, compression contexts are
  isolated per stream and direction, and adversarial length/ratio/corruption tests
  cannot cause cross-session disclosure, unbounded allocation, or delayed
  cancellation;
- direct-path and forced-relay evidence publish connection, first-row, throughput,
  CPU, RSS, stream-count, cancellation, bytes-saved, compression-CPU, and
  compression-ratio distributions on named endpoints;
- automatic mode meets committed byte-saving and CPU/latency budgets for
  maintained compressible relay profiles, while small and incompressible profiles
  stay within a committed raw-path overhead and predominantly select raw blocks;
- unset relay configuration demonstrably uses the public iroh preset, while a
  configured relay list uses only that list and an empty configured list fails on
  bootstrap, worker, and client;
- public and configured relay profiles survive path establishment, fallback,
  reconnect, worker restart, and credential rotation without changing SQL
  semantics; and
- the measured budgets and any required client/worker protocol changes are
  committed before M3 and M5 close and make them expensive to alter.

## N0 — pinned native bundle

**Outcome:** QuackGIS selects, patches, builds, tests, packages, upgrades, and
rolls back DuckDB, DuckLake, Spatial, and QuackGIS-native code as one atomic,
reproducible unit.

N0 supersedes the one-off DuckLake-only source builder without invalidating its
current evidence. Exact design and ownership live in
[docs/NATIVE_BUNDLE.md](./docs/NATIVE_BUNDLE.md).

Deliver:

- one machine-readable upstream adoption review that checks the latest supported
  DuckDB release and current compatible DuckLake/Spatial refs, adopts released
  capabilities before local code, and gives every retained patch an exact search
  record and deletion gate;
- one machine-readable bundle manifest pinning full DuckDB, DuckLake, Spatial,
  vcpkg/toolchain, patch, owned-source, artifact, license, and platform identity;
- one central DuckDB checkout used to build every prepared extension source;
- ignored workspace-local upstream checkouts plus ordered digest-pinned patch
  queues; no floating branch, implicit edit, duplicate DuckDB build, or checked-in
  generated/build tree;
- a separate QuackGIS extension for additive native behavior, retaining source
  patches only for private DuckLake/Spatial/core behavior that cannot use a
  supported hook;
- clean-source, patch-application, upstream-test, QuackGIS-test, artifact, SBOM,
  and license automation;
- one immutable runtime trust policy for every project-built/patched artifact;
  an official signature is retained only when the selected runtime member is the
  exact vendor-built signed binary rather than a local rebuild;
- unmodified-versus-patched differential evidence naming every intentional
  behavior change; and
- one candidate upgrade/reopen/restore/rollback workflow with patch deletion
  review.

Exit gates:

- the recorded latest-release/compatible-branch refs are current at candidate
  review time; every overlapping upstream capability has an adopt/delete decision
  and every retained patch names the exact missing API and deletion gate;
- a clean preparer resolves only exact allowed commits, rejects mismatched
  extension/core pins, verifies every patch hash, and fails on patch conflict or
  dirty/unrecognized source;
- one build invocation emits candidate DuckDB library/CLI and DuckLake, Spatial,
  and QuackGIS extensions against the same core and pinned dependency graph; two
  clean cache-disabled invocations reproduce every selected artifact digest; any
  selected vendor-built signed binary is separately bound to that qualified
  source/ABI and tested as the exact runtime artifact;
- upstream DuckLake/Spatial tests plus QuackGIS storage, identity, pgwire,
  migration, REST, spatial, and package gates pass from the prepared source;
- runtime startup verifies every project-owned native path and digest before
  enabling unsigned loading, denies client `LOAD`/`INSTALL`, and refuses mixed or
  incomplete bundles;
- version-matched independent DuckDB reopens current data; the candidate reopens
  the prior bundle, writes, backs up/restores, and has a tested rollback decision;
- the runtime manifest records source, patch, build-option, artifact, license, and
  SBOM identity without local paths; and
- current DuckLake-only scripts/docs either become compatibility wrappers over N0
  or are deleted after equivalent gates pass.

## M3 — focused compatibility product

**Outcome:** the first named client set and HTTP read edge share one coherent
PostgreSQL 18 catalog, role, privilege, session, and wire-identity contract without
DuckDB-specific setup.

Release-required clients:

- `psql`;
- `psycopg`;
- GDAL/OGR read and COPY load; and
- QGIS read-only discovery, filtering, identify, and render.

Deliver:

- freeze a PostgreSQL 18 compatibility manifest and differential oracle for every
  maintained catalog relation/function/cast;
- derive user-object metadata from DuckDB/DuckLake into an immutable,
  schema-epoch catalog snapshot;
- select and prove a stable OID lifecycle across restart, rename, DDL commit, and
  rollback without making QuackGIS a second user-schema authority;
- implement relational `pg_namespace`, `pg_class`, `pg_attribute`, `pg_type`, and
  required database/search-path/`reg*` behavior rather than exact query matching;
- attach source relation OID and attribute number to RowDescription;
- implement configuration-backed LOGIN/NOLOGIN roles, memberships, ownership,
  schema/table/operation grants, `SET ROLE`, `SET LOCAL ROLE`, `RESET ROLE`, and
  bounded transaction-local request context;
- make `pg_roles`, `pg_auth_members`, privilege inquiry, maintained
  `information_schema`, and statement authorization use one policy engine;
- add constraints, keys, comments, defaults, and spatial metadata only as required
  by the named clients and role-aware REST; relationship embedding remains a
  follow-on slice;
- stabilize geometry RowDescription OIDs and text/binary WKB behavior;
- define or explicitly bound family, subtype, SRID, and dimension metadata;
- assign each release spatial requirement to native, macro/rewrite, Rust edge,
  DuckDB extension, or unsupported;
- replace text signatures with reusable AST/catalog/protocol rules;
- add fuzz/property coverage for the Arrow-to-pgwire encoder;
- run version-pinned psql, psycopg, OGR, and headless QGIS jobs in the minimal Kind
  topology through the tiny client while keeping catalog fixtures client-neutral;
- migrate `quackgis-rest` to catalog-backed discovery, JWT-to-role mapping, one
  authenticator identity, transaction-local role/context, and role-aware OpenAPI;
- package at least two stateless REST replicas behind the local test Service; and
- keep GeoServer, editing, Martin, BI, and broad ORM compatibility deferred unless
  they fit without materially widening the first-release surface.

Exit gates:

- every maintained catalog row and reference is internally consistent and has a
  PostgreSQL 18 differential fixture;
- relation/type OIDs survive normal restart and supported rename; configured role
  OIDs survive restart and configuration reordering; rollback publishes no
  identity or epoch change;
- `current_user`, privilege inquiry, catalog/information-schema visibility,
  OpenAPI, and actual statement authorization agree for denied, anonymous,
  reader, and editor roles;
- role and bounded request context cannot leak across commit, rollback,
  cancellation, disconnect, failed transactions, or native connection reuse;
- each required client has a version-pinned copied-data end-to-end test;
- required catalog queries are fixture-tested independently of client names;
- every supported spatial function has exactly one implementation disposition;
- all supported cases pass through pgwire with exact expected results;
- QGIS and OGR observe maintained geometry fields rather than generic `bytea`;
- two REST replicas expose the same role-specific OpenAPI, stay within both the
  HTTP exposure ceiling and database grants, and access data only through the
  tiny client and assigned complete worker;
- unsupported functions/shapes return stable errors; and
- no release query uses row-wise Rust spatial fallback.

Mutable role/grant DDL, PostgreSQL RLS, REST mutations/RPC, and full PostgREST
parity are explicitly outside this M3 exit. They follow the ordered security
slices in `docs/POSTGRESQL_COMPATIBILITY.md`; table/operation RBAC must not be
described as RLS.

The maintained Local 1.0 client contract may close with OGR read/predeclared COPY
and QGIS query-layer workflows already proven. OGR-created tables and direct
ordinary-table QGIS are promoted into the release contract only by Q0's explicit
product decision and validated-key evidence; they do not justify invented rows or
an unbounded native fork. Authoritative nonzero CRS expansion follows S0.

## S0 — authoritative CRS

**Outcome:** PostgreSQL/GIS clients and migration observe one truthful CRS contract
derived from released DuckDB/Spatial behavior that survives official DuckLake
storage lifecycle operations.

S0 starts only after N0 can reproduce and compare an unmodified candidate and the
QuackGIS bundle. It adopts upstream before adding patches.

Deliver:

- candidate probes for CRS-parameterized native `GEOMETRY`, `ST_CRS`,
  `ST_SetCRS`, `ST_Transform`, and the official coordinate-system catalog;
- DuckLake create/COPY/mutate/rename/compact/snapshot/reopen/backup/restore
  evidence preserving the column CRS and value semantics;
- one explicit mapping between DuckDB string CRS identities and maintained
  PostGIS integer SRIDs, including axis order and unknown/custom CRS policy;
- role-aware `geometry_columns` and `spatial_ref_sys` rows derived from the same
  authority, with stable PostgreSQL wire types and no guessed definitions;
- pgwire text/binary, OGR, QGIS, and PostGIS migration evidence for each accepted
  CRS/family/dimension shape; and
- a narrow DuckLake type-fidelity patch only if the official bundle loses
  otherwise-supported CRS metadata. Spatial is not forked merely to present a
  PostgreSQL catalog.

Exit gates:

- CRS identity is equal before and after commit, independent reopen, rename,
  compaction, backup/restore, and supported bundle upgrade;
- mixed/incompatible CRS operations fail rather than silently relabel or combine
  coordinates;
- unknown CRS behavior is explicit and never drops metadata under an accepted
  migration;
- `geometry_columns`, `spatial_ref_sys`, `ST_SRID`/`Find_SRID`, OGR, and QGIS
  agree on every maintained column; and
- nonzero-SRID migration verifies source/target bytes, CRS identity, extents, and
  client behavior after reopen.

## Q0 — validated key contract

**Outcome:** QuackGIS either publishes keys with enforced semantics or keeps the
dependent client feature explicitly unsupported; names, non-NULL columns, and
unenforced upstream declarations never become fabricated PostgreSQL keys.

Q0 starts after N0. Its first deliverable is a product decision naming whether
direct ordinary-table QGIS and OGR-created tables are Local 1.0 requirements or a
post-release capability.

Deliver:

- a client-neutral key requirement trace and explicit supported key shapes;
- durable declarations keyed by DuckLake table/column identity, with stable
  PostgreSQL constraint/index identity and tombstone lifecycle;
- primary-key NOT NULL and uniqueness validation for existing data, each incoming
  COPY/INSERT batch, key-changing UPDATE, restart, restore, and upgrade;
- writer serialization/fencing that prevents two supported commits from both
  validating against stale state;
- startup and recovery revalidation plus deterministic commit-response-loss
  classification;
- truthful `pg_constraint`, `pg_index`, inquiry, information-schema, OGR, and QGIS
  projection only after enforcement passes; and
- an upstream hook or narrowly reviewed DuckLake patch when every-write
  enforcement cannot use public extension APIs. A separate private storage writer
  or format is not introduced.

Exit gates:

- duplicate and NULL key cases fail atomically across every supported write path;
- concurrent writers cannot publish duplicate values, and indeterminate commits
  reconcile without retrying blindly;
- rename preserves key identity; drop/recreate does not reuse it; rollback and
  failed declaration publish no row or epoch change;
- backup/restore and bundle upgrade reproduce declarations and enforcement before
  accepting writes;
- catalog, privilege inquiry, direct-table QGIS, and OGR behavior agree with the
  same key authority; and
- independent readers remain interoperable while unsupported external writers
  are explicitly outside and fenced by the storage-authority contract.

Shared-worker enforcement remains an M6 concern and must be requalified against
distributed writer fencing; Local Q0 evidence cannot be promoted implicitly.

## M4 — spatial analytical performance

**Outcome:** QuackGIS earns selective spatial and OLAP performance rather than
providing protocol compatibility alone.

Deliver:

- inject safe, planner-visible bbox predicates for proven literal/bound shapes;
- retain the original exact DuckDB predicate;
- maintain bbox/locality columns during COPY and compaction in DuckDB;
- benchmark WKB storage against native geometry before changing representation;
- exercise DuckDB 1.5.4+ native geometry Parquet statistics and
  `OPERATOR_ROW_GROUPS_SCANNED`, deleting maintained bbox machinery if the native
  exact plan meets the same scan and correctness gates;
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

## G0 — offline PostGIS migration

**Outcome:** an operator can move a declared supported PostGIS dataset into a
fresh QuackGIS root without hand-written export SQL, partial publication, or
silent semantic loss.

G0 may proceed after M2 COPY and the required C0 catalog/type identity slices are
stable. It is independent from online replication and does not require M6 shared
storage. The first slice may reject keys, CRS, dimensions, or scalar types whose
target semantics are not yet authoritative; rejection is preferable to
normalizing them silently.

Deliver:

- a version-pinned PostgreSQL/PostGIS source connector that opens one read-only
  repeatable-read snapshot, records source server/database identity and snapshot
  time, and never requires a QuackGIS worker to hold source credentials;
- a client-neutral preflight inventory for schemas, tables, columns, scalar and
  geometry/geography types, typmods, SRIDs, dimensions, primary/unique replica
  identity, defaults, NOT NULL, comments, row counts, estimated bytes, roles and
  unsupported objects, with an explicit migrate/map/reject disposition for every
  item;
- explicit configuration for schema/table selection, target names, role/grant
  mapping, and any approved lossy conversion; RLS, triggers, functions, views,
  sequences, extensions, and role passwords are not copied implicitly;
- target creation only through supported QuackGIS/DuckLake catalog operations,
  followed by bounded source `COPY (SELECT ...) TO STDOUT` to target `COPY FROM
  STDIN` streams through the packaged iroh tiny client; the migrator stores no
  DuckDB, ADBC, object-store credential, or private DuckLake metadata knowledge;
- canonical conversion for the release scalar set plus exact WKB/EWKB/NULL
  handling. Nonzero/mixed SRID, Z/M, geography, curve, raster, or key semantics
  remain rejected until the maintained target catalog can represent them
  truthfully;
- staging under a fresh target namespace/root with no partial release visibility,
  bounded progress/checkpoint metadata, safe retry before promotion, and explicit
  cleanup after failure;
- exact source/target row counts, canonical per-column checksums, NULL counts,
  key duplicate checks where keys are declared, geometry family/SRID/dimension/
  validity counts, extents, and deterministic samples; and
- a path-free migration report naming source identity/version, target
  source/runtime digests, snapshot, mappings/rejections, rows/bytes, checksums,
  duration, errors, and final promotion decision.

Exit gates:

- a pinned PostGIS fixture covering every release scalar, Point/NULL WKB,
  defaults/comments/NOT-NULL, multiple schemas, and deliberately unsupported
  objects either migrates exactly or fails in preflight before target publication;
- source writes concurrent with the copy cannot enter the repeatable-read
  snapshot, and every target row comes from that one snapshot;
- interruption before promotion leaves no visible release, retry into a fresh
  staging target produces the same checksums, and promotion is one explicit
  operator decision;
- all accepted tables match exact counts, canonical checksums, NULL disposition,
  geometry bytes/metadata, extents, and deterministic samples after target reopen;
- copied-data psql, psycopg, OGR, and the maintained QGIS read workflow enter only
  through the packaged tiny client and match the post-migration report; and
- the report lists every rejected or deliberately mapped source feature. A green
  run cannot omit an unclassified table, column, geometry shape, or security
  object.

Offline migration is not `pg_dump` compatibility and does not make QuackGIS a
logical PostgreSQL restore target. PostGIS remains unchanged and is the rollback
source until the operator separately retires it.

## M5 — Local 1.0

**Outcome:** a user can deploy and operate the single-node product without
repository knowledge.

Deliver:

- immutable runtime artifacts with DuckDB/extension provenance and no online
  extension install;
- N0's exact source, ordered patch, ABI, build-option, artifact, SBOM,
  immutable-path, lifecycle, upgrade, and rollback evidence for the complete
  DuckDB/DuckLake/Spatial/QuackGIS native bundle;
- health, readiness, graceful shutdown, and transaction drain;
- backup, restore, compaction, capacity, spill, and incident procedures;
- supported DuckDB/extension upgrade and reopen tests;
- one supported, non-EOL release bundle decision after evaluating the released
  candidates available when N0 closes; release-calendar, preview, and nightly
  artifacts are evidence inputs, not release dependencies;
- classic/PEG parser equivalence for every maintained allow/deny/rewrite family
  while both parser modes exist;
- retain complete in-process ADBC workers unless a released transport can serve
  the attached DuckLake data plane, an explicit product-direction change approves
  the engine split, and cancellation, transaction, security, crash, and resource
  gates pass;
- TLS and secret-rotation evidence;
- catalog/control-metadata backup, restore, upgrade, and cache-invalidation
  evidence;
- authenticator/JWT/database credential rotation and multi-replica REST readiness;
- package the I0 one-client/one-worker iroh path and rerun its committed direct,
  public-relay-default, and configured-relay budgets against the release artifact;
- disable application-facing worker TCP/HTTP listeners in the release profile;
  direct TCP remains an explicit test/development baseline only;
- mixed read/COPY/mutation/cancel/compaction/restart/restore workloads; and
- maintain a minimal Kind topology for packaged functional evidence while keeping
  performance budgets on the host/reference profile.

Exit gates:

- N0 closes and the selected native bundle passes old/new reopen, recovery,
  rollback, package, and mixed-bundle-refusal gates;
- a clean environment starts from published artifacts only;
- backup/restore reproduces the declared committed snapshot and exact counts;
- controlled termination exposes no partial mutation;
- restart recovery completes within 60 seconds for the release catalog;
- a 24-hour mixed workload has no correctness failure, leaked transaction, or
  unbounded memory growth;
- required client and 10M performance gates remain green after packaging;
- the packaged iroh client/worker path meets the I0 connection, throughput,
  adaptive-compression, cancellation, COPY, CPU, RSS, and relay-selection budgets;
- all named pgwire and HTTP client gates enter through the packaged tiny client,
  while a direct worker connection is refused by the release profile;
- role-aware REST and catalog/RBAC gates remain green after restart, restore, and
  upgrade; and
- statement/type/transaction/concurrency limits are published.

## M6 — Shared iroh cluster 1.x

**Outcome:** paired desktop, server, and serverless clients reach a bounded pool
of complete QuackGIS workers over iroh while one managed PostgreSQL and
object-storage stack preserves the local query, policy, and compatibility
contract.

This begins only after Local 1.0 and builds on the measured I0 transport. M6 must
not introduce an unrelated iroh data path when it adds pairing, gossip, worker
assignment, shared control state, and shared storage.

### M6.1 — durable control and client identity

Deliver:

- a protected transactional PostgreSQL control database, separate from official
  DuckLake metadata, for users, SCRAM verifiers, client credentials, roles,
  memberships, grants, policy, worker registration, pool configuration,
  credential-to-worker assignments, revocation, and schema/security/configuration
  epochs;
- one-time bootstrap SCRAM pairing that registers a generated credential public
  key against a LOGIN role, stores only verifier/public credential material
  server-side, and never retains the human password in the client or exposes it to
  workers;
- long-lived credential private keys and handles in an OS keystore, owner-only
  file, KMS, or serverless secret store, with routine password rotation
  independent from credential revocation and compromise rotation able to advance
  a security epoch;
- short-lived bootstrap-signed access leases binding credential public key, role,
  permitted protocols, assigned worker, assignment generation, and
  security/configuration epochs; lease refresh proves the registered key and does
  not require interactive re-pairing while the credential row remains enabled;
- migration from I0's explicit local registration and one-worker bootstrap config
  to this transactional authority without changing `control/1`, `edge/1`, key
  proof, lease, relay, framing, compression, or pgwire/HTTP semantics;
- one common role, privilege, catalog visibility, statement authorization, and
  OpenAPI decision across pgwire and HTTP; and
- backup, restore, migration, audit, revocation, and fail-closed cache-invalidation
  procedures for control metadata; and
- an operator-only versioned control capability over authenticated iroh that
  starts and monitors backend backup/restore operations by opaque operation ID,
  without granting the tiny client storage credentials or proxying catalog/object
  bytes.

### M6.2 — tiny client and relay contract

Deliver:

- one small native binary and reusable library containing local access, iroh,
  pairing, credential proof, access-lease refresh/cache, and typed pgwire/HTTP/
  cancellation stream forwarding plus the bounded I0 compression codec, but no
  DuckDB, ADBC, SQL policy, gossip, worker list/scoring, or storage credentials;
- an owner-protected local desktop transport plus a generated independent local
  credential where OS peer authentication is unavailable;
- an embedded/process-local serverless mode that loads a service credential from
  platform secret storage, supports many ephemeral transport endpoints sharing
  that registered key/handle and bootstrap assignment, and never stores the
  pairing password;
- bootstrap, worker, and client configuration accepting an ordered non-empty iroh
  relay list; when relay configuration is omitted, select iroh's public relay
  preset; reject an explicitly configured empty list rather than silently changing
  network behavior;
- hosted-relay configuration and credential rotation without changing cluster
  identity, pairing, assignment, pgwire, HTTP, or storage semantics;
- compression configuration supporting `off` and evidence-selected `auto`
  behavior without changing SQL/HTTP semantics, worker affinity, or relay choice;
  and
- no relay API secret, client private key, worker private key, SCRAM material, or
  storage credential in gossip, logs, metrics, DuckLake metadata, or generated
  public configuration.

### M6.3 — shared complete workers

Deliver:

- official DuckLake PostgreSQL metadata-catalog and object-storage configuration;
- shared credentials and writer-authority validation;
- measured multi-process readers/writers using supported DuckLake behavior;
- reader visibility and writer consistency policy;
- deterministic conflict/indeterminate-commit classification; and
- throttling, interruption, rotation, cleanup, and independent reader tests; and
- one coordinated recovery-point procedure that fences writers/assignments,
  snapshots both managed PostgreSQL catalogs, protects the referenced versioned
  object-store set with provider-side operations, and verifies independent
  version-matched DuckDB reopen before workers resume.

Each worker retains the Rust policy edge, ADBC, and in-process DuckDB. Iroh does
not introduce a Quack/engine split or grant direct DuckDB access.

### M6.4 — bounded sessions and worker pools

Deliver:

- separate limits for authenticated client sessions, native engine leases, pinned
  transactions/COPY/portals, active queries, queued operations, and per-credential
  fairness so mostly idle clients do not allocate one DuckDB connection or
  blocking thread each;
- a small, explicitly bounded gossip topic containing bootstrap nodes and workers
  only, with signed incarnations, readiness, drain, compatibility, coarse
  capacity, heartbeat expiry, and lag recovery;
- two or more stable bootstrap endpoints that reconcile gossip liveness with
  authoritative PostgreSQL desired state, select the worker, and issue one signed
  access lease rather than returning a pool for client-side scoring;
- one authoritative assignment generation per client credential, enforced by
  access leases and workers across pgwire and HTTP so a credential cannot fan out
  to several workers;
- graceful drain without overlapping new sessions on old and replacement workers,
  and abrupt failure that fences the old generation without replaying SQL; and
- worker autoscaling/replacement that leaves established clients pinned and uses
  new capacity for new assignments until an explicit drain/rebalance.

### M6.5 — common edge and HTTP delivery

Deliver:

- one authenticated `quackgis/edge/1` connection with typed pgwire, HTTP, and
  cancellation stream preludes, using the same access lease, role, assigned
  worker, policy, engine-lease pool, admission, catalog/security epochs, audit
  identity, relay setup, and bounded adaptive-compression framing;
- `quackgis/control/1` for bootstrap SCRAM pairing, registered-key proof, and
  access-lease refresh, with no SQL/HTTP data proxying through bootstrap;
- local HTTP through an owner-protected socket or generated local bearer token,
  plus an embedded serverless adapter;
- role-aware schema discovery and OpenAPI from the common authorization boundary;
- direct registered-service-credential requests using the access lease's LOGIN
  role, plus optional per-request end-user delegation only when the worker itself
  validates a signed JWT's issuer, audience, time bounds, security epoch, and
  bounded role or request context through the same authorization engine; and
- an explicit distinction between a serverless application's service role and
  any end users it serves; unsigned or merely forwarded role, identity, or claims
  headers never grant database authority.

Exit gates:

- two readers and one writer run for 24 hours with no loss, duplicates, or partial
  visibility;
- committed changes meet the declared visibility SLO;
- conflict/response-loss tests have deterministic reconciliation outcomes;
- independent version-matched DuckDB reproduces schemas, counts, samples, and
  snapshots;
- restored catalog/object storage reproduces the recovery point;
- restored control metadata reproduces users, registered credential public keys,
  revocation, assignments, policy, and security/configuration epochs without
  private keys or stale access leases;
- backup start/status/result passes through the operator iroh control capability,
  no backup payload or cloud credential traverses the tiny application client,
  restore works with application workers fenced, and stale worker assignments or
  access leases cannot survive recovery;
- at least 2,000 authenticated, mostly idle pgwire sessions share one reference
  worker within a published RSS budget while native engine leases, active queries,
  pinned work, and queues remain at their configured ceilings;
- many local pgwire sessions under one desktop credential and concurrent cold
  serverless instances sharing one registered service key obtain leases for one
  enforced worker assignment and retain independent
  session/transaction/cancellation identity;
- graceful drain and abrupt failure each produce one assignment generation and no
  old/new-worker overlap for newly accepted sessions;
- clients never join gossip, and the bootstrap/worker topic remains within its
  configured member, message-size, frequency, and expiry bounds under churn;
- public-relay defaults and explicit hosted-relay configuration both pass pairing,
  pgwire, HTTP, adaptive compression, reconnect, cancellation, and rotation tests;
  and
- two managed catalog/object-storage/hosted-relay runs, including one serverless
  client profile, pass on the same release candidate.

## G1 — online PostGIS catch-up and cutover 1.x

**Outcome:** a large PostGIS source can remain writable while QuackGIS takes an
initial consistent snapshot, catches up through logical changes, and performs a
short, measured write-freeze cutover with deterministic rollback information.

G1 begins only after M6 provides durable transactional control metadata, shared
official DuckLake, worker fencing, conflict/response-loss classification, and
backend recovery. It uses PostgreSQL logical decoding; physical streaming
replication, dual-write, bidirectional replication, and reverse apply are out of
scope.

Deliver:

- a version-pinned PostgreSQL logical replication publication/slot whose exported
  initial snapshot and consistent start LSN feed the same table/type preflight and
  bulk-copy oracle as G0;
- durable source identity containing PostgreSQL system identifier, timeline,
  database/publication/slot identity, schema fingerprint, and acknowledged LSN,
  with fail-closed refusal after source replacement, timeline divergence, slot
  loss, or incompatible schema change;
- mandatory stable source replica identity for every UPDATE/DELETE table. Tables
  without a declared supported key are insert-only or rejected; QuackGIS does not
  infer a key from row contents or advertise a target index DuckLake cannot
  enforce;
- transaction-ordered INSERT/UPDATE/DELETE decoding into bounded batches with
  source transaction/LSN identity, schema/type validation, and no execution of
  source triggers, functions, DDL, role changes, or arbitrary logical messages;
- idempotent DuckLake staging/publication keyed by source identity and LSN range,
  with the durable checkpoint advanced only after target commit is reconciled.
  Delivery is at least once; replay after response loss must produce the same
  visible rows rather than being described as distributed exactly-once commit;
- bounded capture/apply queues, backpressure, WAL-retention alarms, lag/throughput
  metrics, dead-letter-free fail-stop behavior, restart/resume, and operator
  pause/resnapshot procedures;
- an explicit cutover state machine: preflight, initial snapshot, catch-up,
  source write fence, final-LSN drain, exact G0 verification, worker/client switch,
  and a timed PostGIS read-only rollback window; and
- path-free audit/evidence for source and target identities, snapshot/start/final/
  acknowledged LSNs, transaction and row counts, lag, retries/replays, checksum
  reconciliation, freeze duration, and cutover/rollback decision.

Exit gates:

- concurrent source INSERT/UPDATE/DELETE transactions during the initial copy and
  catch-up produce no missing, duplicate, partially visible, or out-of-order final
  state across crash/restart and worker replacement;
- failures before target commit, after target commit but before checkpoint, and
  after checkpoint each reconcile deterministically from source-LSN/batch identity;
- schema DDL, replica-identity loss, unsupported type/geometry change, slot loss,
  and source identity/timeline change stop apply before semantic drift or WAL
  acknowledgement;
- sustained capture/apply meets a declared source-WAL retention and p95 lag SLO,
  and overload applies backpressure or stops without unbounded process/control
  metadata growth;
- after the source write fence, the consumer reaches the declared final LSN and
  all G0 count/checksum/NULL/key/spatial/sample oracles pass after independent
  target reopen;
- the measured write-freeze and client-switch window meets its committed budget,
  and rollback before source retirement requires no reverse replication; and
- credentials, WAL payloads, row values, and object paths are absent from iroh
  control messages, logs, metrics, and public migration reports.

## M7 — dataset lifecycle 1.x

**Outcome:** operators can stage, validate, publish, protect, roll back, and retire
dataset versions using official DuckLake primitives.

Official DuckLake protected snapshots are the intended protection primitive.
QuackGIS does not implement a competing snapshot-protection format while that
upstream capability is pending.

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
- Branch/merge and general CDC row functions outside the bounded G1 PostGIS
  source-migration contract.
- A QuackGIS DuckDB extension without an accepted, benchmarked proposal.
- An out-of-process engine split that cannot serve the attached official DuckLake
  data plane through the existing Rust policy and transaction boundary.

## Risk controls

| Risk | Required response |
|---|---|
| native/extension supply-chain or ABI drift | pin artifacts, verify checksums, prohibit production downloads, test upgrades/mixed-version refusal |
| native patches become an unmergeable product fork | N0 keeps pristine upstream sources plus ordered minimal patch queues, runs upstream and differential tests, records one owner/deletion plan per patch, and forbids automatic conflict resolution or floating refs |
| key/CRS work creates parallel native authorities | N0 closes first; S0 adopts official CRS behavior before patching; Q0 publishes only enforced keys; official DuckLake remains the sole user-data writer |
| unbounded ADBC materialization or blocking work | M1 streaming, cancellation, admission, memory/spill budgets before broader clients |
| compatibility sprawl | require client traces, implementation disposition, stable errors, and delete shims replaced upstream |
| metadata and authorization drift | one policy engine plus cross-surface tests for privilege inquiry, information schema, execution, and OpenAPI |
| unstable PostgreSQL identity | prohibit transient DuckDB OIDs and name-only hashes; require restart/rename/rollback identity gates before client claims |
| unsafe RLS shortcut | ship table/operation RBAC first; require a separate structural predicate-injection and adversarial bypass milestone |
| incorrect spatial pruning | conservative candidate oracle plus visible exact recheck for every optimized shape |
| DuckLake API/semantics drift | use official primitives, independent reopen, backup/restore, versioned upgrade gates |
| shared claims outrun local product | Local 1.0 is a hard prerequisite for M6 |
| relay defaults become hidden authority | omitted config selects the public iroh preset; explicit relay lists are validated; relay access never grants a database role |
| compression wastes CPU or leaks across trust boundaries | negotiate `none`; compress only authenticated bounded application blocks after measured gain; isolate contexts per stream/direction; never compress auth/control traffic or share dictionaries |
| gossip becomes an unbounded control plane | only bounded bootstrap/worker membership; PostgreSQL owns desired state, policy, credentials, and assignments |
| client affinity masks storage bugs | require cross-worker visibility/failover evidence even though one credential normally uses one worker |
| shared service credential broadens impact | bind it to one role/pool/worker assignment, enforce per-credential limits, audit every session, and support immediate revocation |
| client grows into a second control plane | bootstrap alone chooses/fences workers and signs leases; client has no gossip, pool list, scoring, policy, or replay logic |
| client and worker transport implementations drift | one shared relay/framing/lease/proof/compression/limits module and cross-endpoint conformance tests |
| offline migration silently loses PostGIS semantics | classify every source object/type/geometry/security feature before copy; reject unsupported keys/CRS/dimensions/DDL; require exact post-reopen checksums and a complete report |
| logical migration duplicates or loses changes | bind slot snapshot to source identity/start LSN; require source replica identity; apply idempotent LSN batches; checkpoint only after commit reconciliation; fail on DDL/slot/timeline drift |
| scale language outruns evidence | publish exact rows/bytes/files/hardware/cost and distinguish routine from stress runs |

## Scope boundaries

QuackGIS is not a full PostgreSQL replacement, OLTP database, document store,
desktop GIS/map server, or heavyweight raster/CAD/point-cloud decoder. PL/pgSQL,
triggers, LISTEN/NOTIFY, logical replication, topology, Tiger geocoder, SFCGAL,
and complete PostgreSQL semantics remain out of scope unless product direction
materially changes.
