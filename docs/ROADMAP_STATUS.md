# Roadmap status

This is the current evidence floor for the DuckDB-only runtime. It intentionally
does not inherit claims from retired DataFusion/Sedona or unsupported shared
deployment profiles.

## Current verified floor

| Area | Evidence | Current boundary |
|---|---|---|
| Engine/storage | `just duckdb-adbc-storage-test` | pinned DuckDB 1.5.4, official local DuckLake, Arrow ingest/query, transaction, snapshot inspection, adjacent-file merge, reopen |
| Pgwire workflow | `just duckdb-pgwire-workflow-test` | structural SELECT/CREATE/INSERT/UPDATE/DELETE, parameters, bounded COPY, independent sessions, portals, restart |
| Auth/policy | real CLI SCRAM and table allowlist cases in pgwire workflow | trust or SCRAM; normalized read/write table policy before ADBC prepare |
| Spatial | pgwire workflow + `tests/duckdb_spatial_compat.json` | 42 original PostGIS expressions: 31 native, 5 rewrites, 6 macros |
| Spatial gaps | `docs/DUCKDB_SPATIAL_GAP_LEDGER.md` | 10 Rust/catalog-edge gaps and 5 extension candidates remain unsupported |
| WKB/Arrow | storage and pgwire native tests; `vendor/arrow-pg` tests | maintained WKB bytes and scalar Arrow encoding; broad geometry discovery remains open |
| Runtime supply chain | `just duckdb-runtime-offline-smoke` | verified context digests/licenses, preinstalled signed extensions, load-only image and server-start smoke |
| Current performance smoke | `just duckdb-current-benchmark` | deterministic 100k-row direct DuckDB/ADBC/pgwire scalar full-scan comparison; correctness and broad liveness budgets only |
| Storage authority | storage unit/native tests | atomic local authority marker; remote authority unsupported |
| Repository gate | `just ci` | Rust fmt/clippy/tests, native storage/pgwire, probes, runtime static checks |

## Important implementation limits

- Query results are materialized as complete Arrow batch vectors.
- COPY buffers the request and is limited to 16 MiB.
- Native DuckDB cancellation is not connected to pgwire cancellation.
- Query/blocking admission, DuckDB memory, thread, temp, and spill configuration
  are not yet stable product controls.
- Supported statement and parameter type surfaces are intentionally narrow.
- Broad `pg_catalog`/`information_schema`, geometry/geography OID discovery, SRID,
  dimension, geography, extent, MVT, and general `ST_GeometryN` behavior remain
  incomplete.
- Named QGIS, GDAL/OGR, GeoServer, Martin, psycopg, ORM, and BI workflows have not
  been requalified against the DuckDB-only server.
- Remote/shared catalog and object-storage paths fail closed.
- Current bbox/exact-recheck evidence is a small explicit oracle, not automatic
  layout maintenance or a scale claim.
- Production backup/restore, rolling upgrade, soak, and disaster recovery remain
  unproven.

## Milestone status

| Milestone | State | Next closure work |
|---|---|---|
| M0 truthful repository | complete | `just project-contract-check` validates links/recipes/spatial counts; required CI publishes the deterministic transport-smoke manifest |
| M1 bounded execution | not started | ADBC streaming, portal stream ownership, cancellation, deadlines, admission, resource metrics |
| M2 streaming ingest | not started | incremental COPY builders/ADBC stream, escaping/types, rollback and throughput gates |
| M3 focused compatibility | partial foundation | first named client traces, catalogs/OIDs, geometry metadata, prioritized spatial gaps |
| M4 analytical performance | fixture foundation | safe bbox injection/layout maintenance; current 10M then 100M profile with DuckDB profiling |
| M5 Local 1.0 | not started | depends on M1–M4, packaging/operations/24-hour soak |
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
