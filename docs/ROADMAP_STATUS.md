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
| E1 M1/M2 local profiles | active | clean 1M/10M BIGINT, 1M wide-result, and 100-cancel references pass; mixed reader/writer/maintenance admission passes its native smoke oracle; COPY passes reduced local but its first clean 10M attempt fails the 0.50 throughput-ratio budget at 0.200; next COPY optimization and transport overhead |
| K0 minimal Kind topology | active foundation | `deploy/kind/` now renders one TLS-required runtime StatefulSet, retained local PV/PVC, generated Secrets, probes, and opt-in psql/psycopg/OGR Jobs from digest-pinned images; next publish/build the client image and execute the real cluster gates |
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
| Current transport profiles | `just duckdb-current-benchmark`, `just duckdb-transport-profile`, and `just evidence-manifest-check` | one deterministic scalar full-scan scenario/oracle runs at smoke/local/reference row counts in the common envelope; current 1M local run passes, but this is not streaming-result, selective-scan, or committed reference evidence |
| Storage authority | storage unit/native tests | atomic local authority marker; remote authority unsupported |
| Status/readiness | lifecycle unit/native storage tests | liveness is process-only; readiness requires a bound pgwire socket and a successful read-only DuckLake snapshot probe, and reports startup/storage-failure/drain states |
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
  the first dirty-tree 1M local direct-ADBC/pgwire profile passes exact
  count/sum/WKB publication, 64 MiB RSS delta, and a 0.272 throughput ratio. The
  first clean 10M attempt fails the required 0.50 throughput-ratio budget at
  0.200; optimization, a passing reference, and a configurable pre-decode pgwire
  frame bound remain open. The current chunk ceiling applies after pgwire decoding; native
  regressions prove oversized decoded chunks and malformed final rows synchronously
  clean up staging with zero visible rows.
- Native cancellation/deadlines abort active query and COPY workers. Pgwire 0.40
  cannot asynchronously deliver a COPY error while the client sends no frames;
  general write/commit cancellation also remains open. Deadline-triggered native
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
| M0 truthful repository | complete | `just project-contract-check` validates links/recipes/spatial counts; required CI invokes maintained Justfile gates and publishes the deterministic transport-smoke manifest |
| M1 bounded execution | active; implementation majority complete | ADBC streams retain ownership; pgwire pulls one batch under a fail-closed byte ceiling; only native EOF permits reuse; native cancel/deadlines use reserved control capacity; cancelled and partial streams quarantine explicitly; failed-transaction rollback/reuse, classed admission, autosized resources, and sampled memory/temp storage are implemented. Clean 1M/10M generated-BIGINT references pass RSS/first-row/one-batch gates, a clean 100-cancel reference passes at 1.51 ms p95, and the clean 1M wide nullable VARCHAR/BLOB reference crosses 489 batches at 19.17 MiB RSS delta. A native mixed-class profile now proves simultaneous reader/writer/maintenance queueing and bounded completion. Remaining: write/commit cancellation and overhead budget. |
| M2 streaming ingest | active; implementation majority complete | incremental bounded parser, exact Arrow batch byte splitting, staging ADBC stream, atomic publication, text escapes, >20 MiB/220k-row regression, synchronous malformed-final-row and oversized-decoded-chunk cleanup, abort zero-row tests, scalar/NULL/WKB reopen, compaction, and metrics are implemented. The same direct-ADBC/pgwire generator and exact oracle pass at 1M local scale with 64 MiB RSS delta and 0.272 throughput ratio. The first clean 10M run measures 0.200 and fails the required 0.50 ratio. Remaining: COPY optimization and passing reference, pre-decode pgwire frame ceiling, and idle-wait error delivery. |
| M3 focused compatibility | active foundation | maintained SET/SHOW, AST `public` mapping, parsed quoted COPY targets, structural sentinel `pg_type` resolution, geometry RowDescription/text/binary/NULL identity, focused encoder parity, panic-free nested errors, and ledger-pinned `0A000` simple/extended behavior for all 15 non-executable spatial cases are verified. Remaining: pinned named-client traces, DuckDB-derived broad catalog fixtures, subtype/SRID/dimension metadata, implementing release-required Rust-edge semantics, geography evidence, and broader generated encoder coverage. |
| M4 analytical performance | active foundation | fail-closed COPY bbox maintenance, direct INSERT and geometry/reserved UPDATE rejection, safe ordinary-column bound UPDATEs with unchanged bbox/WKB reopen evidence, and ordinary file compaction are implemented. Remaining: geometry mutation/spatial-compaction refresh, safe AST predicate injection, conservative geometry matrix, scan-byte plans, and current 10M then 100M profiles. |
| M5 Local 1.0 | active foundation | immutable load-only runtime smoke, opt-in process liveness and pgwire-bind/read-only DuckLake readiness, configured drain, authorized/audited compaction, checksummed offline exact-path backup/restore, and an explicit TLS-required startup policy exist. The initial [minimal Kind topology](../deploy/kind/README.md) renders one TLS-required StatefulSet, retained local storage, generated Secrets, probes, and digest-pinned psql/psycopg/OGR Jobs; it has static evidence only until runnable client/runtime images and Kind are available. Host profiles remain authoritative for performance. Remaining: real cluster/client execution, encrypted-client/plaintext-denial and rotation evidence, write-capacity readiness SLO, published artifacts, online/relocated production recovery, upgrades, write/commit interruption evidence, mixed workload, and 24-hour soak. |
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
