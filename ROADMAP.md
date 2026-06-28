# Roadmap to a SedonaDB + PostGIS superset

Status of the `sedonadb` DuckDB extension against the Apache SedonaDB and
PostGIS spatial surfaces, and what it takes to reach a **superset**.

## Where we are now (~115 functions, all on the `geo`/`wkb` stack)

Constructors & I/O (incl. EWKT **and EWKB**) · accessors · DE-9IM predicates ·
measurements (incl. `ST_MaxDistance`/`ST_LongestLine`/`ST_ShortestLine`) ·
boolean set ops · affine/simplify/segmentize transforms · `ST_MakeValid`
robustness · three aggregates (`ST_Collect`, `ST_Envelope` agg, `ST_Union` agg)
· `ST_DWithin` · bbox accessors (join prefilter) · a custom robust
point-in-polygon · **geodesic/geography**
(`ST_DistanceSphere/DWithinSphere/LengthSphere/AreaSphere`) · EWKT/EWKB/SRID
stubs · typed WKT constructors · `ST_Points/LineLocatePoint/Frechet/\
ClosestPoint/Hausdorff/FlipCoordinates/Reverse/RemoveRepeatedPoints/\
OrientedEnvelope` · `ST_Affine`(6-param)/`ST_Segmentize`/`ST_LineSubstring`/
`ST_LineMerge`/`ST_CollectionExtract`/`ST_ForceCollection`/`ST_Multi`/
`ST_Normalize`/`ST_ForceRHR`/`ST_ForcePolygonCW`/`ST_ForcePolygonCCW`/
`ST_TriangulatePolygon`/`ST_OrderingEquals`/`ST_NRings`. Verified end-to-end in
DuckDB 1.5.4 over a local DuckLake and Apache SpatialBench
(`benchmarks/BENCHMARKS.md`).

## Previously-flagged hard bits — now resolved

1. **`ST_Transform` via PROJ (Tier 3a) — ✅ DONE (option a: accept runtime dep).**
   PROJ 9.8.1 linked; `ST_Transform(geom, from_srid, to_srid)` reprojects between EPSG
   codes (verified: London 4326→3857 = 14227, 6711542; Sedona = -12441066, 4144872).
   A thread-local `Proj` cache avoids re-parsing the CRS per row. **Runtime dep:**
   `libproj.so` must be present to `LOAD sedonadb;` (`ldd` shows `libproj.so.25`).
   arrow/parquet are statically linked (no runtime dep).
2. **Spatial-join via disk-spill (Tier 3b) — ✅ DONE as `sedona_join` table function.**
   DuckDB's `COPY ... TO 'x.parquet'` is the spill; `sedona_join(a_path, b_path,
   predicate)` reads both Parquet files itself (`parquet`/`arrow` crates), builds an
   `rstar` R*-tree over the right side, applies the exact predicate, and streams
   `(a_row, b_row)` pairs. Verified: 20k×20k building self-join returns 37 pairs —
   identical to the bbox-prefilter result. This is the SedonaDB disk-spilling spatial
   join model, realized without needing any DuckDB join-planner/GiST API.
3. **`ST_VoronoiPolygons`** — `geo` 0.31 has no Voronoi; needs a new dep or port. Still
   open (low priority).

## Capability matrix (category-level)

Legend: ✅ shipped · 🟡 partial · ⏳ not yet · ➖ out of scope (niche).

| Category | PostGIS | SedonaDB | sedonadb (this ext.) | Notes |
|---|---|---|---|---|
| Constructors (WKT/WKB/EWKT/EWKB, typed `*FromText`) | ✅ | ✅ | ✅ | WKT/WKB/EWKT/EWKB + typed constructors + `ST_Point` all shipped. `from_wkb` is EWKB-tolerant at the trust boundary. |
| Output (`ST_AsText/Binary/EWKB/GeoJSON/HexEWKB`) | ✅ | ✅ | ✅ | Text/Binary/EWKB/GeoJSON/HexEWKB all done |
| Accessors (X/Y/Z/M, dims, rings, N-th geometry/point) | ✅ | ✅ | 🟡 | 2D accessors + `ST_NRings` done; Z/M stubs return NULL/false |
| DE-9IM predicates (`Intersects`…`Covers`, `OrderingEquals`) | ✅ | ✅ | ✅ | All 10 + `ST_OrderingEquals`; guarded for invalid input |
| Measurements (`Area/Length/Distance/Perimeter/Azimuth/Hausdorff/MaxDistance/LongestLine/ShortestLine`) | ✅ | ✅ | ✅ | core + distance-family done |
| Boolean set ops (`Union/Intersection/Difference/SymDiff`) | ✅ | 🟡 | ✅ | |
| `ST_MakeValid` / validity | ✅ | 🟡 | ✅ | robustness hardening done |
| Editing (`Translate/Scale/Rotate/Flip/Reverse/Affine/Segmentize/LineSubstring/LineMerge/Normalize`) | ✅ | ✅ | ✅ | all done incl. 6-param `ST_Affine` |
| Geometry processing (`Buffer/Simplify/ConvexHull/ConcaveHull/OrientedEnvelope/Triangulate/Voronoi`) | ✅ | 🟡 | 🟡 | Buffer/Simplify/Hull/OrientedEnvelope/TriangulatePolygon done; bounded Voronoi polygons + Polygonize open |
| Linear referencing (`LineInterpolatePoint/Locate/Substring`) | ✅ | 🟡 | ✅ | interpolate/locate/substring all done |
| Aggregates (`Collect/Union/Envelope/Intersection`) | ✅ | ✅ | 🟡 | `ST_Collect`/`ST_Union`/`ST_Envelope` agg done; intersection aggregate open |
| **Geography (geodesic) ops** | ✅ | ✅ | ✅ | `Distance/DWithin/Length/Area` Sphere done (lon/lat) |
| **CRS / PROJ (`ST_Transform`, SRID)** | ✅ | ✅ | ✅ | `ST_Transform` via PROJ (runtime libproj dep) |
| **Spatial index join (R-tree/GiST, `&&`/`<->`)** | ✅ | ✅ | ✅ | `sedona_join` table fn (R-tree over spilled parquet) + bbox-prefilter |
| **Raster / map algebra** | ✅ (PostGIS Raster) | ✅ (`sedona-raster`) | ✅ (core) — `st_raster_info` + `st_raster_stats` via vendored+patched GDAL against libgdal 3.13; full map-algebra pending |
| **3D / Z-M geometry + SFCGAL surfaces** | ✅ (SFCGAL) | ⏳ | ⏳ No mature Rust SFCGAL bindings (see Tier 4) |
| Topology / Tiger geocoder / address standardizer | ✅ | ➖ | ➖ | niche; not in SedonaDB either |

So: **geometry-level SQL surface** is already broad and at feature parity with
SedonaDB for the common cases. The real gaps to a true superset are the four
**infrastructure capabilities** below, not more scalar functions.

## Tiers

### Tier 1 — finish geometry-level parity (small, geo-backed, ~1 line each)
Cheap wins; each is one `register_*!` line + a `geo` call. **Mostly ✅ done.**

- ✅ EWKB/EWKT I/O: `ST_AsEWKB`, `ST_GeomFromEWKB` (EWKB-tolerant `from_wkb`),
  `ST_AsEWKT`, `ST_GeomFromEWKT`, `ST_AsHexEWKB`.
- ✅ SRID stubs: `ST_SRID` (0), `ST_SetSRID` (no-op tag).
- ✅ Typed constructors: `ST_LineFromText`, `ST_PointFromText`,
  `ST_PolygonFromText`, `ST_MLineFromText`, … (route through WKT parser).
- ✅ `ST_Affine`(6 doubles), `ST_Segmentize`, `ST_LineSubstring`,
  `ST_LineMerge`, `ST_CollectionExtract`, `ST_ForceCollection`, `ST_Multi`,
  `ST_Normalize`, `ST_ForceRHR`/`ST_ForcePolygonCW`/`ST_ForcePolygonCCW`,
  `ST_SnapToGrid`.
- ✅ More aggregates: `ST_Union` agg (`st_union_agg`), `ST_Envelope` agg.
  `ST_Collect` already done. Intersection aggregate still open.
- ✅ `ST_TriangulatePolygon` (Delaunay-interior approximation).
- ⏳ `ST_Dump`, `ST_DumpPoints`, `ST_DumpSegments` — needs a **table/set
  function**; the one new FFI shape we haven't built.
- ⏳ `ST_Node`, `ST_Snap`, `ST_Polygonize`, `ST_BuildArea` — topology editing.
- ⏳ `ST_VoronoiPolygons` (bounded cell polygons; `ST_VoronoiLines` already
  ships).

### Tier 1b — PostGIS geo-backed geometry processing
✅ `ST_HausdorffDistance`, `ST_FrechetDistance`, `ST_MaxDistance`,
`ST_LongestLine`, `ST_ClosestPoint`, `ST_ShortestLine`, `ST_Project`,
`ST_OrientedEnvelope`, `ST_TriangulatePolygon` all shipped. Still open:
`ST_MinimumClearance`, `ST_GeneratePoints`, `ST_Subdivide`.

### Tier 2 — Geography (geodesic) — ✅ DONE
`ST_DistanceSphere`, `ST_DWithinSphere`, `ST_LengthSphere`, `ST_AreaSphere`
(launch/lat → metres / m² via `geo`'s Haversine + Chamberlain-Duquette). No new
dep. (PostGIS spheroid-accurate `ST_DistanceSpheroid` and full
geometry-vs-geometry geodesic distance still open.)

### Tier 3 — CRS reprojection + native spatial index — ✅ DONE
- **`ST_Transform` via PROJ** — implemented (`proj` crate). Runtime dep on
  `libproj.so`. Thread-local CRS cache. Verified on 4326↔3857.
- **Spatial index join** — two paths: (1) `sedona_join(a.parquet, b.parquet,
  predicate)` table function: extension reads both files, builds an `rstar`
  R*-tree, streams matching pairs — the disk-spill model; (2) bbox-prefilter via
  materialized `ST_XMin/Max/YMin/MaxY` + DuckDB IEJoin for inline joins. Both
  verified against SpatialBench (20k building self-join = 37 pairs either way).

### Tier 4 — Raster, 3D, topology (long tail)
- **Raster / map algebra** — ✅ Core landed via **vendored + patched GDAL** against
  libgdal 3.13. The upstream `gdal` 0.19 crate lags 3.13 (it renamed `GDT_Byte`→
  `GDT_UInt8` and added `GDALRasterIOExtraArg::bOperateInBufType`); we vendor only
  the high-level `gdal` crate (`vendor/gdal`, with `PATCHES.md`) — `gdal-sys` is
  unpatched and pulled from crates.io — and enable `bindgen` so fresh FFI
  bindings are generated from the installed 3.13 headers. Ships
  `st_raster_info(path)` and `st_raster_stats(path, band)` (read any GDAL format;
  summary stats in the band's native type). **Open:** `ST_MapAlgebra`, `ST_AsRaster`,
  `ST_Clip`, band math. **Build needs** `pkg-config gdal` + `LIBCLANG_PATH`;
  **runtime needs** `libgdal.so` (+ its libproj/libsqlite3) via `LD_LIBRARY_PATH`.
- **Static PROJ** — ✅ Our own PROJ (for `ST_Transform`) is now **bundled +
  statically linked** (`proj-sys/bundled_proj` + `libsqlite3-sys/bundled`), so
  reprojection has no runtime dep of its own. GDAL brings its own dynamic libproj,
  so the extension overall still needs `LD_LIBRARY_PATH` (or system libgdal) while
  GDAL is linked. (GDAL is intentionally **not** feature-gated — the extension is
  a single full-capability build.)
- **Delaunay / Voronoi** — ✅ Done. `ST_DelaunayTriangles` (via `delaunator`)
  and `ST_VoronoiLines` (dual of the Delaunay triangulation — interior edges;
  full bounded cell polygons still open).
- **3D / Z-M + SFCGAL** — ⏳ Not feasible today. `geo`/`wkb`/our pipeline are
  2D-only; full 3D needs Z/M through the entire stack plus surface algorithms
  (extrude, straight skeleton, 3D boolean). **There is no mature Rust SFCGAL/CGAL
  binding** — this would mean writing/maintaining `sfcgal-sys` (weeks+). The one
  genuinely out-of-reach PostGIS surface for a Rust extension.
- **Topology** — ➖ niche; PostGIS topology is a separate subsystem and not in
  SedonaDB. Out of scope.

## What "superset" realistically means

A 100 % byte-compatible PostGIS superset is impractical (operators/GiST, SFCGAL,
raster, topology, Tiger). The pragmatic target — **a SedonaDB superset and a
PostGIS-compatible core** — is reachable:

1. **Tier 1 + 1b** (a few days) → full geometry-level SQL parity (~120 fns),
   indistinguishable from PostGIS for vector analytics.
2. **Tier 2** (geography) → spherical distance/area, the second-most-used
   PostGIS surface.
3. **Tier 3a** (`ST_Transform`/PROJ) → CRS reprojection, the capability users
   most associate with "real" GIS.
4. **Tier 3b** (`sedona_join` table function + R-tree) → indexed spatial joins
   with no bbox-prefilter manual step — the performance story.

Tiers 1→3 are the work that turns this from "a strong ST_* function pack" into
"a SedonaDB-class spatial engine on DuckDB". Tier 4 is open-ended and can be
added incrementally (raster first, 3D much later).
