# Local Kind + Linkerd capability focus

Local QuackGIS work should maximize the capability we can prove on a trusted
developer/CI machine with **Kind + Linkerd + in-cluster PostgreSQL + s3s-fs** while
managed-service and copied-data promotion proceed as separate roadmap work.

This does not create production durability claims. It makes local evidence deeper,
repeatable, and cheap enough to diagnose external-service runs without allowing
endless local refinement to postpone them.

## Local maximum envelope

| Capability | Local/Kind evidence path | What it proves |
|---|---|---|
| Client compatibility | `just kind-compatibility` | QGIS read/edit, OGR load/read, API-client profile surfaces, GeoServer WFS/WMS/WFS-T on maintained traces |
| PostgreSQL/S3-like lake profile | `just kind-alpha-smoke` | in-cluster PostgreSQL DuckLake catalog, s3s-fs object data, multi-pod reads/writes, QPS, OLAP |
| Linkerd mTLS and TCP observability | `just kind-mtls-smoke`, `just kind-qps-mtls-smoke` | injected lake/client pods, Linkerd TLS traffic counters, service load balancing |
| Local scale lever | `just kind-qps-deep-smoke` | tunable million-row-class generated QPS workload across multiple lake pods |
| Layout/pruning on lake profile | `just kind-lake-layoutbench-smoke` | LayoutBench through PostgreSQL catalog + S3-compatible data path |
| Real-data smoke | `just kind-osm-postgis-parity` | Monaco OSM via PostGIS reference into QuackGIS, OGR/QGIS copy/read/render assertions |
| Benchmark artifacts | `Benchmark ladder` workflow, local `just metrics-dashboard`, and `just metrics-budget-check` | QPS/p95/p99, scan budgets, candidate narrowing, dashboard artifacts, and fail-closed budget checks |
| API/client surface | `just api-client-local-smoke`, `just kind-api-client-probe` | psycopg-style params, SQLAlchemy/GeoPandas-style catalog/WKB reads, pg_featureserv-style bbox filters, BI aggregates, and MVT bytes before named client containers |
| Local ops/static gates | `just check-fast`, `just probe-static-check`, `just runtime-static-check` | fast Rust correctness, manifest/probe validity, native-free runtime image |

## Primary execution order

Use this as the default roadmap ladder when working locally:

```sh
just check-fast
just probe-static-check
just runtime-static-check
just api-client-local-smoke
just kind-compatibility
just kind-alpha-smoke
just kind-mtls-smoke
just kind-qps-mtls-smoke
just kind-qps-deep-smoke
just kind-lake-layoutbench-smoke
just kind-osm-postgis-parity
just kind-compat-report
just metrics-dashboard path=.tmp/compatibility out=.tmp/local-kind-linkerd-dashboard.md
just metrics-budget-check path=.tmp/compatibility allow_not_run=true
```

Run the whole ladder only on machines with enough CPU, memory, disk, and time.
For ordinary development, keep the first three gates plus the one relevant Kind
probe.

## Scale knobs that stay local

| Knob | Default / recipe | Use |
|---|---|---|
| `QPS_DEEP_FACTOR` | `10000` | scales generated QPS rows (`factor * 108`) |
| `QPS_DEEP_WORKERS` | `32` | reader concurrency for deep QPS |
| `QPS_DEEP_QUERIES` | `640` | total deep QPS query count |
| `QPS_DEEP_REPLICAS` | Justfile default | lake pod count for deep QPS |
| `QPS_DEEP_DISK_BUDGET_GIB` | `1024` | fail-closed disk budget guard |
| `QPS_SHARED_CATALOG_REFRESH_MS` | `60000` for read phase | reduces catalog polling during stable read benchmarks |
| OSM limits/extract URL | `OSM_*` env vars | widen real OSM layers while keeping data out of the repo |

Increase one dimension at a time and keep the generated `metrics.json` plus
dashboard with the source SHA.

## What local Kind + Linkerd can credibly prove

- pgwire/catalog compatibility for maintained client traces;
- correctness of DuckLake writes, native DML, compaction, and exact spatial recheck;
- multi-pod read/write behavior against a shared SQL catalog/object-store-shaped
  backend;
- Linkerd-injected service-to-service traffic and TCP/TLS observability;
- local scale trends and scan-budget regressions;
- real-data ingestion/copy smoke on small OSM extracts;
- release-evidence mechanics and dashboards.

## What remains outside the local claim

- managed PostgreSQL durability, failover, backups, maintenance windows, and
  credential rotation;
- real object-store semantics: throttling, region behavior, lifecycle policies,
  provider auth, and large-prefix inventory/cleanup;
- multi-terabyte datasets and long-running soak tests;
- production security/RBAC claims beyond the documented coarse local controls;
- client matrices that require external systems not present in Kind.

## Roadmap rule

Give every expensive feature a proof in this local Kind+Linkerd ladder, then run
the relevant managed-service or copied-data gate as soon as the local prerequisite
is stable. The local ladder is a proving ring for the larger goal, not a serial
phase that must be perfected before external work starts.
