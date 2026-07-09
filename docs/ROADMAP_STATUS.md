# Roadmap status index

This index keeps implemented local contracts out of the forward roadmap. A doc or
runbook listed here is a useful contract, but the matching roadmap item stays open
until the intended evidence has run at its target scale and source SHA.

The active execution loop is maximizing the local Kind+Linkerd envelope in
[LOCAL_KIND_LINKERD_FOCUS.md](./LOCAL_KIND_LINKERD_FOCUS.md), then promoting the
same claims to managed external services.

## Closed contracts and active frontiers

| Area | Implemented contract / evidence floor | Active frontier |
|---|---|---|
| Trend dashboards and release evidence | `just metrics-dashboard`; `just metrics-budget-check`; scheduled workflows upload dashboards; `docs/RELEASE_EVIDENCE.md` defines release packet | attach selected scheduled/manual artifacts to real releases with required budget assertions |
| Local Kind+Linkerd Alpha | `docs/LOCAL_KIND_LINKERD_FOCUS.md` defines the maximum local execution ladder, scale knobs, and claim boundaries | make full-ladder artifacts routine and budgeted |
| External PostgreSQL/S3 Alpha | `docs/ALPHA_EXTERNAL_SERVICES.md` defines credential rotation, restart, throttling, backup/restore, cleanup, and refresh drills | run against real platform-managed PostgreSQL/S3 services and publish artifacts |
| Benchmark ladder | `docs/ANALYTICS_BENCHMARKS.md`; manual `Benchmark ladder` workflow; QPS/OLAP/compaction metrics already emitted | run `sf10+`, real-data, external-service, and release-budget scale ladders |
| Real-data client matrix | `docs/REAL_DATA_CLIENT_MATRIX.md`; OSM Monaco opt-in baseline now includes OGR copy/read, SQL sample parity, MVT SQL bytes, and QGIS open/filter/render; MVT encoder has key/value tag coverage | add GeoServer and real Martin binary/SQL attribute side-by-side, then wider OSM/Overture-derived layers |
| API/client expansion | `docs/API_CLIENT_PROBES.md`; `just api-client-local-smoke` is in `just ci`; `just kind-api-client-probe` is in `just kind-compatibility` and compatibility metrics for pgwire/catalog/WKB/bbox/BI/MVT profile surfaces | promote to named psycopg, SQLAlchemy/GeoPandas, pg_featureserv, MVT, and BI client probes over real dependencies/data |
| PostGIS conformance | `docs/POSTGIS_CONFORMANCE.md`; `just postgis-regress`; `just postgis-conformance-summary` | promote broader fixture families through pgwire/client traces |
| Native mutation safety | local native DML/compaction tests; before-commit native delete/update/compaction failpoint tests now also retry the same mutation after the one-shot fault and assert `quackgis_native_mutation_aborts_total`; `docs/MUTATION_FAILURE_DRILLS.md` | extend crash/retry/orphan drills to process kill/retry, Kind/external storage, transaction batching, and reference-reader interop |
| DuckLake alignment | `docs/DUCKLAKE_ALIGNMENT.md` maps storage behavior to upstream direction and migration triggers | reference-reader interop runs, future upstream primitive migrations, and release migration notes |
| Snapshot/time travel | `docs/SNAPSHOT_OPERATIONS.md`; `public.table(snapshot => <id>)` and `public.table(snapshot_id => <id>)` simple snapshot-pinned reads with count/extent parity; metadata UDTFs | implement parser-level SQL `AS OF`, protected retention, rollback integration, positional table-function selectors, and safe CDC row UDTFs |
| Security/RBAC | SCRAM/read-only/TLS docs and tests; recognized read-only write denials increment `quackgis_write_denied_total`; `docs/SECURITY_RBAC.md` defines hardening probes and RBAC target | execute external secret/TLS failure drills and implement object-level RBAC only from traces |
| Multi-modal assets | `docs/MULTIMODAL_ASSETS.md`; footprint discovery and LayoutBench schema coverage | validate raster/point-cloud/3D/CAD/BIM inventories on real data and benchmark asset queries |

## Current hard blockers for production claims

- Real external PostgreSQL/S3-compatible service drills have not been run; they are
  now promotion work after local Kind+Linkerd maximum evidence is stable.
- Larger real-data matrices require copied datasets and external client systems.
- Native mutation crash/failure injection has local native `DELETE`/`UPDATE`/bucket
  compaction before-commit and one-shot retry oracles, but process-kill, orphan
  cleanup, Kind, and external-service drills are not automated.
- Parser-level SQL `AS OF`, protected snapshots, branch/merge, materialized views,
  and CDC row UDTFs depend on implementation work and/or upstream-stable APIs.
- CDC row table functions stay disabled until pgwire/Arrow projection is safe.
- Object/schema/table RBAC remains trace-driven future work.

## Maintenance rule

When a roadmap item gains a new local contract, add it here and link the owning
doc instead of duplicating implemented details throughout `ROADMAP.md`. When
execution evidence lands, update `ROADMAP.md`, `docs/COMPATIBILITY.md`, and the
release evidence packet for the exact source SHA.
