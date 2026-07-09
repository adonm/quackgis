# QuackGIS

A PostGIS-compatible, Sedona-powered spatial lakehouse database in a single Rust
binary: built for platform/application developers who need high-throughput,
parallel-reader spatial SQL and columnar OLAP analysis over DuckLake/Parquet
data.

QuackGIS combines:

PostgreSQL wire protocol via
[datafusion-postgres](https://github.com/datafusion-contrib/datafusion-postgres),
spatial execution/performance via
[Apache SedonaDB](https://github.com/apache/sedona-db), and storage via
[DuckLake](https://ducklake.select)
([datafusion-ducklake](https://github.com/datafusion-contrib/datafusion-ducklake)).

No PostgreSQL query engine. No DuckDB runtime. The goal is a shared spatial data
lake that can answer large, complex spatial questions quickly, then run
DuckDB-style columnar analysis — fanout stats, primitive aggregates/calculations,
and pushdown filters — over the relevant Parquet records while common PostGIS
clients — **QGIS, GeoServer, GDAL/OGR, Martin, psql, psycopg** — connect and work
without significant changes.

```text
QGIS / GeoServer (JDBC) / psql / OGR / psycopg
        │  pgwire
        ▼
quackgis server (one Rust binary)
├── datafusion-postgres   wire protocol · auth · TLS · pg_catalog
├── quackgis compat layer geometry OID/EWKB · geometry_columns ·
│                         spatial_ref_sys · client shims
├── SedonaDB/DataFusion   ST_* kernels · CRS · spatial joins · columnar OLAP
└── datafusion-ducklake   DuckLake catalog + Parquet
        ▼
DuckLake storage = SQL catalog + Parquet objects
profiles: SQLite + local files, PostgreSQL + S3
```

## Status

**Developer preview.** The v0.1 PostgreSQL/DuckDB stack is retired; QuackGIS is
now a single Rust pgwire server over SedonaDB + DuckLake. The preview proves the
local shape with SQLite catalog + local Parquet data, PostGIS-style spatial SQL,
DuckLake writes, PostgreSQL text `COPY FROM STDIN`, automatic hidden spatial
layout columns, and explicit compaction.

Martin, QGIS read/edit, GDAL/OGR load/read, and GeoServer WFS/WMS/WFS-T smoke
probes are green for the maintained paths. See
[docs/DEVELOPER_PREVIEW.md](./docs/DEVELOPER_PREVIEW.md) for the runnable preview
contract and [ROADMAP.md](./ROADMAP.md) for remaining hardening.

Alpha scaled-storage evidence now exists in Kind: PostgreSQL catalog +
S3-compatible object storage, multi-process readers/writers, high-QPS scaled
gates, writer conflict/retry, and OLAP fanout with pruning/pushdown evidence. The
roadmap focus is Alpha hardening and then production ambition: external services,
larger real-data workloads, trend reports, native write maintenance, security, and
operations docs before production claims.

## Quick start (dev storage path)

```sh
mise install              # Rust, just, kind/kubectl/helm, cargo-nextest
eval "$(mise activate bash)" # optional: activate pinned tools/env for this shell
just setup                # also downloads Martin into .tmp/bin
just ref-init             # optional: clone all reference repos into .tmp/ref
just preview-smoke        # one-command preview acceptance smoke
just server               # runs on 127.0.0.1:5434 with .tmp/dev storage
psql -h 127.0.0.1 -p 5434 -U postgres
```

```sql
SELECT ST_AsText(ST_GeomFromText('POINT(1 2)'));        -- POINT(1 2)
CREATE TABLE public.points (id INT, name TEXT, geom BINARY); -- DuckLake + Parquet
COPY public.points (id, name, geom) FROM STDIN;              -- GDAL/OGR-style bulk ingest
1	origin	\\x010100000000000000000000000000000000000000
2	one	\\x0101000000000000000000f03f000000000000f03f
\.
CALL quackgis_compact_table('public.points');                -- rewrite into spatial layout order
CALL quackgis_compact_table('public.points', 0, 0);          -- target one layout bucket (safe full-replace fallback)
SELECT postgis_version();                                -- 3.4 QUACKGIS
```

## Documentation

- [ARCHITECTURE.md](./ARCHITECTURE.md) — layer model, geometry over the wire,
  trust boundaries, what changed from v0.1.
- [docs/PROJECT_DIRECTION.md](./docs/PROJECT_DIRECTION.md) — mission, primary
  user, non-goals, current preview, Alpha evidence, and broader direction.
- [ROADMAP.md](./ROADMAP.md) — current evidence, ambitious forward milestones,
  success metrics, and risks.
- [docs/ROADMAP_STATUS.md](./docs/ROADMAP_STATUS.md) — locally closed roadmap
  contracts vs execution-heavy remaining work.
- [docs/DEVELOPER_PREVIEW.md](./docs/DEVELOPER_PREVIEW.md) — exact local preview
  claim, one-command smoke, manual COPY example, gates, and limitations.
- [docs/COMPATIBILITY.md](./docs/COMPATIBILITY.md) — client compatibility
  targets and known limitations.
- [docs/COMPATIBILITY_MATRIX.md](./docs/COMPATIBILITY_MATRIX.md) — supported
  probe/client versions and evidence commands.
- [docs/POSTGIS_CONFORMANCE.md](./docs/POSTGIS_CONFORMANCE.md) — supported
  PostGIS function families, evidence tiers, known deltas, and unsupported
  surfaces.
- [docs/OPERATIONS.md](./docs/OPERATIONS.md) — current local + Kind client-probe
  workflow for the single Rust pgwire binary.
- [docs/ALPHA_EXTERNAL_SERVICES.md](./docs/ALPHA_EXTERNAL_SERVICES.md) — real
  PostgreSQL/S3 Alpha hardening runbook and failure-drill evidence ladder.
- [docs/SECURITY_RBAC.md](./docs/SECURITY_RBAC.md) — security trust boundaries,
  RBAC target, and auth/TLS/secret-rotation failure-mode checklist.
- [deploy/kubernetes/](./deploy/kubernetes/) — production-style Alpha
  Kubernetes example with external PostgreSQL/S3 secrets, TLS, metrics, probes,
  and resource limits.
- [docs/DEPENDENCY_POLICY.md](./docs/DEPENDENCY_POLICY.md) — fork, rebase,
  upgrade, and data/catalog compatibility policy.
- [docs/DUCKLAKE_ALIGNMENT.md](./docs/DUCKLAKE_ALIGNMENT.md) — DuckLake
  upstream-alignment ledger for storage behavior, interop gates, and migration
  triggers.
- [docs/NATIVE_DML_FORK_PLAN.md](./docs/NATIVE_DML_FORK_PLAN.md) — fail-closed
  native DuckLake delete-file/partial-rewrite fork path.
- [docs/MUTATION_FAILURE_DRILLS.md](./docs/MUTATION_FAILURE_DRILLS.md) — native
  DML/compaction crash, retry, and orphan-cleanup evidence ladder.
- [docs/SNAPSHOT_OPERATIONS.md](./docs/SNAPSHOT_OPERATIONS.md) — snapshot
  rollback, future SQL `AS OF`, protected snapshot, and CDC exposure plan.
- [docs/MULTIMODAL_ASSETS.md](./docs/MULTIMODAL_ASSETS.md) — raster,
  point-cloud, 3D tile, CAD/BIM, aerial, and reality-capture footprint/sidecar
  schema patterns.
- [examples/](./examples/) — QGIS, GDAL/OGR, and GeoServer examples using stable
  demo layers.
- [docs/OSM_POSTGIS_PARITY.md](./docs/OSM_POSTGIS_PARITY.md) — real OSM data
  side-by-side PostGIS parity roadmap and copy/sync recipes.
- [docs/REAL_DATA_CLIENT_MATRIX.md](./docs/REAL_DATA_CLIENT_MATRIX.md) — evidence
  contract for widening real-data client matrices beyond the current OSM gate.
- [docs/ANALYTICS_BENCHMARKS.md](./docs/ANALYTICS_BENCHMARKS.md) — QPS, OLAP,
  compaction, spatial analytics, asset-inventory scale ladder, and budget policy.
- [docs/API_CLIENT_PROBES.md](./docs/API_CLIENT_PROBES.md) — probe contract for
  psycopg, SQLAlchemy/GeoPandas, pg_featureserv-style, BI, and MVT clients.
- [docs/RELEASE_EVIDENCE.md](./docs/RELEASE_EVIDENCE.md) — release artifact,
  metrics dashboard, and evidence-review policy.
- [CHANGELOG.md](./CHANGELOG.md) — history, including the retired v0.1 facade.
- [CONTRIBUTING.md](./CONTRIBUTING.md) — contribution guide.

## Development

Use `mise` for pinned tools/env and `just` for repo workflows. For an
interactive shell, activate mise once, then run recipes directly. For the
guided path, start with [docs/QUICKSTART.md](./docs/QUICKSTART.md).

```sh
eval "$(mise activate bash)"
just --list                    # common entrypoints
just doctor                    # verify pinned local dev tools are available
just smoke                     # smallest pgwire + spatial query smoke test
just preview-smoke             # CREATE + COPY + spatial query + compact smoke
just demo-local                # host-local demo on 127.0.0.1:5434, Ctrl-C to stop
just demo-kind                 # 5-minute Kind demo; see docs/QUICKSTART.md
just ci                        # same fast gate used by GitHub Actions
just build                     # server binary
just test                      # unit + wire integration tests
just test-fast                 # non-ignored QuackGIS regression loop only
just check                     # fmt + clippy + tests
just check-fast                # fmt + clippy + focused regression loop
just layoutbench-sf0           # layout/pruning correctness oracle
just layoutbench-local-smoke   # temp-server layoutbench smoke
just postgis-regress           # starter curated PostGIS function regress subset
just postgis-conformance-summary # static fixture coverage summary
just runtime-static-check      # guard single-binary native-free runtime image
just martin-sql                # Martin-generated SQL compatibility gate
just martin-e2e                # opt-in real Martin binary E2E
just kind-refresh              # host-cached build/load/deploy into Kind
just kind-refresh-fast         # faster no-LTO probe build/load/deploy loop
just kind-ready                # validate podman + create/reuse local Kind cluster
just seed-local-demo           # seed stable public.demo_* layers in a running local server
just seed-kind-demo            # seed stable public.demo_* layers in an existing cluster
just kind-probes               # QGIS read/edit + OGR + GeoServer WFS/WMS/WFS-T jobs
just kind-qgis-probe           # headless PyQGIS add-layer/read-feature gate
just kind-qgis-edit-probe      # headless PyQGIS insert/update/delete/save gate
just kind-ogr-probe            # GDAL/OGR PostgreSQL-driver load/read gate
just kind-geoserver-probe      # GeoServer 3.0.0 datastore + WFS/WMS/WFS-T gate
just kind-compatibility        # build/deploy + QGIS/OGR/GeoServer compatibility probes
just kind-lake-smoke           # Kind PostgreSQL catalog + s3s-fs object storage smoke
just kind-external-alpha-smoke # env-driven external PostgreSQL/S3 storage profile
just kind-lake-multipod-smoke  # shared-catalog smoke through multiple QuackGIS pods
just kind-write-smoke          # parallel ingest + deterministic snapshot conflict/retry evidence
just kind-qps-smoke            # high-QPS spatial readers over the shared lake profile
just kind-olap-smoke           # grouped OLAP fanout + pruning/recheck evidence
just kind-alpha-smoke          # maintained Alpha scaled-storage gate bundle
just kind-osm-postgis-parity   # opt-in real OSM PostGIS -> QuackGIS parity
just metrics-trend path=.tmp/compatibility # flatten metrics.json artifacts to CSV
just metrics-dashboard path=.tmp/compatibility # render release-ready Markdown trends
QUACKGIS_METRICS_PORT=9187 just server # optional Prometheus /metrics endpoint
```

For one-off commands without shell activation, keep the same recipes and let mise
inject the pinned environment:

```sh
mise exec -- just ci
mise exec -- just kind-compatibility
```

Reference/source trees for client-trace work live outside the build graph under
ignored `.tmp/ref/*` (submodule-init equivalent): `just ref-init` materializes
the QuackGIS forks plus Martin, QGIS, GeoServer, GDAL/OGR (`ogr2ogr`), PostGIS,
DuckDB/DuckLake/pg_ducklake, and SQLite.

The current stack is intentionally zero-native-dependency for QuackGIS itself:
no libgeos/libproj/libgdal. Client/test tools such as Martin, QGIS, GeoServer,
and KinD are managed via `mise.toml` environment/tool pins plus Justfile
recipes.

Pushes to `main` and pull requests run the mise-backed fast Rust gate. The
scheduled/manual compatibility workflow builds the Kind image, runs QGIS
read/edit, OGR, GeoServer, and optionally real OSM PostGIS parity probes, then
uploads probe logs as a compatibility report artifact.

Upstreams are consumed through fork branches when needed. DuckLake storage is a
**core product path**, not a placeholder: SQL catalog + Parquet object/file data,
with SQLite + local files and PostgreSQL + S3 both treated as first-class storage
profiles. Extending datafusion-ducklake to meet QuackGIS storage requirements
(SQL DDL routing, UPDATE/DELETE, pruning, PostgreSQL/S3 hardening) is explicitly
in scope, while staying forward-compatible with the official DuckLake 1.0+ spec.

Licensed under the [Apache License 2.0](./LICENSE).
