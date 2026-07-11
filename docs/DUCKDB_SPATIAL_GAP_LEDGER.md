# DuckDB spatial gap ledger

This ledger classifies every case in QuackGIS's maintained 57-case pgwire
PostGIS subset against the pinned DuckDB 1.5.4 `spatial` extension. Run:

```sh
mise run duckdb-bootstrap
mise exec -- just duckdb-spatial-compat-probe
```

The gate writes `.tmp/duckdb-spatial/{README.md,manifest.json}` and fails when a
maintained case is missing/duplicated, a disposition is invalid, or an executable
native/rewrite/macro result differs from the maintained expected scalar. It runs
in required pull-request CI.

## Current local classification

| Disposition | Cases | Meaning |
|---|---:|---|
| Native DuckDB spatial | 31 | The maintained SQL executes directly with the expected scalar result. |
| Mechanical SQL rewrite | 5 | A recorded DuckDB SQL spelling produces the same maintained result. |
| QuackGIS macro | 4 | A small explicit compatibility expression is required and executable. |
| Rust pgwire/catalog edge | 12 | Version, SRID/EWKB identity, extent metadata, or catalog behavior remains owned by the compatibility edge. |
| Extension candidate | 5 | `ST_NDims`/`ST_CoordDim` and `ST_GeometryN` family gaps need a real macro/extension implementation before DuckDB pgwire promotion. |
| Explicit unsupported | 0 | No case in the current claimed subset is intentionally dropped. |

The machine-owned classification is
`tests/duckdb_spatial_compat.json`. Do not adjust expected values merely to match
DuckDB: change a disposition, add a bounded compatibility implementation, or
document a loss. WKT whitespace is normalized; WKB/EWKB hex is compared exactly.

## Claim boundary

This closes classification for the current 57-case subset. The pinned CLI probe
and `duckdb-pgwire-workflow-test` both execute all 40 native, rewrite, or macro
cases with maintained scalar results; the pgwire test reads this ledger and the
curated regress source so the lists cannot drift silently. It does not yet route
the 12 Rust-edge or five extension-candidate cases, prove geometry/geography OIDs,
or classify the broader SQL-portability and client-trace corpus. Those remain D3
work.
