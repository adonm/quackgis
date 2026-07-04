# Operations

> **Stale — v0.1 stack.** This document describes the retired
> PostgreSQL + pg_ducklake + DuckDB architecture. The project has been
> redesigned around a single-binary pgwire server (see
> [ARCHITECTURE.md](../ARCHITECTURE.md)); this doc is refreshed at milestone
> M6 ([ROADMAP.md](../ROADMAP.md)).

Deployment, backup, upgrade, migration, and dependencies for QuackGIS.

## Quick start

```sh
docker build -t quackgis:dev -f container/Dockerfile .
docker run -e POSTGRES_PASSWORD=quackgis -p 5432:5432 quackgis:dev
psql postgres://postgres:quackgis@localhost:5432/postgres
```

## Configuration

Environment variables (or Helm values):

| Variable | Default | Purpose |
|---|---|---|
| `POSTGRES_PASSWORD` | required | PG bootstrap auth |
| `QUACKGIS_DUCKLAKE_DIR` | `/var/lib/quackgis` | DuckLake catalog + data root |
| `QUACKGIS_DUCKLAKE_DATA_PATH` | `$DUCKLAKE_DIR/data/` | Parquet data path |
| `QUACKGIS_REWRITE_MODE` | `warn` | `strict`, `warn`, or `off` |
| `QUACKGIS_DUCKDB_THREADS` | `-1` | DuckDB worker threads |
| `QUACKGIS_DUCKDB_MEMORY_LIMIT` | `4096` | DuckDB memory cap (MB) |

## Kubernetes

Helm chart at `deploy/helm/quackgis/`. Plain manifests at `deploy/k8s/`.

```sh
# Helm
helm install quackgis deploy/helm/quackgis \
    --set quackgis.postgresPassword=secret \
    --set persistence.ducklake.size=100Gi

# Plain
kubectl apply -f deploy/k8s/quackgis.yaml
kubectl port-forward svc/quackgis 55432:5432 -n quackgis
```

Required: PVC for PG data + DuckLake data, Secret for password, ConfigMap for
settings. Non-root UID 999, capability drops, NetworkPolicy.

## DuckLake spatial layout

Spatial tables should materialize layout columns for pruning. Use `minx/miny/
maxx/maxy` (not PostgreSQL-reserved `xmin/xmax`):

```sql
CREATE TABLE parcels USING ducklake AS
SELECT
    id, geom,
    st_xmin(geom) AS minx, st_ymin(geom) AS miny,
    st_xmax(geom) AS maxx, st_ymax(geom) AS maxy,
    st_quadkey(geom, 8)  AS spatial_cell,
    st_hilbert(geom, 16) AS spatial_sort
FROM raw_parcels
ORDER BY spatial_sort;

CALL ducklake.set_partition('parcels', 'spatial_cell');
```

Three-stage query pattern (cell + bbox + exact):

```sql
SELECT * FROM parcels p
WHERE p.spatial_cell IN (
    SELECT quadkey FROM st_covering_quadkeys(
        st_makeenvelope(-5,-5,5,5), 8, 1000)
)
AND p.maxx >= -5 AND p.minx <= 5
AND p.maxy >= -5 AND p.miny <= 5
AND st_intersects(p.geom, st_makeenvelope(-5,-5,5,5));
```

Stages 1–2 are performance filters. Stage 3 (exact predicate) defines correctness.

## PostGIS migration

| PostGIS | QuackGIS | Class |
|---|---|---|
| `ST_GeomFromText(wkt)` | same | direct |
| `'wkt'::geometry` | text→geometry cast | automatic |
| `geom && other` | bbox predicate or `st_bbox_intersects` | rewrite |
| `geom <-> q LIMIT k` | `ORDER BY st_distance LIMIT k` | rewrite |
| `geometry(Point,4326)` | geometry DOMAIN (typmod not enforced) | review |
| `ST_Collect(a,b)` | `st_collect_scalar(a,b)` | automatic |
| `ST_MemUnion(g)` | `ST_Union_Agg(g)` | automatic |
| GiST indexes | DuckLake layout columns | unsupported |

Use `quackgis.rewrite_sql('SELECT ...')` to preview rewrites.

## Backup

**PVC snapshot** (preferred):

```sh
kubectl snapshot volumesnapshot quackgis-snap \
    --persistentvolumeclaim=quackgis-ducklake
```

**pg_dump** (metadata only, use `--insert` to bypass COPY):

```sh
pg_dump --insert --no-owner -t mytable -f backup.sql "$DATABASE_URL"
```

DuckLake Parquet files are immutable — snapshot the data volume for crash-
consistent backup.

## Restore

From PVC snapshot: create new PVC from snapshot, start new pod.
From pg_dump: `pg_restore -d postgres backup.sql`.

## Upgrade

```sh
kubectl set image statefulset/quackgis quackgis=quackgis:0.2.0 -n quackgis
kubectl rollout status statefulset/quackgis -n quackgis
```

DuckLake Parquet files are backward-compatible. PostgreSQL major version
upgrades require `pg_upgrade`. Verify with `SELECT * FROM quackgis.diagnostics;`.

Rollback: `kubectl rollout undo statefulset/quackgis -n quackgis`.

## Object-store credentials

```sh
# S3
kubectl create secret generic quackgis-s3 \
    --from-literal=AWS_ACCESS_KEY_ID='...' \
    --from-literal=AWS_SECRET_ACCESS_KEY='...' -n quackgis

# Rotate
kubectl update secret quackgis-s3 ...
kubectl rollout restart statefulset/quackgis -n quackgis
```

## Dependencies (bundled in image)

| Layer | Dependency | Purpose |
|---|---|---|
| Facade | PostgreSQL 18 | pgwire, auth, sessions, catalog |
| Integration | pg_ducklake | DuckDB lifecycle, DuckLake table AM |
| Execution | DuckDB | vectorized analytical engine |
| Spatial | sedonadb extension | PostGIS/SedonaDB `ST_*` kernels |
| Lakehouse | DuckLake | table metadata, snapshots, Parquet |
| Topology | GEOS | planar topology, overlay, relate |
| CRS | PROJ | coordinate transforms |
| Raster | GDAL | raster I/O |

Operator provides: PostgreSQL data volume, DuckLake data volume or object store,
credentials, TLS config.

## Release

```sh
./deploy/release.sh 0.1.0
```

Runs engine checks → builds image → generates version manifest + SBOM → prints
tag/push guidance. On tag push, `.github/workflows/release.yml` builds and pushes
to GHCR with SBOM and image report.
