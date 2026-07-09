# Operations

QuackGIS v0.2 is a single Rust pgwire server for PostGIS-compatible access to
DuckLake/Parquet spatial data. It does **not** run PostgreSQL as the query engine,
DuckDB, pg_ducklake, or C extensions in-process.

Current operations remain developer-preview/local-first for the default path:
SQLite DuckLake catalog + local Parquet files. Alpha scaled-storage evidence now
exists in Kind for PostgreSQL catalog + S3/object-store Parquet, many stateless
readers, parallel ingest writers, OLAP fanout probes over columnar spatial data,
and deterministic conflict/compaction behavior. The operations roadmap is to
harden that profile against external PostgreSQL/S3 services, production auth/TLS,
backup/restore, failed-writer cleanup, and trendable observability before making
production claims.

## Local development

```sh
mise install
eval "$(mise activate bash)"
just ci
just preview-smoke
just build
just server
just demo-local
```

Use an activated mise shell for interactive work; this keeps the pinned Rust,
container, Kubernetes, and probe-tool environment on `PATH`. In CI or one-off
scripts, keep the same Justfile entrypoints and prefix them with mise instead,
for example `mise exec -- just ci`.

The default local server listens on `127.0.0.1:5434` and uses:

| Variable | Default | Purpose |
|---|---|---|
| `QUACKGIS_HOST` | `127.0.0.1` | bind host |
| `QUACKGIS_PORT` | `5434` | pgwire port |
| `QUACKGIS_CATALOG_PATH` | `.tmp/dev/quackgis.db` | DuckLake SQLite catalog |
| `QUACKGIS_CATALOG_URL` | unset | PostgreSQL DuckLake catalog URL; switches storage profile when set |
| `QUACKGIS_DUCKLAKE_CATALOG_NAME` | `quackgis` | DuckLake catalog name inside PostgreSQL metadata |
| `QUACKGIS_DATA_PATH` | `.tmp/dev/data` | Parquet data directory |
| `QUACKGIS_S3_ENDPOINT` | unset | S3-compatible endpoint for `s3://` data paths |
| `QUACKGIS_S3_ACCESS_KEY_ID` / `QUACKGIS_S3_SECRET_ACCESS_KEY` | unset | S3 credentials |
| `QUACKGIS_S3_REGION` | `us-east-1` | S3 signing region |
| `QUACKGIS_S3_ALLOW_HTTP` | `false` | allow HTTP S3 endpoints for local development only |
| `QUACKGIS_AUTH_MODE` | `trust` | `trust` dev mode or `password` pgwire password mode |
| `QUACKGIS_READWRITE_USER` / `QUACKGIS_READWRITE_PASSWORD` | `postgres` / unset | read/write login; password required in password mode |
| `QUACKGIS_READONLY_USER` / `QUACKGIS_READONLY_PASSWORD` | `quackgis_readonly` / unset | optional read-only login in password mode |
| `QUACKGIS_METRICS_HOST` / `QUACKGIS_METRICS_PORT` | `127.0.0.1` / unset | optional Prometheus `/metrics` bind; disabled when port is unset |
| `QUACKGIS_LOG` | `info` | Rust log filter |

Dev auth is intentionally minimal: connect as user `postgres` to database
`quackgis` with no password in `trust` mode. Password mode uses SCRAM-SHA-256
and coarse read/write vs read-only roles; platform deployments still need TLS,
secret management, and catalog/object-store credential controls.

## Security profile

Current QuackGIS defaults to trusted developer and controlled Alpha probe
networks: `QUACKGIS_AUTH_MODE=trust` accepts startup packets without password
enforcement. For non-dev deployments, switch to `QUACKGIS_AUTH_MODE=password`,
set `QUACKGIS_READWRITE_PASSWORD`, and optionally enable a read-only login with
`QUACKGIS_READONLY_PASSWORD`. Read-only users can run SQL reads but fail closed
on DuckLake write and maintenance statements such as `CREATE TABLE`, `COPY FROM
STDIN`, DML, and compaction. PostgreSQL `pg_roles` and explicit-user privilege
helpers reflect configured read/write vs read-only users; current-user privilege
helper forms remain compatibility-oriented because the DuckLake write hook is the
authoritative authorization boundary.

Transport TLS is available with matching certificate/key configuration:

```sh
QUACKGIS_TLS_CERT=/run/secrets/tls/tls.crt \
QUACKGIS_TLS_KEY=/run/secrets/tls/tls.key \
QUACKGIS_AUTH_MODE=password \
QUACKGIS_READWRITE_PASSWORD='change-me' \
QUACKGIS_READONLY_PASSWORD='read-only-change-me' \
quackgis-server --host 0.0.0.0 --port 5434
```

The server fails closed if only one of `QUACKGIS_TLS_CERT` or `QUACKGIS_TLS_KEY`
is set, or if password auth is enabled without a non-empty read/write password.
Password auth negotiates PostgreSQL SASL/SCRAM-SHA-256; still pair it with
direct TLS, a trusted mTLS mesh, or an authenticated PostgreSQL-aware proxy so
queries, metadata, and object/catalog paths are not exposed on the wire.

Safe Alpha defaults before any external exposure:

- bind only to private interfaces/services;
- keep `QUACKGIS_S3_ALLOW_HTTP=false` outside local Kind/s3s-fs probes;
- enable `QUACKGIS_AUTH_MODE=password` and use distinct read/write and read-only
  credentials for services that do not need writes;
- source PostgreSQL catalog and object-store credentials from a secret manager or
  Kubernetes Secret, not command history or committed manifests;
- rotate catalog/object-store credentials by updating the secret and rolling all
  QuackGIS pods; verify with a storage smoke immediately after rotation;
- use platform TLS/mTLS for pod-to-pod traffic when direct pgwire TLS is not the
  trust boundary;
- keep `QUACKGIS_LOG=info` unless debugging in a trusted workspace, because query
  text and object paths may be sensitive.

Target M8 production profiles still need full object-level RBAC, documented
default TLS manifests, and broader failure-mode probes. A production-style
Kubernetes starting point lives under `deploy/kubernetes/`; it keeps secrets out
of Git and enables pgwire TLS, password auth, metrics, probes, and resource
limits for the external PostgreSQL/S3 profile. See
[SECURITY_RBAC.md](./SECURITY_RBAC.md) for the security/RBAC hardening contract
and failure-mode probe checklist.

Storage profiles:

| Profile | Status | Intended use |
|---|---|---|
| SQLite catalog + local files | current preview gate | local development, correctness parity, simple demos |
| PostgreSQL catalog + S3/object storage | Alpha Kind smoke | scaled multi-process readers/writers, shared platform deployment |

`just demo-local` uses isolated `.tmp/demo` storage, starts the local server,
seeds stable `public.demo_points` and `public.demo_polygons` layers, prints
client hints, and keeps the server running until Ctrl-C. Use
`just seed-local-demo` to seed the same stable layers into an already-running
local server.

## Kind client probes

Containerized client tests should run inside Kind, not via host networking. This
gives stable service DNS, consistent auth, and room to add multi-pod/multi-client
DuckLake tests later.

```sh
eval "$(mise activate bash)"
just kind-ready          # validate Podman/Kind and create or reuse the cluster
just demo-kind           # deploy, seed stable public.demo_* layers, print hints
just kind-up
just kind-compatibility  # build/deploy + QGIS read/edit, OGR, GeoServer probes
just kind-lake-smoke     # PostgreSQL catalog + s3s-fs object storage smoke
just kind-external-alpha-smoke # env-driven external PostgreSQL/S3 profile wiring
just kind-qps-smoke      # parallel-reader QPS gate over the lake profile
just kind-qps-mtls-smoke # QPS gate with Linkerd TCP/TLS observability evidence
just kind-qps-deep-smoke # larger Linkerd-observed reader gate; QPS_DEEP_* tunable
just kind-mtls-smoke     # Linkerd-injected lake profile with mTLS evidence
```

`mise.toml` pins Rust, Just, Kind, kubectl, Helm, and cargo-nextest; Podman is the
host container runtime. The repo defaults `CONTAINER_ENGINE=podman`,
`KIND_EXPERIMENTAL_PROVIDER=podman`, `KIND_CLUSTER=quackgis`, and
`QUACKGIS_IMAGE=localhost/quackgis:dev`, so the same commands work in activated
shells and under `mise exec -- ...`.

`just kind-up` is idempotent: it reuses the `quackgis` cluster when present and
creates it when missing. `just kind-status` prints cluster/node/QuackGIS namespace
state. `just kind-refresh` builds the release binary locally with Cargo's normal
`target/` cache, copies it into a tiny runtime image, loads that image into Kind,
then restarts the StatefulSet so the fixed dev tag is picked up. For iterative
probe triage, `just kind-refresh-fast` uses Cargo's `probe` profile (release-like
but no thin-LTO/single-codegen-unit) to reduce rebuild latency. Use
`just kind-build-image-container` for a slower clean build inside the container
image. `just runtime-static-check` guards the maintained runtime image against
native GIS packages, package-manager installs, or build-tool leakage into the
runtime stage.

Client probe scripts are versioned under `deploy/kind/probes/` and published into
the cluster by `just kind-probe-scripts` as a `quackgis-probe-scripts` ConfigMap.
The QGIS, OGR, GeoServer, and demo Jobs all use this shared probe core instead of
embedding large scripts directly in YAML.

`just seed-kind-demo` refreshes stable `public.demo_points` and
`public.demo_polygons` layers in an existing deployment. `just demo-kind` wraps
cluster readiness, deployment refresh, and that seed job for quick onboarding.

`just kind-probes` starts the maintained QGIS read/render/identify/filter, QGIS edit, OGR, and
GeoServer WFS/WMS/WFS-T Jobs together and waits once. Individual `kind-qgis-probe`,
`kind-qgis-edit-probe`, `kind-ogr-probe`, and `kind-geoserver-probe` targets
remain available for focused reruns. `just kind-compatibility` is the stable
full local/CI recipe: it runs `kind-refresh-fast` and then `kind-probes`.

`just kind-lake-smoke` deploys the short-named lake profile: `lake` QuackGIS pods,
`pg` for the DuckLake PostgreSQL catalog, and `s3` for local S3-compatible Parquet
storage. The probe covers CREATE TABLE, text COPY, spatial read-back, compaction,
and read-back after compaction against that storage profile.

`just kind-external-alpha-smoke` deploys a separate `external-lake` QuackGIS
service whose storage is configured only through the `external-storage` Secret.
By default it points at the same Kind `pg` and `s3` emulators so local/CI can
exercise external-profile wiring deterministically; the default emulator path
uses a separate `quackgis_external` PostgreSQL database so DuckLake data-path
metadata does not collide with the main lake profile. Set
`EXTERNAL_ALPHA_USE_KIND_EMULATORS=false` and provide `EXTERNAL_QUACKGIS_*`
values to run the same probe against real PostgreSQL/S3-compatible services. It
also checks that autocommit `DELETE` creates two native DuckLake delete files
under one PostgreSQL metadata snapshot and scrapes the opt-in Prometheus metrics
endpoint exposed on the `external-lake` Service.

On the shared PostgreSQL catalog profile, QuackGIS refreshes DuckLake catalog
metadata strongly for DDL/write/compaction statements and caches read-side
refreshes for `QUACKGIS_SHARED_CATALOG_REFRESH_MS` milliseconds (default 1000).
Set it to `0` to force the older refresh-before-every-read behavior during
catalog-coherence debugging.

`just kind-qps-smoke` scales `lake` to three pods, seeds a compacted
LayoutBench-style aerial table in the PostgreSQL/S3 lake profile, and runs 16
parallel pgwire readers issuing 240 selective spatial count queries across five
window/predicate shapes. The gate asserts unchanged exact results, hidden-bbox
pruning evidence, `EXPLAIN ANALYZE` file-group and bytes-scanned ceilings, at
least two backend instances via `quackgis_instance_id()`, and prints QPS plus
p50/p95/p99 latencies overall and per query shape. Tune `QPS_MAX_FILE_GROUPS`
and `QPS_MAX_BYTES_SCANNED` only when intentionally changing layout scale or
scan accounting.

`just kind-qps-mtls-smoke` runs the same reader gate from the Linkerd-injected
`mesh-client` deployment and asserts the local Linkerd proxy reports outbound
TLS TCP opens/read bytes/write bytes to `lake` destination pods. This is the
preferred observability evidence path when Linkerd is enabled. The QPS recipes
set `QUACKGIS_SHARED_CATALOG_REFRESH_MS` on `lake` from
`QPS_SHARED_CATALOG_REFRESH_MS` (default 60000) because they run a stable
read-only phase after seeding. Static hidden-bbox spatial reads automatically run
with `QUACKGIS_SELECTIVE_READ_TARGET_PARTITIONS` target partitions (default 1) in
a per-query DataFusion session clone, avoiding many object-store range opens
without rolling `lake` or slowing seed/compaction and broad scans. Set
`QUACKGIS_SELECTIVE_READ_TARGET_PARTITIONS=0` to disable that automatic tuning.
`QUACKGIS_TARGET_PARTITIONS` remains a global startup override; invalid values
fail server startup instead of silently changing scan parallelism.

`just kind-qps-deep-smoke` is the opt-in scale lever. Defaults seed 1.08M rows
(`QPS_DEEP_FACTOR=10000`), run 32 reader connections and 640 queries across four
`lake` pods, and require at least three backend instances plus Linkerd TCP/TLS
metric deltas. Increase `QPS_DEEP_FACTOR`, `QPS_DEEP_WORKERS`, and
`QPS_DEEP_QUERIES` for larger machines; `QPS_DEEP_DISK_BUDGET_GIB` defaults to
1024 and the recipe prints/guards an estimated disk budget before running.

`just kind-mtls-smoke` installs Linkerd with Helm, deploys Linkerd-injected
`lake`, `pg`, `s3`, and `mesh-client` pods, runs storage plus TCP load-balancing
probes from the injected client, and asserts Linkerd proxy metrics report TLS
traffic. The default `LINKERD_IPTABLES_MODE=nft` matches Podman-backed Kind.

The QGIS probe is a read-path gate. Current expected output includes:

```text
valid True
feature_count 2
fields ['id', 'name']
features_read 2
filter_names ['one']
identify_names ['one']
render_ok True
```

The QGIS edit probe opens a keyless spatial table through the postgres provider,
uses `_quackgis_rowid` as feature identity, and commits insert/update/delete
edits. Current expected output ends with:

```text
after_insert ... 'inserted' ... 'Point (1 1)' ...
after_update ... 'updated' ... 'Point (2 2)' ...
after_delete ... 'updated' ... 'Point (2 2)' ...
edit_ok True
compaction_after_edit_ok True
```

The OGR probe uses GDAL's PostgreSQL driver to read a WKB-backed table, append a
GeoJSON layer with `PG_USE_COPY=NO` + `-addfields`, and export both paths to
GeoJSON. Current expected output includes:

```text
feature_count 2
names ['one', 'origin']
geometry_types ['Point', 'Point']
loaded_rows [('load-a', 'client', 'POINT(2 2)'), ('load-b', 'client', 'POINT(3 3)')]
load_feature_count 2
load_names ['load-a', 'load-b']
load_geometry_types ['Point', 'Point']
ogr_keyless_rowids ['1', '2']
ogr_keyless_compact_ok True
```

The GeoServer probe uses official `docker.osgeo.org/geoserver:3.0.0`, supplies a
pgjdbc jar, registers QuackGIS as a PostGIS datastore, publishes a WKB-backed
layer, and exercises WFS GeoJSON, WMS PNG rendering, and real WFS-T
insert/update/delete. Current expected output includes:

```text
wfs_point_count 2
wms_png_header 89504e470d0a1a0a
wfst_transaction_ok True
geoserver_keyless_wfs_point_count 2
geoserver_keyless_update_ok True
geoserver_probe_ok True
```

In-cluster clients connect to:

```text
host: quackgis.quackgis.svc.cluster.local
port: 5434
user: postgres
database: quackgis
password: <empty>
```

Relevant files:

| Path | Purpose |
|---|---|
| `deploy/Containerfile.runtime` | runtime-only image used by the cached host-build Kind path |
| `deploy/Containerfile` | clean container-native fallback image for Kind probes |
| `deploy/kind/cluster.yaml` | Kind cluster config |
| `deploy/kind/quackgis.yaml` | QuackGIS StatefulSet + Service |
| `deploy/kind/demo.yaml` | one-command demo Job |
| `deploy/kind/probes/` | shared probe scripts and common probe core |
| `deploy/kind/qgis-probe.yaml` | headless PyQGIS add-layer probe Job |
| `deploy/kind/qgis-edit-probe.yaml` | headless PyQGIS edit/save probe Job |
| `deploy/kind/ogr-probe.yaml` | GDAL/OGR PostgreSQL-driver load/read probe Job |
| `deploy/kind/geoserver-probe.yaml` | official GeoServer 3.0.0 datastore + WFS/WMS/WFS-T probe Job |
| `deploy/kind/postgis-osm.yaml` | opt-in PostGIS reference deployment for real OSM parity |
| `deploy/kind/osm-postgis-parity-probe.yaml` | opt-in real OSM PostGIS → QuackGIS copy/read parity Job |

### Opt-in real OSM PostGIS parity

The real-data parity track is intentionally outside `just kind-probes` because it
pulls a PostGIS image and downloads a live OSM extract. It uses Geofabrik Monaco
by default, loads real OSM points, lines, and multipolygons into PostGIS, copies
deterministic samples into QuackGIS with `ogr2ogr`, and compares GeoJSON exports
plus SQL samples from both databases.

```sh
eval "$(mise activate bash)"
just kind-refresh-fast
just kind-osm-postgis-parity
```

Current expected output includes:

```text
postgis_osm_named_points_count 50
quackgis_osm_named_points_count 50
postgis_osm_named_lines_count 50
quackgis_osm_named_lines_count 50
postgis_osm_named_multipolygons_count 50
quackgis_osm_named_multipolygons_count 50
osm_postgis_to_quackgis_copy_ok True
```

The gate asserts stable IDs, `osm_id`, UTF-8 names, geometry type, count, and
bbox. It prints PostGIS and QuackGIS SQL samples as evidence for text and
attribute parity.

Useful overrides:

```sh
OSM_EXTRACT_URL=https://download.geofabrik.de/europe/andorra-latest.osm.pbf \
OSM_POINT_LIMIT=100 \
OSM_LINE_LIMIT=100 \
OSM_POLYGON_LIMIT=100 \
just kind-osm-postgis-parity
```

Stop the reference PostGIS deployment when finished:

```sh
just kind-postgis-osm-down
```

See [OSM_POSTGIS_PARITY.md](./OSM_POSTGIS_PARITY.md) for the long roadmap and
copy/sync recipes.

## Alpha scaled-storage profile

The maintained Alpha profile runs QuackGIS as stateless pods over one shared
DuckLake PostgreSQL metadata catalog and an S3-compatible object-store prefix
(`s3s-fs` in Kind). It is the profile to use before claiming multi-process or
object-storage behavior.

```sh
eval "$(mise activate bash)"
just kind-lake-smoke           # PostgreSQL catalog + S3-compatible storage basics
just kind-external-alpha-smoke # env-driven storage wiring + native DML metadata
just kind-lake-multipod-smoke  # concurrent probes through multiple QuackGIS pods
just kind-write-smoke          # parallel ingest plus snapshot conflict/retry evidence
just kind-qps-smoke            # high-QPS selective spatial readers
just kind-olap-smoke           # grouped OLAP fanout + pruning/recheck evidence
```

`just kind-alpha-smoke` runs the maintained Alpha gate bundle. Current expected
evidence includes `storage_ok True`, `write_conflict conflict_observed=True`,
`write_ok True`, `qps_ok True`, and `olap_ok True`. QPS and OLAP gates enforce
bytes-scanned ceilings from `EXPLAIN ANALYZE` (`QPS_MAX_BYTES_SCANNED`,
`OLAP_MAX_BYTES_SCANNED`) so pruning/pushdown regressions fail closed.

Kind uses disposable local defaults (`postgres/postgres`, database `quackgis`,
S3 access key `quackgis`). Override the `storage` Secret for any non-dev
cluster; do not reuse the Kind credentials outside a trusted local workspace.

For external service wiring, run:

```sh
EXTERNAL_ALPHA_USE_KIND_EMULATORS=false \
EXTERNAL_QUACKGIS_CATALOG_URL='postgres://user:pass@postgres.example:5432/quackgis' \
EXTERNAL_QUACKGIS_DATA_PATH='s3://bucket/quackgis-alpha' \
EXTERNAL_QUACKGIS_S3_ENDPOINT='https://s3.example' \
EXTERNAL_QUACKGIS_S3_ACCESS_KEY_ID='...' \
EXTERNAL_QUACKGIS_S3_SECRET_ACCESS_KEY='...' \
EXTERNAL_QUACKGIS_S3_ALLOW_HTTP=false \
just kind-external-alpha-smoke
```

The default emulator mode is a useful preflight, but not a production durability
claim. Use [ALPHA_EXTERNAL_SERVICES.md](./ALPHA_EXTERNAL_SERVICES.md) for the
real PostgreSQL/S3 evidence ladder: credential rotation, catalog restart,
object-store latency/throttling, backup/restore, failed-writer cleanup, and
catalog-refresh visibility.

### Alpha backup/restore runbook

The PostgreSQL/S3 profile has two durability boundaries that must be treated as
one backup set:

1. the DuckLake metadata catalog in PostgreSQL; and
2. the Parquet/object data under the configured object-store prefix.

A PostgreSQL dump without the matching object prefix is not a restorable QuackGIS
backup, and an object-prefix snapshot without the matching catalog can expose
unreferenced or missing files. Until a coordinated backup primitive exists, use a
quiesced backup window for any restore drill:

1. Stop application writers or route them away from QuackGIS.
2. Wait for in-flight `COPY`, DML, and `CALL quackgis_compact_table(...)` jobs to
   finish; failed or cancelled writers should be investigated before backup.
3. Back up the PostgreSQL catalog with the platform's normal PostgreSQL tooling.
4. Snapshot or copy the exact object-store prefix used by `QUACKGIS_DATA_PATH`.
5. Restore into an isolated catalog + object prefix, never directly over the live
   prefix.
6. Start one QuackGIS pod against the restored pair and run a read-only smoke:
   table discovery, representative counts/bboxes, and a small spatial query.
7. Only then point clients at the restored service.

Kind's in-cluster PostgreSQL and `s3s-fs` deployments are smoke-test stand-ins,
not backup tooling. Production backup/restore evidence must be collected against
the external PostgreSQL and S3-compatible services that will actually hold the
catalog and object prefix. The fast local gate includes a SQLite/filesystem
copy-restore oracle for the shape of a matched catalog+data backup set; it is not
a substitute for external-service restore drills.

### Failed-writer and object-prefix cleanup

DuckLake commits table state through catalog snapshots. A writer that fails
before publishing its snapshot can still leave temporary or unreferenced objects
in the object-store prefix. Current Alpha guidance is conservative:

- Do not manually delete objects from the live prefix while QuackGIS writers or
  compaction jobs are running.
- Prefer a dedicated prefix per environment/table group so inventory and cleanup
  scope is small.
- On failed writer cleanup, first quiesce writers, preserve the failed job logs,
  and capture an object inventory. Quarantine suspected orphan objects outside the
  live prefix before deleting them permanently.
- Validate cleanup by restarting QuackGIS, refreshing catalog metadata, and
  running the same representative counts/spatial reads used for backup restore.

Automatic orphan detection/garbage collection is still future work. Treat any
manual object cleanup as an operational change that needs a restore point.

### Catalog refresh tuning

On shared PostgreSQL catalog deployments, writes/DDL/compaction force a catalog
refresh on the connection that performed the mutation. Read-only statements use
`QUACKGIS_SHARED_CATALOG_REFRESH_MS` as a bounded staleness/cache interval for
cross-pod visibility:

| Setting | Use when | Trade-off |
|---|---|---|
| `0` | catalog-coherence debugging, demos immediately after writes | more catalog reads before every query |
| `1000` (default) | mixed small workloads | fresh enough for interactive clients with low catalog overhead |
| `60000+` | stable read-only QPS/OLAP phases after seeding | fewer catalog refreshes; other pods may not see new tables immediately |

Tune it explicitly during probes:

```sh
kubectl -n quackgis set env deployment/lake QUACKGIS_SHARED_CATALOG_REFRESH_MS=0
kubectl -n quackgis rollout status deployment/lake --timeout=180s
```

The QPS recipes intentionally raise this interval after the seed/compact phase so
the benchmark measures data reads instead of catalog polling. If a client reports
missing just-created tables across pods, temporarily set the interval to `0` and
rerun the workflow before investigating deeper catalog bugs.

## CI and compatibility reports

GitHub Actions uses `mise.toml` as the CI toolchain source of truth and calls the
same Justfile recipes as local development through `mise exec -- just ...`.

- `CI` runs `just ci` (`check-fast`) on pushes to `main` and pull requests.
- `Compatibility probes` runs the Kind QGIS read/edit, OGR, and GeoServer probes
  with `just kind-compatibility` on a nightly schedule and by manual dispatch. It
  appends `.tmp/compatibility/README.md` to the GitHub job summary, uploads logs
  collected by `just kind-compat-report` as a compatibility report artifact, and
  fails explicitly if the rendered report contains a failed probe row.
- The nightly compatibility run also executes the opt-in real OSM PostGIS parity
  probe; manual dispatch can enable it with the `run_osm` input.
- `Storage smoke` runs `just kind-alpha-smoke` weekly, appends the generated
  Kind compatibility/storage report to the job summary, and uploads Kind logs.
  Manual dispatch can still run any single storage recipe, including deeper
  QPS/mTLS variants, for focused triage.
- `Benchmark ladder` is a manual workflow for LayoutBench/QPS/OLAP scale recipes;
  it uploads `benchmark-report-*` and `benchmark-metrics-*` artifacts and renders
  `metrics-dashboard.md` when metrics are available.
- External PostgreSQL/S3 hardening runs should follow
  [ALPHA_EXTERNAL_SERVICES.md](./ALPHA_EXTERNAL_SERVICES.md); Kind emulator runs
  remain smoke evidence, not production durability evidence.
- Native DML and compaction crash/retry/orphan drills should follow
  [MUTATION_FAILURE_DRILLS.md](./MUTATION_FAILURE_DRILLS.md); successful ordinary
  DML tests alone are not production failure-mode evidence.
- Snapshot rollback, future SQL `AS OF`, protected snapshot, and CDC exposure
  policy is documented in [SNAPSHOT_OPERATIONS.md](./SNAPSHOT_OPERATIONS.md).

Every `kind-compat-report` artifact includes `metrics.json` with both probe
metrics and GitHub run metadata (`github_sha`, workflow, run id, run URL, and
storage recipe when applicable). The scheduled workflows also upload explicit
metrics-only artifacts named with the source SHA:

- `compatibility-metrics-<sha>-<run_id>`
- `storage-metrics-kind-alpha-smoke-<sha>-<run_id>`

Tag/main artifact builds write `release-evidence-<version>.json` next to the
binary archive. That manifest records source SHA, binary SHA256, image tags, and
the metrics artifact prefixes a release manager should attach to the release
evidence record. See [RELEASE_EVIDENCE.md](./RELEASE_EVIDENCE.md) for the
release artifact set, dashboard attachment policy, and review checklist.

Use `just metrics-trend path/to/artifact-dir` to flatten one or more downloaded
`metrics.json` files into CSV for trend dashboards. The helper also supports
`format=json`, `format=markdown`, and `format=dashboard` for release notes or
manual inspection. `just metrics-dashboard path=path/to/artifact-dir` writes a
release-ready Markdown dashboard; see
[Metrics trend dashboard](./METRICS_TRENDS.md) for the signal contract.

## Logs and observability

`QUACKGIS_LOG=info` emits one structured-ish line for every statement that enters
the QuackGIS DuckLake hook:

```text
quackgis_query_start query_id=42 protocol=simple pid=123 user=postgres statement_kind=query
```

The `query_id` is process-local and monotonic; it is intended for correlating a
client action with adjacent plan/scan/probe logs, not as a durable audit id.
Statement text is intentionally not logged at info level because it can contain
tenant data, object paths, or credentials in ad-hoc SQL. Read-only authorization
denials emit a separate counter line:

```text
quackgis_write_denied user=quackgis_readonly statement_kind=create_table denied_total=1
```

M8 observability now includes process-local catalog refresh and native
DML/compaction mutation counters without requiring log scraping. Object-store IO
and writer-conflict counters remain future work.

For process-local scrape evidence, set `QUACKGIS_METRICS_PORT` (and optionally
`QUACKGIS_METRICS_HOST`, default `127.0.0.1`) to expose a small Prometheus text
endpoint:

```sh
QUACKGIS_METRICS_PORT=9187 just server
curl http://127.0.0.1:9187/metrics
```

The endpoint is disabled by default and currently exports only safe process
counters: pgwire hook statements started, transaction staging ids allocated,
read-only write denials, DuckLake catalog refreshes, shared-catalog read/strong
refreshes, native delete/update/bucket-compaction mutations, and successful
compaction calls. It intentionally does not expose SQL text, object-store paths,
usernames, or secrets.

## Persistence model

The Kind StatefulSet mounts one `ducklake` PVC at `/var/lib/quackgis` containing:

- `quackgis.db` — DuckLake SQLite catalog
- `data/` — Parquet data files

This is suitable for single-pod restart/persistence smoke tests. Multi-server
tests must move to a shared catalog/data backend (for example PostgreSQL catalog
+ object-store data) before scaling replicas.

The lake Deployment in `deploy/kind/lake.yaml` is that shared backend profile:
PostgreSQL stores DuckLake metadata, `s3s-fs` serves the object-store API, and
QuackGIS pods are stateless readers/writers configured by `QUACKGIS_CATALOG_URL`,
`QUACKGIS_DATA_PATH=s3://...`, and `QUACKGIS_S3_*`.

## Reference source checkouts

`just ref-init` materializes source trees under ignored `.tmp/ref/*` for fork
work and client-trace research. This is intentionally submodule-like but outside
the build graph: Cargo continues to consume canonical git dependencies pinned by
`Cargo.lock`.

## Removed stale v0.1 deploy assets

The old PostgreSQL-container Helm chart, `container/Dockerfile*`, BuildKit
scripts, `pg_isready` probes, and DuckDB/pg_ducklake environment variables were
removed from the current deploy path. Git history retains them for archaeology;
new deployment work should target the single `quackgis-server` binary.
