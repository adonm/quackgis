# Release evidence policy

Every release claim must be reviewable from artifacts tied to one source SHA and
one exact DuckDB/extension bundle.

## Current preview evidence

| Artifact/gate | Purpose |
|---|---|
| `just check-fast` | Rust formatting, clippy, unit and compile-time integration targets |
| `just project-contract-check` | documentation/recipe/spatial claims plus the machine-readable PostgreSQL 18 profile/reference consistency contract |
| `just duckdb-adbc-storage-test` | native Arrow/DuckLake write/query/transaction/reopen behavior |
| `just duckdb-pgwire-workflow-test` | real server protocol, auth, policy, COPY, transaction, spatial, restart behavior |
| `just duckdb-catalog-contract-test` | client-neutral DuckDB-derived metadata plus bounded geometry type/transport contract |
| `just duckdb-catalog-identity-test` | independent-process DuckLake table/column identity across rename, reopen, and drop/recreate plus the PostgreSQL OID-registry decision |
| `just ducklake-pinned-source-check` / `just duckdb-pinned-ducklake-test` | tracked DuckLake source/patch/tool/artifact authority plus durable PostgreSQL identity/epoch lifecycle through the accepted binary |
| `just duckdb-spatial-compat-probe` | maintained spatial disposition/result classification |
| `just duckdb-runtime-static-check` / `just duckdb-runtime-offline-smoke` | immutable no-install runtime contract plus network-disabled pinned-extension load and server startup |
| `just evidence-manifest-check` | common evidence envelope, source/runtime/host metadata, and claim-level validation |
| `just duckdb-transport-profile` | parameterized smoke/local/reference scalar transport scenario using one correctness oracle |
| `just duckdb-result-stream-profile` | parameterized result RSS/first-row/cardinality profile; 1M/10M reference runs close the M1 scale gate |
| `just duckdb-wide-result-profile` | nullable variable-width VARCHAR/BLOB RSS, exact-value, and native-batch profile |
| `just duckdb-cancellation-profile` | sequential long-query cancel latency, SQLSTATE, quarantine, fresh-session, and native counter evidence |
| `just duckdb-mixed-concurrency-profile` | retained native reader/writer work, all-class queueing, class/global ceilings, completion, and rejection/timeout evidence |
| `just duckdb-termination-profile` | actual-process forced drain, uncommitted rollback, same-path restart timing, exact state, and post-restart write evidence |
| `just duckdb-tls-rotation-profile` | actual-process TLS/SCRAM, certificate/hostname verification, plaintext and wrong-trust denial, restart rotation, old-password rejection, and exact-state evidence |
| `just duckdb-copy-profile` | direct ADBC versus pgwire COPY RSS, throughput, publication metrics, and exact WKB/count/sum oracle |
| `just duckdb-spatial-scan-profile` | M4 mixed point/line/polygon maintained-bbox versus native-geometry selective scan, grouped aggregate, bounded join, wide projection, exact plan/recheck, row-group/scan-byte, first-row/p50/p95/p99, RSS, DuckDB-memory, spill, file sizing, and compaction evidence |
| `just kind-static-check` | minimal DuckDB-only topology shape, immutable image input, secret rendering, and client-job contract |
| `just doctor` / `just doctor-kind` | installed/missing project tools, selected container engine, and local Kind prerequisites |
| `just kind-up-local` / `just kind-client-gates` / `just kind-qgis-gate` / `just kind-restart-gate` / `just kind-secret-rotation-gate` | rootless Podman-or-Docker execution of the node-local digest-addressed server/bootstrap/worker/client package; pinned psql full describe, psycopg copied-data WKB/NULL COPY/reconnect, OGR SQL-result/direct Point/NULL reads plus predeclared-target COPY, and optional digest-pinned QGIS query-layer fields/count/binary/filter/extent/viewport-identify/render reads pass through mTLS. Direct/plaintext/certificate-free paths fail, and rotated old client credentials fail |
| `just rest-check` | pinned pg-rest-server parser/query contract and REST trust-boundary unit tests |
| `just rest-postgrest-smoke` | authenticated read-only PostgREST subset plus WKB extension through an actual DuckDB/DuckLake pgwire server |
| `.tmp/duckdb/manifest.json` | native library/extension paths and SHA-256 values |
| `.tmp/duckdb-current-benchmark/manifest.json` | deterministic direct DuckDB/ADBC/pgwire correctness and liveness comparison |

`just ci` is the required aggregate local/hosted CI gate after native bootstrap.
The maintained workflow calls the same Justfile recipes and uploads verification
manifests, not redistributable native binaries.

## Evidence levels

| Level | Use |
|---|---|
| smoke | fast code/contract regression; never a scale claim |
| local | complete functional scenario and oracle at reduced scale |
| reference | exact scale/duration/budgets on a named host; may close local gates |
| external | published-artifact or managed-service proof explicitly required by a gate |

Performance evidence runs directly on the named host or one constrained
container. Minimal Kind runs provide packaged topology, client, TLS, lifecycle,
recovery, upgrade, mixed-workload, and soak evidence; they are not primary
performance evidence. PostgreSQL/MinIO in Kind is M6 rehearsal only.

## Local 1.0 release packet

The release packet must include:

- source SHA, version, Rust lockfile, DuckDB/extension versions and digests,
  DuckLake upstream/patch/build pins, and patch checksum;
- server binary checksum and runtime image digest;
- all M1–M4 and I0 test/client/performance reports;
- the declared PostgreSQL 18 compatibility manifest and normalized differential
  catalog/session fixtures;
- stable OID, RowDescription origin, schema/security epoch, restart, rename,
  rollback, backup/restore, and upgrade evidence;
- denied/anonymous/reader/editor role matrices proving privilege inquiry,
  catalog/information-schema visibility, execution, and OpenAPI agree;
- named client versions and copied-data manifests;
- exact 10M profile hardware/data/budget/results;
- query/COPY memory, spill, cancellation, and admission evidence;
- direct TCP, iroh direct-path, public-relay-default, configured-relay, and forced-
  relay reports covering connection/first-row latency, result/COPY throughput,
  CPU, RSS, streams, cancellation, reconnect, and relay selection;
- named pgwire and HTTP client evidence entering through the packaged tiny client,
  plus release-profile refusal of direct application connections to the worker;
- adaptive-compression reports covering disabled/automatic modes, compressible and
  incompressible shapes, bytes saved, codec CPU/latency, raw-path overhead,
  bounded decode failures, and proof that authentication/control traffic and
  cross-session dictionaries are never compressed;
- backup/restore and upgrade/rollback transcripts, including the format-v2
  backup's runtime identity and deliberate matching bundle selection;
- TLS/auth/secret-rotation evidence;
- authenticator/JWT role-mapping, request-context cleanup, multi-replica REST,
  cache-invalidation, and credential-rotation evidence;
- 24-hour soak summary and raw metrics/log locations; and
- known limits copied from `COMPATIBILITY.md` and `ROADMAP_STATUS.md`.

## Evidence rules

- Artifacts identify exact QuackGIS source SHA and N0 bundle ID plus DuckDB,
  DuckLake, Spatial, QuackGIS-extension, patch, toolchain, build-option, license,
  SBOM, and artifact digests. Two clean cache-disabled builds must reproduce every
  selected digest; the current central Spatial candidate fails this gate and stays
  unaccepted. Before N0 closes, the current DuckLake pin and separate official
  artifact identities remain explicit.
- Candidate review records the latest supported DuckDB release, current
  DuckDB-versioned DuckLake/Spatial tips, every local native capability's
  adopt/retain/delete disposition, and one deletion review per patch. A moved ref
  requires a fresh `just native-upstream-check` review before acceptance.
- Artifacts identify their evidence level and execution environment.
- Performance claims identify rows, bytes, files, row groups, load method,
  hardware, concurrency, plans, and budget outcome.
- Result checks use counts/checksums/exact spatial oracles, not successful exit
  alone.
- Metrics and logs exclude SQL text, parameters, credentials, WKB, signed URIs,
  and sensitive paths.
- Missing required evidence downgrades the artifact to a preview/manual build.
- Retired-engine, emulator-only, static-profile, and design evidence cannot satisfy
  current release gates.

## Shared 1.x evidence

Shared claims additionally require two repeatable managed-service runs covering:

- multi-process DuckLake visibility, conflicts/response loss,
  throttling/interruption, cleanup, independent DuckDB reopen, and the 24-hour
  measured topology;
- PostgreSQL control-database backup/restore of users, roles, client credentials,
  revocation, pools, assignments, and security/configuration epochs without
  private keys;
- one-time pairing, registered-key/access-lease/password rotation, explicit
  revocation, and one-credential/one-worker assignment fencing during graceful
  drain and abrupt failure;
- public-default and explicitly configured hosted relays, adaptive compression,
  reconnect, cancellation, common pgwire/HTTP delivery, and one serverless client
  profile; and
- the declared mostly-idle client-session target with bounded native leases,
  pinned work, active queries, queues, RSS, and per-credential fairness on one
  worker.

Local emulator results are companion evidence, not managed-service proof.
