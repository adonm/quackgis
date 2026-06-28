# duckdb_sedona

A DuckDB loadable extension that exposes a catalog of vector spatial (`ST_*`)
functions over WKB-encoded geometries, built in pure Rust with
[`quack-rs`](https://github.com/tomtom215/quack-rs) and the same
`wkb` / `geo` / `geo-traits` stack that
[Apache SedonaDB](https://github.com/apache/sedona-db) builds on.

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
| Route DuckDB vectors "straight into SedonaDB's registry" | SedonaDB's functions are **DataFusion columnar UDFs** over Arrow arrays, not per-row geometry fns. | We reuse the *same algorithmic stack* SedonaDB uses (`geo`, `wkb`) and run it directly on DuckDB vectors. Bridging the real SedonaDB DataFusion UDFs is a documented future extension point (see below). |

So the deliverable honours the brief's architecture — unified vectorized
dispatch + declarative macro registry, low-maintenance single-line registration
— against the actual `quack-rs` C-API and the actual georust geometry stack.

---

## Function catalog

All functions take **BLOB** (ISO WKB) arguments. Geometry-returning functions
return **BLOB** (ISO WKB). `NULL` in → `NULL` out; an input that fails to parse
or an operation that is undefined for the input also yields `NULL`. **~115
functions** across constructors, accessors, predicates, measurements, set ops,
transforms, validity, three aggregates (`st_collect`, `st_envelope_agg`,
`st_union_agg`), **geodesic/geography** (`st_distancesphere`…), EWKT/EWKB/SRID,
affine & segmentize transforms, line editing, and I/O.

See **[ROADMAP.md](./ROADMAP.md)** for a category-level capability matrix vs
SedonaDB and PostGIS and the tiered plan to reach a superset (next up:
`ST_Dump` as a set-returning table function, bounded `ST_VoronoiPolygons`, and
a DuckDB-chunk ⇄ Arrow bridge to the real SedonaDB DataFusion UDFs).

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
| `st_makevalid` | `(BLOB) → BLOB` | `buffer(0)` topology repair |
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
| **PROJ (`libproj`)** | **build**: bundled & **statically linked** (`proj-sys/bundled_proj` + `libsqlite3-sys/bundled`) | `ST_Transform` (CRS reprojection) | Our own PROJ is **static** → `ST_Transform` has **no runtime dep** of its own. (But GDAL, below, brings its own dynamic libproj.) |
| **GDAL (`libgdal` ≥ 3.13)** | **build (`pkg-config gdal` + `LIBCLANG_PATH` for bindgen) + runtime (`LD_LIBRARY_PATH`)** | **Raster** (`st_raster_info`, `st_raster_stats`, …) via a **vendored + patched** `gdal` 0.19 crate (only the high-level `gdal` crate is vendored; `gdal-sys` comes unpatched from crates.io — see `vendor/gdal/PATCHES.md`). | `LOAD` needs `libgdal.so` (and its transitive `libproj`/`libsqlite3`) resolvable. |
| arrow / parquet (Rust crates) | build only (statically linked) | `sedona_join` (R-tree spatial join over spilled parquet) | no runtime dep |
| `geo`/`wkb`/`wkt`/`geo-traits`/`delaunator`/`rstar` | build only | the ST_* geometry surface, Delaunay/Voronoi, the R-tree | pure-Rust, statically linked |

**Build the extension** (PROJ bundled static; GDAL present):
```sh
export PKG_CONFIG_PATH="$(brew --prefix gdal)/lib/pkgconfig"   # gdal.pc (for bindgen)
export LIBCLANG_PATH="$(brew --prefix llvm)/lib"               # bindgen needs libclang
cargo build --release
./target/release/sedonadb-package target/release/libsedonadb.so build/dev/sedonadb.duckdb_extension linux_amd64
```
**Load it** (GDAL runtime libs):
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
> empty for non-UTF-8 bytes — and WKB is arbitrary binary (e.g. coordinate
> `1.0` encodes an `0xF0` byte that breaks UTF-8). sedonadb reads the raw
> `duckdb_string_t` bytes itself (`src/dispatch.rs`, `BlobCol`), so binary WKB
> round-trips correctly. Unit tests in `src/functions.rs` pin this.

In practice you feed it any `BLOB` column containing ISO-WKB (e.g. a
SpatialBench parquet geometry column, or DuckDB `spatial`'s `GEOMETRY` storage).
Note: DuckDB 1.5 also ships its own `GEOMETRY`-returning `ST_GeomFromWKB`; to
avoid overload ambiguity, call sedonadb's functions on the raw `BLOB` column
directly (they accept WKB natively) and `CAST(... AS BLOB)` literals.

> **Runtime dependency:** since `ST_Transform` was wired in (PROJ), the
> extension links `libproj.so` — set `LD_LIBRARY_PATH` (or install libproj
> system-wide) before `LOAD`, e.g.
> `LD_LIBRARY_PATH=/opt/homebrew/lib duckdb -unsigned`.

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
  dispatch.rs   — generic vectorized executors + ST_Collect aggregate state
  geometry.rs   — WKB ⇄ geo_types::Geometry (ported from Apache SedonaDB)
  functions.rs  — geo-crate-backed ST_* implementations
  bin/package.rs— appends the 512-byte DuckDB metadata trailer (.duckdb_extension)
benchmarks/
  setup_lake.sql, spatialbench_lake.sql, spatialbench_full.sql, run_queries.sh, run.sh
```

## Roadmap status

What the brief's "Future work" section asked for, and where it stands:

| Item | Status |
| --- | --- |
| **Constructor/accessor parity** (`ST_GeomFromText`, `ST_AsText`, `ST_AsBinary`, `ST_Point`, …) | ✅ Done — ~115 functions now, incl. DE-9IM predicates, transforms, affine/segmentize/line editing, `ST_TriangulatePolygon`, `ST_Collect`/`ST_Union`/`ST_Envelope` aggregates |
| **Robustness on real-world (invalid / over-complex) polygons** | ✅ Done — `ST_MakeValid` + an internal `ensure_valid` guard across all relate/boolean ops, plus a custom iterative ray-cast PIP for `ST_Within`/`ST_Contains` (so 100k+ vertex polygons no longer crash). See `benchmarks/BENCHMARKS.md`. |
| **Spatial joins at scale** | ✅ Done — two paths: (1) `sedona_join(a.parquet, b.parquet, predicate)` R*-tree table function over spilled parquet (the disk-spill model); (2) inline bbox-prefilter (`ST_XMin/Max/YMin/MaxY` + DuckDB IEJoin). |
| **Geography (geodesic)** | ✅ Done — `ST_DistanceSphere/DWithinSphere/LengthSphere/AreaSphere` (lon/lat → metres/m²). |
| **CRS / PROJ (`ST_Transform`)** | ✅ Done — `ST_Transform(geom, from_srid, to_srid)` via PROJ. Runtime dep: `libproj.so`. |
| **Non-flat (dictionary/sequence) input vectors** | ⏳ Open — an `ORDER BY … LIMIT` on a geometry column feeding a scalar fn segfaults (DuckDB C API exposes no vector-encoding inspection; `BlobCol` can't decode it). Workaround: materialize/filter. See BENCHMARKS "Known limitations". |
| **Bridge to real Apache SedonaDB DataFusion UDFs** (`sedona-functions::default_function_set`) | ⏳ Open — would need a DuckDB-chunk ⇄ Arrow bridge. |
| **Raster / map algebra**, **3D Z/M + SFCGAL**, `ST_VoronoiPolygons`, topology | ✅ Raster core (vendored GDAL: `st_raster_info`/`st_raster_stats`) + `ST_DelaunayTriangles`/`ST_VoronoiLines`/`ST_TriangulatePolygon` done. Map-algebra/`ST_AsRaster`/bounded Voronoi polygons open. 3D/SFCGAL has no mature Rust binding (out of reach); topology is niche. Static PROJ now bundled. |

These map onto the brief's dependency table: the DuckDB interface stays stable
through the C-API, while SedonaDB remains a plain `cargo update`.

## License

Apache License 2.0 (see [LICENSE](./LICENSE)). Attribution for upstream code we
build on — Apache SedonaDB's geometry converter and the `quack-rs` / georust
crates — is in [NOTICE](./NOTICE).
