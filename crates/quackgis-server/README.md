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
portals, official DuckLake reopen, and the curated spatial subset.

Not yet supported as product claims: result/COPY streaming, native cancellation,
resource admission, broad catalogs, named GIS clients, or remote/shared storage.

See the root `README.md`, `ARCHITECTURE.md`, and `docs/COMPATIBILITY.md` for the
current contract.
