# Roadmap status

This is the current evidence floor for the DuckDB-only runtime. It intentionally
does not inherit claims from retired DataFusion/Sedona or unsupported shared
deployment profiles.

## Current verified floor

| Area | Evidence | Current boundary |
|---|---|---|
| Engine/storage | `just duckdb-adbc-storage-test` | pinned DuckDB 1.5.4, official local DuckLake, Arrow ingest/query, transaction, snapshot inspection, adjacent-file merge, reopen, checksummed offline exact-path backup/restore |
| Pgwire workflow | `just duckdb-pgwire-workflow-test` | structural statements/parameters, incremental >20 MiB/220k-row COPY with atomic abort paths, scalar/NULL/WKB reopen, fragmented-load compaction, independent sessions, failed-transaction rollback/reuse, streaming results/portals, restart |
| Auth/policy | real CLI SCRAM and table allowlist cases in pgwire workflow | trust or SCRAM; normalized read/write table policy before ADBC prepare |
| Spatial | pgwire workflow + `tests/duckdb_spatial_compat.json` | 42 original PostGIS expressions: 31 native, 5 rewrites, 6 macros |
| Spatial gaps | `docs/DUCKDB_SPATIAL_GAP_LEDGER.md` | 10 Rust/catalog-edge gaps and 5 extension candidates remain unsupported |
| WKB/Arrow | storage and pgwire native tests; generated `vendor/arrow-pg` properties | maintained WKB bytes, `geom_wkb` geometry identity, scalar/fixed-binary/null encoding, and fail-closed unsupported list/invalid JSON handling; broad geometry discovery and the complete advertised-type property matrix remain open |
| Runtime supply chain | `just duckdb-runtime-offline-smoke` | verified context digests/licenses, preinstalled signed extensions, load-only image and server-start smoke |
| Current performance smoke | `just duckdb-current-benchmark` | deterministic 100k-row direct DuckDB/ADBC/pgwire scalar full-scan comparison; correctness and broad liveness budgets only |
| Storage authority | storage unit/native tests | atomic local authority marker; remote authority unsupported |
| Status/readiness | lifecycle unit/native storage tests | liveness is process-only; readiness requires a bound pgwire socket and a successful read-only DuckLake snapshot probe, and reports startup/storage-failure/drain states |
| Repository gate | `just ci` | Rust fmt/clippy/tests, native storage/pgwire, probes, runtime static checks |

## Important implementation limits

- Query results stream one driver-produced Arrow batch at a time and fail closed
  before pgwire encoding when the configured byte ceiling is exceeded. Only
  native EOF permits connection reuse; partial delivery, failure, and cancellation
  quarantine uncertain stream state. The native pgwire cancellation regression
  proves the cancelled client receives a stable quarantine error while an
  independent session remains usable. Native allocation and 1M/10M RSS evidence
  remains open.
- COPY incrementally decodes bounded Arrow batches into one staging ADBC stream;
  1 GiB/RSS/throughput evidence and a configurable pre-decode pgwire frame bound
  are still open. The current chunk ceiling applies after pgwire decoding; native
  regressions prove oversized decoded chunks and malformed final rows synchronously
  clean up staging with zero visible rows.
- Native cancellation/deadlines abort active query and COPY workers. Pgwire 0.40
  cannot asynchronously deliver a COPY error while the client sends no frames;
  general write/commit cancellation also remains open. Deadline-triggered native
  cancellation uses the reserved control worker rather than a Tokio executor.
- Connection, queue, global active-query, reader/writer/maintenance-class, fixed
  blocking-worker, and DuckDB memory/thread/temp/spill controls are productized.
  Aggregate DuckDB memory and temporary storage are sampled on an independent
  session when metrics are enabled. A 32-contender unit gate and a 32-client
  suspended-portal native workload both prove the eight-operation admission
  ceiling.
- Supported statement and parameter type surfaces are intentionally narrow.
- Broad `pg_catalog`/`information_schema`, geometry/geography OID discovery, SRID,
  dimension, geography, extent, MVT, and general `ST_GeometryN` behavior remain
  incomplete.
- Named QGIS, GDAL/OGR, GeoServer, Martin, psycopg, ORM, and BI workflows have not
  been requalified against the DuckDB-only server.
- Remote/shared catalog and object-storage paths fail closed.
- COPY validates and maintains the explicit four-column bbox layout in DuckDB SQL;
  partial/wrong-type/caller-supplied/ambiguous layouts fail before staging, and a
  pgwire rejection/reuse plus exact/reopen oracle passes. Automatic DDL, mutation
  and compaction refresh, predicate injection, and scale evidence remain open.
  Direct `INSERT`/`UPDATE` on those layouts now fails closed so it cannot create
  stale or forged bounds before mutation maintenance exists.
- Online/relocated production backup/restore, rolling upgrade, soak, and disaster
  recovery remain unproven.

## Milestone status

| Milestone | State | Next closure work |
|---|---|---|
| M0 truthful repository | complete | `just project-contract-check` validates links/recipes/spatial counts; required CI publishes the deterministic transport-smoke manifest |
| M1 bounded execution | active; implementation majority complete | ADBC streams retain ownership; pgwire pulls one batch under a fail-closed byte ceiling; only native EOF permits reuse; native cancel/deadlines use reserved control capacity; cancelled and partial streams explicitly quarantine the same client while independent sessions remain usable; failed-transaction rollback/reuse, global plus reader/writer/maintenance-class admission, an authorized maintenance path, autosized DuckDB resources, sampled memory/temporary storage, class metrics, and unit plus 32-client native eight-admission proofs are implemented. Remaining: write/commit cancellation, 1M/10M RSS, 100-cancel p95, mixed-class native concurrency, and overhead budget. |
| M2 streaming ingest | active; implementation majority complete | incremental bounded parser, exact Arrow batch byte splitting, staging ADBC stream, atomic publication, text escapes, >20 MiB/220k-row regression, synchronous malformed-final-row and oversized-decoded-chunk cleanup, abort zero-row tests, scalar/NULL/WKB reopen, compaction, and metrics are implemented. Remaining: pre-decode pgwire frame ceiling, idle-wait error delivery, and reference-profile 10M/1 GiB RSS/throughput evidence. |
| M3 focused compatibility | active foundation | maintained SET/SHOW, AST `public` mapping, parsed quoted COPY targets, `geom_wkb` sentinel identity, initial generated WKB/fixed-binary encoder properties, and ledger-pinned `0A000` errors for all five extension-candidate spatial cases are verified through simple/extended pgwire with session reuse. Remaining: pinned named-client traces, DuckDB-derived catalog fixtures/OIDs, subtype/SRID/dimension metadata, Rust-edge spatial gaps, and the complete advertised-type property matrix. |
| M4 analytical performance | active foundation | fail-closed COPY bbox layout validation/maintenance, direct `INSERT`/`UPDATE` bypass rejection, pgwire rejection/reuse, and exact/reopen evidence are implemented; ordinary compaction already halves fragmented file count without scalar result changes. Remaining: schema-aware mutation/spatial-compaction refresh, safe AST predicate injection, conservative geometry matrix, scan-byte plans, and current 10M then 100M profiles. |
| M5 Local 1.0 | active foundation | immutable load-only runtime smoke, opt-in process liveness and pgwire-bind/read-only DuckLake readiness with explicit startup/storage-failure/drain states, configured connection/transaction drain, authorized/audited adjacent-file compaction, and checksummed offline exact-path backup/restore with snapshot/count evidence exist. Remaining: write-capacity readiness SLO, published artifacts, online/relocated production recovery, upgrades, TLS rotation, write/commit interruption evidence, mixed workload, and 24-hour soak. |
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
