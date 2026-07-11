# quackgis-server

The QuackGIS binary. Single Rust process serving PostGIS-compatible SQL over the
PostgreSQL wire protocol for SedonaDB + DuckLake/Parquet spatial lakehouse
workloads, including columnar OLAP-style analysis over spatial datasets.

```
psql / JDBC / QGIS / GeoServer / OGR
        │  pgwire
        ▼
quackgis-server
├── datafusion-postgres   wire protocol · pg_catalog emulation
├── quackgis compat       geometry OID/encoding · geometry_columns
├── SedonaDB              ST_* kernels · CRS · spatial joins
└── DuckLake              Parquet + catalog
```

## Status (developer preview)

SedonaDB `SessionContext` is served by datafusion-postgres; DuckLake storage is
wired and tested through the pgwire server. The deterministic preview uses SQLite
catalog + local Parquet files. Kind Alpha gates exercise the library-specific
PostgreSQL multicatalog + S3/object-store profile with multiple readers/writers;
managed-service and standard-reader/export evidence remain forward work.

Primary direction: platform/application developers running many parallel readers
and ingest jobs against a shared DuckLake catalog/object-store dataset. Common
PostGIS GIS clients are the compatibility surface, not the storage architecture.
DuckDB-style fanout analytics are also a target user experience, implemented with
DataFusion/Sedona/DuckLake rather than embedding DuckDB.

Current gates cover DuckLake CTAS/CREATE/INSERT/UPDATE/DELETE/COPY, restart
persistence, WKB geometry persistence, automatic hidden spatial layout columns,
explicit compaction, Martin SQL/E2E, QGIS read/edit smoke, GDAL/OGR
PostgreSQL-driver load/read, and GeoServer PostGIS datastore WFS/WMS/WFS-T
smoke. See [../../docs/DEVELOPER_PREVIEW.md](../../docs/DEVELOPER_PREVIEW.md)
for the runnable preview contract and
[../../docs/COMPATIBILITY.md](../../docs/COMPATIBILITY.md) for current claims and
limitations.

## Run

```sh
cargo run -p quackgis-server --release -- --host 0.0.0.0 --port 5434
psql -h 127.0.0.1 -p 5434 -U postgres -c "SELECT ST_AsText(ST_GeomFromText('POINT(1 2)'))"
```

Against a running server, exercise the preview pgwire path:

```sh
cargo run -p quackgis-server --example developer_preview -- --host 127.0.0.1 --port 5434
```

Or run the full temporary-server smoke from the repository root:

```sh
just preview-smoke
```

CLI flags (all optional, env-overridable per `clap`'s `env` feature):

| Flag | Env | Default | Notes |
|---|---|---|---|
| `--engine-backend` | `QUACKGIS_ENGINE_BACKEND` | `legacy-datafusion` | explicit backend; `duckdb` fails closed until D2 pgwire parity |
| `--host` | `QUACKGIS_HOST` | `127.0.0.1` | bind addr |
| `--port` | `QUACKGIS_PORT` | `5434` | bind port (5434 to coexist with system PG) |
| `--catalog-path` | `QUACKGIS_CATALOG_PATH` | `quackgis.db` | SQLite DuckLake catalog path (dev) |
| `--catalog-url` | `QUACKGIS_CATALOG_URL` | unset | PostgreSQL DuckLake catalog URL (Alpha storage) |
| `--ducklake-catalog-name` | `QUACKGIS_DUCKLAKE_CATALOG_NAME` | `quackgis` | catalog name inside PostgreSQL metadata |
| `--data-path` | `QUACKGIS_DATA_PATH` | `./data` | local Parquet data dir (dev) |
| `--s3-endpoint` | `QUACKGIS_S3_ENDPOINT` | unset | S3-compatible endpoint for `s3://` data paths |
| `--s3-access-key-id` / `--s3-secret-access-key` | `QUACKGIS_S3_ACCESS_KEY_ID` / `QUACKGIS_S3_SECRET_ACCESS_KEY` | unset | S3 credentials |
| `--s3-region` | `QUACKGIS_S3_REGION` | `us-east-1` | S3 signing region |
| `--s3-allow-http` | `QUACKGIS_S3_ALLOW_HTTP` | `false` | allow HTTP S3 endpoints for local development |
| `--tls-cert` / `--tls-key` | — | none | optional TLS |
| `--log` | `RUST_LOG` | `info` | log filter |

Trust-mode development defaults to user `postgres`, database `quackgis`, with no
password. Password mode supports SCRAM, optional TLS is wired, and coarse
read-only/read-write authorization is implemented; object-level RBAC and external
rotation/failure evidence remain production hardening.
