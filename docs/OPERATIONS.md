# Operations

QuackGIS is a local-first developer preview. The sole runtime is an owned Rust
pgwire server that dynamically loads a checksum-pinned DuckDB ADBC library and
uses DuckDB's official `spatial` and `ducklake` extensions.

## Bootstrap and verify

```sh
mise install
mise run duckdb-bootstrap
mise exec -- just ci
```

The bootstrap writes ignored artifacts below `.tmp/duckdb`, verifies the official
DuckDB 1.5.4 archive/library digests, and installs version-matched signed
extensions into an isolated DuckDB home. Runtime startup uses `LOAD` only; it does
not install extensions over the network.

## Run locally

```sh
mkdir -p .tmp/duckdb-server
mise exec -- cargo run -p quackgis-server -- \
  --catalog-path=.tmp/duckdb-server/catalog.ducklake \
  --data-path=.tmp/duckdb-server/data
```

The mise environment supplies `QUACKGIS_DUCKDB_ADBC_DRIVER` and the isolated
DuckDB home. The server defaults to `127.0.0.1:5434`.

## Configuration

| Variable | Default | Purpose |
|---|---|---|
| `QUACKGIS_DUCKDB_ADBC_DRIVER` | unset outside mise | required absolute path to pinned `libduckdb` |
| `QUACKGIS_DUCKDB_DATABASE_URI` | `:memory:` | DuckDB control database URI |
| `QUACKGIS_HOST` / `QUACKGIS_PORT` | `127.0.0.1` / `5434` | pgwire bind |
| `QUACKGIS_CATALOG_PATH` | `quackgis.ducklake` | official local DuckLake catalog |
| `QUACKGIS_DUCKLAKE_CATALOG_NAME` | `quackgis` | attached catalog name |
| `QUACKGIS_DATA_PATH` | `./data` | local Parquet root |
| `QUACKGIS_AUTH_MODE` | `trust` | `trust` for development or `password` for SCRAM |
| `QUACKGIS_READWRITE_USER` | `postgres` | write-capable login |
| `QUACKGIS_READWRITE_PASSWORD` | unset | required in password mode |
| `QUACKGIS_READONLY_USER` / `QUACKGIS_READONLY_PASSWORD` | `quackgis_readonly` / unset | optional read-only login |
| `QUACKGIS_WRITE_ALLOWLIST` / `QUACKGIS_READ_ALLOWLIST` | unset | comma-separated normalized table policy |
| `QUACKGIS_TLS_CERT` / `QUACKGIS_TLS_KEY` | unset | optional PEM certificate and PKCS#8 key; configure together |
| `QUACKGIS_METRICS_HOST` / `QUACKGIS_METRICS_PORT` | `127.0.0.1` / unset | optional `/metrics` endpoint |
| `QUACKGIS_LOG` | `info` | log filter |

`QUACKGIS_CATALOG_URL` and remote/object-store data paths are reserved and fail
closed. S3 credentials and the retired engine selector are not runtime options.

## Storage authority

Startup atomically creates `_quackgis/storage-authority-v1` below the local data
root. A mismatched marker fails before DuckLake attach. Migration must target a
separate root; never copy a retired writer's authority marker.

## Security baseline

- Trust mode is development-only.
- Password mode uses SCRAM-SHA-256.
- TLS configuration fails startup if only one path is supplied or material is
  malformed; deployments must still enforce network policy because TLS is not
  currently mandatory when configured.
- Read/write allowlists are enforced against parsed statements before ADBC
  prepare or schema lookup.
- The native driver path is an operator-controlled code-loading trust boundary and
  is verified against the committed SHA-256 before opening storage.

## Shutdown, backup, and recovery

SIGINT/SIGTERM closes the listener by terminating the server task. Active
transaction sessions roll back on drop where the native connection remains usable.
Back up the official DuckLake catalog, data root, and authority marker together.
Point-in-time restore, shared-storage recovery, rolling upgrades, and automated
disaster-recovery evidence are not yet production claims.

## Maintained checks

```sh
mise exec -- just check-fast
mise exec -- just duckdb-adbc-storage-test
mise exec -- just duckdb-pgwire-workflow-test
mise exec -- just duckdb-runtime-static-check
```

The release/runtime image must package the exact verified `libduckdb.so`, signed
`spatial` and `ducklake` extensions, and isolated DuckDB home. Bare Rust binaries
without those artifacts are not runnable server distributions.

The preview image binds pgwire to container loopback by default and does not
publish a port. It is a verification artifact, not a production deployment.
Override the bind address only together with SCRAM credentials and an enforced
TLS/network boundary.

CI uploads manifests and license/provenance evidence, not the native runtime
binary context. Redistribution remains blocked until Local 1.0 closes Rust and
Spatial transitive-license/source obligations for the exact bundle.
