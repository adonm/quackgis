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
| E0 evidence harness | active | common evidence envelope/host fingerprint, then split gate-oriented native scenarios |
| E1 M1/M2 local profiles | queued behind E0 | reduced local result/cancel/COPY profiles using the same reference oracle |
| K0 minimal Kind topology | queued | one runtime workload, durable local volume, TLS secret, pinned client job image |
| C0 focused clients | queued behind K0/catalog fixtures | psql and psycopg first, then OGR and headless QGIS |
| P0 M4 host profiles | queued behind E0/layout implementation | 10M twice before 100M |
| K1 operations/shared rehearsal | deferred by dependency | Local 1.0 operations first; PostgreSQL/MinIO is rehearsal, not managed evidence |

## Current verified floor

| Area | Evidence | Current boundary |
|---|---|---|
| Engine/storage | `just duckdb-adbc-storage-test` | pinned DuckDB 1.5.4, official local DuckLake, Arrow ingest/query, transaction, snapshot inspection, adjacent-file merge, reopen, checksummed offline exact-path backup/restore |
| Pgwire workflow | `just duckdb-pgwire-workflow-test` | structural statements/parameters, incremental >20 MiB/220k-row COPY with atomic abort paths, scalar/NULL/WKB reopen, geometry `pg_type`/RowDescription/text/binary/NULL identity, 15 stable spatial-gap dispositions, fragmented-load compaction, independent sessions, failed-transaction rollback/reuse, streaming results/portals, restart |
| Auth/policy | real CLI SCRAM and table allowlist cases in pgwire workflow | trust or SCRAM; normalized read/write table policy before ADBC prepare |
| Spatial | pgwire workflow + `tests/duckdb_spatial_compat.json` | 42 original PostGIS expressions: 31 native, 5 rewrites, 6 macros |
| Spatial gaps | `docs/DUCKDB_SPATIAL_GAP_LEDGER.md` | 10 Rust/catalog-edge gaps and 5 extension candidates have ledger-pinned `0A000` simple/extended pgwire behavior; semantics remain unsupported |
| WKB/Arrow | storage/pgwire native tests + `just arrow-encoder-test` | maintained WKB bytes, structural geometry sentinel `pg_type` lookup, RowDescription/text/binary/NULL identity, generated WKB/fixed-binary properties, scalar/list parity, fail-closed invalid shapes, and panic-free nested errors; broad client discovery and generated temporal/decimal/dictionary/nested coverage remain open |
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
- Broad `pg_catalog`/`information_schema`, geography discovery, SRID, dimension,
  geography, extent, MVT, and general `ST_GeometryN` behavior remain incomplete.
  A narrow structural `pg_type` adapter resolves only the maintained geometry and
  geography sentinel OIDs; geometry RowDescription/text/binary/NULL behavior is
  proven through the current Rust pgwire client.
- Named QGIS, GDAL/OGR, GeoServer, Martin, psycopg, ORM, and BI workflows have not
  been requalified against the DuckDB-only server.
- Remote/shared catalog and object-storage paths fail closed.
- COPY validates and maintains the explicit four-column bbox layout in DuckDB SQL;
  partial/wrong-type/caller-supplied/ambiguous layouts fail before staging, and a
  pgwire rejection/reuse plus exact/reopen oracle passes. Automatic DDL, mutation
  and compaction refresh, predicate injection, and scale evidence remain open.
  Direct INSERT plus geometry/reserved-column UPDATEs fail closed so they cannot
  create stale or forged bounds. Bound UPDATEs touching only ordinary columns are
  permitted and have pgwire/reopen evidence that geometry/bounds remain unchanged.
- Online/relocated production backup/restore, rolling upgrade, soak, and disaster
  recovery remain unproven.

## Milestone status

| Milestone | State | Next closure work |
|---|---|---|
| M0 truthful repository | complete | `just project-contract-check` validates links/recipes/spatial counts; any future CI must invoke maintained Justfile gates rather than duplicate them |
| M1 bounded execution | active; implementation majority complete | ADBC streams retain ownership; pgwire pulls one batch under a fail-closed byte ceiling; only native EOF permits reuse; native cancel/deadlines use reserved control capacity; cancelled and partial streams explicitly quarantine the same client while independent sessions remain usable; failed-transaction rollback/reuse, global plus reader/writer/maintenance-class admission, an authorized maintenance path, autosized DuckDB resources, sampled memory/temporary storage, class metrics, and unit plus 32-client native eight-admission proofs are implemented. E0/E1 now own the local/reference evidence path. Remaining: write/commit cancellation, 1M/10M RSS, 100-cancel p95, mixed-class native concurrency, and overhead budget. |
| M2 streaming ingest | active; implementation majority complete | incremental bounded parser, exact Arrow batch byte splitting, staging ADBC stream, atomic publication, text escapes, >20 MiB/220k-row regression, synchronous malformed-final-row and oversized-decoded-chunk cleanup, abort zero-row tests, scalar/NULL/WKB reopen, compaction, and metrics are implemented. E1 will run one oracle at reduced local and exact 10M/1 GiB reference scales. Remaining: pre-decode pgwire frame ceiling, idle-wait error delivery, and RSS/throughput evidence. |
| M3 focused compatibility | active foundation | maintained SET/SHOW, AST `public` mapping, parsed quoted COPY targets, structural sentinel `pg_type` resolution, geometry RowDescription/text/binary/NULL identity, focused encoder parity, panic-free nested errors, and ledger-pinned `0A000` simple/extended behavior for all 15 non-executable spatial cases are verified. Remaining: pinned named-client traces, DuckDB-derived broad catalog fixtures, subtype/SRID/dimension metadata, implementing release-required Rust-edge semantics, geography evidence, and broader generated encoder coverage. |
| M4 analytical performance | active foundation | fail-closed COPY bbox maintenance, direct INSERT and geometry/reserved UPDATE rejection, safe ordinary-column bound UPDATEs with unchanged bbox/WKB reopen evidence, and ordinary file compaction are implemented. Remaining: geometry mutation/spatial-compaction refresh, safe AST predicate injection, conservative geometry matrix, scan-byte plans, and current 10M then 100M profiles. |
| M5 Local 1.0 | active foundation | immutable load-only runtime smoke, opt-in process liveness and pgwire-bind/read-only DuckLake readiness, configured drain, authorized/audited compaction, checksummed offline exact-path backup/restore, and an explicit TLS-required startup policy exist. K0 will provide the minimal packaged topology; host profiles remain authoritative for performance. Remaining: encrypted-client/plaintext-denial and rotation evidence, write-capacity readiness SLO, published artifacts, online/relocated production recovery, upgrades, write/commit interruption evidence, mixed workload, and 24-hour soak. |
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
