# Roadmap status index

This index keeps implemented local contracts out of the forward roadmap. A doc or
runbook listed here is a useful contract, but the matching roadmap item stays open
until the intended evidence has run at its target scale and source SHA.

The active execution loop keeps the local Kind+Linkerd envelope in
[LOCAL_KIND_LINKERD_FOCUS.md](./LOCAL_KIND_LINKERD_FOCUS.md) as a cheap companion
gate while promoting the same claims to managed external services. Local
refinement must not postpone the first external evidence run indefinitely.

## Closed contracts and active frontiers

| Area | Implemented contract / evidence floor | Active frontier |
|---|---|---|
| Trend dashboards and release evidence | `just metrics-dashboard`; `just metrics-budget-check`; scheduled workflows upload dashboards; `docs/RELEASE_EVIDENCE.md` defines release packet | attach selected scheduled/manual artifacts to real releases with required budget assertions |
| Local Kind+Linkerd Alpha | `docs/LOCAL_KIND_LINKERD_FOCUS.md` defines the maximum local execution ladder, scale knobs, and claim boundaries | make full-ladder artifacts routine and budgeted |
| External PostgreSQL/S3 Alpha | `docs/ALPHA_EXTERNAL_SERVICES.md` defines credential rotation, restart, throttling, backup/restore, cleanup, and refresh drills | run against real platform-managed PostgreSQL/S3 services and publish artifacts |
| Benchmark ladder | `docs/ANALYTICS_BENCHMARKS.md`; manual `Benchmark ladder` workflow; QPS/OLAP/compaction metrics already emitted | run `sf10m`, `sf100m`, `sf1b`, real-data, managed-service, and release-budget scale ladders |
| Real-data client matrix | `docs/REAL_DATA_CLIENT_MATRIX.md`; OSM Monaco opt-in baseline now includes OGR copy/read, SQL sample parity, MVT SQL bytes, and QGIS open/filter/render; MVT encoder and SQL/client probes have key/value tag coverage | add GeoServer and real Martin binary/OSM attribute side-by-side, then wider OSM/Overture-derived layers |
| API/client expansion | `docs/API_CLIENT_PROBES.md`; `just api-client-local-smoke` is in `just ci`; `just kind-api-client-probe` is in `just kind-compatibility` and compatibility metrics for pgwire/catalog/WKB/bbox/BI/MVT profile surfaces, including MVT attribute tags | promote to named psycopg, SQLAlchemy/GeoPandas, pg_featureserv, MVT, and BI client probes over real dependencies/data |
| PostGIS conformance | `docs/POSTGIS_CONFORMANCE.md`; `just postgis-regress`; `just postgis-conformance-summary` | promote broader fixture families through pgwire/client traces |
| Native mutation safety | local native DML/compaction tests; before-commit native delete/update/compaction failpoint tests now also retry the same mutation after the one-shot fault and assert `quackgis_native_mutation_aborts_total`; `docs/MUTATION_FAILURE_DRILLS.md` | extend crash/retry/orphan drills to process kill/retry, Kind/managed-service storage, transaction batching, and reference-reader interop |
| DuckLake alignment | SQLite/local is the spec-oriented single-catalog path but is not yet drop-in DuckDB-writable; `docs/DUCKLAKE_ALIGNMENT.md` records that the current PostgreSQL multicatalog backend is library-specific/non-spec | reference-reader/export gates for both profiles plus a blocking 1.0 PostgreSQL migration decision |
| Geometry identity | WKB/EWKB bytes and sentinel OIDs work for maintained clients; conventional column names drive current discovery | add durable DuckLake geometry/type metadata and prove unconventionally named columns/reference readers without regressing wire behavior |
| Spatial layout | ordinary hidden bbox/time/space/Morton columns, statistics-based pruning, exact recheck, and native bucket compaction are implemented | real-data scale, structural partition/file stats, catalog budgets, and only then possible true coarse DuckLake partitions |
| Pgwire protocol boundary | raw pre-parse rewrites, parsed hooks, type/catalog encoding, dedicated COPY, and maintained cursor paths are covered | realistic pgjdbc fetch-size/portal suspension evidence and structural replacement of remaining SQL-text classifiers |
| Snapshot/time travel | `docs/SNAPSHOT_OPERATIONS.md`; `AS OF SNAPSHOT <id>` and named table selectors, count/extent parity, metadata UDTFs, and snapshot success/error counters | implement positional/timestamp resolution, protected retention, rollback integration, and safe CDC row UDTFs |
| Security/RBAC | SCRAM/read-only/TLS docs and tests; recognized read-only write denials increment `quackgis_write_denied_total`; `docs/SECURITY_RBAC.md` defines hardening probes and RBAC target | execute external secret/TLS failure drills and implement object-level RBAC only from traces |
| Multi-modal assets | `docs/MULTIMODAL_ASSETS.md`; footprint discovery and LayoutBench schema coverage | validate raster/point-cloud/3D/CAD/BIM inventories on real data and benchmark asset queries |

## Current hard blockers for production claims

- Real external PostgreSQL/S3-compatible service drills have not been run; they are
  now promotion work after local Kind+Linkerd maximum evidence is stable.
- Larger real-data matrices require copied datasets and external client systems.
- The PostgreSQL DuckLake backend is a non-spec multicatalog layout until a
  reference-reader or tested export/migration gate proves a stronger claim.
- Durable geometry identity is incomplete: current catalog discovery still relies
  on conventional column names and sentinel wire OIDs.
- Native mutation crash/failure injection has local native `DELETE`/`UPDATE`/bucket
  compaction before-commit and one-shot retry oracles, but process-kill, orphan
  cleanup, Kind, and external-service drills are not automated.
- Timestamp-based SQL time travel, protected snapshots, branch/merge, materialized
  views, and CDC row UDTFs depend on implementation work and/or upstream-stable
  APIs.
- CDC row table functions stay disabled until pgwire/Arrow projection is safe.
- Object/schema/table RBAC remains trace-driven future work.

## Maintenance rule

When a roadmap item gains a new contract, add it here and link the owning doc
instead of duplicating implemented details throughout `ROADMAP.md`. Update the
forward roadmap only when an exit gate, priority, or product outcome changes.
When execution evidence lands, update the relevant compatibility/operations
record and release-evidence packet for the exact source SHA.
