# Architecture and design

How the `sedonadb` extension is layered, what each layer promises, and the
design for spatial workloads on DuckLake / Hive-partitioned stores.

## Product goals

1. **PostGIS workload portability.** Existing PostGIS analysis SQL ports with
   mostly mechanical rewrites. The `ST_*` namespace matches PostGIS/SedonaDB
   names, argument order, units, NULL behavior, and edge-case semantics where
   feasible; every delta is documented and fixture-backed.
2. **DuckDB/DuckLake-native spatial lakehouse.** Spatial data scales in
   DuckLake the way Sedona scales in Spark — via deterministic spatial
   partition keys, bbox zone-map columns, space-filling-curve clustering, and
   explicit cell-covering query patterns. The ceiling target is PB/trillion-row
   workloads with 100 MB–1 GB Parquet objects, not tiny row-count partitions.
   No hidden planner hooks, no extension-owned index state.

These are separate layers. Compatibility functions never depend on layout
helpers; layout helpers never change `ST_*` semantics. PostGIS is named first in
product positioning because it is the most common migration source; SedonaDB is
first-class as the literal kernel bridge and scale-design reference.

Status: the v2.0 target acceptance gates are met (ROADMAP Milestone 20;
CHANGELOG v2.0.0 gate audit). This document describes the design that must stay
true post-target.

## Layer model

```
┌──────────────────────────────────────────────────────────────┐
│ L4  Workload recipes & harnesses                             │
│     EXAMPLES.md · EXAMPLES_DUCKLAKE.md · tests/postgis_port/ │
│     migration tools · upstream fixture import                 │
├──────────────────────────────────────────────────────────────┤
│ L3  Spatial layout primitives (DuckLake/Hive-facing)         │
│     bbox columns · ST_QuadKey/ST_GeoHash · covering cells    │
│     ST_Hilbert/ST_Morton sort keys · partition spec tables   │
├──────────────────────────────────────────────────────────────┤
│ L2  PostGIS/SedonaDB-compatible SQL surface (`st_*`)         │
│     constructors · accessors · predicates · overlay ·        │
│     validity · editing · geodesy · aggregates · table fns    │
├──────────────────────────────────────────────────────────────┤
│ L1  Backends (canonical engines, narrow boundaries)          │
│     literal SedonaDB bridge (`sedona_st_*`) · local geo ·    │
│     GEOS · PROJ · GeographicLib · GDAL                       │
├──────────────────────────────────────────────────────────────┤
│ L0  Vectorized dispatch pipeline                             │
│     dispatch.rs executors · registry.rs macro registry ·     │
│     geometry.rs WKB ⇄ geo-types (trust boundary)             │
└──────────────────────────────────────────────────────────────┘
```

### L0 — dispatch pipeline

One generic executor per result shape; one registry line per SQL function.
`geometry.rs` is the trust boundary: all WKB entering the extension is parsed
and validated there; unparseable input yields NULL, never a panic and never
fabricated geometry. See README "Design" for the executor/macro details.

### L1 — backends

Boring and canonical, each behind a small module boundary:

| Backend | Owns | Module |
|---|---|---|
| Literal SedonaDB bridge | matching SedonaDB kernels (canonical where routed) | `bridge.rs` |
| local `geo` crate | functions SedonaDB lacks; PostGIS extras | `functions.rs` |
| GEOS | hard planar topology; DE-9IM `ST_Relate` matrix/pattern; overlay panic fallback | `geos_backend.rs` |
| PROJ | CRS transforms | `functions.rs::transform` |
| GeographicLib/Karney | spheroid geodesics | `functions.rs` |
| GDAL | raster I/O | `raster.rs` |

Routing policy and the intentionally-local allowlist are enforced by
`tools/catalog_audit.py --compat-check`.

### L2 — compatibility surface

The public `st_*` namespace. Portability rules:

- PostGIS names/arity/units/NULL behavior where feasible.
- Every semantic delta is listed in COMPATIBILITY.md with a fixture.
- PostgreSQL-isms that DuckDB cannot host are **documented mechanical
  rewrites**, never partial emulation:

| PostGIS-ism | Rewrite |
|---|---|
| `a && b` | bbox column predicate (`xmax >= … AND xmin <= …`) |
| `a <-> b` KNN | `ORDER BY st_distance(a, b) LIMIT k` |
| `x::geometry`, `geometry(Point,4326)` typmods | WKB BLOB + explicit `ST_SetSRID` |
| `CREATE INDEX … USING gist` | L3 layout: bbox + cell + sort columns |
| `ST_MemUnion` etc. | aggregate equivalents (`ST_Union_Agg`) |

DuckDB C-API extensions cannot register binary operators — `&&`/`<->` will
never exist here; the harness in `tests/postgis_port/` proves the rewrites, and
rewrite tooling flags these patterns in user SQL with suggested rewrites. The
near-term design is a shared Rust `sqlparser-rs` AST rewriter used by CLI tools,
SQL helper functions, and upstream-test import. Regex rewriting remains only a
fallback/linter because it cannot safely parse complex operands.

### Rust-first PostGIS rewrite engine

The extension cannot transparently intercept invalid PostGIS SQL: DuckDB parses
SQL before extension functions run, so syntax like `a && b`, `<->`, typmods, and
PostgreSQL casts must be rewritten before execution. The intended architecture is
one shared Rust rewrite engine exposed through multiple surfaces:

| Surface | Purpose | Notes |
|---|---|---|
| `sedonadb-rewrite` CLI | file/stdin migration | best UX for real migrations and upstream corpus import |
| `sedonadb_rewrite_postgis(sql)` scalar | notebooks/docs/tests | rewrites SQL text, not transparent execution |
| `sedonadb_rewrite_file(path)` table fn | migration review reports | returns line/rule/confidence/original/rewritten |
| upstream importer | external fixtures | ports PostGIS/SedonaDB tests with same rewrite engine |

Implementation target: `sqlparser-rs` (crate `sqlparser`, already present via
DataFusion) with PostgreSQL dialect parsing and AST transforms. High-confidence
rewrites are AST-only (`BinaryOp`, `Cast`, `ColumnDef`, `CreateIndex`,
`Function`); low-confidence semantic changes remain diagnostics.

Validation from reference repos:

- `sqlparser-rs` already recognizes PostgreSQL `&&` as
  `BinaryOperator::PGOverlap` and parses custom PG operators/casts, so the
  operator/cast rewrite plan is realistic in Rust.
- DuckDB Spatial's MVT tests use record/layer-oriented `ST_AsMVT` plus
  `ST_Read_Meta` round-trips. Our scalar MVT encoder is useful, but a future
  multi-feature MVT layer API should follow that shape rather than adding
  bespoke SQL.
- DuckLake's upstream transaction tests cover conflict/time-travel semantics;
  our spatial tests should reuse those transaction patterns and only add spatial
  key/query assertions.

### L3 — spatial layout primitives

Design constraints (verified against DuckLake docs):

1. **DuckLake partition transforms are `identity`, `bucket(N, col)`,
   `year/month/day/hour` only.** There is no expression partitioning, so every
   spatial partition key must be a **materialized column**.
2. **Pruning is per-file column min/max stats** (`ducklake_file_column_stats`
   zone maps). DOUBLE bbox columns prune well only when file contents are
   spatially clustered — the space-filling-curve sort key is load-bearing.
3. **DuckLake's catalog is the source of truth** for files, partitions, and
   multi-writer commits. The extension contributes only pure, deterministic
   functions over geometry; it holds no index state, no sidecar files, no
   shared mutable memory.

Primitives (Milestone 9+):

| Function | Kind | Contract |
|---|---|---|
| `ST_BBoxIntersects(a, b)` | predicate | envelope-only intersection; cheap prefilter |
| `ST_QuadKey(geom, zoom)` | scalar key | envelope-center cell at `zoom`; deterministic; NULL for NULL/EMPTY; assumes EPSG:4326 lon/lat |
| `ST_GeoHash(geom, precision)` | scalar key | PostGIS-compatible geohash |
| `ST_CoveringQuadKeys(geom, zoom[, max_cells])` | table fn | all cells intersecting the envelope; **fails closed** above `max_cells` |
| `ST_Hilbert(geom, bits)` / `ST_Morton(geom, bits)` | sort key | space-filling-curve value for clustering writes |
| `ST_TileEnvelope(z, x, y)` | constructor | PostGIS-compatible; Web Mercator default |
| `ST_EstimatePartitionCount(total_rows, avg_row_bytes, target_object_bytes)` | scalar sizing helper | estimates partition count for target Parquet object size |
| `ST_RecommendZoom(n_partitions)` | scalar sizing helper | recommends a quadkey zoom for an estimated partition count |
| Adaptive partition spec SQL recipe | workflow | histogram cells → Hilbert/quadkey sort → cumulative pack into plain spec rows |

Key rules:

- **Deterministic.** Same input → same key, across sessions and writers. This
  is what makes multi-writer DuckLake appends safe without coordination.
- **Cheap.** Envelope-based, not centroid/interior-point-based (collections and
  empties make centroids expensive or NULL-prone).
- **Fail closed.** Oversized covering requests error; they do not silently
  truncate.
- **CRS-explicit.** Cell keys assume lon/lat EPSG:4326; out-of-range
  coordinates yield NULL. Antimeridian-crossing envelopes are not split in v1
  (documented, fixture-pinned).
- **Adaptive specs are plain data.** Sort-then-pack specs are ordinary rows
  `(partition_id, cell_min, cell_max, total_rows)` — inspectable, joinable, and
  storable inside DuckLake itself.

### L4 — canonical workload pattern

Table layout:

```sql
CREATE TABLE parcels AS
SELECT
    *,                                   -- attributes
    geom,                                -- WKB BLOB
    st_xmin(geom) AS xmin, st_ymin(geom) AS ymin,
    st_xmax(geom) AS xmax, st_ymax(geom) AS ymax,   -- zone-map columns
    st_quadkey(geom, 8)  AS spatial_cell,           -- partition key
    st_hilbert(geom, 16) AS spatial_sort            -- clustering key
FROM source
ORDER BY spatial_sort;                   -- cluster files spatially
```

DuckLake partitioning (identity or bucketed for skew):

```sql
ALTER TABLE parcels SET PARTITIONED BY (spatial_cell);
-- or: ALTER TABLE parcels SET PARTITIONED BY (bucket(64, spatial_cell));
```

Three-stage query — partition pruning, then zone-map pruning, then exact:

```sql
WITH q AS (SELECT st_geomfromtext('POLYGON(...)') AS geom),
cells AS (SELECT quadkey FROM st_covering_quadkeys((SELECT geom FROM q), 8))
SELECT p.*
FROM parcels p, q
WHERE p.spatial_cell IN (SELECT quadkey FROM cells)   -- 1. partition prune
  AND p.xmax >= st_xmin(q.geom) AND p.xmin <= st_xmax(q.geom)
  AND p.ymax >= st_ymin(q.geom) AND p.ymin <= st_ymax(q.geom)  -- 2. zone maps
  AND st_intersects(p.geom, q.geom);                  -- 3. exact predicate
```

Stages 1–2 are performance filters; stage 3 alone defines correctness. Dropping
stages 1–2 changes speed, never results.

### Multi-writer model (Milestone 12)

- DuckLake catalog: commit atomicity, conflict resolution, snapshot isolation.
- Extension: pure functions only — identical partition keys from any writer.
- Partition evolution: DuckLake keeps old files under their old partitioning;
  queries stay correct across mixed layouts because stage 3 is exact.

## Product rules

1. **No silent wrong geometry.** NULL, documented limitation, or unimplemented —
   never approximate output that looks authoritative.
2. **Exact predicates always last.** Cells and bboxes filter; geometry decides.
3. **No hidden planner magic.** All pruning is visible in the SQL.
4. **No high-cardinality partition explosions by default.** Coarse zoom +
   `bucket(N, …)` is the documented bootstrap; adaptive specs are the intended
   path for skewed PB-scale datasets. Target 100 MB–1 GB Parquet objects, not
   a fixed number of rows per partition.
5. **DuckLake metadata stays authoritative.** Any spatial stats sidecar is a
   refreshable accelerator, never a source of truth.
6. **Compatibility and layout stay separate layers.** Layout helpers are
   extension-native and labeled as such in COMPATIBILITY.md.
7. **Fail closed at trust boundaries.** Invalid WKB, oversized cell coverings,
   unsupported CRS ranges → NULL or error, never panic or truncation.

## Generated compatibility artifacts and CI gates

The compatibility contract is mechanically audited, not manually claimed.
`tools/catalog_audit.py` parses `src/registry.rs` and the upstream SedonaDB
kernel inventory, and generates:

| Artifact | Purpose |
|---|---|
| `docs/SEDONA_LEDGER.md` | classification of every upstream SedonaDB kernel and live-registry function (routed / intentionally-local / bridge-only / not-bridgeable) |
| `docs/sedonadb_compat.json` | machine-readable export of the same data for tools and release notes |
| doc counts in README/ROADMAP/COMPATIBILITY | drift-checked against the live registry |

CI gates:

- `ci/check.sh` — drift gate: catalog counts, routing/compat classification,
  ledger freshness, JSON export freshness. Fails on any docs/registry skew.
- `ci/all-checks.sh` — full 5-phase pipeline: Rust unit tests → drift gate →
  SQL regression suite (standard + macro + DuckLake phases, DuckDB errors
  counted as failures) → package-and-smoke (18 backend checks from a packaged
  `.duckdb_extension`) → scale-harness smoke tier (exact-result parity oracle).

Generated artifacts are committed; regenerating them must be a no-op on a clean
checkout or CI fails.

## Testing strategy by layer

| Layer | Evidence |
|---|---|
| L0/L1 | Rust unit tests; SQL regressions; GEOS fallback fixtures |
| L2 | parity fixtures (`st_*` == `sedona_st_*`); PostGIS port harness (`tests/postgis_port/`, M8); DE-9IM/overlay fidelity audit (M14); generated SedonaDB ledger + JSON export (M15/M19); migration workbook + rewriter fixtures (M19); upstream PostGIS/SedonaDB curated imports (planned M26); COMPATIBILITY deltas |
| L3 | determinism fixtures; boundary/empty/NULL/antimeridian pins; covering-vs-envelope verification; adaptive-layout checks |
| L4 | DuckLake round-trip (M10); workload report (M13); multi-writer test (M12); layout benchmarks; M17 scale harness; SpatialBench snapshots |

All of the above run through `ci/all-checks.sh`; see
[docs/DEPENDENCIES.md](./docs/DEPENDENCIES.md) for the pipeline and load
diagnostics.

### Upstream-first fixture policy

Local bespoke SQL should shrink over time. Prefer fixtures in this order:

1. **Upstream PostGIS regress fixtures** for common analysis semantics.
2. **Upstream SedonaDB SQL/kernel fixtures** for literal bridge/routing parity.
3. **Generated compatibility fixtures** from the live registry/ledger.
4. **Local custom fixtures** only for DuckDB-specific shapes, discovered deltas,
   trust-boundary regressions, and workload/layout behavior upstream does not
   cover.

When an upstream fixture covers a local custom case exactly, keep the upstream
case and delete or narrow the local case. Custom tests should explain why an
upstream source cannot cover the behavior.

### Test consolidation and risk tiers

Speedups should not hide crashes. The attempted batch runner exposed a real
aggregate `ORDER BY` segfault (`st_collect(g ORDER BY ...)`,
`st_makeline_agg(g ORDER BY ...)`), so runner hardening comes before batching.

Risk tiers:

| Tier | Scope | Execution policy |
|---|---|---|
| A | Stateless deterministic SQL fixtures | batch or parallelize once crash handling is fixed |
| B | Macro/helper state | isolated session with macro preload |
| C | DuckLake stateful tests | isolated session/catalog cleanup |
| D | PostgreSQL container, packaging, scale, SpatialBench | separate opt-in/release phases |

Runner requirements before broad batching:

- non-zero DuckDB process exits and signals count as failures;
- failure output identifies file and test label;
- `SEDONA_TEST_MODE=isolated` remains available for debugging;
- low-risk batching never includes stateful DuckLake/PostgreSQL/package tests.

## Non-goals

- PostgreSQL planner/operator compatibility (`&&`, `<->`, GiST hooks).
- PostGIS topology schema, Tiger geocoder, address standardizer.
- SFCGAL 3D solids before mature Rust bindings.
- A custom raster expression language (DuckDB SQL is the map-algebra engine).
- Extension-owned spatial index files or any authoritative state outside
  DuckDB/DuckLake tables.
