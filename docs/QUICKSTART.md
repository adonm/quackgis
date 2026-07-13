# QuackGIS quickstart

This is the maintained path to the local DuckDB-only developer preview.

## 1. Install and bootstrap

```sh
mise install
mise run duckdb-bootstrap
eval "$(mise activate bash)" # optional interactive shell
just doctor
```

The bootstrap verifies the pinned DuckDB library and installs signed,
version-matched `spatial` and `ducklake` extensions into ignored `.tmp` paths.

## 2. Run the acceptance gate

```sh
just smoke
```

This runs the real DuckDB pgwire workflow: startup, SCRAM, structural policy,
create, COPY, parameters, mutations, transactions, portals, spatial queries,
snapshot inspection, shutdown, and reopen.

Run the storage kernel separately when diagnosing ADBC/DuckLake behavior:

```sh
just duckdb-adbc-storage-test
```

## 3. Start a server

```sh
mkdir -p .tmp/duckdb-server
cargo run -p quackgis-server -- \
  --catalog-path=.tmp/duckdb-server/catalog.ducklake \
  --data-path=.tmp/duckdb-server/data
```

With an activated mise shell, the required driver and DuckDB home are already in
the environment. Otherwise prefix the command with `mise exec --`.

Connect in development trust mode:

```sh
psql -h 127.0.0.1 -p 5434 -U postgres
```

```sql
SELECT ST_AsText(ST_GeomFromText('POINT(1 2)'));
```

The currently maintained write examples are exercised by `just smoke`; the public
SQL surface is deliberately bounded. Do not infer support for arbitrary DDL,
general multi-statement batches, broad catalogs, compaction calls, or GIS clients
from ordinary DuckDB/PostgreSQL behavior. The exact QGIS all-`SET` session bootstrap
is a narrow exception, not general batching support.

## 4. Verify changes

```sh
just check-fast
just ci
```

`just ci` includes pinned native storage and pgwire gates, so bootstrap must have
completed first.

## Troubleshooting

- Missing driver/extensions: rerun `mise run duckdb-bootstrap`.
- Startup rejects remote paths: shared profiles are intentionally disabled.
- Startup rejects a data root: inspect `_quackgis/storage-authority-v1`; migrate to
  a separate root rather than replacing a mismatched marker.
- Unsupported SQL returns a stable error by design; check
  [COMPATIBILITY.md](./COMPATIBILITY.md).
- Use `QUACKGIS_LOG=debug` for protocol/runtime diagnostics, but do not include
  credentials or sensitive paths in issue reports.
