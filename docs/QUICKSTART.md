# QuackGIS quickstart

This is the shortest path to a local QuackGIS developer preview and the Kind
client probes that validate real GIS workflows.

For the focused project direction — platform/app developers, high-throughput
spatial lakehouse workloads, and Alpha scaled storage — see
[PROJECT_DIRECTION.md](./PROJECT_DIRECTION.md).

## 1. Install pinned tools

```sh
mise install
eval "$(mise activate bash)"
just doctor
```

Podman is the default local container runtime. `mise.toml` pins Rust, Just, Kind,
kubectl, Helm, and cargo-nextest.

## 2. Run the developer-preview acceptance smoke

```sh
just preview-smoke
```

This starts a temporary QuackGIS server on `127.0.0.1:15434`, creates a DuckLake
table, bulk-loads WKB points through PostgreSQL text `COPY FROM STDIN`, queries
the rows with `ST_AsText(ST_GeomFromWKB(...))`, runs
`CALL quackgis_compact_table('public.preview_points')`, verifies results are
unchanged, and then stops the server.

Expected tail:

```text
preview_table public.preview_points
preview_copy_rows 3
developer_preview_ok True
```

See [DEVELOPER_PREVIEW.md](./DEVELOPER_PREVIEW.md) for the preview contract,
manual SQL, and verification checklist.

## 3. Run a local demo without Kubernetes

```sh
just demo-local
```

This starts QuackGIS on `127.0.0.1:5434`, seeds `public.demo_points` and
`public.demo_polygons`, prints QGIS/OGR connection hints, and keeps the server in
the foreground until Ctrl-C.

Seed only, after `just server` or another local deployment is already running:

```sh
just seed-local-demo
```

## 4. Run the five-minute Kind demo

```sh
just demo-kind
```

This creates or reuses the `quackgis` Kind cluster, deploys QuackGIS, seeds stable
demo layers, and prints client connection hints. Expected tail:

```text
demo_tables ['public.demo_points', 'public.demo_polygons']
demo_ok True
```

Seed only, after an existing deployment is ready:

```sh
just seed-kind-demo
```

## 5. Connect clients inside Kind

The in-cluster connection is:

```text
host: quackgis.quackgis.svc.cluster.local
port: 5434
database: quackgis
user: postgres
password: <empty>
tables: public.demo_points, public.demo_polygons
```

Example OGR command from a container/job using the cluster DNS:

```sh
ogrinfo 'PG:host=quackgis.quackgis.svc.cluster.local port=5434 user=postgres dbname=quackgis' demo_points -so
```

## 6. Run the full client gate

```sh
just kind-compatibility
just kind-compat-report
```

The compatibility report is written to `.tmp/compatibility/README.md` and includes
QGIS read/edit, OGR load/read, and GeoServer WFS/WMS/WFS-T status.

## Troubleshooting

- `just kind-ready` validates Podman, Kind, kubectl, and current cluster state.
- `examples/` has QGIS, OGR, and GeoServer setup notes for the stable demo layers.
- `just kind-status` shows nodes plus QuackGIS namespace pods/jobs/services.
- `just kind-logs` prints the QuackGIS server log tail.
- `just kind-down` deletes the local Kind cluster if you want a clean slate.
