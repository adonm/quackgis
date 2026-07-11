# Fork divergence ledger

## Active vendored code

### `vendor/arrow-pg`

QuackGIS retains a small Arrow-to-pgwire encoder derived from
`datafusion-contrib/datafusion-postgres`'s `arrow-pg` crate.

Local ownership:

- DataFusion dataframe support and its optional dependency are removed;
- GeoArrow/PostGIS optional implementation branches are removed;
- binary Arrow fields may advertise QuackGIS geometry/geography sentinel OIDs;
- `quackgis.spatial_family` field metadata takes precedence over conservative
  column-name fallback;
- WKB/EWKB remains raw binary in PostgreSQL binary format and hex in text format;
  and
- the crate is built and linted as a normal member of the root dependency graph.

This encoder should eventually become a QuackGIS-owned crate with focused
property/fuzz coverage for every advertised Arrow type. Until then, Arrow and
pgwire versions are pinned together with the server.

## Retired forks

The following forks/vendors are no longer compiled or retained in the repository:

- `adonm/sedona-db` / Sedona SQL function crates;
- `datafusion-postgres` and `datafusion-pg-catalog`;
- `datafusion-ducklake`; and
- DataFusion itself.

Their historical patches remain available in Git history through commit
`81328a3` and earlier. New compatibility work belongs at the owned pgwire edge,
DuckDB SQL/macros, a narrowly scoped DuckDB extension, or upstream DuckDB/DuckLake.
