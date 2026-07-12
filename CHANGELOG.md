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
- Opt-in maintenance identity and a literal-only server-owned adjacent-file
  compaction call with write-policy enforcement, maintenance admission, audit
  events, transaction rejection, and pgwire/reopen evidence.
- Opt-in `/healthz` and startup/drain-aware `/readyz` responses beside `/metrics`,
  plus active-transaction and explicit session-quarantine metrics.
- Bounded SIGINT/SIGTERM connection drain that stops acceptance, rejects new
  transactions, permits established work to finish, and aborts at a configured
  deadline.
- Checksummed offline local backup/restore with symlink/source-change rejection,
  exact-path enforcement, staged publication, and native snapshot/count recovery
  evidence.
- Periodic aggregate DuckDB tracked-memory and temporary-storage samples with
  current/high-water gauges and sampler health counters.
- Structural compatibility for maintained PostgreSQL session settings,
  `SHOW search_path`, `public`→`quackgis.main` relation mapping, and quoted
  one-/two-/three-part COPY targets.
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
- Fail-closed `0A000` rejection for direct INSERT and geometry/reserved-column
  UPDATEs against maintained bbox layouts, preventing stale or caller-forged
  bounds until schema-aware DML recomputation is available. Ordinary-column bound
  UPDATEs are permitted with unchanged WKB/bbox reopen evidence.
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
- Narrow structural `pg_type` resolution for the maintained geometry/geography
  sentinel OIDs, with geometry RowDescription, binary WKB, text hex-WKB, and NULL
  evidence through the native pgwire client.
- Ledger-pinned `0A000` simple/extended protocol behavior and session-reuse
  evidence for all 10 deferred Rust-edge and five extension-candidate spatial gaps.
- Explicit `preferred`/`required` TLS policy: required mode needs paired valid
  material and rejects insecure startup before trust or SCRAM authentication.

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
