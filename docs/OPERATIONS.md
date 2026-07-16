# Operations

QuackGIS is a local-first developer preview. The sole runtime is an owned Rust
pgwire server that dynamically loads a checksum-pinned DuckDB ADBC library and
uses DuckDB's official `spatial` and `ducklake` extensions.

## Bootstrap and verify

```sh
mise install
mise run duckdb-bootstrap
mise exec -- just ci
```

The bootstrap writes ignored artifacts below `.tmp/duckdb`, verifies the official
DuckDB 1.5.4 archive/library digests, and installs version-matched signed
extensions into an isolated DuckDB home. Runtime startup uses `LOAD` only; it does
not install extensions over the network.

## Run locally

```sh
mkdir -p .tmp/duckdb-server
mise exec -- cargo run -p quackgis-server -- \
  --catalog-path=.tmp/duckdb-server/catalog.ducklake \
  --data-path=.tmp/duckdb-server/data
```

The mise environment supplies `QUACKGIS_DUCKDB_ADBC_DRIVER` and the isolated
DuckDB home. The server defaults to `127.0.0.1:5434`.

## Configuration

| Variable | Default | Purpose |
|---|---|---|
| `QUACKGIS_DUCKDB_ADBC_DRIVER` | unset outside mise | required absolute path to pinned `libduckdb` |
| `QUACKGIS_DUCKDB_DATABASE_URI` | `:memory:` | DuckDB control database URI |
| `QUACKGIS_DEV_DUCKLAKE_EXTENSION` / `QUACKGIS_DEV_DUCKLAKE_EXTENSION_SHA256` | unset | paired development-only absolute extension path and exact lowercase SHA-256; never release-supported |
| `QUACKGIS_HOST` / `QUACKGIS_PORT` | `127.0.0.1` / `5434` | pgwire bind |
| `QUACKGIS_MAX_CONNECTIONS` | `64` | accepted connection bound |
| `QUACKGIS_MAX_ACTIVE_QUERIES` / `QUACKGIS_MAX_QUEUED_QUERIES` | `8` / `64` | execution and admission queue bounds |
| `QUACKGIS_MAX_READER_QUERIES` / `QUACKGIS_MAX_WRITER_QUERIES` | `8` / `2` | reader and writer ceilings within the global active bound |
| `QUACKGIS_MAX_MAINTENANCE_QUERIES` | `1` | server-owned maintenance concurrency ceiling |
| `QUACKGIS_MAX_BLOCKING_WORKERS` | `9` | native worker ceiling; one slot is reserved for cancellation/control |
| `QUACKGIS_QUEUE_TIMEOUT_MS` / `QUACKGIS_STATEMENT_TIMEOUT_MS` | `30000` / `300000` | queue and active query deadlines |
| `QUACKGIS_SHUTDOWN_TIMEOUT_MS` | `30000` | graceful connection/transaction drain deadline |
| `QUACKGIS_RESULT_BATCH_BYTES` | `8388608` | fail-closed Arrow result batch ceiling before pgwire encoding |
| `QUACKGIS_PGWIRE_MAX_FRAME_BYTES` | `16777216` | maximum PostgreSQL declared frontend-message length, checked from the header before body read/decode |
| `QUACKGIS_COPY_BATCH_ROWS` | `65536` | maximum rows per COPY Arrow batch |
| `QUACKGIS_COPY_BATCH_BYTES` | `8388608` | maximum Arrow batch and accepted COPY chunk after pgwire decoding |
| `QUACKGIS_COPY_MAX_ROW_BYTES` | `1048576` | maximum encoded bytes in one COPY row |
| `QUACKGIS_DUCKDB_THREADS` | effective cgroup/host CPUs | DuckDB execution threads |
| `QUACKGIS_DUCKDB_MEMORY_LIMIT_BYTES` | 60% of effective cgroup/host memory; 1 GiB detection fallback | DuckDB memory limit |
| `QUACKGIS_DUCKDB_TEMP_DIRECTORY` | `<data>/.tmp` | local spill directory |
| `QUACKGIS_DUCKDB_MAX_TEMP_DIRECTORY_BYTES` | max(10 GiB, 4 × DuckDB memory limit) | spill ceiling |
| `QUACKGIS_CATALOG_PATH` | `quackgis.ducklake` | official local DuckLake catalog |
| `QUACKGIS_DUCKLAKE_CATALOG_NAME` | `quackgis` | attached catalog name |
| `QUACKGIS_DATA_PATH` | `./data` | local Parquet root |
| `QUACKGIS_AUTH_MODE` | `trust` | `trust` for development or `password` for SCRAM |
| `QUACKGIS_READWRITE_USER` | `postgres` | write-capable login |
| `QUACKGIS_READWRITE_PASSWORD` | unset | required in password mode |
| `QUACKGIS_READONLY_USER` / `QUACKGIS_READONLY_PASSWORD` | `quackgis_readonly` / unset | optional read-only login |
| `QUACKGIS_WRITE_ALLOWLIST` / `QUACKGIS_READ_ALLOWLIST` | unset | comma-separated normalized table policy |
| `QUACKGIS_MAINTENANCE_USER` | unset | existing read/write identity allowed to invoke bounded maintenance; disabled when unset |
| `QUACKGIS_TLS_CERT` / `QUACKGIS_TLS_KEY` | unset | optional PEM certificate and PKCS#8 key; configure together |
| `QUACKGIS_TLS_MODE` | `preferred` | `preferred` permits plaintext; `required` needs both TLS paths and rejects plaintext startup |
| `QUACKGIS_METRICS_HOST` / `QUACKGIS_METRICS_PORT` | `127.0.0.1` / unset | optional `/healthz`, `/readyz`, and `/metrics` HTTP listener |
| `QUACKGIS_LOG` | `info` | log filter |

`QUACKGIS_CATALOG_URL` and remote/object-store data paths are reserved and fail
closed. S3 credentials and the retired engine selector are not runtime options.
The unsigned DuckLake override is opt-in native-code execution and is rejected
unless both values pass strict path and digest validation. Default startup still
loads only signed extensions. See [DEVELOPMENT_DUCKLAKE.md](DEVELOPMENT_DUCKLAKE.md)
for its isolated development workflow and deletion plan.

The optional `quackgis-rest` process has a separate configuration and failure
domain. It requires an authenticator `QUACKGIS_REST_DATABASE_URL`,
owner-only `QUACKGIS_REST_DATABASE_PASSWORD_FILE`,
`QUACKGIS_REST_JWT_SECRET_FILE`, exact `QUACKGIS_REST_JWT_ISSUER` and
`QUACKGIS_REST_JWT_AUDIENCE`, comma-separated `QUACKGIS_REST_JWT_ROLES`, and the
explicit `QUACKGIS_REST_TABLES` ceiling. `QUACKGIS_REST_DATABASE_CA` enables
hostname-verified pgwire TLS; `QUACKGIS_REST_HOST`, `QUACKGIS_REST_PORT`, and
`QUACKGIS_REST_STATEMENT_TIMEOUT_MS` default to `127.0.0.1`, `3000`, and `30000`.
The database URL must not contain a password. The authenticator must have
configured set-option membership in every listed API role. See
[REST_API.md](REST_API.md) for the read-only contract, credential-rotation order,
and load-balancer trust boundary.

The pgwire socket is bound and the configured local DuckLake read/write-capacity
probe passes before `/readyz` can return `200 ready`. The probe reads the snapshot
surface, creates/syncs/removes one 4 KiB file under the claimed local data root,
and executes unique internal DuckLake DDL in an ADBC transaction that must roll
back. Native evidence proves it leaves no table, local probe file, or new DuckLake
snapshot. The endpoint returns `503 starting`, `503 storage_unavailable`, or `503
draining` for those explicit states; `/healthz` reports process liveness only.
When enabled, an independent ADBC session repeats the probe and samples aggregate
`duckdb_memory()` memory and temporary-storage gauges every 15 seconds, stopping
when drain begins. This is a non-publishing local-singleton capacity signal, not a
remote catalog/object-store latency SLO. Scrapes read atomics and never execute
native SQL. Keep the listener on loopback or an authenticated network boundary
because `/metrics` is served on the same socket.

By default QuackGIS detects the lower of host RAM and the active cgroup memory
limit, assigns 60% to DuckDB, and leaves 40% for Arrow, pgwire, other process
memory, and the OS. CPU threads are similarly bounded by process parallelism and
the cgroup CPU quota. Eligible DuckDB operators spill to `<data>/.tmp`; the spill
ceiling is the larger of 10 GiB or four times the DuckDB memory budget. Startup
creates the directory and applies these settings before attaching DuckLake.
Explicit environment or CLI values override autosizing. Keep the spill directory
on fast local storage with enough free space, and override
`QUACKGIS_DUCKDB_TEMP_DIRECTORY` when the DuckLake volume is not appropriate
scratch storage.

The native pgwire workflow opens 32 independent clients against the default
eight-reader ceiling. Suspended portals retain their native streams and admission
permits; the gate observes four waves of eight and proves that a ninth reader does
not enter before a permit is released.

COPY has no total request ceiling. Each client `CopyData` message must fit
`QUACKGIS_COPY_BATCH_BYTES`; normal PostgreSQL clients already send smaller
chunks. Startup also requires the row limit to bound one chunk to at most 128
Arrow batches, preventing pathological valid configurations from defeating
channel backpressure. `QUACKGIS_PGWIRE_MAX_FRAME_BYTES` is an earlier, per-message
wire trust boundary: the pgwire codec rejects the declared length after the
five-byte typed header (or four-byte startup header) and before reading or decoding
the body. It must be at least `QUACKGIS_COPY_BATCH_BYTES + 4`. The default 16 MiB
permits the default 8 MiB COPY chunk while bounding every frontend message; it is
not a total COPY-size limit. The native raw-wire regression sends no oversized
body, observes immediate closure, and verifies zero publication.

The blocking-worker limit must be greater than the active-query limit. Regular
ADBC work can use at most `max_blocking_workers - 1`; the final slot is reserved
for native cancellation and control cleanup. Both client cancellation and
statement-deadline cancellation use that reserved blocking slot rather than a
Tokio executor thread. Worker active/queued/high-water and regular/control gauges
are exported when metrics are enabled.

Reader, writer, and maintenance limits must be positive and no larger than the
global active-query limit. Admission acquires both a class permit and a global
permit under the same bounded queue deadline. Metrics expose active, queued, and
high-water values by class. A deterministic 32-contender unit gate proves that an
eight-operation global limit never admits nine. The native pgwire workflow above
proves the same ceiling for readers. `just duckdb-mixed-concurrency-profile`
additionally saturates a three-operation global limit with two suspended reader
portals and one open COPY, observes reader, writer, and maintenance work queued,
then proves every class completes without rejection or timeout. This is bounded
admission evidence, not the Local 1.0 mixed-workload soak.

Adjacent-file compaction is the only server-exposed maintenance operation. It is
available through simple protocol only, requires the explicitly configured
maintenance identity and the ordinary write allowlist, accepts five positional
literals, and is rejected inside an explicit transaction:

```sql
CALL quackgis_merge_adjacent_files(
  'public', 'fragmented_copy', 8, 16777216, NULL
);
```

The arguments are schema, table, maximum compacted files, maximum file bytes, and
minimum file bytes. The final three accept a positive integer or `NULL`. The
schema is limited to `public`/`main`; arbitrary `CALL`, named/dynamic arguments,
and extended-protocol invocation fail closed. The operation uses maintenance-class
admission, the fixed native worker pool, typed DuckLake SQL construction, and a
redacted success/failure audit event.

## Storage authority

Startup atomically creates `_quackgis/storage-authority-v1` below the local data
root. A mismatched marker fails before DuckLake attach. Migration must target a
separate root; never copy a retired writer's authority marker.

## Security baseline

- Trust mode is development-only.
- Password mode uses SCRAM-SHA-256.
- TLS configuration fails startup if only one path is supplied or material is
  malformed. Set `QUACKGIS_TLS_MODE=required` with both paths to reject plaintext
  startup; the default `preferred` mode preserves the local development behavior.
- Read/write allowlists are enforced against parsed statements before ADBC
  prepare or schema lookup.
- The native driver path is an operator-controlled code-loading trust boundary and
  is verified against the committed SHA-256 before opening storage.

Production-style local deployments should use all three TLS settings together:

```sh
QUACKGIS_TLS_MODE=required \
QUACKGIS_TLS_CERT=/run/secrets/quackgis.crt \
QUACKGIS_TLS_KEY=/run/secrets/quackgis.key \
mise exec -- just server
```

`just duckdb-tls-rotation-profile` starts the actual binary twice and proves
client-side certificate/hostname verification, SCRAM, plaintext denial, wrong-CA
denial, committed-state preservation, old-certificate-trust rejection, old-password
rejection, and a post-rotation write. The current server loads TLS and password
material only at process startup; it does not hot reload files or environment.

Rotate local credentials as one restart operation:

1. stage a replacement certificate/key and password without modifying the live
   files;
2. stop the singleton through its normal drain path;
3. atomically replace the certificate/key and password source;
4. restart, wait for `/readyz`, and verify the new trust/password; then
5. verify both the old certificate trust and old password are rejected.

The K0 Kind topology uses mutual TLS at the tiny-client Service and separate iroh
keys for bootstrap, worker, credential, and client transport. `just
kind-secret-rotation-gate` stages a new development CA/certificate set and new
edge keys, renders a content hash into the Pod template, performs ordered
replacement, rejects the old client certificate, and reruns every packaged client
and denial Job. A failed rollout retains previous owner-only material under
`.tmp/kind/` for explicit recovery. This is a completed local package rotation
drill, not JWT/authenticator/database-password rotation, online revocation, or a
production PKI procedure.

The direct REST preview independently hot-reloads its HS256 signing key. Stage a
valid 32–4096-byte secret beside `QUACKGIS_REST_JWT_SECRET_FILE`, protect it from
other users, and atomically replace the configured path as shown in
[`REST_API.md`](./REST_API.md). `/ready` validates the current file; after
replacement, new-key tokens succeed and old-key tokens fail with no overlap
window. This does not rotate the authenticator's pgwire password or package the
REST process in Kind.

## Shutdown, backup, and recovery

SIGINT/SIGTERM marks readiness as draining, stops accepting new pgwire sockets,
rejects new explicit transactions on existing sockets, and lets established
connections finish for `QUACKGIS_SHUTDOWN_TIMEOUT_MS`. Active transaction count is
exported as `quackgis_transactions_active`. At the deadline, remaining connection
tasks are aborted; their active transaction sessions roll back on drop where the
native connection remains usable. `just duckdb-termination-profile` exercises the
actual process with an uncommitted row at the forced deadline, restarts the same
local DuckLake paths, proves the row is absent, and proves a new write succeeds.
Its clean smoke run on source `59c1a381` becomes queryable in 135 ms against the
60-second budget. This is
explicit-transaction atomicity evidence, not yet proof that every native write or
commit can be interrupted.

Within an explicit transaction, an ordinary simple/extended statement error moves
pgwire to the failed transaction state. Subsequent simple or extended work returns
stable `25P02` until `ROLLBACK`; `COMMIT` performs rollback and ends the block
without publishing prior writes. The maintained workflow proves the session is
reusable and an independent observer sees no rows from the failed transaction.
COPY's dependency-owned ReadyForQuery edge, general in-flight write/commit
COPY's dependency-owned ReadyForQuery edge remains documented. Ordinary writes
run behind a cancellable transaction boundary. Cancellation before commit returns
`57014`; an autocommit write is rolled back and its session remains reusable,
while an explicit transaction is rolled back and quarantined. The cancellation
handle closes before commit starts. Commit is intentionally non-cancellable
because interruption can make durable outcome indeterminate; commit failure is
reported as indeterminate and the native session is never reused. Response-loss
reconciliation remains an operations gate.
Back up the official DuckLake catalog, data root, and authority marker together.
The maintained local procedure is offline and exact-path only:

1. stop QuackGIS and confirm the process has exited;
2. create a checksum manifest and copy all durable files, excluding the spill
   directory:

   ```sh
   mise exec -- just duckdb-local-backup \
     catalog=.tmp/duckdb-server/catalog.ducklake \
     data=.tmp/duckdb-server/data \
     out=/path/on-independent-storage/quackgis-backup
   ```

3. retain the entire backup directory without adding files; and
4. after removing or moving both failed original paths, restore to those exact
   paths:

   ```sh
   mise exec -- just duckdb-local-restore \
     backup=/path/on-independent-storage/quackgis-backup \
     catalog=.tmp/duckdb-server/catalog.ducklake \
     data=.tmp/duckdb-server/data
   ```

Backup requires the authority marker, rejects symlinks and source changes detected
during copying, and publishes through a staging directory. Restore verifies the
complete file set and every SHA-256 before creating targets, refuses existing or
relocated targets, publishes the catalog last, and removes partial output on
failure. `just duckdb-adbc-storage-test` deletes the originals, restores, reopens,
and verifies the exact latest snapshot ID and table count/sum. This is functional
offline recovery evidence, not an online snapshot, point-in-time, relocated,
shared-storage, rolling-upgrade, or automated disaster-recovery claim.

The maintained pgwire workflow creates eight deliberately fragmented COPY files,
runs official `ducklake_merge_adjacent_files`, requires the active file count to
at least halve, and verifies unchanged row count/sum afterward. This is functional
compaction evidence, not yet a production scheduling or capacity policy.

Tables may opt into bbox maintenance by declaring all four reserved nullable
`DOUBLE` columns `_qg_minx`, `_qg_miny`, `_qg_maxx`, and `_qg_maxy` and exactly one
recognized binary geometry column. DuckDB Spatial computes the values during
atomic publication. COPY must omit the reserved bbox columns; partial, wrong-type,
caller-supplied, or ambiguous layouts fail with `0A000` before staging and leave
the session reusable. If the nullable geometry is omitted, all four bounds are
published as NULL. Direct INSERT and reserved-column UPDATEs fail closed. A
geometry UPDATE atomically refreshes all bounds only when the geometry value is a
numbered bound parameter (optionally cast) or `NULL`; arbitrary expressions and
tuple assignment fail closed. Ordinary-column UPDATEs are allowed because they
leave maintained geometry and bounds unchanged. Actual pgwire tests cover point,
linestring, and polygon updates plus NULL, malformed, rollback, and point-reopen
behavior.

One-table `ST_Intersects` reads over a valid maintained layout automatically gain
four planner-visible bbox overlap candidates when the data geometry is the
maintained WKB column and the other geometry is a bounded literal envelope/text
value or numbered-bound WKB. The original exact predicate remains in the plan.
The native gate checks holes, an outer boundary, NULL and empty data, an empty
probe, invalid bow-tie data/probe, literal and bound probes, reopen, and `EXPLAIN`.
Every edge case compares the injected result to a deliberately unoptimized exact
oracle; the pgwire workflow executes the literal form without client-written bbox
SQL. OR/NOT placement, joins, subqueries, multiple matching predicates, and
arbitrary or oversized geometry expressions are not optimized; malformed or
ambiguous reserved layouts fail closed. The registered profile records row groups,
conservative compressed-byte bounds, five pgwire latency samples per layout,
process RSS, sampled DuckDB memory, and temporary storage. The v5 profile adds
grouped aggregates, bounded spatial joins, and nine-column wide projections to
the selective scan and compaction oracle. Two clean 10M runs precede two 100M
runs on source `8490ed7`; all results and plans survive compaction, all committed
load/first-row/p50/p95/p99/RSS/DuckDB-memory/zero-spill/scan-byte/file/row-group
budgets pass, and 25 files reduce to 7 bbox/4 native files at 100M. Local 1.0
retains maintained WKB/bbox storage, a 1 GiB maximum compaction file, and DuckDB's
default row-group sizing. Automatic DDL and arbitrary geometry-expression
maintenance remain unsupported by design rather than open M4 work.

## Maintained checks

```sh
mise exec -- just check-fast
mise exec -- just duckdb-adbc-storage-test
mise exec -- just duckdb-pgwire-workflow-test
mise exec -- just duckdb-runtime-static-check
```

The release/runtime image must package the exact verified `libduckdb.so`, signed
`spatial` and `ducklake` extensions, and isolated DuckDB home. Bare Rust binaries
without those artifacts are not runnable server distributions.

The preview image binds pgwire to container loopback by default and does not
publish a port. It is a verification artifact, not a production deployment.
Override the bind address only together with SCRAM credentials and an enforced
TLS/network boundary.

CI uploads manifests and license/provenance evidence, not the native runtime
binary context. Redistribution remains blocked until Local 1.0 closes Rust and
Spatial transitive-license/source obligations for the exact bundle.
