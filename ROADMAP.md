# Roadmap

## Goal

Highest PostGIS compatibility for smallest binary/container size. All data in
DuckLake. Upstream PostGIS tests as the primary quality metric. Custom tests
deleted as coverage grows.

**Success metric: PostGIS test pass rate = passing / total.**

## Strategy

1. **Layer on DuckDB spatial, not replace it.** DuckDB's built-in `spatial`
   extension provides ~100 functions and the `GEOMETRY` type. Sedonadb adds
   what spatial lacks (SedonaDB kernels, GEOS topology, PROJ, GDAL, spatial
   layout keys). This cuts binary size and type-mismatch bugs.

2. **Real PostgreSQL `geometry` type via C extension.** A thin C extension
   (~1000 lines) registers `geometry` as a real PG type with typmods, casts,
   and I/O functions. All computation still happens in DuckDB/sedonadb.

3. **Upstream PostGIS tests as the metric.** Run PostGIS regress tests.
   Prioritize fixes by test failure count. Delete custom tests when upstream
   tests cover the same functionality. Target: zero custom tests.

4. **Transparent spatial partitioning.** `CREATE TABLE ... USING ducklake`
   with a geometry column auto-adds bbox + cell + sort columns. Future:
   adaptive partition planning based on data distribution.

## Phases

### Phase A — sedonadb as sole spatial engine ✅ VALIDATED

Sedonadb is the sole spatial engine. We do NOT load DuckDB's built-in spatial
extension because:
1. Function name conflicts (st_asmvt, st_area, etc.)
2. sedonadb's Rust/GeoRust kernels are faster than DuckDB spatial's C++
3. sedonadb uses BLOB (WKB) which pg_ducklake translates naturally from PG bytea

Validated:
- sedonadb loads: `"sedonadb spatial extension loaded"`
- Spatial queries on DuckLake tables work (st_intersects, st_area, st_distance)
- Operators (&&, <->, <#>) registered as DuckDB BLOB functions
- Three-stage query parity confirmed
- Persistence across restarts confirmed

### Phase B — PostgreSQL geometry type (C extension)

The critical blocker for PostGIS test compatibility is the PG-side type system.
PostGIS tests use `'POINT(0 0)'::geometry` which requires a real PG type with:
- `geometry_in(cstring)` → parse WKT to WKB bytes
- `geometry_out(geometry)` → WKB to WKT
- Typmod support (`geometry(Point, 4326)`)
- Casts (`text::geometry`, `geometry::text`)

A DOMAIN over bytea cannot do this. Need a real PostgreSQL type registered via
a C extension (~500 lines). The type stores WKB bytes internally (same as
bytea). pg_ducklake translates it to DuckDB BLOB. sedonadb processes BLOB.

### Phase C — PostGIS test harness + baseline

Run upstream PostGIS regress tests. Track pass rate as the primary metric.
Baseline expected to be low initially; fix failures in priority order.

### Phase D — Compatibility sprint

Fix test failures in priority order (most failures first). Each fix increases
the pass rate metric. Track in CI.

### Phase E — Delete custom tests

As PostGIS test coverage grows, remove bespoke tests:
- `tests/reference/m*_fixtures.sql` — replaced by PostGIS regress
- `tests/postgis_port/cases/*.sql` — replaced by PostGIS regress
- `tests/upstream_curated/*.sql` — replaced by PostGIS regress
- `container/tests/postgis-fixtures/*.sql` — replaced by PostGIS regress
- Engine tests (`cargo test --lib`, `tests/run_sql.sh`) remain for FFI/kernel safety.

### Phase F — Transparent spatial partitioning

```sql
-- User writes this:
CREATE TABLE parcels (id int, geom geometry) USING ducklake;

-- QuackGIS automatically adds:
--   minx, miny, maxx, maxy (bbox zone-map columns)
--   spatial_cell (quadkey partition key)
--   spatial_sort (Hilbert clustering key)
-- And sets: PARTITIONED BY (spatial_cell)
```

Future: adaptive partition planning, spatial join optimization, operator-level
pruning hints.

### Phase G — Slim image

| Component | Current | Target |
|---|---|---|
| sedonadb extension | 32 MB | ~15 MB (remove spatial duplicates) |
| GDAL | ~50 MB | 0 MB default (feature-gated raster) |
| **Total image** | ~500 MB | ~200 MB (vector-only slim) |

## Current validated state

- Docker image builds and runs.
- sedonadb loads in DuckDB via vendored pg_ducklake.
- DuckLake tables work with spatial queries.
- Persistence across restarts confirmed.
- Three-stage query parity confirmed.

See `docs/COMPATIBILITY.md` for what works and what doesn't.
