# Changelog

QuackGIS has not published a stable release. Prototype-era details and commit
anchors live in [docs/HISTORY.md](./docs/HISTORY.md) and Git history.

## Unreleased — DuckDB-only local preview

### Added

- Local-first smoke/local/reference/external roadmap levels with explicit host
  performance, minimal-Kind topology, and managed-service claim boundaries.
- A common profile evidence envelope with source dirty hashes, checksum-only native
  provenance, host/cgroup fingerprint, data/oracle/measurement/budget sections,
  atomic publication, and an independent CI validator.
- A parameterized smoke/local/reference transport entrypoint that runs one
  deterministic DuckDB/ADBC/pgwire scenario and exact-result oracle from 100k
  through the configured row count; the first dirty-tree 1M local run passes.
- Warm, interleaved ADBC/pgwire transport sampling with an exact-result oracle and
  fail-closed reference enforcement of the one-second eligibility and 15% p50
  overhead limits.
- Scale-safe transport fixture names retain complete IDs above six digits; the
  exact byte-count oracle now has boundary coverage through 10M rows.
- A separately registered result-stream profile with two-millisecond process RSS
  sampling, time-to-first-row, throughput, row/sum oracle, and Arrow batch
  high-water evidence; its first 1M local run passes with one in-flight batch.
- Clean serial 1M and 10M generated-BIGINT reference runs on source `12817bcd`
  pass exact cardinality/sum, first-row-before-completion, one-batch high water,
  and the +128 MiB RSS budget with approximately 1.72/2.36 MiB RSS growth.
- A reusable fresh DuckLake/server/client profile fixture and cancellation profile
  covering request-to-`57014` latency, native counters, explicit same-session
  quarantine, and fresh-session reuse; the first 25-sample local run passes at
  1.41 ms p95.
- A clean 100-sample cancellation reference on source `8b0d1e46` passes the 500 ms
  M1 budget at 1.51 ms p95 with 100 completed native calls, zero failures, and
  deterministic quarantine/fresh-session behavior.
- Parameterized nullable VARCHAR/BLOB result profiling with every-row value/NULL
  checks, native batch counts, first-row timing, throughput, and RSS; its first
  dirty-tree 100k local run crosses 49 batches at 9 MiB RSS delta.
- The clean 1M wide-result reference on source `b240507e` checks every value over
  489 native batches with one in flight, zero rejections, and 19.17 MiB RSS delta.
- Parameterized direct streaming ADBC versus bounded pgwire COPY profiling with
  exact count/sum/WKB publication, RSS, rows/bytes/batches, commit timing, and
  throughput ratio; its first dirty-tree 1M local run passes at 64 MiB RSS delta
  and a 0.272 pgwire/direct ratio.
- The initial minimal DuckDB-only Kind topology: one TLS-required StatefulSet,
  retained node-local PV/PVC, generated TLS/auth Secrets, health probes, and
  opt-in psql/psycopg/OGR Jobs that reject mutable image references at render time.
- Owned Rust pgwire/TLS/SCRAM edge over DuckDB ADBC.
- Official local DuckLake create, Arrow ingest/query, transaction, snapshot
  inspection, adjacent-file merge, and reopen workflows.
- Structural single-statement admission and parsed read/write table policy.
- Parameterized reads/mutations, incremental bounded PostgreSQL text COPY with
  atomic publication, independent client sessions, transaction cleanup, and
  portal paging.
- Failed explicit transactions reject subsequent simple/extended work with
  `25P02`; `COMMIT` rolls back prior writes and returns the session to service.
- DuckDB Spatial execution with 42 curated original-PostGIS expressions routed
  through native functions or bounded server-owned rewrites/macros.
- Checksum/version validation for `libduckdb` and signed `spatial`/`ducklake`
  extensions, plus a load-only runtime image contract.
- DataFusion-free Arrow-to-pgwire encoder with maintained WKB sentinel identity.
- Generated Arrow encoder properties for geometry WKB payload/null identity and
  fixed-size binary values, plus fail-closed invalid JSON/unsupported list shapes
  and null-safe interval encoding.
- Configured fail-closed Arrow result-batch ceiling with batch byte/in-flight
  metrics.
- Fixed native blocking-worker budget with a reserved cancellation/control slot
  and active/queued/high-water metrics.
- Global plus reader/writer/maintenance-class admission limits and per-class
  active/queued/high-water metrics, with a 32-contender ceiling regression.
- A registered mixed-class native profile that saturates admission with retained
  reader/COPY work, observes reader, writer, and maintenance queueing together,
  and completes without rejection or timeout.
- Opt-in maintenance identity and a literal-only server-owned adjacent-file
  compaction call with write-policy enforcement, maintenance admission, audit
  events, transaction rejection, and pgwire/reopen evidence.
- Opt-in `/healthz` and startup/drain-aware `/readyz` responses beside `/metrics`,
  plus active-transaction and explicit session-quarantine metrics.
- Bounded SIGINT/SIGTERM connection drain that stops acceptance, rejects new
  transactions, permits established work to finish, and aborts at a configured
  deadline.
- Actual-process forced-drain/restart profiling that proves an explicit
  uncommitted transaction publishes zero rows, committed state remains exact,
  restart meets the 60-second smoke budget, and post-restart writes succeed.
- Checksummed offline local backup/restore with symlink/source-change rejection,
  exact-path enforcement, staged publication, and native snapshot/count recovery
  evidence.
- Periodic aggregate DuckDB tracked-memory and temporary-storage samples with
  current/high-water gauges and sampler health counters.
- Structural compatibility for maintained PostgreSQL session settings,
  `SHOW search_path`, `public`→`quackgis.main` relation mapping, and quoted
  one-/two-/three-part COPY targets.
- A registered client-neutral catalog fixture for DuckDB-derived table/column
  metadata, exact-shape geometry/geography sentinel lookup, all seven returned
  type fields, and exact geometry/geography RowDescription/text/binary/NULL
  behavior.
- DuckDB-computed bbox maintenance during COPY for the explicit reserved-column
  layout contract, including NULL, exact-recheck, and reopen evidence.
- Fail-closed bbox layout validation rejects partial, wrong-type, caller-supplied,
  or ambiguous reserved columns before staging and keeps the pgwire session usable.
- COPY deadline evidence after flushed batches, with an explicit pre-publication
  cancellation check preventing aborted ADBC EOF normalization from publishing.
- A generated 220k-row, greater-than-20-MiB pgwire COPY regression using bounded
  60-KiB client chunks, proving the request is no longer capped at 16 MiB.
- COPY persistence checks for NULL/scalar/WKB values and official DuckLake
  adjacent-file compaction after fragmented loads.
- Stable `0A000` errors for the five maintained `ST_NDims`, `ST_CoordDim`, and
  `ST_GeometryN` extension-candidate cases, verified through simple and extended
  pgwire with session reuse.
- Fail-closed native query-stream cleanup: only observed EOF returns a connection;
  partial delivery, reader failure, or cancellation quarantines the session with
  an explicit engine error.
- Deep local readiness that binds pgwire and queries the DuckLake snapshot surface
  before reporting ready, with explicit starting, storage-unavailable, and draining
  states kept separate from process liveness.
- Schema-aware maintained-layout UPDATEs atomically recompute all four bbox
  columns when the geometry is assigned from one numbered bound parameter or
  `NULL`; malformed geometry rolls back, explicit rollback restores geometry and
  bounds together, and reopen preserves the refreshed values. Direct INSERT,
  arbitrary geometry expressions, tuple geometry assignment, and caller-written
  bbox values remain fail-closed; ordinary-column UPDATEs remain supported.
- A deterministic 32-client native pgwire admission regression: suspended portals
  retain eight reader permits and no ninth reader enters before release.
- Statement deadlines run native cancellation through reserved blocking-worker
  capacity, including a regression with all regular worker capacity occupied.
- Synchronous COPY cleanup for malformed final rows and oversized decoded chunks,
  with zero-row observer evidence and exact Arrow batch-memory splitting tests.
- Native pgwire evidence that a cancelled streaming client is explicitly
  quarantined while an independent session remains usable.
- Explicit CI execution of the vendored Arrow encoder suite; Float16 and UInt32
  OID scalar parity, Float16/fixed-binary list parity, fail-closed unsupported time
  units, and panic-free nested error propagation now have focused regressions.
- Narrow exact-AST-shape `pg_type` resolution for the maintained geometry/geography
  sentinel OIDs, with geometry/geography RowDescription, binary WKB, text hex-WKB,
  and NULL evidence through the native pgwire client. Near-miss projections, predicates,
  and constants remain native rather than being intercepted.
- Ledger-pinned `0A000` simple/extended protocol behavior and session-reuse
  evidence for all 10 deferred Rust-edge and five extension-candidate spatial gaps.
- Explicit `preferred`/`required` TLS policy: required mode needs paired valid
  material and rejects insecure startup before trust or SCRAM authentication.
- Actual-process TLS/SCRAM evidence with generated client trust: valid encrypted
  access succeeds, plaintext and an untrusted certificate fail, committed data
  survives restart-based certificate/password rotation, old trust and password
  fail afterward, and a post-rotation write succeeds.

### Changed

- DuckDB and official DuckLake are unconditional and are the sole engine/storage
  authority for new roots.
- Project direction now prioritizes bounded streaming, cancellation, admission,
  bulk ingest, focused named clients, and measured spatial performance before
  shared or broad compatibility claims.
- Missing capabilities follow the native DuckDB → SQL macro/rewrite → Rust edge →
  vectorized DuckDB extension decision ladder.
- CI bootstraps and verifies the pinned native runtime before native gates.
- Runtime artifacts package the complete DuckDB bundle rather than a bare server
  binary.

### Removed

- DataFusion, Sedona SQL execution, `datafusion-postgres`,
  `datafusion-pg-catalog`, and forked `datafusion-ducklake` runtime/vendor trees.
- Legacy engine modules, catalog hooks, native writer/mutation code, and associated
  tests.
- Optional backend selection and the `duckdb-adbc` feature gate.
- Unsupported shared PostgreSQL/S3 deployment automation and stale Kind/client
  probes.
- Nonfunctional examples, benchmark runners, migration plans, and scheduled jobs
  that depended on retired code or absent tests.

### Known limits

- Query and COPY streaming have strict scale/RSS/throughput evidence still open.
- Query cancellation/deadlines, classed admission/resource controls, sampled
  native memory/temporary storage, and a fixed blocking-worker budget are
  implemented; general write cancellation and native scale/concurrency evidence
  remain open.
- Local official DuckLake only.
- Incomplete catalogs, geometry identity, spatial gaps, and named GIS clients.
- No online/relocated production recovery, upgrade, soak, or shared-profile
  evidence.
