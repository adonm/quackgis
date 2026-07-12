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
| `QUACKGIS_HOST` / `QUACKGIS_PORT` | `127.0.0.1` / `5434` | pgwire bind |
| `QUACKGIS_MAX_CONNECTIONS` | `64` | accepted connection bound |
| `QUACKGIS_MAX_ACTIVE_QUERIES` / `QUACKGIS_MAX_QUEUED_QUERIES` | `8` / `64` | execution and admission queue bounds |
| `QUACKGIS_MAX_READER_QUERIES` / `QUACKGIS_MAX_WRITER_QUERIES` | `8` / `2` | reader and writer ceilings within the global active bound |
| `QUACKGIS_MAX_MAINTENANCE_QUERIES` | `1` | server-owned maintenance concurrency ceiling |
| `QUACKGIS_MAX_BLOCKING_WORKERS` | `9` | native worker ceiling; one slot is reserved for cancellation/control |
| `QUACKGIS_QUEUE_TIMEOUT_MS` / `QUACKGIS_STATEMENT_TIMEOUT_MS` | `30000` / `300000` | queue and active query deadlines |
| `QUACKGIS_SHUTDOWN_TIMEOUT_MS` | `30000` | graceful connection/transaction drain deadline |
| `QUACKGIS_RESULT_BATCH_BYTES` | `8388608` | fail-closed Arrow result batch ceiling before pgwire encoding |
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

The pgwire socket is bound and the configured local DuckLake snapshot surface is
queried before `/readyz` can return `200 ready`. The endpoint returns `503
starting`, `503 storage_unavailable`, or `503 draining` for those explicit states;
`/healthz` reports process liveness only. When enabled, an independent ADBC
session repeats the read-only DuckLake probe and samples aggregate
`duckdb_memory()` memory and temporary-storage gauges every 15 seconds. This
proves local catalog readability, not future write capacity. Scrapes read atomics
and never execute native SQL. Keep the listener on loopback or an authenticated
network boundary because `/metrics` is served on the same socket.

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
channel backpressure. With pgwire 0.40, the dependency decodes a declared frame
before QuackGIS applies this limit; a configurable pre-allocation frame ceiling is
still required before this is a complete untrusted-wire memory bound.

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

Certificate/client verification and restart-based rotation evidence remain Local
1.0 gates; required mode alone is not that evidence.

## Shutdown, backup, and recovery

SIGINT/SIGTERM marks readiness as draining, stops accepting new pgwire sockets,
rejects new explicit transactions on existing sockets, and lets established
connections finish for `QUACKGIS_SHUTDOWN_TIMEOUT_MS`. Active transaction count is
exported as `quackgis_transactions_active`. At the deadline, remaining connection
tasks are aborted; their active transaction sessions roll back on drop where the
native connection remains usable. This is a bounded cooperative drain, not yet a
proof that every native write or commit can be interrupted.

Within an explicit transaction, an ordinary simple/extended statement error moves
pgwire to the failed transaction state. Subsequent simple or extended work returns
stable `25P02` until `ROLLBACK`; `COMMIT` performs rollback and ends the block
without publishing prior writes. The maintained workflow proves the session is
reusable and an independent observer sees no rows from the failed transaction.
COPY's dependency-owned ReadyForQuery edge, general in-flight write/commit
cancellation, and indeterminate acknowledgment classification remain open.
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

Tables may opt into COPY bbox maintenance by declaring all four reserved nullable
`DOUBLE` columns `_qg_minx`, `_qg_miny`, `_qg_maxx`, and `_qg_maxy` and exactly one
recognized binary geometry column. DuckDB Spatial computes the values during
atomic publication. COPY must omit the reserved bbox columns; partial, wrong-type,
caller-supplied, or ambiguous layouts fail with `0A000` before staging and leave
the session reusable. If the nullable geometry is omitted, all four bounds are
published as NULL. Direct INSERT and geometry/reserved-column UPDATEs fail closed;
ordinary-column UPDATEs are allowed because they leave maintained geometry and
bounds unchanged. Automatic DDL, geometry mutation maintenance, compaction
refresh, and predicate injection remain roadmap work.

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
