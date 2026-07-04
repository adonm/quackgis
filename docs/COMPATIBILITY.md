# Compatibility and limitations

## Engine function catalog

The sedonadb DuckDB extension exposes 254 SQL functions: 180 public `st_*`, 72
literal `sedona_st_*` bridge functions, and 2 extension-specific helpers. 46
public `st_*` route to the literal Apache SedonaDB kernel.

Run `python3 tools/catalog_audit.py` for the current count. The full per-function
table is in `COMPATIBILITY.md` at the repo root (auto-generated).

## PostgreSQL versions

| PostgreSQL | Status | Notes |
|---|---|---|
| 17 | ✅ Recommended | |
| 18 | ✅ Container base | Current `pgducklake/pgducklake` image |

## DuckDB ABI

The `sedonadb` extension must be compiled against the same DuckDB version that
pg_ducklake bundles. The Dockerfile builds from source to match. Check at
runtime via `quackgis.diagnostics`.

## DuckLake catalog modes

| Mode | Catalog | Data | Use case |
|---|---|---|---|
| File (default) | PVC | PVC | Dev, single-node |
| PostgreSQL | PG instance | PVC or S3 | Production, multi-writer |

## Object-store backends

| Backend | Protocol | Auth |
|---|---|---|
| Local PVC | file path | N/A |
| AWS S3 | `s3://` | `AWS_ACCESS_KEY_ID` + `AWS_SECRET_ACCESS_KEY` |
| GCS | `gs://` | Service account JSON |
| Azure Blob | `az://` | Connection string |
| R2 / S3-compatible | `s3://` | S3 creds + endpoint |

## Client compatibility

| Client | Status |
|---|---|
| `psql` | ✅ Full |
| psycopg (v3) | ✅ Full |
| JDBC | ✅ Expected (standard PG driver) |
| BI tools | ✅ Expected (pg_catalog/information_schema) |
| `pg_dump` | ⚠️ Use `--insert` mode; COPY protocol untested |

## Known limitations

### Architecture constraints (unlikely to change)

- **GiST indexes / planner hooks**: DuckDB C-API cannot register binary operators
  or planner hooks. Use DuckLake layout columns instead.
- **Topology schema**: PostgreSQL-specific, not supported.
- **SFCGAL 3D solids**: No mature Rust binding.
- **Raster `ST_MapAlgebra`**: DuckDB SQL replaces the expression language.
- **`LISTEN`/`NOTIFY`**: Not routed through DuckDB.

### Known gaps (may improve)

- **Typmod enforcement**: `geometry(Point, 4326)` is accepted but PostgreSQL does
  not enforce the type/SRID constraint. Handled at the EWKB level instead.
- **COPY protocol**: `pg_dump`/`pg_restore` use COPY, which is untested through
  the facade. Use `pg_dump --insert` or PVC snapshots.
- **PL/pgSQL function bodies**: Spatial calls inside stored procedures use stub
  functions that delegate to DuckDB. Performance implications for complex logic.
- **Aggregate functions**: PG-level aggregate stubs (`st_union_agg`, `st_collect`)
  collect rows into memory. For large tables, queries must route to DuckDB via
  DuckLake tables.
- **DuckDB ABI coupling**: Every DuckDB version bump requires rebuilding sedonadb.
- **DuckLake pruning without clustering**: Spatial pruning only works when files
  are Hilbert-sorted. Without layout columns, queries fall back to full scans.

### Known gaps (being addressed by strategy in ROADMAP.md)

- **Operators (`&&`, `<->`)**: sedonadb now registers these as DuckDB functions
  but pg_ducklake's deparser doesn't always resolve them. Being fixed by
  switching to DuckDB spatial's GEOMETRY type (Phase A).
- **Typmod enforcement**: `geometry(Point, 4326)` accepted but not enforced.
  Will be fixed by the C extension geometry type (Phase C).
- **COPY protocol**: `pg_dump`/`pg_restore` untested through the facade.
- **Bridge table standalone calls**: type conversion issue for non-DuckLake
  queries. Primary use case (DuckLake table queries) works correctly.

### Strategy for improvement

See [ROADMAP.md](../ROADMAP.md) for the full strategy. Key metric:
**PostGIS regress test pass rate**. Upstream PostGIS tests replace custom
tests as coverage grows.
