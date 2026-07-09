# Roadmap status index

This index separates **locally closed roadmap contracts** from work that still
requires external services, larger real datasets, new client systems, or upstream
DuckLake/Sedona/DataFusion features.

## Locally closed contracts

| Area | Completed local contract / evidence | Remaining execution-heavy work |
|---|---|---|
| Trend dashboards and release evidence | `just metrics-dashboard`; scheduled workflows upload dashboards; `docs/RELEASE_EVIDENCE.md` defines release packet | attach selected scheduled/manual artifacts to real releases |
| External PostgreSQL/S3 Alpha | `docs/ALPHA_EXTERNAL_SERVICES.md` defines credential rotation, restart, throttling, backup/restore, cleanup, and refresh drills | run against real platform-managed PostgreSQL/S3 services |
| Benchmark ladder | `docs/ANALYTICS_BENCHMARKS.md`; manual `Benchmark ladder` workflow; QPS/OLAP/compaction metrics already emitted | run `sf10+`, real-data, and external-service scale ladders |
| Real-data client matrix | `docs/REAL_DATA_CLIENT_MATRIX.md`; OSM Monaco opt-in baseline | add GeoServer/Martin side-by-side and wider OSM/Overture-derived layers |
| API/client expansion | `docs/API_CLIENT_PROBES.md` defines psycopg, SQLAlchemy/GeoPandas, pg_featureserv-style, BI, and MVT probe bar | implement and schedule those probes |
| PostGIS conformance | `docs/POSTGIS_CONFORMANCE.md`; `just postgis-regress`; `just postgis-conformance-summary` | promote broader fixture families through pgwire/client traces |
| Native mutation safety | local native DML/compaction tests; `docs/MUTATION_FAILURE_DRILLS.md` | automate or execute crash/retry/orphan drills, especially on external storage |
| DuckLake alignment | `docs/DUCKLAKE_ALIGNMENT.md` maps storage behavior to upstream direction and migration triggers | reference-reader interop runs and future upstream primitive migrations |
| Snapshot/time travel | `docs/SNAPSHOT_OPERATIONS.md` defines rollback, `AS OF`, protected snapshot, and CDC target policy | implement SQL `AS OF`, protected retention, and safe CDC row UDTFs |
| Security/RBAC | SCRAM/read-only/TLS docs and tests; `docs/SECURITY_RBAC.md` defines hardening probes and RBAC target | execute external secret/TLS failure drills and implement object-level RBAC only from traces |
| Multi-modal assets | `docs/MULTIMODAL_ASSETS.md`; footprint discovery and LayoutBench schema coverage | validate raster/point-cloud/3D/CAD/BIM inventories on real data |

## Current hard blockers for production claims

- Real external PostgreSQL/S3-compatible service drills have not been run.
- Larger real-data matrices require copied datasets and external client systems.
- Native mutation crash/failure injection is documented but not automated.
- SQL `AS OF`, protected snapshots, branch/merge, materialized views, and CDC row
  UDTFs depend on implementation work and/or upstream-stable APIs.
- CDC row table functions stay disabled until pgwire/Arrow projection is safe.
- Object/schema/table RBAC remains trace-driven future work.

## Maintenance rule

When a roadmap item gains a new local contract, add it here and link the owning
doc. When execution evidence lands, update `ROADMAP.md`, `docs/COMPATIBILITY.md`,
and the release evidence packet for the exact source SHA.
