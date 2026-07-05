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

## Status (M1)

SedonaDB `SessionContext` is served by datafusion-postgres; DuckLake storage is wired and tested. Dev storage path is SQLite catalog + local Parquet files. Production target is PostgreSQL catalog + AWS S3. Current tests validate SQL CTAS, bare CREATE TABLE, INSERT SELECT, INSERT VALUES with column mapping, UPDATE, DELETE, writer API roundtrip, restart persistence, filter predicates, and WKB geometry persistence. Supported M1 SQL shape targets `quackgis.main.<table>`; advanced multi-table UPDATE/DELETE, RETURNING, and production PostgreSQL/S3 hardening are later milestones.

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

No auth at M0 (datafusion-postgres default `SimpleStartupHandler`); RBAC
arrives at M6.
