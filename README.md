# duckdb_sedona

A DuckDB loadable extension that exposes a catalog of vector spatial (`ST_*`)
functions over WKB-encoded geometries, built in pure Rust with
[`quack-rs`](https://github.com/tomtom215/quack-rs) and the same
`wkb` / `geo` / `geo-traits` stack that
[Apache SedonaDB](https://github.com/apache/sedona-db) builds on.

Demand-backed target: a **highest-fidelity PostGIS + SedonaDB spatial engine for
DuckDB** with two first-class goals (see
[ARCHITECTURE.md](./ARCHITECTURE.md)):

1. **PostGIS workload portability** — existing PostGIS analysis SQL ports with
   mostly mechanical rewrites: literal Apache SedonaDB kernels as the canonical
   implementation wherever they exist, a familiar PostGIS-like `ST_*` SQL
   namespace, and explicit/documented semantic deltas where exact compatibility
   is not feasible.
2. **DuckLake-native spatial lakehouse** — deterministic spatial partition
   keys, bbox zone-map columns, and space-filling-curve clustering so spatial
   data partitions, prunes, and scales in DuckLake / Hive-partitioned stores
   the way Sedona scales in Spark — without hidden planner hooks or
   extension-owned index state.

This target is not just aspirational catalog breadth: roadmap work is prioritized
around user demand for PostGIS workload portability and DuckLake spatial
lakehouse operations, with unsupported edge cases documented instead of hidden.
PostGIS is named first because it is the most common migration source; SedonaDB
is the literal-kernel bridge and scale-design reference.

```sql
INSTALL sedonadb;
LOAD sedonadb;

-- geom is a BLOB containing ISO Well-Known Binary (e.g. a DuckDB `spatial`
-- GEOMETRY column). All ST_* functions consume and return WKB BLOBs.
SELECT st_geometrytype(geom)               -- 'ST_Polygon'
     , st_area(geom)                       -- 16.0
     , st_intersects(geom, other_geom)     -- true
FROM   my_table;
```

Licensed under the [Apache License 2.0](./LICENSE).

---

## Design — the Unified Vectorized Dispatch Pipeline

The core idea (from the project brief) is to **never write one FFI callback per
SQL function**. Instead there is:

1. **One generic executor per result shape** (`src/dispatch.rs`). Each reads
   WKB blobs out of DuckDB's columnar vectors, applies an operation, and writes
   the result back — handling NULL propagation explicitly.
2. **A declarative macro registry** (`src/registry.rs`) that maps every SQL
   name to one of those executors. **Adding a function is exactly one line.**

```
              DuckDB query
                   │  (BLOB column = WKB)
                   ▼
   ┌──────────────────────────────────────────────┐
   │ registry.rs  —  register_all(con)            │
   │   register_unary_geom!("st_centroid", ...)   │   ← one line per function
   │   register_predicate!("st_intersects", ...)  │
   └───────────────┬──────────────────────────────┘
                   │ generates a unique unsafe extern "C" fn per call
                   ▼
   ┌──────────────────────────────────────────────┐
   │ dispatch.rs  —  generic executors            │
   │   unary_geom / binary_geom / binary_predicate│
   │   unary_geom_double / _varchar / _int        │
   └───────────────┬──────────────────────────────┘
                   │ read_blob → from_wkb → f(geom) → to_wkb → write_blob
                   ▼
   ┌──────────────────────────────────────────────┐
   │ geometry.rs  —  WKB ⇄ geo_types::Geometry    │
   │ functions.rs —  geo-crate algorithms          │
   └──────────────────────────────────────────────┘
```

The `quack-rs` reader/writer wrappers (`VectorReader`, `VectorWriter`,
`DataChunk`) are the only code that touches DuckDB's in-memory vector format,
so the pipeline stays vectorized: each callback iterates a whole chunk
(DuckDB's standard 2048 rows) with no per-row allocations beyond the geometry
itself.

### Why this maps cleanly to the upstream APIs

The brief sketched the architecture as `Fn(&Geometry) -> Geometry` callbacks
passed straight into `.function(|i, c, o| …)`. Against the *real* crates that
shape needs two adjustments, both faithful to the intent:

| Draft assumption | Reality | What this extension does |
| --- | --- | --- |
| `sedona_db::core::Geometry` concrete type | Apache SedonaDB has **no** such type; it represents geometry as **WKB bytes** and parses via the `wkb` crate into `geo-traits`. | We use `geo_types::Geometry<f64>` (the `geo` crate's concrete enum) and convert WKB ⇄ that type in `src/geometry.rs`, porting SedonaDB's `to_geo` converter. |
| `.function(\|info, chunk, out\| { … })` closure | `quack-rs`' `ScalarFunctionBuilder::function` takes an `unsafe extern "C" fn` **pointer**, not a closure. | The `register_*!` macros mint one unique `unsafe extern "C" fn` per registration (each in its own block scope), forwarding to a monomorphized generic executor. One line of registry per function is preserved. |
| Route DuckDB vectors "straight into SedonaDB's registry" | SedonaDB's functions are **DataFusion columnar UDFs** over Arrow arrays, not per-row geometry fns. | We reuse the *same algorithmic stack* SedonaDB uses (`geo`, `wkb`) and run it directly on DuckDB vectors. **We now *also* link the real SedonaDB DataFusion UDFs and invoke them through a DuckDB-chunk ⇄ Arrow bridge** — see [Literal SedonaDB bridge](#literal-sedonadb-bridge). |

So the deliverable honours the brief's architecture — unified vectorized
dispatch + declarative macro registry, low-maintenance single-line registration
— against the actual `quack-rs` C-API and the actual georust geometry stack.

---

## Function catalog

All functions take **BLOB** (ISO WKB) arguments. Geometry-returning functions
return **BLOB** (ISO WKB). `NULL` in → `NULL` out; an input that fails to parse
or an operation that is undefined for the input also yields `NULL`. The catalog
covers constructors, accessors, predicates, measurements, set ops, transforms,
validity, five aggregates (`st_collect`, `st_envelope_agg`, `st_union_agg`,
`st_makeline_agg`, `st_intersection_agg`), geodesic/geography
(`st_distancesphere`, `*spheroid`, …),
EWKT/EWKB/SRID, affine & segmentize transforms, line editing, GEOS topology,
raster transform/statistics/pixel streaming/value sampling, a set-returning
`ST_Dump` family (`st_dump`/`st_dumppoints`/`st_dumpsegments`/`st_dumprings`),
and I/O.

Current registry audit: **254 SQL functions** — 180 public `st_*`, 72 literal
`sedona_st_*` bridge functions, and one extension-specific helper
(`sedona_join`). 46 public `st_*` functions route to the literal SedonaDB kernel.
See **[COMPATIBILITY.md](./COMPATIBILITY.md)** for the full PostGIS/SedonaDB
compatibility table. Run `python3 tools/catalog_audit.py` to regenerate counts.

See **[ARCHITECTURE.md](./ARCHITECTURE.md)** for the layer model, the DuckLake
spatial layout design, and product rules. See **[ROADMAP.md](./ROADMAP.md)** for
a category-level capability matrix, the v2.0 target audit, and the post-target
milestone plan. See **[EXAMPLES.md](./EXAMPLES.md)** and
**[EXAMPLES_DUCKLAKE.md](./EXAMPLES_DUCKLAKE.md)** for copy-pasteable workflows;
`sql/ducklake_spatial_macros.sql` provides optional macros to reduce DuckLake
layout/query boilerplate. The literal SedonaDB bridge (below) means the real
Apache SedonaDB kernels now run inside DuckDB, not just a reimplementation of
them.

| SQL function | Signature | Backed by |
| --- | --- | --- |
| `st_convexhull` | `(BLOB) → BLOB` | `geo::ConvexHull` |
| `st_envelope` | `(BLOB) → BLOB` (Polygon) | `geo::BoundingRect` |
| `st_centroid` | `(BLOB) → BLOB` (Point) | `geo::Centroid` |
| `st_intersection` | `(BLOB, BLOB) → BLOB` (MultiPolygon) | `geo::BooleanOps` |
| `st_union` | `(BLOB, BLOB) → BLOB` (MultiPolygon) | `geo::BooleanOps` |
| `st_difference` | `(BLOB, BLOB) → BLOB` | `geo::BooleanOps` |
| `st_symdifference` | `(BLOB, BLOB) → BLOB` | `geo::BooleanOps` |
| `st_makeline` | `(BLOB, BLOB) → BLOB` | point pair → LineString |
| `st_intersects` | `(BLOB, BLOB) → BOOLEAN` | `geo::Intersects` |
| `st_contains` | `(BLOB, BLOB) → BOOLEAN` | `geo::Contains` |
| `st_within` | `(BLOB, BLOB) → BOOLEAN` | `geo::Contains` (reversed) |
| `st_disjoint` | `(BLOB, BLOB) → BOOLEAN` | `geo::Intersects` (negated) |
| `st_dwithin` | `(BLOB, BLOB, DOUBLE) → BOOLEAN` | distance ≤ threshold |
| `st_geomfromtext` | `(VARCHAR) → BLOB` | WKT parser (`wkt`) |
| `st_astext` | `(BLOB) → VARCHAR` | WKT writer (`wkt`) |
| `st_point` | `(DOUBLE, DOUBLE) → BLOB` | point constructor |
| `st_geomfromwkb` | `(BLOB) → BLOB` | validate + normalize WKB |
| `st_area` | `(BLOB) → DOUBLE` | `geo::Area` |
| `st_length` | `(BLOB) → DOUBLE` | `geo::EuclideanLength` |
| `st_distance` | `(BLOB, BLOB) → DOUBLE` | `geo::EuclideanDistance` |
| `st_buffer` | `(BLOB, DOUBLE) → BLOB` | `geo::Buffer` |
| `st_simplify` | `(BLOB, DOUBLE) → BLOB` | `geo::Simplify` (RDP) |
| `st_x` / `st_y` | `(BLOB) → DOUBLE` | point ordinate |
| `st_numpoints` | `(BLOB) → INTEGER` | vertex count |
| `st_isvalid` | `(BLOB) → BOOLEAN` | `geo::Validation` |
| `st_isempty` | `(BLOB) → BOOLEAN` | empty geometry |
| `st_makevalid` | `(BLOB) → BLOB` | GEOS `make_valid` |
| `st_geometrytype` | `(BLOB) → VARCHAR` | variant match |
| `st_dimension` | `(BLOB) → INTEGER` | OGC dimension |
| `st_affine` | `(BLOB, DOUBLE×6) → BLOB` | 2D affine matrix |
| `st_segmentize` | `(BLOB, DOUBLE) → BLOB` | split long segments |
| `st_linesubstring` | `(BLOB, DOUBLE, DOUBLE) → BLOB` | line slice by fraction |
| `st_linemerge` | `(BLOB) → BLOB` | chain touching linestrings |
| `st_collectionextract` | `(BLOB, INTEGER) → BLOB` | filter collection by dim |
| `st_forcecollection` | `(BLOB) → BLOB` | wrap in GeometryCollection |
| `st_multi` | `(BLOB) → BLOB` | promote to multi type |
| `st_normalize` | `(BLOB) → BLOB` | canonical vertex order |
| `st_triangulatepolygon` | `(BLOB) → BLOB` | Delaunay interior triangulation |
| `st_maxdistance` | `(BLOB, BLOB) → DOUBLE` | greatest vertex-vertex distance |
| `st_longestline` | `(BLOB, BLOB) → BLOB` | line realizing max distance |
| `st_shortestline` | `(BLOB, BLOB) → BLOB` | line realizing min distance |
| `st_orderingequals` | `(BLOB, BLOB) → BOOLEAN` | structural (coord-order) equality |
| `st_union_agg` | `(BLOB) → BLOB` (aggregate) | cascaded polygonal union |
| `st_makeline_agg` | `(BLOB) → BLOB` (aggregate) | points → LineString |
| `st_makeenvelope` | `(DOUBLE×4) → BLOB` | bbox → Polygon |
| `st_makepolygon` | `(BLOB) → BLOB` | closed LineString → Polygon |
| `st_removepoint` / `st_addpoint` | `(BLOB, INT)` / `(BLOB, BLOB)` | line editing |
| `st_simplifypreservetopology` | `(BLOB, DOUBLE) → BLOB` | RDP with validity fallback |
| `st_minimumclearance` / `...line` | `(BLOB) → DOUBLE` / `BLOB` | vertex-move clearance |
| `st_minimumboundingcircle` | `(BLOB, DOUBLE) → BLOB` | Welzl min enclosing circle |
| `st_generatepoints` | `(BLOB, INTEGER) → BLOB` | seeded random points in polygon |
| `st_isvalidreason` | `(BLOB) → VARCHAR` | structural validity report |
| `st_dump` / `st_dumppoints` / `st_dumpsegments` | `(BLOB)` → table | set-returning dump (path, geom) |

### Adding a function

Add the implementation to `src/functions.rs` and append **one line** to the
catalog in `src/registry.rs`:

```rust
register_unary_geom!("st_simplify", functions::simplify);
```

No FFI boilerplate, no per-function callback. That is the whole maintenance
surface.

---

## Build

Requires Rust ≥ 1.87 (a toolchain is provided automatically by most setups).

```sh
cargo build --release
# → target/release/libsedonadb.so   (Linux)
# → target/release/sedonadb.dylib   (macOS)
# → target/release/sedonadb.dll     (Windows)
```

The build does **not** require DuckDB itself: `libduckdb-sys` is used with its
`loadable-extension` feature, so the DuckDB host process supplies the C-API
symbols at load time. The produced shared object exports the entry-point symbol
`sedonadb_init_c_api`, which DuckDB looks up for an extension named `sedonadb`.

## Build & runtime dependencies

| Dependency | When | What it enables | Notes |
|------------|------|-----------------|-------|
| DuckDB 1.5.x | runtime host | loadable-extension host | ABI is 1.5.4 (`libduckdb-sys 1.10504.0`); built with the `loadable-extension` feature |
| **`quack-rs`** | build (statically linked) | DuckDB C-API SDK; **vendored + patched** | upstream's `VectorReader::read_blob` validated UTF-8 and dropped binary WKB bytes (the README's original bug); the vendored copy fixes `read_blob` at the source and adds `Value::as_blob` for table-function bind params. See [vendor/quack-rs/PATCHES.md](./vendor/quack-rs/PATCHES.md). |
| **PROJ (`libproj`)** | **build**: bundled & **statically linked** (`proj-sys/bundled_proj` + `libsqlite3-sys/bundled`) | `ST_Transform` (CRS reprojection) | Our own PROJ is **static** → `ST_Transform` has **no runtime dep** of its own. (But GDAL, below, brings its own dynamic libproj.) |
| **GDAL (`libgdal` ≥ 3.13)** | **build (`pkg-config gdal` + `LIBCLANG_PATH` for bindgen) + runtime (`LD_LIBRARY_PATH`)** | **Raster** (`st_raster_info`, `st_raster_stats`, `st_raster_transform`, `st_pixeldata`) via a **vendored + patched** `gdal` 0.19 crate (only the high-level `gdal` crate is vendored; `gdal-sys` comes unpatched from crates.io — see `vendor/gdal/PATCHES.md`). | `LOAD` needs `libgdal.so` (and its transitive libs) resolvable. |
| **GEOS (`libgeos_c`)** | build (`pkg-config geos`) + runtime | PostGIS-grade topology/editing: `st_node`, `st_polygonize`, `st_buildarea`, `st_voronoipolygons`, `st_snap`, `st_makevalid` | same canonical engine PostGIS uses for these hard planar-topology operations |
| GeographicLib (`geographiclib-rs`) | build only (statically linked) | Karney spheroid geodesics: `st_distancespheroid`, `st_lengthspheroid`, `st_areaspheroid`, `st_dwithinspheroid` | converges on antipodal inputs |
| arrow / parquet (Rust crates) | build only (statically linked) | `sedona_join` (R-tree spatial join over spilled parquet) | no runtime dep |
| **Apache SedonaDB** (`sedona-functions` + `sedona-expr`/`schema`, git rev `b23ccd15`) + `datafusion-expr`/`-common` | build only (statically linked) | the **literal SedonaDB bridge** — real SedonaDB DataFusion UDFs invoked via DuckDB⇄Arrow (`src/bridge.rs`) | pure-Rust; no GDAL/PROJ/GEOS added |
| `geo`/`wkb`/`wkt`/`geo-traits`/`delaunator`/`rstar` | build only | the ST_* geometry surface, Delaunay/Voronoi, the R-tree | pure-Rust, statically linked |

**Build the extension** (PROJ bundled static; GDAL present):
```sh
export PKG_CONFIG_PATH="$(brew --prefix gdal)/lib/pkgconfig"   # gdal.pc (for bindgen)
export LIBCLANG_PATH="$(brew --prefix llvm)/lib"               # bindgen needs libclang
cargo build --release
./target/release/sedonadb-package target/release/libsedonadb.so build/dev/sedonadb.duckdb_extension linux_amd64
```
**Load it** (GDAL/GEOS runtime libs):
```sh
LD_LIBRARY_PATH="$(brew --prefix gdal)/lib" duckdb -unsigned
duckdb> LOAD '/abs/path/sedonadb.duckdb_extension';
```
> If `libgdal` is in the standard library path (e.g. installed via the system
> package manager — `dnf install gdal`, `apt install libgdal-dev`), no
> `LD_LIBRARY_PATH` is needed; `LOAD` works directly. The extension ships the
> **full** capability set (vector + geodesic + CRS/PROJ + R-tree joins + raster)
> in a single `.duckdb_extension` — there is deliberately no slim/feature-gated
> variant.

### Reproducible build (container)

For a build with no dependency on a host Homebrew, `ci/` ships a reproducible
builder on the official GDAL 3.13.1 image. One-time image build, then build+test
inside it (cached cargo volumes across runs):

```sh
./ci/container-build.sh        # FROM ghcr.io/osgeo/gdal:ubuntu-full-3.13.1 + Rust 1.88 + clang
./ci/container-test.sh         # cargo test --lib + cargo build --release, GDAL/PROJ/clang in-image
```

## Load into DuckDB

DuckDB requires a 512-byte metadata trailer on the shared object before it will
accept a `LOAD`. The `sedonadb-package` binary (built from `src/bin/package.rs`)
appends it:

```sh
cargo build --release
./target/release/sedonadb-package \
    target/release/libsedonadb.so build/dev/sedonadb.duckdb_extension linux_amd64
```

Then load it in an unsigned DuckDB session (developed against **DuckDB 1.5.4**;
`libduckdb-sys 1.10504.0` maps 1:1 to DuckDB 1.5.4):

```sh
duckdb -unsigned
```
```sql
LOAD '/abs/path/to/sedonadb.duckdb_extension';
SELECT st_astext(st_geomfromtext('POINT(1 2)'));    -- POINT(1 2)
SELECT st_area(st_geomfromtext('POLYGON((0 0,4 0,4 4,0 4,0 0))'));  -- 16.0
SELECT st_distance(st_geomfromtext('POINT(0 0)'), st_geomfromtext('POINT(3 4)')); -- 5.0
```

> **Bug found while wiring up SQL tests.** ~half of all geometries came back
> `NULL` because the upstream `quack-rs` blob reader validates UTF-8 and returns
> empty for non-UTF-8 bytes — and WKB is arbitrary binary (e.g. a coordinate
> `1.0` encodes an `0xF0` byte that breaks UTF-8). **Fixed at the source:** we
> [vendor `quack-rs`](./vendor/quack-rs/PATCHES.md) and made
> `VectorReader::read_blob` binary-safe, so the dispatch layer reads WKB blobs
> directly with no work-around. Unit tests in `src/functions.rs` and the
> vendored crate pin this.

In practice you feed it any `BLOB` column containing ISO-WKB (e.g. a
SpatialBench parquet geometry column, or DuckDB `spatial`'s `GEOMETRY` storage).
Note: DuckDB 1.5 also ships its own `GEOMETRY`-returning `ST_GeomFromWKB`; to
avoid overload ambiguity, call sedonadb's functions on the raw `BLOB` column
directly (they accept WKB natively) and `CAST(... AS BLOB)` literals.

> **Runtime dependency:** the full-capability build links dynamic GDAL and GEOS
> (`libgdal.so`, `libgeos_c.so`, plus GDAL transitives as needed). Set
> `LD_LIBRARY_PATH` (or install those libs system-wide) before `LOAD`, e.g.
> `LD_LIBRARY_PATH=/opt/homebrew/lib duckdb -unsigned`. `ST_Transform` uses the
> bundled/static PROJ path; GDAL may still bring its own dynamic PROJ.

### PostGIS compatibility & common workflows

The `ST_*` namespace mirrors PostGIS names, arities, and units. Where Apache
SedonaDB has the same function, `st_*` routes to the **literal SedonaDB kernel**
(one implementation; `sedona_st_*` is the explicit provenance twin). A few
DuckDB-native shapes differ by necessity (noted below).

```sql
-- CRS reprojection (PROJ): EPSG:4326 → 3857 (web mercator)
SELECT st_astext(st_transform(st_geomfromtext('POINT(-0.1278 51.5074)'), 4326, 3857));
-- → POINT(-14227.16 6711542.71)

-- Geodesic distance (metres) over lon/lat points — no projection needed
SELECT st_distancesphere(st_point(-0.1278, 51.5074), st_point(2.3522, 48.8566));
-- → ~343 km (London → Paris)

-- WGS84 spheroid geodesics (Karney/GeographicLib, antipodal-safe)
SELECT st_distancespheroid(st_point(-0.1278, 51.5074), st_point(2.3522, 48.8566));

-- Bbox prefilter + exact predicate for inline spatial joins (no extension call)
SELECT a.id, b.id FROM a JOIN b
  ON a._xmin <= b._xmax AND a._xmax >= b._xmin
 AND a._ymin <= b._ymax AND a._ymax >= b._ymin
 WHERE st_intersects(a.geom, b.geom);

-- Literal SedonaDB kernel vs public ST_* route — same canonical implementation
SELECT st_dimension(geom), sedona_st_dimension(geom) FROM t;  -- identical

-- Raster map algebra is plain DuckDB SQL over streamed pixels
SELECT avg(value) FROM st_pixeldata('raster.tif', 1) WHERE value > 100;
```

### Semantic deltas from PostGIS

| Area | PostGIS | This extension | Workaround |
|------|---------|----------------|------------|
| **Dimensionality** | Full 3D/Z-M through the pipeline | 2D WKB local stack; Z/M via literal `sedona_st_*` constructors/accessors | Use `sedona_st_pointz/m/zm`, `sedona_st_force3d/3dm/4d` |
| **Operators** | `&&`, `<->`, KNN/GiST | No PG operators; `sedona_join` table fn + bbox prefilter columns | `sedonadb_rewrite_postgis()` rewrites both; or `sedona_join(...)` / bbox columns + DuckDB IEJoin |
| **SRID** | Embedded in EWKB/typmod | EWKB SRID tag on the blob: `ST_SetSRID`/`ST_SRID`/`ST_GeomFromText(wkt,srid)`/`ST_AsEWKT`, propagated through geometry-producing functions; `ST_Transform(geom, to_srid)` reads the source CRS from the tag | Closed — PostGIS semantics |
| **ST_Collect** | Both scalar `(g,g)` and aggregate `(g)` | Aggregate `st_collect(g)` + scalar `st_collect_scalar(g1,g2)` (DuckDB can't overload scalar+aggregate on one name) | `sedonadb_rewrite_postgis()` maps 2-arg `ST_Collect` automatically |
| **Spheroid parameter** | `ST_DistanceSpheroid(…, 'SPHEROID[…]')` | Same: accepts `SPHEROID["name",a,rf]` strings for any ellipsoid (Karney/GeographicLib); WGS84 default | Closed — PostGIS semantics |
| **Topology schema** | `topology.topology` subsystem | Not available | Out of scope (PG-specific) |

### Migration examples

```sql
-- Load GeoParquet (DuckDB reads Parquet natively; geometry is WKB BLOB)
COPY (SELECT * FROM read_parquet('data.parquet')) TO 'spatial.db';
ATTACH 'spatial.db' AS db;

-- Transform CRS, then bbox-prefilter join (no GiST needed)
SELECT a.id, b.id
FROM   db.buildings a JOIN db.parcels b
       ON a._xmin <= b._xmax AND a._xmax >= b._xmin
      AND a._ymin <= b._ymax AND a._ymax >= b._ymin
WHERE  st_intersects(st_transform(a.geom, 4326, 3857), b.geom);

-- Dissolve by category (cascaded polygonal union)
SELECT category, st_union_agg(geom) AS geom FROM db.parcels GROUP BY category;

-- Dump to atomic geometries for per-feature processing
SELECT id, (d).path, st_astext((d).geom)
FROM (SELECT id, st_dump(geom) AS d FROM db.collections) t;

-- Raster reclassification (SQL map algebra via pixel streaming)
SELECT row, col,
       CASE WHEN value > 80 THEN 3
            WHEN value > 40 THEN 2
            ELSE 1 END AS reclass
FROM   st_pixeldata('elevation.tif', 1);
```

## Indexed spatial join (`sedona_join`)

Because DuckDB's C API has no join-planner/index operators, `sedona_join`
implements the SedonaDB disk-spill model directly: spill both tables to Parquet,
then the extension reads them, builds an `rstar` R*-tree, and streams matching
pairs.

```sql
COPY (SELECT id, geom FROM a) TO 'a.parquet';   -- geometry must be the last BLOB column
COPY (SELECT id, geom FROM b) TO 'b.parquet';
SELECT * FROM sedona_join('a.parquet', 'b.parquet', 'intersects');  -- (a_row, b_row)
-- predicates: intersects | contains | within | covers | disjoint | equals
--              touches | crosses | overlaps | dwithin
```

## Set-returning dump (`ST_Dump` family)

The one FFI shape the extension was missing — set-returning table functions.
`ST_Dump` explodes a collection into its atomic geometries; `ST_DumpPoints`
yields one `POINT` per vertex; `ST_DumpSegments` yields one `LINESTRING` per
edge. Each carries a PostGIS-style `{path}` (`{1}`, `{1,2}`, …) so you can
navigate back to the source.

```sql
-- One row per point of a multipoint, with navigation paths.
SELECT path, st_astext(geom) FROM st_dump(st_geomfromtext('MULTIPOINT(1 2,3 4)'));
--  path | st_astext
-- ------+-----------
--  {1}  | POINT(1 2)
--  {2}  | POINT(3 4)

-- Explode a column: DuckDB evaluates the table function per row (lateral).
SELECT t.id, d.path, st_astext(d.geom)
FROM   my_table t, st_dumppoints(t.geom) d;

-- Vertices of a polygon ring (exterior = ring 1, holes 2..n).
SELECT path, npt FROM st_dumppoints(st_geomfromtext('POLYGON((0 0,4 0,4 4,0 4,0 0))'));
```

## Literal SedonaDB bridge

The functions above reimplement SedonaDB's SQL surface on the same `geo`/`wkb`
crates it uses. This extension now also links **the real Apache SedonaDB**
function catalog and invokes its own DataFusion UDF kernels directly from
DuckDB — making the "SedonaDB superset" literal rather than a clean-room
reimplementation.

`src/bridge.rs` is a DuckDB-chunk ⇄ Arrow bridge: it reads each DuckDB input
column into an Arrow array (BLOB/WKB → `BinaryArray`, DOUBLE → `Float64Array`,
…), tags geometry inputs with `SedonaType::Wkb`, and calls
`SedonaScalarUDF::invoke_with_args` exactly as DataFusion would. Geometry
round-trips as WKB the whole way, so SedonaDB-backed functions are
interchangeable with the reimplemented ones.

Fidelity details: constant non-geometry arguments (`ST_PointN(geom, 2)`) are
detected and emitted as Arrow `Scalar`s so SedonaDB kernels that match on the
`Scalar` arm select their implementation; SedonaDB item-crs returns
(`struct<item: WKB, crs: Utf8View>` from `ST_GeomFromEWKT`/`ST_SetSRID`) are
unwrapped to the WKB `item` at the extension's native SRID-less fidelity; a
missing/renamed UDF or an invoke error **fails closed to NULL** rather than
crashing (critical under the release `panic = "abort"`).

The registered SQL names are prefixed `sedona_` so provenance is explicit. For
routed functions, `st_*` and `sedona_st_*` share the literal SedonaDB kernel; for
unrouted functions, the pair remains useful for fidelity comparisons:

```sql
-- For routed functions, both names use the same literal SedonaDB kernel.
SELECT st_dimension(geom);
SELECT sedona_st_dimension(geom);
```

72 functions bridged (one registry line each): accessors
(`sedona_st_{x,y,z,m,xmin,xmax,ymin,ymax,zmin,zmax,mmin,mmax,dimension,
numpoints,numgeometries,geometrytype,isempty,isclosed,iscollection,hasz,hasm,zmflag}`),
geometry transforms
(`sedona_st_{envelope,reverse,flipcoordinates,startpoint,endpoint,force2d,force3d,
force3dm,force4d,points,segmentize,geometryn,pointn,interiorringn,setsrid,
translate,scale,linesubstring,makeline,rotate,rotate_x,rotate_y,point,pointz,
pointm,pointzm,geogpoint}`), WKT/WKB constructors (`geomfromwkt`, `geomfromewkt`,
`geomfromwkb`, typed line/point/polygon/multi constructors), measurements
(`sedona_st_{azimuth}`), serialization (`sedona_st_{asbinary,asewkb}`), SRID/CRS
helpers, and CRS sidecars such as `sedona_st_geomfromewkt_crs` (extracts
SedonaDB's CRS string, e.g. `OGC:CRS84`).
The entire `sedona_functions::register::default_function_set()` is reachable by
appending lines to `registry.rs`. SQL regression: `tests/sedona_bridge.sql`; fidelity
diff vs the local reimplementation: `tests/fidelity.sql`; overhead bench:
`benchmarks/bridge.sql`.

Dependency: the `sedona-functions` crate (git rev `b23ccd15`) plus
`datafusion-expr`/`datafusion-common` (trait types only — no executor). The
SedonaDB tree is pure-Rust (no GDAL/PROJ/GEOS dragged in), so it cannot collide
with the vendored GDAL/PROJ. Runtime-verified: `cargo test --lib bridge` drives
the real `st_dimension`/`st_astext`/`st_isempty` kernels through the bridge.

## Test

Pure-Rust unit tests exercise the full data path (WKB → geometry → algorithm →
WKB) without a DuckDB process:

```sh
cargo test --lib
```

SQL-level tests run against a live DuckDB via the packaged extension. See
`benchmarks/` for end-to-end SpatialBench queries.

## Benchmarks

`benchmarks/` runs the [Apache SpatialBench](https://github.com/apache/sedona-spatialbench)
queries over a **local DuckLake** (DuckDB catalog file + local Parquet data
folder). Summary (DuckDB 1.5.4, 600k trips, 20k buildings):

| Query | Workload | Time |
|-------|----------|------|
| Q1 | `ST_DWithin` + `ST_Distance`, full scan of 600k trips | 0.021 s |
| Q7 | `ST_MakeLine` + `ST_Length` over 600k trips | 0.056 s |
| FN_dist | `ST_Distance(pickup, dropoff)` over 600k trips | 0.023 s |
| FN_area | `ST_Area` over 20k polygons | 0.007 s |

Reproduce with `./benchmarks/run.sh`. See `benchmarks/BENCHMARKS.md` for the
full table, methodology, and the spatial-join finding (joins need an index;
brute-force cross joins are the documented bottleneck).

---

## Project layout

```
src/
  lib.rs        — extension entry point (entry_point! macro → register_all)
  registry.rs   — THE CATALOG: declarative macro matrix, one line per function
  dispatch.rs   — generic vectorized executors + aggregate state machines
  geometry.rs   — WKB ⇄ geo_types::Geometry (ported from Apache SedonaDB; EWKB-tolerant)
  functions.rs  — geo-crate-backed ST_* implementations
  dump.rs       — ST_Dump / ST_DumpPoints / ST_DumpSegments table functions
  bridge.rs     — DuckDB-chunk ⇄ Arrow bridge to the real Apache SedonaDB DataFusion UDFs
  spatial_join.rs — sedona_join (R-tree spatial join over spilled parquet)
  geos_backend.rs — GEOS topology/editing backend (WKB → GEOS → WKB)
  raster.rs     — st_raster_info / st_raster_stats / st_raster_transform / st_pixeldata
  bin/package.rs— appends the 512-byte DuckDB metadata trailer (.duckdb_extension)
vendor/
  quack-rs/     — vendored SDK (binary-safe read_blob + Value::as_blob; see PATCHES.md)
  gdal/         — vendored GDAL crate patched for libgdal 3.13
benchmarks/
  setup_lake.sql, spatialbench_lake.sql, spatialbench_full.sql, run_queries.sh, run.sh
```

## Roadmap status

What the brief's "Future work" section asked for, and where it stands:

| Item | Status |
| --- | --- |
| **Constructor/accessor parity** (`ST_GeomFromText`, `ST_AsText`, `ST_AsBinary`, `ST_Point`, …) | ✅ Done — broad catalog incl. DE-9IM predicates, transforms, affine/segmentize/line editing, `ST_TriangulatePolygon`, `ST_MinimumBoundingCircle`/`MinimumClearance`, `ST_MakeEnvelope`/`MakePolygon`, `ST_Dump` family, `ST_Collect`/`ST_Union`/`ST_Envelope`/`ST_MakeLine` aggregates |
| **Robustness on real-world (invalid / over-complex) polygons** | ✅ Done — `ST_MakeValid` + an internal `ensure_valid` guard across all relate/boolean ops, plus a custom iterative ray-cast PIP for `ST_Within`/`ST_Contains` (so 100k+ vertex polygons no longer crash). See `benchmarks/BENCHMARKS.md`. |
| **Spatial joins at scale** | ✅ Done — two paths: (1) `sedona_join(a.parquet, b.parquet, predicate)` R*-tree table function over spilled parquet (the disk-spill model); (2) inline bbox-prefilter (`ST_XMin/Max/YMin/MaxY` + DuckDB IEJoin). |
| **Geography (geodesic)** | ✅ Done — sphere + WGS84 spheroid `ST_Distance/DWithin/Length/Area` variants (lon/lat → metres/m²). |
| **CRS / PROJ (`ST_Transform`)** | ✅ Done — `ST_Transform(geom, from_srid, to_srid)` via bundled/static PROJ. |
| **Non-flat (dictionary/sequence/constant) input vectors** | ✅ Done — `ORDER BY … LIMIT`, filter/projection, and constant folding now feed dictionary/sequence/constant vectors into scalar `ST_*` (and `sedona_*`) callbacks without segfaulting. Resolved by the vendored binary-safe `VectorReader::read_blob`; pinned by `tests/vector_encodings.sql`. |
| **Bridge to real Apache SedonaDB DataFusion UDFs** (`sedona-functions::default_function_set`) | ✅ Done — `src/bridge.rs` links the real `sedona-functions` workspace and invokes SedonaDB's own kernels through a DuckDB-chunk ⇄ Arrow bridge; 72 `sedona_st_*` functions registered side-by-side, runtime-verified. |
| **Raster / map algebra**, **3D Z/M + SFCGAL**, `ST_VoronoiPolygons`, topology | ✅ Raster pixel streaming (`st_pixeldata`) + `ST_Value` + transform/stat helpers + SQL clip workflow, `ST_AsSVG`, GEOS topology/Voronoi/Snap/MakeValid, `ST_Subdivide`, `ST_DelaunayTriangles`/`ST_VoronoiLines`/`ST_TriangulatePolygon` done. Active non-blocking backlog: `ST_AsRaster` and `ST_AsMVT/TWKB/KML` encoders. 3D/SFCGAL is out of scope until mature Rust bindings exist. |

These map onto the brief's dependency table: the DuckDB interface stays stable
through the C-API, while SedonaDB remains a plain `cargo update`.

## License

Apache License 2.0 (see [LICENSE](./LICENSE)). Attribution for upstream code we
build on — Apache SedonaDB's geometry converter and the `quack-rs` / georust
crates — is in [NOTICE](./NOTICE).
