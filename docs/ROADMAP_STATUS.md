# Roadmap status

This is the current evidence floor for the DuckDB-only runtime. It intentionally
does not inherit claims from retired DataFusion/Sedona or unsupported shared
deployment profiles.

## Local-first closure program

The roadmap now separates smoke, local, reference, and external evidence. Direct
host/container profiles own performance budgets; a new minimal DuckDB-only Kind
topology will own packaged client/lifecycle/recovery evidence. The retired broad
Kind tree will not be restored. Current iteration state:

| Work package | State | Next executable result |
|---|---|---|
| E0 evidence harness | active; shared fixture established | common evidence envelope plus reusable fresh DuckLake/server/client runtime now support separately registered profiles; continue extracting fixtures only when the next profile needs them |
| E1 M1/M2 local profiles | complete | clean 1M/10M BIGINT, 1M wide-result, 100-cancel, eligible 50M transport, and 10M COPY references pass; mixed reader/writer/maintenance admission and cancellable-write rollback/reuse/quarantine oracles pass |
| K0 minimal Kind topology | local execution passes | rootless Podman runs one TLS-required runtime StatefulSet, retained local PV/PVC, generated Secrets, probes, and psql/psycopg/OGR Jobs from node-local digest-addressed images; next packaged rotation and recovery |
| C0 PostgreSQL compatibility | C3 implementation complete in the exact-1.5.4 gated lane; C4 complete; C5 enforcement, relational role/membership catalogs, durable owner/default/comment projection, bounded privilege inquiry, and role-aware schema/table/column/grant information-schema views consume the same role decisions; actual pgwire proves PostgreSQL-typed, role/legacy-filtered defaults/comments plus writer/ungranted/SELECT-only inquiry, discovery, and execution agreement | implement traced constraints/indexes/spatial work; obtain official identity bundle before release and run copied-data clients/role-aware REST |
| H0 role-aware HTTP | authenticated read-only actual-pgwire preview passes | catalog-backed discovery, JWT role mapping, transaction-local role/context, role-aware OpenAPI, immutable image, and Kind replicas |
| P0 M4 host profiles | active foundation; conservative single-table correctness matrix passes | add the scan-byte/plan oracle with `OPERATOR_ROW_GROUPS_SCANNED` and a native-geometry Parquet-statistics baseline, then delete redundant bbox machinery if it wins; run 10M twice before 100M |
| K1 operations/shared rehearsal | deferred by dependency | Local 1.0 operations first; PostgreSQL/MinIO is rehearsal, not managed evidence |
| U0 upstream adoption | design active | conditional deletion/adoption matrix now covers 1.5.5/2.0, PEG, Quack, async I/O, C/Rust extension APIs, native spatial/pruning, and DuckLake protected snapshots/RBAC/UDTs/materialized views; executable candidate matrices remain open |

## Current verified floor

| Area | Evidence | Current boundary |
|---|---|---|
| Engine/storage | `just duckdb-adbc-storage-test` | pinned DuckDB 1.5.4, official local DuckLake, Arrow ingest/query, transaction, snapshot inspection, adjacent-file merge, reopen, checksummed offline exact-path backup/restore |
| Pgwire workflow | `just duckdb-pgwire-workflow-test` | structural statements/parameters; exact all-allowlisted QGIS 3.44 four-`SET` bootstrap with bounded batch/application name; incremental >20 MiB/220k-row COPY with atomic abort paths; scalar/NULL/WKB reopen; catalog/wire identity; spatial gaps; compaction; independent/failed transactions; streaming/portals; restart |
| Auth/policy | real CLI SCRAM, table allowlist, role-session, request-context, effective-role grant/denial, relational role/membership catalogs, role/schema/table/column inquiry, and role-aware information-schema cases in pgwire workflow | bounded immutable graph and identity lifecycle; common enforcement, inquiry, and discovery decisions; stable role/edge OIDs and resolving PostgreSQL 18 options with NULL credentials. Name inquiry and schema/table/column/grant discovery work in the official lane; OID/expression inquiry is durable-identity gated; OpenAPI consistency remains open |
| Spatial | pgwire workflow + `tests/duckdb_spatial_compat.json` | 42 original PostGIS expressions: 31 native, 5 rewrites, 6 macros |
| Spatial gaps | `docs/DUCKDB_SPATIAL_GAP_LEDGER.md` | 10 Rust/catalog-edge gaps and 5 extension candidates have ledger-pinned `0A000` simple/extended pgwire behavior; semantics remain unsupported |
| Spatial pruning | `just duckdb-adbc-storage-test` and `just duckdb-pgwire-workflow-test` | one-table maintained-layout `ST_Intersects` adds planner-visible four-axis candidates for bounded literal envelope/text and numbered-bound WKB probes while retaining the exact predicate. Shape/size-denial units plus native hole/boundary/NULL/empty/invalid/bound/reopen/`EXPLAIN` exact-oracle comparisons and pgwire literal cases pass; scan-byte and scale claims remain open |
| WKB/Arrow | storage/pgwire native tests + `just arrow-encoder-test` | maintained WKB bytes, relational geometry/geography catalog lookup with PostgreSQL 18 result types, RowDescription/text/binary/NULL identity for both families, generated WKB/fixed-binary properties, scalar/list parity, fail-closed invalid shapes, and panic-free nested errors; broad client discovery and generated temporal/decimal/dictionary/nested coverage remain open |
| Catalog contract | `just duckdb-catalog-contract-test` | explicit/implicit namespace/database/type/range/collation/owner-role mapping passes with stable logical-database identity, PostgreSQL-shaped `current_database`/`current_schema`/`current_schemas`, 24 exact PostgreSQL 18 built-ins, and PostGIS-shaped spatial scalar/array rows. Full-row oracle equality, reciprocal references, inferred/explicit OID parameters, wire types including `name[]`, alias safety, and spatial transport pass. All unimplemented/private routes plus wildcard/nested/set/derived/implicit-join/CTE/cross-database shapes fail closed; `TABLE` is rejected before authorization. User-object catalogs/origins remain open |
| PostgreSQL 18 target profile | `just project-contract-check` | machine-readable `pg18-column-core-v1` validates required relation/query/result-type shape against a digest-pinned PostgreSQL 18.4 oracle. Credential-free OGR 3.11.5 (21 queries), psql 18.3 `\d+` (12), and QGIS 3.44.11 (32 statements/26 unique) traces pin clients and PostgreSQL/PostGIS images; only dependent RBAC/OpenAPI traces remain. Target traces are not QuackGIS implementation evidence |
| Catalog identity feasibility | `just duckdb-catalog-identity-test` plus opt-in `just duckdb-development-ducklake-test` | independent DuckDB 1.5.4 processes prove table ID/UUID and field-ID durability. The exact-1.5.4 development function now drives protected DuckLake mappings: fixed public/dynamic namespace OIDs, globally allocated relation/row-type OIDs, monotonic per-table attribute numbers, retained tombstones, serialized two-session commits, all public create APIs, explicit unconstrained-table invariants/corruption rejection, and a schema/default/comment fingerprint epoch across autocommit/explicit commit/rollback/rename/add/reopen/drop-recreate/non-public-schema evidence. Empty schemas have no durable API identity and remain unsupported. Upstream merge/signed-bundle evidence remain release gates |
| REST preview | `just rest-check` and `just rest-postgrest-smoke` | separate authenticated read-only sidecar pins `pg-rest-server` parser/schema inputs and proves projection, typed filters, ordering, pagination, OpenAPI, schema discovery, denial paths, and escaped WKB through actual QuackGIS pgwire; full PostgREST parity, writes, RPC, embedding, GeoJSON, and packaged deployment remain open |
| Runtime supply chain | `just duckdb-runtime-offline-smoke` | verified context digests/licenses, preinstalled signed extensions, load-only image and server-start smoke |
| Current transport profiles | `just duckdb-current-benchmark`, `just duckdb-transport-profile`, and `just evidence-manifest-check` | one deterministic scalar full-scan scenario/oracle runs at smoke/local/reference row counts in the common envelope; ADBC/pgwire are warmed and interleaved. A clean 50M reference on source `fc0b6069` records 1200.05 ms ADBC p50 and a 0.999 pgwire/ADBC p50 ratio, passing the one-second eligibility floor and 1.15 ceiling |
| Storage authority | storage unit/native tests | atomic local authority marker; remote authority unsupported |
| Status/readiness | lifecycle unit/native storage tests | liveness is process-only; readiness requires a bound pgwire socket and a successful read-only DuckLake snapshot probe, and reports startup/storage-failure/drain states |
| Termination/restart | `just duckdb-termination-profile` | clean source `59c1a381` actual-process smoke forced-drain rolls back an explicit uncommitted row, preserves exact committed state on same-path restart, becomes queryable in 135 ms, and accepts a post-restart write; general write/commit interruption and release-catalog recovery remain open |
| TLS/credential rotation | `just duckdb-tls-rotation-profile` | actual-process TLS-required/SCRAM smoke verifies client hostname/trust, rejects plaintext and wrong trust, restarts with replacement certificate/password while preserving committed state, rejects old trust/password, and accepts a post-rotation write; packaged Kind rotation and production revocation remain open |
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
- Broad `pg_catalog`/unmaintained `information_schema`, spatial discovery, SRID,
  dimension, extent, MVT, and general `ST_GeometryN` behavior remain incomplete.
  Role-bound maintained information-schema views cover schemas, tables, columns,
  and portable table/column grant rows with PostgreSQL `name`/`varchar` wire
  identity. The gated identity lane additionally projects DuckLake defaults and
  table/column comments through `pg_attrdef`, `pg_description`, `pg_get_expr`,
  `col_description`, and `obj_description` with PostgreSQL wire types and the
  same role/legacy ceiling. Relational
  process-local namespace/database/type/range/collation/owner views resolve the frozen core
  and QGIS-required built-ins, all referenced arrays/collations, and both spatial
  sentinels through explicit/implicit structural mapping. Logical database and
  search-path discovery return PostgreSQL `name`/`name[]` wire types. PostgreSQL
  18 lookup types and spatial RowDescription/text/binary/NULL behavior pass
  through pgwire.
- Rootless-Podman Kind TLS/SCRAM `SELECT 1` smokes pass with psql 18.3, psycopg
  3.2.13, and GDAL/OGR 3.11.5. OGR still encounters the documented unsupported
  `ST_SRID` discovery path before its explicit scalar query succeeds. Named QGIS,
  GeoServer, Martin, ORM, and BI workflows remain unqualified.
- Remote/shared catalog and object-storage paths fail closed.
- COPY validates and maintains the explicit four-column bbox layout in DuckDB SQL;
  partial/wrong-type/caller-supplied/ambiguous layouts fail before staging, and a
  pgwire rejection/reuse plus exact/reopen oracle passes. A narrow AST rule adds
  conservative candidates to one-table exact `ST_Intersects` predicates over the
  maintained geometry for bounded literal envelope/text and numbered-bound WKB probes.
  It runs consistently before describe and execution, retains the exact predicate,
  and leaves OR/NOT, joins, subqueries, multiple matches, and arbitrary probes
  unchanged. Automatic DDL, broader mutation and compaction refresh, scan-byte
  plans, and scale evidence remain open.
  Direct INSERT, reserved-column assignment, tuple geometry assignment, and
  arbitrary geometry expressions fail closed so they cannot create stale or
  forged bounds. A numbered-bound or NULL geometry UPDATE recomputes all four
  bounds in the same DuckDB statement, with malformed-input atomicity, rollback,
  and reopen evidence. Ordinary-column UPDATEs preserve geometry/bounds.
- Online/relocated production backup/restore, rolling upgrade, soak, and disaster
  recovery remain unproven.

## Milestone status

| Milestone | State | Next closure work |
|---|---|---|
| M0 truthful repository | complete | `just project-contract-check` validates links/recipes/spatial counts; required CI invokes maintained Justfile gates and publishes the deterministic transport-smoke manifest |
| M1 bounded execution | complete | ADBC streams retain ownership; pgwire pulls one batch under a fail-closed byte ceiling; only native EOF permits reuse; native query/write cancel and deadlines use reserved control capacity; cancelled and partial streams have explicit rollback/reuse/quarantine outcomes; failed-transaction rollback/reuse, classed admission, autosized resources, and sampled memory/temp storage are implemented. Clean 1M/10M generated-BIGINT references pass RSS/first-row/one-batch gates, a clean 100-cancel reference passes at 1.51 ms p95, the clean 1M wide nullable VARCHAR/BLOB reference crosses 489 batches at 19.17 MiB RSS delta, and a clean eligible 50M transport reference passes at a 0.999 pgwire/ADBC p50 ratio. A native mixed-class profile proves simultaneous reader/writer/maintenance queueing and bounded completion. Commit is a documented non-cancellable boundary with indeterminate-failure classification. |
| M2 streaming ingest | complete | configurable pre-body frontend-frame rejection, incremental contiguous bounded parsing, exact Arrow batch byte splitting, staging ADBC stream, atomic publication, text escapes, >20 MiB/220k-row regression, synchronous malformed-final-row and oversized-decoded-chunk cleanup, abort zero-row tests, scalar/NULL/WKB reopen, compaction, and metrics are implemented. The clean 10M reference on source `9e4611ed` passes exact publication, 126 MiB RSS delta, and a 0.528 pgwire/direct ratio. Pgwire 0.40 still cannot deliver an asynchronous COPY error while an idle client sends no frame; cancellation is enforced before publication and delivered when the client resumes or disconnects. |
| M3 focused compatibility | C3/C4 complete; C5 active | core profile/oracle, durable DuckLake identity, exact psql/OGR/QGIS traces, built-in/spatial/logical-database identity, user catalogs/origins/defaults/comments/epochs, immutable role sessions/context, cancellation isolation, common privilege enforcement, role/membership catalogs, owner projection, bounded privilege inquiry, and role-aware schema/table/column/grant information schema pass focused/actual pgwire evidence. Remaining: upstream identity bundle; traced constraint/index/spatial catalogs; REST epoch consumption; richer provenance; copied-data clients and role-aware REST. Mutable role DDL, RLS, and HTTP mutations remain follow-on work. |
| M4 analytical performance | active foundation | fail-closed COPY bbox maintenance, atomic numbered-bound/NULL geometry UPDATE refresh, direct INSERT/arbitrary geometry/reserved UPDATE rejection, ordinary-column UPDATE preservation, and ordinary file compaction are implemented. Conservative one-table AST injection now handles bounded literal envelope/text and numbered-bound WKB probes, retains exact recheck, and passes hole/boundary/NULL/empty/invalid/reopen/plan exact-oracle evidence. Remaining: broader geometry mutation/spatial-compaction policy, scan-byte plans, and current 10M then 100M profiles. |
| M5 Local 1.0 | active foundation | immutable load-only runtime smoke, process liveness and pgwire-bind/read-only DuckLake readiness, configured drain, authorized/audited compaction, checksummed offline exact-path backup/restore, actual-process TLS/SCRAM/plaintext-denial/restart-rotation evidence, explicit TLS-required startup policy, and real rootless-Podman Kind execution exist. The Kind run uses a pinned node, node-local digest aliases, one TLS-required StatefulSet, retained local storage, probes, and passing psql/psycopg/OGR smoke Jobs. A clean actual-process profile on source `59c1a381` proves forced-drain rollback of an explicit uncommitted transaction, exact same-path restart in 135 ms at smoke scale, and a successful post-restart write. Host profiles remain authoritative for performance. Remaining: PostgreSQL 18 catalog/RBAC and named-client M3 closure; catalog/control-metadata lifecycle evidence; packaged role-aware multi-replica REST/OpenAPI; authenticator/JWT/database credential rotation; packaged rotation, write-capacity readiness SLO, published artifacts, online/relocated production recovery, upgrades, mixed workload, and 24-hour soak. |
| M6 Shared DuckLake 1.x | deferred | begins only after Local 1.0 |
| M7 dataset lifecycle | deferred | official snapshot protection/promotion after shared/local operations mature |

## Claim maintenance rule

When evidence lands:

1. add or update the executable gate;
2. update this status table and `docs/COMPATIBILITY.md`;
3. record relevant resource/performance numbers with hardware/data/artifact pins;
4. update `ROADMAP.md` only if an exit gate, priority, or outcome changes; and
5. remove compatibility code and stale documentation superseded by upstream
   DuckDB/DuckLake behavior.
