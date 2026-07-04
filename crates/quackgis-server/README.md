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

## Status (M0)

Skeleton server: SedonaDB `SessionContext` served by datafusion-postgres.
Spatial SQL works (`ST_AsText`, `ST_Area`, `ST_Intersects`, ...). No
persistence yet — data lives only in the running process. See
[ROADMAP.md](../../ROADMAP.md) for what's next.

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
| `--tls-cert` / `--tls-key` | — | none | optional TLS |
| `--log` | `RUST_LOG` | `info` | log filter |

No auth at M0 (datafusion-postgres default `SimpleStartupHandler`); RBAC
arrives at M6.
