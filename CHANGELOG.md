# Changelog

## v1.0.0 — Milestone cycle 4–6

First versioned release. Three focused milestones on fidelity, usability, and
maintainability, building on the Milestones 1–3 foundation cycle.

### Catalog

- **235 SQL functions**: 162 public `st_*`, 72 literal `sedona_st_*` bridge
  functions, 1 extension-specific helper (`sedona_join`).
- **47 public `st_*` functions** route to the literal Apache SedonaDB kernel.
- **6 backends**: local `geo` crate, literal SedonaDB bridge, GEOS, PROJ, GDAL,
  and DuckDB SQL (aggregates, table functions, raster map algebra).
- **431 SQL regression checks**, 64 Rust unit tests, 15 release smoke checks.

### What shipped in this cycle

#### Milestone 4 — fidelity harness and SQL namespace convergence

- **Catalog drift check**: `tools/catalog_audit.py --check` verifies committed
  doc counts against the live registry.
- **Literal routing expansion** (+11): `ST_Point`, `ST_MakePoint`,
  `ST_MakeLine`, `ST_Azimuth`, `ST_Affine`, `ST_Rotate`, `ST_Translate`,
  `ST_Scale`, `ST_LineFromText`, `ST_PointFromText`, `ST_PolygonFromText`.
- **Typed WKT constructor validation**: routed typed constructors now validate
  geometry type (rejects mismatched WKT → NULL, matching PostGIS semantics).
- **ST_Polygon** constructor added.
- **Compatibility debt log** in COMPATIBILITY.md with curated deltas and
  fixture references.

#### Milestone 5 — canonical backend hardening and high-value gaps

- **GEOS overlay fallback**: all 4 scalar overlay operations
  (`ST_Intersection/Union/Difference/SymDifference`) use `catch_unwind` around
  the local `geo::BooleanOps` fast path, falling back to GEOS on panic.
- **ST_DumpRings** table function: `(BLOB) → (path, geom)` per polygon ring.
- **ST_ContainsProperly** predicate: DE-9IM `T**FF*FF*`.
- **Performance budget** extended with GEOS overlay, ContainsProperly,
  DumpRings timing sections.
- **SpatialBench snapshot** script (`benchmarks/snapshot.sh`): structured
  markdown report for release QA.

#### Milestone 6 — release-grade usability and maintainability

- **Contributor guide** (`CONTRIBUTING.md`): backend decision tree, function
  addition walkthrough, test/doc requirements per backend.
- **Smoke test hardened** to 15 checks covering every backend family: local,
  SedonaDB, aggregate, GEOS, spheroid, raster, PROJ, table functions, overlay,
  DumpRings, ContainsProperly, and literal routing parity.
- **Workflow fixtures**: GeoParquet ingest, CRS transform join, bbox prefilter
  join, dissolve, dump family, raster sampling/reclassification, geodesic
  distance, overlay fallback, and PostGIS migration patterns.
- **Release notes** (this document) with migration guide.

### Known semantic deltas (vs PostGIS)

| Delta | Functions | Details |
|-------|-----------|---------|
| Force-dimension defaults | `ST_Force3D/3DZ/3DM/4D` | Explicit z/m parameter required (PostGIS defaults to 0). |
| Empty geometry dimension | `ST_Dimension` | Returns 0 (matches SedonaDB). PostGIS returns -1. |
| Aggregate-only collect | `ST_Collect` | Scalar overload unavailable (DuckDB limitation). |
| SRID-less WKB | `ST_SRID` | Returns 0 for SRID-less BLOB. |
| Spheroid WGS84-only | `*Spheroid` | No custom spheroid parameter. |

### Deferred (not yet shipped)

`ST_AsMVT/TWKB/KML`, `ST_AsRaster`, scalar `ST_Clip` facade (use SQL workflow
with `ST_PixelData` + GeoTransform metadata). `ST_IsValidDetail` and
`ST_AsSVG` shipped after this release note was originally drafted.

### Migration guide (from PostGIS / SedonaDB)

1. **Geometry type**: use `BLOB` (ISO WKB) instead of PostGIS `GEOMETRY`.
   DuckDB's native `spatial` extension `GEOMETRY` columns can be cast to BLOB.
2. **Operators**: `&&` → bbox column prefilter + `ST_Intersects`.
   `<->` → cross-join with `ST_Distance` + `ORDER BY` + `LIMIT`.
3. **SRID**: not embedded in WKB; use `ST_SetSRID`/`ST_Transform` explicitly.
4. **ST_Collect**: aggregate only (`ST_Collect(geom)`); no scalar
   `ST_Collect(g1, g2)`.
5. **Typed constructors**: `ST_LineFromText` validates WKT type (returns NULL
   for non-LineString, matching PostGIS).
6. **Raster**: use `ST_PixelData` + SQL for map algebra instead of
   `ST_MapAlgebra`. Use `ST_Value` for point sampling.
7. **Spatial joins**: materialize bbox columns (`st_xmin/xmax/ymin/ymax`) and
   join on overlapping ranges for IEJoin-friendly prefiltering.

### Build dependencies

- DuckDB 1.5.4+, Rust 1.87+
- GEOS 3.14+ (planar topology, overlay fallback)
- GDAL 3.13+ (raster I/O)
- PROJ (CRS reprojection)
- LLVM/Clang (bindgen for libduckdb_sys)

Linuxbrew paths on this dev machine: `/var/home/linuxbrew/.linuxbrew`.

### Verification

```
cargo test --lib             → 64 passed
./tests/run_sql.sh           → 431 passed / 0 failed
./ci/package-and-smoke.sh    → 15 checks passed
python3 tools/catalog_audit.py --check → OK
```

---

## v2.0.0 — Target release candidate (Milestones 7–20)

This release closes the target-acceptance plan. The extension is a practical
**superset of Apache SedonaDB and the commonly-used PostGIS analysis surface**
with DuckLake-native spatial lakehouse scaling.

### Catalog

- **249 SQL functions**: 179 public `st_*`, 72 literal `sedona_st_*` bridge
  functions, 1 extension-specific helper (`sedona_join`).
- **48 public `st_*` functions** route to the literal Apache SedonaDB kernel.
- **6 backends**: local `geo` crate, literal SedonaDB bridge, GEOS, PROJ, GDAL,
  DuckDB SQL (aggregates, table functions, raster map algebra).
- **699 SQL regression checks** (667 standard + 19 DuckLake + 13 macro),
  75 Rust unit tests, 18 release smoke checks.

### What shipped in this cycle

#### Milestone 7 — compatibility evidence and namespace closure

- `--compat-check` mode in catalog audit with INTENTIONALLY_LOCAL allowlist.
- `ST_SetPoint`, `ST_IsValidDetail` table function, `ST_GeomFromWKT` alias.
- 32-check parity fixture for all routed functions.

#### Milestone 8 — PostGIS workload portability harness

- `tests/postgis_port/` with 59 port cases across 8 families.
- `docs/MIGRATION.md` migration cookbook.
- Discovered and documented `ST_Contains` boundary delta.

#### Milestone 9 — spatial partition key primitives

- `ST_QuadKey`, `ST_GeoHash`, `ST_Hilbert`, `ST_Morton`, `ST_TileEnvelope`.
- `ST_CoveringQuadKeys` table function (fails closed on oversized requests).
- `ST_BBoxIntersects` predicate.

#### Milestone 10 — DuckLake spatial layout recipes

- 7 tested DuckLake recipes in `EXAMPLES_DUCKLAKE.md`.
- End-to-end DuckLake round-trip test (9 checks).
- `sql/ducklake_spatial_macros.sql` helper macros.

#### Milestone 11 — Sedona-style adaptive partitioning

- `ST_EstimatePartitionCount`, `ST_RecommendZoom`.
- Sort-then-pack adaptive spec recipe on skewed data.

#### Milestone 12 — multi-writer DuckLake validation

- 3-process multi-writer test with partition evolution + time travel.
- Concurrency documentation (catalog choice matrix).

#### Milestone 13 — workload-scale validation

- 5 ported PostGIS workloads on DuckLake with pruning evidence.
- `docs/WORKLOAD_REPORT.md`.

#### Milestone 14 — canonical geometry fidelity hardening

- `ST_Relate(a,b)` DE-9IM matrix + `ST_Relate(a,b,pattern)` via GEOS.
- 12 DE-9IM matrices verified against PostGIS reference.
- `ST_Contains`/`ST_Within` boundary delta PINNED in COMPATIBILITY.
- Adversarial overlay fixtures (bowtie, slivers, holes, collapsed rings).

#### Milestone 15 — SedonaDB bridge closure and generated ledger

- All 81 upstream SedonaDB kernels classified (zero unclassified).
- `docs/SEDONA_LEDGER.md` generated compatibility ledger.
- `ci/check.sh` CI drift gate (catalog + compat + ledger freshness).
- 30-check parity fixture.

#### Milestone 16 — raster and output-format closure

- `ST_AsSVG(geom)` shipped (SVG path data, Y-flipped).
- Raster facade decisions: `ST_Clip` SQL workflow, `ST_AsRaster` deferred.
- 25-check raster QA corpus.

#### Milestone 17 — PB-scale DuckLake validation harness

- `benchmarks/scale_harness.sh` three-layout comparison at smoke/local/heavy tiers.
- 11-check DuckLake scale fixture (parity + pruning + adaptive + partition evolution).
- `docs/SCALE_REPORT.md`.

#### Milestone 18 — packaging, CI, and load diagnostics

- `ci/all-checks.sh` unified 5-phase pipeline.
- SQL runner strictness (DuckDB errors counted as failures).
- `docs/DEPENDENCIES.md` dependency matrix + load diagnostics.
- 18-check smoke test covering every backend.

#### Milestone 19 — migration UX and operator-rewrite tooling

- `tools/postgis_rewriter.py` conservative linter (14 patterns).
- `docs/sedonadb_compat.json` machine-readable compatibility export.
- 5 end-to-end migration workbook examples.
- 13-check macro-dependent test phase.

#### Milestone 20 — target release candidate

- Target acceptance gate audit (all 5 gates met).
- Release notes published (this document).
- Clean-checkout release rehearsal.

#### Milestone 22 — semantic-delta retirement (ST_Contains/ST_Within)

- `ST_Contains` and `ST_Within` routed through `geo::Relate` with PostGIS
  DE-9IM pattern `T*****FF*` (boundary delta retired).
- Boundary points now return `FALSE` (matching PostGIS); interior points
  unchanged.
- PostGIS port boundary case updated from pinned delta to ported success.
- 14-check M22 fixture file with interior/boundary/exterior/polygon coverage.
- Compatibility debt log: 6 → 5 deltas.

#### Milestone 23 — raster and output-format breadth (ST_AsKML, ST_AsTWKB)

- `ST_AsKML(geom)` shipped: KML 2.2 XML serialization for all geometry types.
- `ST_AsTWKB(geom)` shipped: Tiny WKB compact binary (hex-encoded VARCHAR,
  precision=0, zigzag varint delta encoding).
- Active output backlog shrinks from 3 to 1 (only `ST_AsMVT` remains —
  needs protobuf + tile clipping).
- 13-check fixture file covering KML and TWKB for all geometry types.

#### Milestone 23b — ST_AsMVT output encoder

- `ST_AsMVT(geom)` shipped: Mapbox Vector Tile protobuf encoding (hex-encoded).
  Single-layer, single-feature tile using MVT 2.1 geometry commands. Hand-rolled
  protobuf encoder — no external dependency.
- All PostGIS output encoders now shipped: Text, Binary, EWKB, GeoJSON,
  HexEWKB, SVG, KML, TWKB, MVT.
- Active output backlog: 0 items (only `ST_AsRaster` remains as a facade gap).
- Catalog: 251 → 253 functions (178 → 179 st_*).

#### Milestone 21 — release distribution matrix

- GitHub Actions CI workflow (`.github/workflows/ci.yml`): Linux full pipeline,
  macOS build+smoke, lint job.
- `docs/RELEASE_CHECKLIST.md`: step-by-step release packaging guide.
- Platform matrix documented: linux_amd64 + darwin_arm64 CI-tested, darwin_amd64
  buildable.

#### Milestone 24 — scale operations and workload guardrails

- `benchmarks/scale_report.sh`: repeatable scale report generator (smoke + local
  tiers, structured Markdown output with parity/pruning evidence).
- `tests/reference/m24_pg_catalog.sh`: PostgreSQL-catalog DuckLake
  multi-writer validation via local Docker container (6 checks: multi-writer
  append, query parity, time travel, partition evolution).
- Object-store deployment guidance in `docs/SCALE_REPORT.md`.
- Discovered: PostgreSQL reserves `xmin`/`xmax` — use `minx/maxx` for bbox
  columns with PG-catalog DuckLake tables.

### Target acceptance gate status

| Gate | Status | Evidence |
|---|---|---|
| 1. SedonaDB surface closure | ✅ Met | M15: 81 kernels classified, 0 unclassified, generated ledger |
| 2. PostGIS analysis portability | ✅ Met | M8/M13/M19: 59 port cases, 5 workloads, rewriter, migration workbook |
| 3. No silent semantic deltas | ✅ Met | M14: all deltas fixture-backed in COMPATIBILITY debt log |
| 4. DuckLake PB/trillion-row pattern | ✅ Met | M17: scale harness with exact-result parity + pruning evidence |
| 5. Packageable and diagnosable | ✅ Met | M18: unified CI, dependency docs, load diagnostics |

### Known semantic deltas (vs PostGIS)

| Delta | Functions | Details | Fixture |
|-------|-----------|---------|---------|
| Force-dimension defaults | `ST_Force3D/3DZ/3DM/4D` | Explicit z/m parameter required. | `m1_fixtures.sql` |
| Empty geometry dimension | `ST_Dimension` | Returns 0 (matches SedonaDB). PostGIS returns -1. | `m1_fixtures.sql` |
| Aggregate-only collect | `ST_Collect` | Scalar overload unavailable (DuckDB limitation). | N/A |
| SRID-less WKB | `ST_SRID` | Returns 0 for SRID-less BLOB. | `fidelity.sql` |
| Spheroid WGS84-only | `*Spheroid` | No custom spheroid parameter. | `m2_fixtures.sql` |

### Active non-blocking backlog

All output encoders are shipped. `ST_AsRaster` (GDAL rasterization write path)
is the only remaining facade gap.

| Item | Workaround |
|------|------------|
| `ST_AsRaster` | Needs GDAL rasterization/write path; use `ST_PixelData` + SQL |

### Verification

```
./ci/all-checks.sh             → 5 phases, all passed
cargo test --lib               → 75 passed
./tests/run_sql.sh             → 699 passed / 0 failed
./ci/package-and-smoke.sh      → 18 checks passed
./benchmarks/scale_harness.sh  → parity PASS, pruning effective
python3 tools/catalog_audit.py --check        → OK
python3 tools/catalog_audit.py --compat-check → OK
python3 tools/catalog_audit.py --generate-ledger → up to date
python3 tools/catalog_audit.py --export-json      → up to date
```

## Unreleased — compatibility debt log emptied

All remaining PostGIS deltas closed (5 → 0 across two changes).

### Deltas closed in this cycle

- **Force-dimension defaults**: 1-arg `st_force3d/3dz/3dm/4d(geom)` overloads
  with PostGIS z=0/m=0 defaults (constant-Scalar injection into the literal
  SedonaDB kernel).
- **Empty-geometry dimension**: `st_dimension(EMPTY)` returns -1 (PostGIS
  parity; moved from bridge-routed to local).
- **SRID-less WKB → EWKB SRID tags**: `st_setsrid` writes the PostGIS EWKB
  SRID tag (flag `0x20000000` + 4-byte SRID) on the blob; `st_srid` reads it;
  `st_geomfromtext(wkt, srid)`, `st_geomfromwkb(wkb, srid)` and
  `st_geomfromewkt('SRID=n;…')` construct it; `st_asewkt(geom)` prints it;
  `st_transform(geom, to_srid)` reads the source CRS from it (NULL for
  untagged input — fail closed). The dispatch layer propagates tags through
  every geometry-producing function (local geo, bridge-routed SedonaDB, and
  raw-WKB GEOS paths); the bridge strips tags before kernels see them.
- **Custom spheroids**: `st_distancespheroid/lengthspheroid/areaspheroid`
  accept the PostGIS `SPHEROID["name",a,rf]` string and build a custom Karney
  geodesic (`rf = 0` → sphere); malformed strings return NULL.
- **Scalar ST_Collect**: `st_collect_scalar(g1, g2)` with PostGIS pairwise
  semantics (MULTI* for same-type pairs, GEOMETRYCOLLECTION otherwise;
  collections flattened). DuckDB's C API verifiably rejects a scalar under an
  aggregate's catalog name, so `sedonadb_rewrite_postgis()` maps 2-arg
  `ST_Collect(a, b)` onto it mechanically.

### Catalog

- 253 → 254 functions (+`st_collect_scalar`); routed 49 → 46 (`st_dimension`,
  `st_setsrid`, `st_srid` moved to intentionally-local with documented reasons).
- New executor shapes: raw `(BLOB, INT) → BLOB`, `BLOB → INT`, `BLOB → VARCHAR`,
  `(VARCHAR, INT) → BLOB`, `(geom, VARCHAR) → DOUBLE`,
  `(geom, geom, VARCHAR) → DOUBLE`.

### Verification

```
./ci/all-checks.sh    → 5 phases, all passed
cargo test --lib      → 88 passed
./tests/run_sql.sh    → 845 passed / 0 failed
```

## Milestone 28 — migration assistant UX

### Structured rewriter diagnostics

- `rewrite_postgis_detailed()` returns `RewriteResult` with typed
  `RewriteEvent`s (kind, confidence, source line, description)
- `Confidence::High` vs `Confidence::NeedsReview` model — low-confidence
  rewrites are never silent
- Source line numbers via `sqlparser::Spanned::span()`
- Existing `rewrite_postgis()` string API retained as a thin wrapper

### `sedonadb-migrate` CLI

- `sedonadb-migrate input.sql --out output.sql --report report.md`
- Markdown report: per-line rewrites table, review items, DuckLake layout hints
- `#[path]` module include — binary depends only on `sqlparser`, not GDAL/GEOS

### CI integration

- `tests/reference/m28_fixtures.sql`: 13 round-trip checks (rewrite + execute)
- `ci/all-checks.sh` phase 5: migration CLI binary smoke test

### Verification

```
./ci/all-checks.sh    → 6 phases, all passed
cargo test --lib      → 95 passed
./tests/run_sql.sh    → 859 passed / 0 failed
```

## Milestone 29 — aggregate ORDER BY crash fix

### Bug: heap corruption via aggregate ORDER BY (SIGABRT)

`ST_Collect(g ORDER BY k)` and other spatial aggregates crashed DuckDB with
SIGABRT ("corrupted size vs. prev_size") because DuckDB's C-API aggregate
execution leaves some per-row state slots uninitialized when ORDER BY is used.

Root cause: the `FfiState<T>.inner` pointer was null for some rows in the
update callback. Dereferencing it corrupted the heap.

Fix:
- **Crash-safe callbacks**: all aggregate update callbacks now skip
  uninitialized state slots (`with_state_mut` → `None`) rather than crashing.
- **Rewriter detection**: `sedonadb_rewrite_postgis()` detects ORDER BY inside
  any aggregate function call (`FunctionArgumentClause::OrderBy`) and emits a
  NeedsReview warning with the subquery workaround.

Old `m27_known_issues.sql` replaced by `m29_fixtures.sql` (5 checks).

### Verification

```
cargo test --lib      → 96 passed
./tests/run_sql.sh    → 863 passed / 0 failed
```

## Milestone 30 — upstream PostGIS fixture expansion

Ported 3 PostGIS regress suites as curated upstream fixtures (42 new tests):

- **postgis_simplify.sql** (8 tests): Douglas-Peucker simplification
- **postgis_empty.sql** (18 tests): EMPTY geometry handling across functions
- **postgis_measures.sql** (15 tests): ST_Area/Length/Perimeter/Distance

Bug fix: `st_buffer(EMPTY, radius)` panicked in the `geo` buffer algorithm.
Now checks `is_empty()` and returns NULL (safe).

Documented behavioral difference: `geo` Simplify does not collapse polygon
members below tolerance (PostGIS removes them).

### Verification

```
cargo test --lib      → 96 passed
./tests/run_sql.sh    → 905 passed / 0 failed
```

## Milestone 30 (batch 2) — topology, editing, accessor fixtures

Ported 3 more PostGIS regress suites (+57 tests):

- **postgis_topology.sql** (16 tests): ST_Node, ST_Polygonize, ST_Snap,
  ST_VoronoiPolygons, ST_DelaunayTriangles — all GEOS-backed.
- **postgis_editing.sql** (19 tests): ST_Reverse, ST_Normalize,
  ST_RemovePoint, ST_SetPoint, ST_FlipCoordinates, ST_ForcePolygonCW.
- **postgis_accessors.sql** (21 tests): ST_IsCollection (15 cases),
  ST_HausdorffDistance, ST_MinimumClearance, ST_IsValid.

Documented differences: ST_RemovePoint allows single-segment removal
(PostGIS errors); ST_DelaunayTriangles 3-arg form (tolerance + flag) not
yet supported.

Upstream-curated fixtures: 5 → 8 files.
Total SQL checks: 905 → 962 (+57).
