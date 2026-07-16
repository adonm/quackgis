# Roadmap status

This is the current evidence floor for the DuckDB-only runtime. It intentionally
does not inherit claims from retired DataFusion/Sedona or unsupported shared
deployment profiles.

## Local-first closure program

The roadmap now separates smoke, local, reference, and external evidence. Direct
host/container profiles own performance budgets; the minimal DuckDB-only Kind
topology owns packaged client/lifecycle evidence while recovery and upgrade gates
remain open. The retired broad Kind tree will not be restored. Current iteration state:

| Work package | State | Next executable result |
|---|---|---|
| E0 evidence harness | active; shared fixture established | common evidence envelope plus reusable fresh DuckLake/server/client runtime now support separately registered profiles; continue extracting fixtures only when the next profile needs them |
| E1 M1/M2 local profiles | complete | clean 1M/10M BIGINT, 1M wide-result, 100-cancel, eligible 50M transport, and 10M COPY references pass; mixed reader/writer/maintenance admission and cancellable-write rollback/reuse/quarantine oracles pass |
| I0 early iroh transport | complete at the measured pre-packaging boundary | the shared authenticated path differentially matches direct TCP over direct, forced-custom-relay, and public-default-relay iroh; bounded adaptive LZ4 and payload-free metrics pass adversarial tests and clean 8/32/64 MiB TCP/direct/relay resource profiles. Custom relay also proves fallback, restart, credential rotation, and old-key denial. K0 now packages the local direct path; M5 still owns packaged resource/hosted-relay reruns, while M6 owns durable control/pairing/pools/gossip/HTTP |
| K0 minimal Kind topology | complete | clean source `f8b3e6c` on rootless Podman runs runtime `localhost/quackgis-duckdb-runtime@sha256:b546763517b356a85c10e67d635b4f821c7628b723749d1be285abad63a75e03`. One core StatefulSet Pod uses loopback-only role-catalog edge preauthentication; bootstrap maps distinct proven credentials to `postgres` and `authenticator`. The mutual-TLS pgwire ingress plus pinned psql/psycopg/OGR copied-data gates pass. Two REST Pods each use a unique transport key and loopback authenticator tiny client; both pass exact reader/denied OpenAPI/data gates, expose two ready endpoints, and survive one-Pod deletion. Core replacement reconnects all gates in 3.935 s. mTLS/edge/authenticator rotation denies the old certificate and old REST lease; JWT replacement denies the old token against each replacement Pod |
| C0 PostgreSQL compatibility | C3/C4 implementation and C5 enforcement/discovery pass; QuackGIS now owns the exact-source/patch/artifact-pinned identity lane and runtime packaging policy. Durable schema/security epochs, role-aware REST/OpenAPI, pinned psycopg copied-data, and OGR SQL-result slices pass | rerun the pinned package and close client-neutral query shapes: psql/OGR currently stop at `pg_class`/`pg_proc`; QGIS also exposes `pg_is_in_recovery`/transaction gaps. Primary/unique/foreign-key indexes and authoritative CRS/subtype/dimension metadata remain upstream gaps, not reasons to invent rows |
| H0 role-aware HTTP | packaged two-replica implementation complete at K0 | bounded HS256 JWT validation drives an exact credential-bound `authenticator` lease, transaction-local role/claims, grant-filtered per-role caches, role-specific OpenAPI, exact revision validation, two ready endpoints, one-Pod failover, core reconnect, and old authenticator/JWT denial. The pinned artifact enables shared epochs; its Kind rerun plus public HTTP TLS/rate limits, multi-key overlap, full PostgREST, and RLS remain open |
| P0 M4 host profiles | complete | v5 runs selective scans, grouped aggregates, bounded spatial joins, wide projections, and fragmented-file compaction through the same exact mixed-shape oracle at smoke, 10M, and 100M scales |
| K1 operations/shared rehearsal | deferred by dependency | Local 1.0 operations first; PostgreSQL/MinIO is rehearsal, not managed evidence |
| U0 upstream adoption | design active | conditional deletion/adoption matrix now covers 1.5.5/2.0, PEG, Quack, async I/O, C/Rust extension APIs, native spatial/pruning, and DuckLake protected snapshots/RBAC/UDTs/materialized views; executable candidate matrices remain open |

## Current verified floor

| Area | Evidence | Current boundary |
|---|---|---|
| Engine/storage | `just duckdb-adbc-storage-test` and `just duckdb-pinned-ducklake-test` | pinned DuckDB 1.5.4, DuckLake's official writer plus the tracked read-only identity patch, Arrow ingest/query, transaction, snapshot inspection, adjacent-file merge, reopen, checksummed offline exact-path backup/restore, and durable catalog identity lifecycle |
| Pgwire workflow | `just duckdb-pgwire-workflow-test` | structural statements/parameters; exact all-allowlisted QGIS 3.44 four-`SET` bootstrap with bounded batch/application name; incremental >20 MiB/220k-row COPY with atomic abort paths; scalar/NULL/WKB reopen; catalog/wire identity; spatial gaps; compaction; independent/failed transactions; streaming/portals; bounded read-only SQL cursors over simple and extended transaction control with metadata-only/text/binary/EOF/early-close coverage; restart |
| Auth/policy | real CLI SCRAM, table allowlist, role-session, request-context, effective-role grant/denial, relational role/membership catalogs, role/schema/table/column inquiry, role-aware information schema, and packaged REST OpenAPI/direct-read cases | bounded immutable graph and identity lifecycle; common enforcement, inquiry, discovery, and HTTP decisions; stable role/edge OIDs and resolving PostgreSQL 18 options with NULL credentials. Loopback edge preauthentication validates configured `LOGIN` roles before `AuthenticationOk`; signed leases derive exact roles from a bounded credential map. Name inquiry and schema/table/column/grant discovery work without dynamic identity; OID/expression inquiry and shared epochs pass in the pinned lane and await the complete Kind client rerun |
| Spatial | pgwire workflow + `tests/duckdb_spatial_compat.json` | 43 original PostGIS expressions: 31 native, 5 rewrites, 7 macros; CRS/SRID readback and 2D extent now execute without claiming unsupported PostGIS SRID assignment |
| Spatial gaps | `docs/DUCKDB_SPATIAL_GAP_LEDGER.md` | 9 Rust/catalog-edge gaps and 5 extension candidates have ledger-pinned `0A000` simple/extended pgwire behavior; semantics remain unsupported |
| Spatial analytics | `just duckdb-adbc-storage-test`, `just duckdb-pgwire-workflow-test`, and `just duckdb-spatial-scan-profile` | one-table maintained-layout `ST_Intersects` adds planner-visible four-axis candidates while retaining the exact predicate. Clean v5 references on source `8490ed7` run selective counts, eight-group aggregates, four-probe bounded spatial joins, and nine-column/1 KiB-payload wide projections over mixed `POINT`/`LINESTRING`/`POLYGON` data in maintained WKB/bbox and native `GEOMETRY` layouts. Every result/plan oracle passes before and after compaction. Two 10M runs precede two 100M runs; at 100M all selective/grouped/wide plans scan 1/825 row groups with 0.128–0.129% conservative compressed-byte bounds, all p95/p99 values stay at or below 23.58 ms, query RSS grows by at most 113.03 MiB, DuckDB memory by 32.62 MiB, spill stays zero, and compaction reduces 25 files to 7 bbox/4 native under the 1 GiB file policy |
| WKB/Arrow | storage/pgwire native tests + `just arrow-encoder-test` | maintained WKB bytes, relational geometry/geography catalog lookup with PostgreSQL 18 result types, RowDescription/text/binary/NULL identity for both families, generated WKB/fixed-binary properties, scalar/list parity, fail-closed invalid shapes, and panic-free nested errors; broad client discovery and generated temporal/decimal/dictionary/nested coverage remain open |
| Catalog contract | `just duckdb-catalog-contract-test` | explicit/implicit namespace/database/type/range/collation/owner-role mapping passes with stable logical-database identity, PostgreSQL-shaped `current_database`/`current_schema`/`current_schemas`, 24 exact PostgreSQL 18 built-ins, and PostGIS-shaped spatial scalar/array rows. Full-row oracle equality, reciprocal references, inferred/explicit OID parameters, wire types including `name[]`, alias safety, and spatial transport pass. All unimplemented/private routes plus wildcard/nested/set/derived/implicit-join/CTE/cross-database shapes fail closed; `TABLE` is rejected before authorization. User-object catalogs/origins remain open |
| PostgreSQL 18 target profile | `just project-contract-check` | machine-readable `pg18-column-core-v1` validates required relation/query/result-type shape against a digest-pinned PostgreSQL 18.4 oracle. Credential-free OGR 3.11.5 (21 queries), psql 18.3 `\d+` (12), and QGIS 3.44.11 (32 statements/26 unique) traces pin clients and PostgreSQL/PostGIS images; only dependent RBAC/OpenAPI traces remain. Target traces are not QuackGIS implementation evidence |
| Catalog identity feasibility | `just duckdb-catalog-identity-test` plus `just duckdb-pinned-ducklake-test` | independent DuckDB 1.5.4 processes prove table ID/UUID and field-ID durability. The source/artifact-pinned function drives protected DuckLake mappings: fixed public/dynamic namespace OIDs, globally allocated relation/row-type OIDs, monotonic per-table attribute numbers, retained tombstones, serialized two-session commits, all public create APIs, explicit unconstrained-table invariants/corruption rejection, and a schema/default/comment fingerprint epoch across autocommit/explicit commit/rollback/rename/add/reopen/drop-recreate/non-public-schema evidence. Empty schemas have no durable API identity and remain unsupported. Runtime assembly/offline startup pass; the complete Kind and direct named-client reruns remain open |
| REST preview | `just rest-check`, `just rest-postgrest-smoke`, `just rest-shared-epoch-smoke`, `just rest-edge-preauth-smoke`, `just kind-rest-gates`, and `just kind-rest-jwt-rotation-gate` | direct mode proves HS256 and owner-only SCRAM credential rotation, mismatch readiness failure, reconnect without a REST restart, transaction-local cleanup, role-filtered catalog/OpenAPI/direct denial, shared epoch invalidation with the pinned artifact, and exact revision fallback otherwise. K0 packages two replicas through separate authenticator tiny clients with no database password; both pass reader/denied data/OpenAPI, two endpoints, one-Pod failover, core reconnect, old service-credential denial, and old-JWT denial after replacement. Full PostgREST parity, writes, RPC, embedding, GeoJSON, signed epochs, public HTTP policy, and multi-key overlap remain open |
| Runtime supply chain | `just ducklake-pinned-source-check`, `just duckdb-pinned-ducklake-test`, and `just duckdb-runtime-offline-smoke` | tracked DuckLake base/patch/tool/artifact pins, exact compile-time/config/file digest agreement, immutable image path, signed Spatial plus the pinned read-only DuckLake extension, no online install, network-disabled CLI load, and server-start smoke |
| Current transport profiles | `just duckdb-current-benchmark`, `just duckdb-transport-profile`, and `just evidence-manifest-check` | one deterministic scalar full-scan scenario/oracle runs at smoke/local/reference row counts in the common envelope; ADBC/pgwire are warmed and interleaved. A clean 50M reference on source `fc0b6069` records 1200.05 ms ADBC p50 and a 0.999 pgwire/ADBC p50 ratio, passing the one-second eligibility floor and 1.15 ceiling |
| Iroh control/edge protocol | `just iroh-protocol-test` | shared versioned ALPN/types prove bounded five-minute signed one-worker leases, registered credential-key refresh proof, fresh worker challenge/key proof, transport endpoint binding, typed pgwire/cancellation/HTTP preludes, mandatory `none`, optional LZ4 selection, 16 KiB control bounds, and omitted-public/non-empty-custom relay policy. Compression units reject corrupt, truncated, oversized, and ratio-abusive blocks before delivery |
| Iroh local-direct seam | `just iroh-direct-smoke` | real relay-disabled local bootstrap, worker, and tiny-client endpoints issue one lease and multiplex two concurrent pgwire sessions; the loopback client path rejects nested TLS, structurally binds startup to the leased LOGIN role, permits typed cancellation, and refuses backend credential negotiation beyond `AuthenticationOk` |
| Packaged iroh ingress | `just kind-up-local`, `just kind-client-gates`, `just kind-restart-gate`, `just kind-secret-rotation-gate`, and `just kind-rest-jwt-rotation-gate` | one rootless-Podman core Pod runs the immutable server/bootstrap/worker/pgwire-client bundle; two REST Pods run independent authenticator clients. The server stays loopback-only in role-catalog edge-preauthenticated mode. Pinned psql, psycopg copied-data COPY/reconnect, and OGR SQL-result Point/NULL gates pass. Both REST Pods pass exact role-aware data/OpenAPI, two endpoints, and one-Pod failover. Direct worker TCP, plaintext, certificate-free, rotated-old-certificate, old authenticator credential, and old JWT access fail. A stable internal UDP Service plus bounded stale-session invalidation reconnects REST after a 3.935 s core replacement |
| Iroh DuckDB direct parity | `just iroh-duckdb-smoke` | one reusable oracle runs unchanged against direct TCP and the real local iroh bridge to a fresh official-DuckLake worker. Result values and PostgreSQL types, stable unsupported SQLSTATE, typed parameters, spatial SQL, one-row portals, commit/rollback/disconnect, successful and malformed COPY atomicity, cancellation `57014`, quarantine `XX000`, concurrent sessions, and fresh reconnect are equal; the iroh side runs with adaptive LZ4 and exercises compressed result/COPY blocks |
| Iroh custom-relay correctness | `just iroh-custom-relay-smoke` and `just iroh-duckdb-relay-smoke` | relay-only endpoints prove no active direct route, adaptive compression, typed cancellation, reconnect, unusable-direct fallback, same-identity worker restart, bootstrap credential rotation, denial of the old credential's next lease, and replacement-client success. The native differential oracle has the same exact direct-TCP outcome through this forced relay |
| Iroh public-default correctness | `just iroh-public-relay-smoke` and `just iroh-duckdb-public-relay-smoke` | opt-in outbound runs prove omitted configuration selects a real public preset relay with no active direct route, reconnects two sessions, and gives the native DuckDB differential oracle exact result/type/error/transaction/COPY/cancellation/reconnect parity. This is current external evidence, not a CI network dependency or hosted-relay SLO |
| Iroh transport resources | `just iroh-transport-profile` | release-mode deterministic echo traffic publishes three-sample connection/first-byte/throughput distributions, process CPU/RSS, stream count, cancellation, bytes, block decisions, ratio, codec/decode CPU, and failures for TCP plus direct/forced-relay `off`/`auto`. Clean 8/32/64 MiB runs on source `93c68be` and a 16-logical-CPU Ryzen 7 7700X pass the +64 MiB transport allowance, 5 s connection, 2 s first-byte, 1 s cancellation, 5% direct/raw-TCP, 2% relay/raw-TCP, 50% auto/raw incompressible, and 50% compressible-savings budgets |
| Storage authority | storage unit/native tests | atomic local authority marker; remote authority unsupported |
| Status/readiness | lifecycle unit/native storage tests | liveness is process-only; readiness requires a bound pgwire socket, readable DuckLake snapshots, a synced/removable 4 KiB local-root file, and rolled-back internal DuckLake DDL. Tests prove no table/file/snapshot residue; startup/storage-failure/drain states are explicit |
| Termination/restart | `just duckdb-termination-profile` | clean source `59c1a381` actual-process smoke forced-drain rolls back an explicit uncommitted row, preserves exact committed state on same-path restart, becomes queryable in 135 ms, and accepts a post-restart write; general write/commit interruption and release-catalog recovery remain open |
| TLS/credential rotation | `just duckdb-tls-rotation-profile`, `just kind-secret-rotation-gate`, `just kind-rest-jwt-rotation-gate`, and `just rest-postgrest-smoke` | direct TLS/SCRAM and REST smokes preserve committed state, reject old trust/password/JWT material, and recover readiness. Packaged Kind rotates mutual-TLS and edge keys, denies the old client certificate and old authenticator credential's next lease, then reruns pgwire/REST gates. A separate two-replica JWT replacement accepts the new key and denies an old token against each replacement Pod. Zero-downtime multi-key overlap, immediate durable lease revocation, and production revocation remain open |
| Repository gate | `just ci` | Rust fmt/clippy/tests, native storage/pgwire, common evidence validation, probes, runtime static checks |

## Important implementation limits

- Query results stream one driver-produced Arrow batch at a time and fail closed
  before pgwire encoding when the configured byte ceiling is exceeded. Only
  native EOF permits connection reuse; partial delivery, failure, and cancellation
  quarantine uncertain stream state. The native pgwire cancellation regression
  proves the cancelled client receives a stable quarantine error while an
  independent session remains usable. Clean serial 1M/10M generated-BIGINT
  reference runs on source `12817bcd` observe one in-flight batch, first row before
  completion, and approximately 1.72/2.36 MiB RSS delta against the +128 MiB
  budget. The same envelope now covers nullable variable-width VARCHAR/BLOB data;
  its clean 1M reference on source `b240507e` checks every value across 489 native
  batches with one in flight, zero limit rejections, and 19.17 MiB RSS delta.
- COPY incrementally decodes bounded Arrow batches into one staging ADBC stream;
  the clean 10M reference on source `9e4611ed` passes exact count/sum/WKB
  publication over 647,777,780 wire bytes with 126 MiB RSS delta, 152.83 ms
  commit publication, and a 0.528 pgwire/direct ratio. A configurable
  pre-body frontend-frame ceiling now rejects a header-only oversized declaration
  without buffer growth, and the native workflow proves zero publication.
  Post-decode chunk/row/Arrow limits remain defense in depth; oversized decoded
  chunks and malformed final rows synchronously clean up staging with zero rows.
- Native cancellation/deadlines abort active query and COPY workers. Pgwire 0.40
  cannot asynchronously deliver a COPY error while the client sends no frames;
  ordinary writes now run in a cancellable pre-commit transaction. Autocommit
  cancellation rolls back and remains reusable; explicit-transaction cancellation
  rolls back and quarantines the session. Commit is non-cancellable and a failure
  is classified indeterminate. Deadline-triggered native
  cancellation uses the reserved control worker rather than a Tokio executor. A
  clean serial 100-sample reference run on source `8b0d1e46` records 1.51 ms p95,
  100 completed native calls, zero failures, explicit quarantine for every
  cancelled session, and a usable fresh session against the 500 ms M1 budget.
- Connection, queue, global active-query, reader/writer/maintenance-class, fixed
  blocking-worker, and DuckDB memory/thread/temp/spill controls are productized.
  Aggregate DuckDB memory and temporary storage are sampled on an independent
  session when metrics are enabled. A 32-contender unit gate and a 32-client
  suspended-portal native workload both prove the eight-operation admission
  ceiling. The separately registered mixed-class profile saturates a global limit
  with retained reader portals and COPY, observes reader, writer, and maintenance
  work queued together, and completes all three classes without rejection or
  timeout.
- The scalar transport profile takes five interleaved warm ADBC/pgwire samples
  and records their p50 overhead ratio. Reference mode fails unless direct ADBC
  lasts at least one second and pgwire is at most 15% slower. The clean 50M run
  on source `fc0b6069` is eligible at 1200.05 ms ADBC p50 and passes with a 0.999
  pgwire/ADBC ratio.
- Supported statement and parameter type surfaces are intentionally narrow.
- Broad `pg_catalog`/unmaintained `information_schema`, authoritative CRS/subtype/
  dimension metadata, SRID assignment, PostGIS box types, MVT, and general
  `ST_GeometryN` behavior remain incomplete.
  Role-bound maintained information-schema views cover schemas, tables, columns,
  and portable table/column grant rows with PostgreSQL `name`/`varchar` wire
  identity. The gated identity lane additionally projects DuckLake defaults and
  table/column comments through `pg_attrdef`, `pg_description`, `pg_get_expr`,
  `col_description`, and `obj_description` with PostgreSQL wire types and the
  same role/legacy ceiling. It also projects durable PostgreSQL 18 NOT-NULL rows
  through `pg_constraint`; `pg_index` is intentionally empty because DuckLake
  cannot enforce primary, unique, foreign-key, check, or index semantics. The
  same lane advertises `geometry_columns`/`spatial_ref_sys` through
  `information_schema.tables`, returns role-filtered generic geometry rows
  (`GEOMETRY`, dimension 2, SRID 0), and executes bounded `ST_SRID`, version,
  `ST_Extent`, and `ST_3DExtent` queries. The CRS catalog is intentionally empty.
  Process-local namespace/database/type/range/collation/owner views resolve the frozen core
  and QGIS-required built-ins, all referenced arrays/collations, and both spatial
  sentinels through explicit/implicit structural mapping. Logical database and
  search-path discovery return PostgreSQL `name`/`name[]` wire types. PostgreSQL
  18 lookup types and spatial RowDescription/text/binary/NULL behavior pass
  through pgwire.
- The direct-pgwire REST preview validates HS256 signatures against one
  operator-provisioned key, exact issuer/audience, expiry and optional not-before
  with 30-second skew, 24 KiB encoded/16 KiB normalized-claims ceilings, and one
  statically allowlisted role claim. One SCRAM authenticator serializes each read
  in a transaction with `SET LOCAL ROLE` and bound `request.jwt.claims`; packaged
  replicas instead use a passwordless exact authenticator lease.
  Role-filtered PostgreSQL catalog caches drive OpenAPI and direct request denial;
  successful request cleanup is proven. Every API/OpenAPI request validates a
  length-framed SHA-256 revision of the exact visible exposed schema; changed
  columns or grant visibility replace the cache, validation failure returns `503`,
  and database authorization remains authoritative. The identity lane consumes
  shared epochs with the pinned artifact; signed-only startup uses the exact digest fallback. Two
  replicas, readiness, balancing, failover, reconnect, and rotation pass.
- Rootless-Podman Kind mTLS tiny-client smoke passes with psql 18.3. Pinned
  psycopg 3.2.13 creates/reuses copied data, streams exact WKB/NULL rows with text
  COPY, closes/reconnects, and verifies exact scalar/spatial readback. Pinned
  GDAL/OGR 3.11.5 reads that fixture through its extended SQL-result cursor path
  and verifies exact Point/NULL GeoJSON; both workflows rerun after replacement
  and key rotation. Direct worker TCP, plaintext, and certificate-free access
  fail. The earlier direct-worker TLS/SCRAM baseline and traced OGR/QGIS generic
  spatial metadata, empty-geometry SRID, version, and extent query shapes execute
  in focused actual-pgwire tests. Current retries keep copied-data psql and OGR
  direct discovery/COPY/no-FID blocked at unavailable `pg_class`/`pg_proc`.
  QGIS reaches durable OID inquiry and also exposes `pg_is_in_recovery` and
  failed-transaction cleanup gaps. GeoServer, Martin, ORM, and BI remain
  unqualified.
- Remote/shared catalog and object-storage paths fail closed.
- The I0 binaries load owner-only key files and validate omitted-public versus
  explicit non-empty relay policy. The registered direct-path test disables
  relays only inside the test and proves framing/authentication/multiplexing
  against a fake trust-mode pgwire backend. The executable worker requires a
  loopback backend, validates startup `user` against the lease, and rejects TLS,
  GSS encryption, or any backend credential challenge across the iroh leg.
  The native direct, forced-custom-relay, and opt-in public-default-relay gates
  differentially cover result values/types, stable errors, scalar/typed/spatial
  queries, portals, commit/rollback/disconnect, successful and malformed COPY
  atomicity, cancellation/quarantine, concurrent sessions, and fresh reconnect.
  The transport profile uses a deterministic trust-mode echo backend to isolate
  connection/framing/codec cost; it does not replace the native SQL oracle or the
  existing M1/M2 result/COPY memory gates. The package now protects the local
  boundary with mTLS, per-process key mounts, exact credential-to-role leases, and
  loopback role-catalog preauthentication. Packaged resource budgets and
  hosted-relay qualification remain open.
- COPY validates and maintains the explicit four-column bbox layout in DuckDB SQL;
  partial/wrong-type/caller-supplied/ambiguous layouts fail before staging, and a
  pgwire rejection/reuse plus exact/reopen oracle passes. A narrow AST rule adds
  conservative candidates to one-table exact `ST_Intersects` predicates over the
  maintained geometry for bounded literal envelope/text and numbered-bound WKB probes.
  It runs consistently before describe and execution, retains the exact predicate,
  and leaves OR/NOT, joins, subqueries, multiple matches, and arbitrary probes
  unchanged. The registered profile now combines DuckDB 1.5.4's
  `OPERATOR_ROW_GROUPS_SCANNED` with Parquet compressed-row-group metadata. It
  treats the largest N row groups as a conservative byte upper bound when the
  profiler reports N scans. Native geometry files are about 45% smaller, but
  Local 1.0 retains the maintained WKB/bbox representation because it is the
  proven COPY, mutation, pgwire, and catalog contract. The complete v5 workload
  on source `8490ed7` adds eight-group aggregates, four-probe bounded spatial
  joins, nine-column wide
  projections with 1 KiB payloads and first-row timing, line/polygon maintained
  updates, and explicit file/row-group sizing. Two clean 10M runs pass before two
  100M runs. At 100M both pairs load in 24.35–24.39 seconds; every selective,
  grouped, join, and wide exact/plan oracle survives compaction; all p50/p95/p99
  measurements stay below 23.59 ms; process RSS grows by at most 113.03 MiB and
  sampled DuckDB memory by 32.62 MiB; temporary storage remains zero; selective,
  grouped, and wide scans read one of 825 row groups; compressed-byte upper bounds
  are at most 0.129%; and compaction reduces 25 files to 7 bbox/4 native files
  with maximum files of 541/584 MiB under the 1 GiB policy. The observed 121,212
  average rows per 100M row group retain DuckDB's default row-group sizing.
  Direct INSERT, reserved-column assignment, tuple geometry assignment, and
  arbitrary geometry expressions fail closed so they cannot create stale or
  forged bounds. Numbered-bound point, linestring, polygon, or NULL geometry
  UPDATEs recompute all four bounds in the same DuckDB statement, with
  malformed-input atomicity, rollback, and point-reopen evidence. Ordinary-column
  UPDATEs preserve geometry/bounds.
- Online/relocated production backup/restore, rolling upgrade, soak, and disaster
  recovery remain unproven.

## Milestone status

| Milestone | State | Next closure work |
|---|---|---|
| M0 truthful repository | complete | `just project-contract-check` validates links/recipes/spatial counts; required CI invokes maintained Justfile gates and publishes the deterministic transport-smoke manifest |
| M1 bounded execution | complete | ADBC streams retain ownership; pgwire pulls one batch under a fail-closed byte ceiling; only native EOF permits reuse; native query/write cancel and deadlines use reserved control capacity; cancelled and partial streams have explicit rollback/reuse/quarantine outcomes; failed-transaction rollback/reuse, classed admission, autosized resources, and sampled memory/temp storage are implemented. Clean 1M/10M generated-BIGINT references pass RSS/first-row/one-batch gates, a clean 100-cancel reference passes at 1.51 ms p95, the clean 1M wide nullable VARCHAR/BLOB reference crosses 489 batches at 19.17 MiB RSS delta, and a clean eligible 50M transport reference passes at a 0.999 pgwire/ADBC p50 ratio. A native mixed-class profile proves simultaneous reader/writer/maintenance queueing and bounded completion. Commit is a documented non-cancellable boundary with indeterminate-failure classification. |
| M2 streaming ingest | complete | configurable pre-body frontend-frame rejection, incremental contiguous bounded parsing, exact Arrow batch byte splitting, staging ADBC stream, atomic publication, text escapes, >20 MiB/220k-row regression, synchronous malformed-final-row and oversized-decoded-chunk cleanup, abort zero-row tests, scalar/NULL/WKB reopen, compaction, and metrics are implemented. The clean 10M reference on source `9e4611ed` passes exact publication, 126 MiB RSS delta, and a 0.528 pgwire/direct ratio. Pgwire 0.40 still cannot deliver an asynchronous COPY error while an idle client sends no frame; cancellation is enforced before publication and delivered when the client resumes or disconnects. |
| M3 focused compatibility | C3/C4 complete; C5 active; H0 packaged preview complete; psycopg copied-data and OGR SQL-result read slices complete | core profile/oracle, durable DuckLake identity, exact psql/OGR/QGIS traces, built-in/spatial/logical-database identity, user catalogs/origins/defaults/comments/NOT-NULL constraints/epochs, empty index discovery, role-aware catalogs/information schema, common privilege enforcement, two-replica JWT-to-role REST/OpenAPI, packaged psycopg COPY/reconnect, and OGR SQL-result Point/NULL readback pass. Retried psql `\d+` remains blocked at `pg_class`; OGR direct discovery reaches `pg_proc` then `pg_class`; QGIS 3.44.11 reaches durable OID inquiry and then exposes missing `pg_is_in_recovery` plus failed-transaction cleanup gaps. Remaining: pinned package/client rerun, generic query-shape closure, key/index policy, authoritative CRS metadata, copied-data psql/QGIS, and OGR direct discovery/COPY/no-FID. Mutable role DDL, RLS, and HTTP mutations remain follow-on work. |
| M4 analytical performance | complete | fail-closed bbox maintenance, exact recheck, hole/null/empty/invalid/boundary correctness, point/line/polygon mutation, compressed scan-byte accounting, and fragmented-file compaction pass. The v5 profile covers selective scans, grouped aggregates, bounded spatial joins, and wide projections. Two clean 10M runs precede two clean 100M runs on source `8490ed7`; all exact results and plans survive compaction, scan reduction exceeds 20x and stays below 5% bytes, first-row/p50/p95/p99/RSS/DuckDB-memory/zero-spill/file/row-group/load budgets pass, and 25 files compact under the chosen 1 GiB policy. Local 1.0 retains maintained WKB/bbox storage despite native files being about 45% smaller because only the maintained path has the required write/client contract. |
| M5 Local 1.0 | active; K0 package boundary complete | immutable load-only runtime smoke, local readiness/drain/compaction, checksummed offline backup/restore, host TLS/SCRAM and direct REST rotation, and packaged direct iroh exist. Rootless-Podman Kind uses node-local digest aliases, retained storage, one ordered core Pod, mutual-TLS `postgres` ingress, and two load-balanced authenticator REST Pods with no database password. Psycopg COPY/reconnect, OGR SQL-result readback, role-aware REST, one-Pod failover, and core reconnect pass; direct/plaintext/certificate-free access fails; a 3.935 s core replacement reconnects all gates; packaged mTLS/edge/authenticator/JWT replacement denies old credentials. Host I0 profiles remain authoritative for performance. Remaining local work: pinned identity and resource/hosted-relay package reruns, PostgreSQL query-shape/copied-client closure, catalog/control lifecycle, published artifacts, production recovery/upgrade, mixed workload, and soak. Final bundle selection remains externally blocked on unreleased DuckDB candidates. |
| M6 Shared iroh cluster 1.x | deferred | begins only after Local 1.0; migrates I0's config-backed registered key and one-worker lease to durable control/users, SCRAM pairing, multiple bootstrap nodes, bounded bootstrap/worker gossip, enforced worker affinity, shared DuckLake, and one common pgwire/HTTP edge connection without changing the tiny client transport contract |
| M7 dataset lifecycle | deferred | official snapshot protection/promotion after shared/local operations mature |

## Claim maintenance rule

When evidence lands:

1. add or update the executable gate;
2. update this status table and `docs/COMPATIBILITY.md`;
3. record relevant resource/performance numbers with hardware/data/artifact pins;
4. update `ROADMAP.md` only if an exit gate, priority, or outcome changes; and
5. remove compatibility code and stale documentation superseded by upstream
   DuckDB/DuckLake behavior.
