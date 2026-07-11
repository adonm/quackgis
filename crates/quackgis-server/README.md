# quackgis-server

The owned Rust PostgreSQL wire/control edge over DuckDB Spatial and official
DuckLake.

## Run

Bootstrap the mandatory native runtime, then use the repository-managed command:

```sh
mise run duckdb-bootstrap
mise exec -- cargo run -p quackgis-server -- \
  --catalog-path=.tmp/dev/quackgis.ducklake \
  --data-path=.tmp/dev/data
```

This binds `127.0.0.1:5434` in development trust mode. Do not expose trust mode on
an untrusted interface. `mise.toml` supplies `QUACKGIS_DUCKDB_ADBC_DRIVER`; outside
mise, provide an absolute verified driver path explicitly.

Run the complete real-server workflow with:

```sh
mise exec -- just smoke
```

## Current boundary

Supported locally: bounded simple/extended pgwire, parameters, create/insert/
update/delete, transactions, text COPY for maintained types, SCRAM/table policy,
portals, `public` schema mapping, maintained SET/SHOW forms, quoted COPY targets,
official DuckLake reopen, and the curated spatial subset.

Query results stream from ADBC with native cancellation, deadlines, bounded
admission, and autosized DuckDB resource controls. COPY incrementally decodes
bounded Arrow batches into one ADBC stream and publishes atomically. Not yet
supported as product claims: COPY/query scale and RSS budgets, broad catalogs,
named GIS clients, or remote/shared storage.

See the root `README.md`, `ARCHITECTURE.md`, and `docs/COMPATIBILITY.md` for the
current contract.
