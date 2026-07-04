# PostGIS / SedonaDB Compatibility Table

This table lists common PostGIS functions and their status in this extension.
"Supported" means the function exists and matches PostGIS semantics. "Alias"
means it is available under a different name or shape. "Delta" means there is a
documented semantic difference. "Not yet" means it is planned. "Out of scope"
means it is intentionally not implemented.

Generated counts: **254 SQL functions** (180 `st_*` + 72 `sedona_st_*` + 1 extension).

Legend: ✅ supported · 🔄 alias/different shape · ⚠️ semantic delta · ⏳ not yet · ➖ out of scope

## Constructors

| PostGIS function | Status | Notes |
|---|---|---|
| `ST_GeomFromText` | ✅ | Also aliased as `ST_GeometryFromText`. |
| `ST_GeomFromWKT` | ✅ | SedonaDB naming; alias for `ST_GeomFromText`. Routes to literal kernel. |
| `ST_GeomFromWKB` | ✅ | Local reimplementation (trust-boundary validation). |
| `ST_GeomFromEWKB` | ✅ | Local reimplementation. |
| `ST_GeomFromEWKT` | ✅ | Local. Literal twin: `sedona_st_geomfromewkt`. |
| `ST_LineFromText` | ✅ | Routes to literal SedonaDB typed kernel. Type-validates WKT (rejects non-LineString → NULL). |
| `ST_PointFromText` | ✅ | Routes to literal kernel. Type-validates WKT (rejects non-Point → NULL). |
| `ST_PolygonFromText` | ✅ | Routes to literal kernel. Type-validates WKT (rejects non-Polygon → NULL). |
| `ST_Point` | ✅ | Also aliased as `ST_MakePoint`. Both route to literal kernel. |
| `ST_MakeEnvelope` | ✅ | |
| `ST_MakePolygon` | ✅ | |
| `ST_MakeLine` | ✅ | Scalar routes to literal kernel + aggregate (`ST_MakeLine_Agg`). |
| `ST_Polygon` | ✅ | `(linestring, srid) → polygon`. SRID accepted but not embedded in WKB. |

## Output

| PostGIS function | Status | Notes |
|---|---|---|
| `ST_AsText` | ✅ | Routes to literal SedonaDB kernel. |
| `ST_AsBinary` | ✅ | Routes to literal kernel. |
| `ST_AsEWKB` | ✅ | Routes to literal kernel. |
| `ST_AsEWKT` | ✅ | Local. |
| `ST_AsGeoJSON` | ✅ | Local. |
| `ST_AsHEXEWKB` | ✅ | Local. |
| `ST_AsSVG` | ✅ | Absolute coordinates, Y-flipped. Matches PostGIS `ST_AsSVG(geom, 0)`. |
| `ST_AsMVT` | ✅ | Scalar MVT encoder (hex-encoded protobuf). Single-geometry tile; use SQL aggregation for multi-feature tiles. |

## Accessors

| PostGIS function | Status | Notes |
|---|---|---|
| `ST_X` / `ST_Y` | ✅ | Routes to literal kernel. |
| `ST_Z` / `ST_M` | ✅ | Routes to literal kernel. Returns NULL on 2D WKB. |
| `ST_XMin..Max` / `ST_YMin..Max` | ✅ | Routes to literal kernel. |
| `ST_GeometryType` | ✅ | Routes to literal kernel. |
| `ST_Dimension` | ✅ | Matches PostGIS: returns -1 for EMPTY, 0 for Point, 1 for Line, 2 for Polygon. |
| `ST_NumPoints` | ✅ | Routes to literal kernel. |
| `ST_NumGeometries` | ✅ | Routes to literal kernel. |
| `ST_SRID` | ✅ | Reads the EWKB SRID tag on the blob; 0 when untagged (PostGIS semantics). |
| `ST_ZMFlag` | ✅ | Routes to literal kernel. |
| `ST_HasZ` / `ST_HasM` | ✅ | Routes to literal kernel. |
| `ST_IsEmpty` | ✅ | Routes to literal kernel. |
| `ST_IsClosed` | ✅ | Routes to literal kernel. |
| `ST_IsRing` | ✅ | Local. |
| `ST_IsCollection` | ✅ | Routes to literal kernel. |
| `ST_IsValid` | ✅ | Local. |
| `ST_IsValidReason` | ✅ | Local. |
| `ST_IsValidDetail` | ✅ | Table function `(BLOB) → (valid BOOL, reason VARCHAR, geom BLOB)`. |
| `ST_NumInteriorRings` | ✅ | Also aliased as `ST_NumInteriorRing`. |
| `ST_NRings` | ✅ | Local. |
| `ST_CoordDim` | ✅ | Local. |

## Predicates (DE-9IM)

| PostGIS function | Status | Notes |
|---|---|---|
| `ST_Intersects` | ✅ | |
| `ST_Contains` | ✅ | Matches PostGIS DE-9IM exactly (M22: boundary delta retired). Use `ST_Covers` for boundary-inclusive semantics. |
| `ST_Within` | ✅ | Matches PostGIS DE-9IM exactly (M22: boundary delta retired). |
| `ST_Disjoint` | ✅ | |
| `ST_Equals` | ✅ | |
| `ST_Touches` | ✅ | |
| `ST_Crosses` | ✅ | |
| `ST_Overlaps` | ✅ | |
| `ST_Covers` | ✅ | |
| `ST_CoveredBy` | ✅ | |
| `ST_ContainsProperly` | ✅ | DE-9IM `T**FF*FF*`. Interior containment only (no boundary contact). |
| `ST_Relate` | ✅ | `(a, b) → VARCHAR` DE-9IM matrix; `(a, b, pattern) → BOOLEAN` pattern match. Via GEOS. |
| `ST_OrderingEquals` | ✅ | |
| `ST_DWithin` | ✅ | Also `ST_DWithinSphere` / `ST_DWithinSpheroid`. |

## Measurements

| PostGIS function | Status | Notes |
|---|---|---|
| `ST_Area` | ✅ | |
| `ST_Length` | ✅ | |
| `ST_Perimeter` | ✅ | |
| `ST_Distance` | ✅ | |
| `ST_Azimuth` | ✅ | Routes to literal kernel. |
| `ST_MaxDistance` | ✅ | |
| `ST_LongestLine` | ✅ | |
| `ST_ShortestLine` | ✅ | |
| `ST_ClosestPoint` | ✅ | |
| `ST_HausdorffDistance` | ✅ | |
| `ST_FrechetDistance` | ✅ | |
| `ST_MinimumClearance` | ✅ | |
| `ST_MinimumClearanceLine` | ✅ | |
| `ST_DistanceSphere` | ✅ | Also `ST_LengthSphere`, `ST_AreaSphere`. |
| `ST_DistanceSpheroid` | ✅ | WGS84 default; accepts PostGIS `SPHEROID["name",a,rf]` string for any ellipsoid (Karney/GeographicLib). |
| `ST_LengthSpheroid` | ✅ | Same: WGS84 default + spheroid-string variant. |
| `ST_AreaSpheroid` | ✅ | Same: WGS84 default + spheroid-string variant. |

## Boolean set operations

| PostGIS function | Status | Notes |
|---|---|---|
| `ST_Intersection` | ✅ | |
| `ST_Union` | ✅ | Scalar + aggregate (`ST_Union_Agg`). |
| `ST_Difference` | ✅ | |
| `ST_SymDifference` | ✅ | |
| `ST_Intersection_Agg` | ✅ | Aggregate. |

## Topology / validity

| PostGIS function | Status | Notes |
|---|---|---|
| `ST_MakeValid` | ✅ | GEOS `make_valid` (canonical PostGIS engine). |
| `ST_Node` | ✅ | GEOS. |
| `ST_Polygonize` | ✅ | GEOS. |
| `ST_BuildArea` | ✅ | GEOS. |
| `ST_Snap` | ✅ | GEOS `snap` (canonical PostGIS engine). |
| `ST_Subdivide` | ✅ | Local. |

## Geometry processing

| PostGIS function | Status | Notes |
|---|---|---|
| `ST_Buffer` | ✅ | |
| `ST_Simplify` | ✅ | RDP. |
| `ST_SimplifyPreserveTopology` | ✅ | |
| `ST_SimplifyVW` | ✅ | Visvalingam-Whyatt. |
| `ST_ConvexHull` | ✅ | |
| `ST_ConcaveHull` | ✅ | |
| `ST_OrientedEnvelope` | ✅ | |
| `ST_MinimumBoundingCircle` | ✅ | |
| `ST_TriangulatePolygon` | ✅ | |
| `ST_DelaunayTriangles` | ✅ | |
| `ST_VoronoiPolygons` | ✅ | GEOS (bounded). |
| `ST_VoronoiLines` | ✅ | Local. |
| `ST_GeneratePoints` | ✅ | Seeded random. |

## Editing

| PostGIS function | Status | Notes |
|---|---|---|
| `ST_Translate` | ✅ | Routes to literal kernel. |
| `ST_Scale` | ✅ | Routes to literal kernel. |
| `ST_Rotate` | ✅ | Routes to literal kernel. |
| `ST_Affine` | ✅ | 6-param 2D. Routes to literal kernel. |
| `ST_FlipCoordinates` | ✅ | Routes to literal kernel. |
| `ST_Reverse` | ✅ | Routes to literal kernel. |
| `ST_Segmentize` | ✅ | Routes to literal kernel. |
| `ST_Force2D` | ✅ | Routes to literal kernel. |
| `ST_Force3D` / `ST_Force3DZ` | ✅ | Matches PostGIS: 1-arg form defaults z=0; 2-arg form takes explicit z. Routes to literal SedonaDB kernel. |
| `ST_Force3DM` | ✅ | Matches PostGIS: 1-arg form defaults m=0. |
| `ST_Force4D` | ✅ | Matches PostGIS: 1-arg form defaults z=0, m=0. |
| `ST_ForceCollection` | ✅ | |
| `ST_ForcePolygonCW` / `CCW` | ✅ | |
| `ST_ForceRHR` | ✅ | |
| `ST_Multi` | ✅ | |
| `ST_Normalize` | ✅ | |
| `ST_RemoveRepeatedPoints` | ✅ | |
| `ST_SetPoint` | ✅ | `(linestring, index, point) → linestring`. 0-based index (matches PostGIS). |
| `ST_SnapToGrid` | ✅ | |
| `ST_CollectionExtract` | ✅ | |
| `ST_LineMerge` | ✅ | |
| `ST_LineSubstring` | ✅ | Routes to literal kernel. |
| `ST_LineInterpolatePoint` | ✅ | |
| `ST_LineLocatePoint` | ✅ | |
| `ST_Project` | ✅ | |
| `ST_RemovePoint` | ✅ | |
| `ST_AddPoint` | ✅ | |

## CRS / projection

| PostGIS function | Status | Notes |
|---|---|---|
| `ST_Transform` | ✅ | PROJ (bundled static). `ST_Transform(geom, from_srid, to_srid)`. |
| `ST_SetSRID` | ✅ | Writes the EWKB SRID tag on the blob (srid 0 clears). Local; the literal kernel cannot express SRID in plain WKB. |
| `ST_SRID` | ✅ | Reads the EWKB SRID tag; 0 when untagged. |

## Aggregates

| PostGIS function | Status | Notes |
|---|---|---|
| `ST_Collect` | ✅ | Aggregate `ST_Collect(g)` + scalar `st_collect_scalar(g1, g2)` (DuckDB rejects a scalar under an aggregate name; the AST rewriter maps 2-arg `ST_Collect` automatically). |
| `ST_Union_Agg` | ✅ | Cascaded polygonal union. |
| `ST_Envelope_Agg` | ✅ | Bbox union. |
| `ST_MakeLine_Agg` | ✅ | Points → LineString. |
| `ST_Intersection_Agg` | ✅ | Cascaded polygonal intersection. |
| `ST_MemUnion` | 🔄 | Use `ST_Union_Agg`. |

## Set-returning / table functions

| PostGIS function | Status | Notes |
|---|---|---|
| `ST_Dump` | ✅ | Returns `(path, geom)` rows. |
| `ST_DumpPoints` | ✅ | Returns `(path, geom)` per vertex. |
| `ST_DumpSegments` | ✅ | Returns `(path, geom)` per edge. |
| `ST_DumpRings` | ✅ | Returns `(path, geom)` per ring. Path `{0}` = exterior, `{1}+` = interior. |
| `ST_IsValidDetail` | ✅ | Returns `(valid, reason, geom)` row for validity detail. |

## Raster

| PostGIS function | Status | Notes |
|---|---|---|
| `ST_RasterInfo` | ✅ | Extension-specific: metadata table function. |
| `ST_RasterStats` | ✅ | Extension-specific: per-band statistics. |
| `ST_RasterTransform` | ✅ | Extension-specific: GeoTransform + spatial bounds. |
| `ST_PixelData` | ✅ | Extension-specific: `(row, col, value)` pixel streaming. |
| `ST_Value` | ✅ | Scalar: `ST_Value(path, band, x, y)`. Inverts GeoTransform, reads one pixel. For bulk work, use `ST_PixelData`. |
| `ST_Clip` | 🔄 | DuckDB-native: filter `ST_PixelData` by computed geographic bbox using GeoTransform (see `tests/reference/m16_fixtures.sql`). No raster-returning facade. |
| `ST_AsRaster` | ⏳ | Only remaining raster facade gap. Needs GDAL rasterization/write path. |
| `ST_MapAlgebra` | ➖ | DuckDB SQL is the map algebra engine (via `ST_PixelData`). |

## Spatial join

| PostGIS function | Status | Notes |
|---|---|---|
| `&&` operator | 🔄 | Use bbox columns + DuckDB predicates, or `sedona_join`. |
| `<->` KNN | 🔄 | Use `sedona_join` or cross-join with `ST_Distance` + `ORDER BY` + `LIMIT`. |
| `sedona_join` | ✅ | Extension-specific: R-tree spatial join over spilled parquet. |

## Intentionally out of scope

| Feature | Status | Reason |
|---|---|---|
| PostgreSQL GiST/R-tree planner hooks | ➖ | DuckDB C API has no join-planner hooks. |
| PostGIS topology schema | ➖ | PostgreSQL-specific subsystem. |
| Tiger geocoder | ➖ | PostgreSQL-specific. |
| Address standardizer | ➖ | PostgreSQL-specific. |
| SFCGAL 3D solids/surfaces | ➖ | No mature Rust binding. |
| Raster map-algebra expression language | ➖ | DuckDB SQL is the expression language. |

## Compatibility debt log

Curated list of known semantic deltas between this extension and PostGIS/SedonaDB.
Each entry has a fixture or an explicit defer rationale.

### Confirmed deltas (tested)

**None.** All previously tracked deltas are closed:

| Closed delta | Resolution | Fixture |
|--------------|------------|---------|
| Scalar `ST_Collect(g1, g2)` | DuckDB's C API rejects a scalar under an aggregate's catalog name (verified), so the scalar form is `st_collect_scalar` with full PostGIS semantics (MULTI* for same-type pairs, GEOMETRYCOLLECTION otherwise). `sedonadb_rewrite_postgis()` maps 2-arg `ST_Collect` onto it mechanically — same class as the `&&`/`<->` operator rewrites. | `delta_closure_fixtures.sql` |
| SRID-less WKB | PostGIS SRID semantics via an EWKB SRID tag on the blob: `ST_SetSRID` writes it, `ST_SRID` reads it, `ST_GeomFromText(wkt, srid)` / `ST_GeomFromWKB(wkb, srid)` / `ST_GeomFromEWKT('SRID=n;…')` construct it, `ST_AsEWKT(geom)` prints it, `ST_Transform(geom, to_srid)` reads the source CRS from it, and the dispatch layer propagates it through geometry-producing functions (local, bridge-routed, and GEOS paths). | `delta_closure_fixtures.sql` |
| Spheroid WGS84-only | `ST_DistanceSpheroid/LengthSpheroid/AreaSpheroid` accept the PostGIS `SPHEROID["name",a,rf]` string and build a custom Karney geodesic (`rf = 0` → sphere). Malformed strings return NULL. | `delta_closure_fixtures.sql` |

### Remaining caveats (documented, non-delta)

| Caveat | Details |
|--------|---------|
| SRID tags and aggregates | Aggregate functions (`ST_Collect(g)`, `ST_Union_Agg`, …) do not propagate SRID tags (rows may carry mixed tags). Tag the aggregate result with `ST_SetSRID` if needed. |
| Mixed-SRID binary inputs | PostGIS raises an error when binary function inputs have different SRIDs; we take the first argument's tag without validation (DuckDB scalar functions cannot raise data-dependent errors without aborting the query). |
| `sedona_st_*` namespace | The literal namespace keeps SedonaDB behavior: `sedona_st_setsrid` returns plain WKB and `sedona_st_srid` returns 0 (SedonaDB models CRS at the type level, which plain-WKB output cannot express). |

### Active non-blocking backlog

All output encoders are shipped (SVG in M16, KML/TWKB in M23, MVT in M23b).
No SQL-surface backlog items remain.

### Out of scope (intentionally not implemented)

| Feature | Rationale |
|---------|-----------|
| PostgreSQL GiST/R-tree planner hooks | DuckDB C API has no join-planner hooks. |
| PostGIS topology schema | PostgreSQL-specific subsystem. |
| Tiger geocoder / address standardizer | PostgreSQL-specific. |
| SFCGAL 3D solids/surfaces | No mature Rust binding. |
| Raster map-algebra expression language | DuckDB SQL is the expression language. |

## PostGIS portability ledger

Representative PostGIS SQL verified against the extension via
`tests/postgis_port/`. See [docs/OPERATIONS.md](./docs/OPERATIONS.md) for the
  full migration cookbook.

| Family | Cases | Status | Notes |
|---|---|---|---|
| Constructors | 8 | ✅ port | WKT/WKB/EWKT/point/makeenvelope/makeline/makepolygon/buffer. |
| Accessors | 11 | ✅ port | NumPoints, X/Y, Area, Length, Perimeter, Dimension, IsEmpty, rings, bbox. |
| DE-9IM predicates | 11 | ✅ port | Intersects, Contains (interior + boundary), Within, Disjoint, Touches, Crosses, Overlaps, Equals, DWithin, Covers. |
| Overlay | 5 | ✅ port | Intersection/Union/Difference/SymDifference area; disjoint → empty/NULL. |
| Validity | 7 | ✅ port | IsValid, IsValidReason, MakeValid, IsValidDetail (table fn). |
| Dump family | 5 | ✅ port | Dump, DumpPoints, DumpRings, DumpSegments. |
| Line editing | 8 | ✅ port | LineSubstring, LineInterpolatePoint, LineLocatePoint, SetPoint, AddPoint, RemovePoint, Translate, Simplify. |
| Operator rewrites | 6 | ✅ rewrite | `&&` → bbox columns, `<->` → ORDER BY distance, casts, SRID, geography distance. |

Operational packaging is now tracked in the facade/container roadmap; it is not
a SQL compatibility gap.
