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
| QuackGIS macro | 6 | A small explicit compatibility expression is required and executable. |
| Rust pgwire/catalog edge | 10 | SRID/EWKB identity, extent metadata, or catalog behavior remains owned by the compatibility edge; each currently returns a ledger-pinned `0A000`. |
| Extension candidate | 5 | `ST_NDims`/`ST_CoordDim` and `ST_GeometryN` family gaps return ledger-pinned `0A000` errors until a real macro/extension implementation is promoted. |
| Explicit unsupported | 0 | No case in the current claimed subset is intentionally dropped. |

The machine-owned classification is
`tests/duckdb_spatial_compat.json`. Do not adjust expected values merely to match
DuckDB: change a disposition, add a bounded compatibility implementation, or
document a loss. WKT whitespace is normalized; WKB/EWKB hex is compared exactly.

## Claim boundary

This closes classification for the current 57-case subset. The pinned CLI probe
executes the classified DuckDB expressions, while `duckdb-pgwire-workflow-test`
sends all 42 original PostGIS expressions through the server-owned rewrite/macro
edge with maintained scalar results. The pgwire test reads this ledger and the
curated regress source so the lists cannot drift silently. All 15 non-executable
cases pass through simple and extended pgwire to prove ledger-pinned errors and
session reuse. The maintained workflow also proves explicit/implicit relational
namespace/type/range/collation rows, profile/QGIS-required built-ins with complete
references, all seven custom lookup result types, and geometry RowDescription
binary WKB, text hex-WKB, and NULL behavior with `tokio-postgres`. Implementing the
10 Rust-edge semantics and broader user-object catalog discovery remains M3 work.
Geography has the same catalog and wire-identity fixture as geometry; captured GIS
traces are oracles but do not yet execute end to end against QuackGIS.
