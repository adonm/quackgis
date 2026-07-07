# quackgis-server

The QuackGIS binary. Single Rust process serving PostGIS-compatible SQL over
the PostgreSQL wire protocol.

```
psql / JDBC / QGIS / GeoServer / OGR
        │  pgwire
        ▼
quackgis-server
├── datafusion-postgres   wire protocol · pg_catalog emulation
├── quackgis compat       (M2+) geometry OID/encoding · geometry_columns
├── SedonaDB              ST_* kernels · CRS · spatial joins
└── DuckLake              (M1) Parquet + catalog
```

## Status (M3/M4)

SedonaDB `SessionContext` is served by datafusion-postgres; DuckLake storage is
wired and tested through the pgwire server. Dev storage path is SQLite catalog +
local Parquet files. Production PostgreSQL-catalog/S3 hardening remains a later
roadmap item.

Current gates cover DuckLake CTAS/CREATE/INSERT/UPDATE/DELETE, restart
persistence, WKB geometry persistence, Martin SQL/E2E, QGIS read/edit smoke,
GDAL/OGR PostgreSQL-driver load/read, and GeoServer PostGIS datastore
WFS/WMS/WFS-T smoke. See [../../docs/COMPATIBILITY.md](../../docs/COMPATIBILITY.md) for
current claims and limitations.

## Run

```sh
cargo run -p quackgis-server --release -- --host 0.0.0.0 --port 5434
psql -h 127.0.0.1 -p 5434 -U postgres -c "SELECT ST_AsText(ST_GeomFromText('POINT(1 2)'))"
```

CLI flags (all optional, env-overridable per `clap`'s `env` feature):

| Flag | Env | Default | Notes |
|---|---|---|---|
| `--host` | `QUACKGIS_HOST` | `127.0.0.1` | bind addr |
| `--port` | `QUACKGIS_PORT` | `5434` | bind port (5434 to coexist with system PG) |
| `--catalog-path` | `QUACKGIS_CATALOG_PATH` | `quackgis.db` | SQLite DuckLake catalog path (dev) |
| `--data-path` | `QUACKGIS_DATA_PATH` | `./data` | local Parquet data dir (dev) |
| `--tls-cert` / `--tls-key` | — | none | optional TLS |
| `--log` | `RUST_LOG` | `info` | log filter |

Development auth is intentionally minimal: user `postgres`, database
`quackgis`, no password. Production auth/RBAC/TLS hardening is tracked for M6.
