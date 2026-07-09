# PostGIS conformance ledger

QuackGIS is **PostGIS-compatible where evidence exists**, not a full PostGIS
server. A function, operator, catalog behavior, or client workflow is supported
only when it appears in one of the maintained gates below.

## Evidence tiers

| Tier | Scope | Evidence |
|---|---|---|
| Pgwire claimed subset | Functions and metadata exercised through the QuackGIS server protocol boundary | `just postgis-regress`; `crates/quackgis-server/tests/postgis_regress.rs` currently has 39 cases and prints `postgis_regress_subset passed=<n> total=<n> pass_rate=<x>` |
| SQL portability fixtures | SedonaDB/PostGIS function-kernel parity and documented mechanical rewrites | `tests/run_sql.sh` includes the SQL fixtures summarized by `just postgis-conformance-summary` |
| Client traces | Real client SQL/catalog/wire behavior | Kind probes for QGIS, GDAL/OGR, GeoServer, and Martin; see `docs/COMPATIBILITY_MATRIX.md` |

The pgwire subset is the release claim. The SQL portability fixtures are broader
function-kernel evidence and are promoted to pgwire/client claims only when a
server or client trace needs that surface.

Use `just postgis-conformance-summary` to render the current fixture counts for
release notes or conformance reviews.

## Current covered families

| Family | Status | Notes |
|---|---|---|
| Version/metadata helpers | ✅ pgwire | `postgis_version()`, `postgis_lib_version()`, `geometry_columns`, `geography_columns`, `spatial_ref_sys`, `Find_SRID`, `ST_Extent` |
| Constructors and text/WKB IO | ✅ pgwire starter + SQL fixtures | `ST_GeomFromText`, `ST_GeomFromEWKT`, `ST_Point`, `ST_MakeEnvelope`, WKB/EWKB client payloads |
| SRID helpers | ✅ pgwire starter | EWKB SRID tags are preserved/read by `ST_SetSRID`, `ST_SRID`, `ST_GeomFromEWKT`, `ST_MakeEnvelope(..., srid)`, and `ST_Transform(..., srid)` |
| Predicates and DE-9IM | ✅ SQL fixtures; starter pgwire subset for `ST_Intersects` | Includes intersects, contains/within boundary semantics, disjoint, touches, crosses, overlaps, equals, covers, DWithin, ordering-equals, and 40 curated `ST_Relate` matrices |
| Measures/accessors | ✅ SQL fixtures; starter pgwire subset for area/distance/length, `ST_X`/`ST_Y`, `ST_XMin`/`ST_YMin`/`ST_XMax`/`ST_YMax`, `GeometryType`/`ST_GeometryType`, `ST_NDims`/`ST_CoordDim`/`ST_Dimension`, `ST_NPoints`/`ST_NumPoints`, `ST_StartPoint`/`ST_EndPoint`/`ST_PointN`, `ST_NumGeometries`, `ST_IsEmpty`, `ST_IsValid`, and helper metadata | Area, length, perimeter, distance, bbox accessors, dimension/`ST_Zmflag`, collection/validity helpers |
| Overlay/processing | ✅ SQL fixtures | Intersection, union, difference, symdifference, make-valid, hull/envelope, shortest/longest/closest line, max distance |
| Dump/set-returning functions | ✅ SQL fixtures | `ST_Dump`, `ST_DumpPoints`, `ST_DumpRings`, `ST_DumpSegments`; SQL syntax follows DataFusion table-function shape (`FROM st_dump(...)`) |
| Editing/affine/simplify | ✅ SQL fixtures with documented deltas; pgwire renderer helpers | Reverse, normalize, set/remove/add point, flip coordinates, force polygon CW, translate/scale/rotate/affine/snap-to-grid, simplify; `ST_Force2D`, `ST_CurveToLine`, and `ST_HasArc` are covered through pgwire because maintained clients use them in render/tile SQL |
| Operators/client rewrites | ✅ maintained where clients require them | `&&` and `<->` are mapped in QuackGIS/client paths; SQL portability fixtures also document mechanical DuckDB/Sedona rewrites |

## Known deltas and skips

| Surface | Status | Reason / current behavior |
|---|---|---|
| Full PostGIS extension surface | ❌ unclaimed | QuackGIS is not PostgreSQL + PostGIS. Functions not covered by the gates above are unsupported until a fixture or client trace is added. |
| PostgreSQL extension/server features | ❌ non-goals | PL/pgSQL, triggers, LISTEN/NOTIFY, logical replication, and `pg_dump` compatibility are outside the architecture. |
| Raster pixel algebra / PostGIS raster package | ❌ unclaimed | Current raster/asset support is footprint + sidecar metadata over object-store artifacts, not in-engine raster decoding or pixel operations. |
| PostGIS topology schema, geocoder, address normalizer | ❌ unclaimed | No maintained client trace or product requirement yet. Add trace-driven fixtures before claiming support. |
| Exact PostgreSQL geometry typmods | ⚠️ partial | QuackGIS exposes geometry metadata and preserves EWKB SRID tags, but does not implement PostgreSQL typmod enforcement such as `geometry(Point,4326)`. |
| `ST_Boundary` edge cases | ⚠️ documented delta | Empty boundaries return `GEOMETRYCOLLECTION EMPTY`; single-ring polygon boundary returns `MULTILINESTRING`; multipolygon/geometry-collection boundary is incomplete; `TRIANGLE` is unsupported. |
| EMPTY geometry edge cases | ⚠️ documented delta | `ST_Buffer(EMPTY, 0)` returns `NULL`; some centroid/envelope paths accept either `NULL` or empty output in fixtures. |
| `ST_Simplify` collapsed members/rings | ⚠️ documented delta | Large geometry is preserved and topology-preserve fixtures pass, but collapsed small polygon members/rings do not exactly match PostGIS removal behavior. |
| `ST_RemovePoint` on a single-segment line | ⚠️ documented delta | PostGIS errors; current implementation is permissive and returns a degenerate non-NULL geometry. |
| `ST_DelaunayTriangles` tolerance/flag overload | ⚠️ unsupported overload | The curated topology fixture documents that the 3-argument tolerance + flag form is not implemented. |
| Projection/geodesy parity beyond covered fixtures | ⚠️ unclaimed | SRID metadata behavior is tested. Numeric CRS transformation/projection equivalence should be treated as future evidence unless a fixture covers the exact path. |

## Adding a new support claim

1. Add the smallest upstream-derived or trace-derived fixture that proves the
   behavior.
2. If the behavior crosses the QuackGIS server boundary, add a pgwire integration
   test or client probe, not only a SQL portability fixture.
3. Update this ledger and `docs/COMPATIBILITY_MATRIX.md` with the exact command
   that proves the claim.
4. Treat any failing or intentionally skipped case as a documented delta with a
   reason, not as an implicit support claim.
