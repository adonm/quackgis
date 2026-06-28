-- delta_closure_fixtures.sql — proof fixtures for the three retired deltas:
--   #3 scalar ST_Collect  → st_collect_scalar + AST rewriter mapping
--   #4 SRID-less WKB      → EWKB SRID tag: write/read/propagate/transform
--   #5 spheroid WGS84-only → SPHEROID["name",a,rf] string variants
.mode list

-- ======================================================================
-- 1. SRID write / read / clear (PostGIS ST_SetSRID / ST_SRID contract)
-- ======================================================================
SELECT CASE WHEN st_srid(st_setsrid(st_point(1,2), 4326)) = 4326
            THEN 'PASS srid roundtrip' ELSE 'FAIL srid roundtrip' END;
SELECT CASE WHEN st_srid(st_point(1,2)) = 0
            THEN 'PASS srid untagged zero' ELSE 'FAIL srid untagged zero' END;
SELECT CASE WHEN st_srid(st_setsrid(st_setsrid(st_point(1,2), 4326), 3857)) = 3857
            THEN 'PASS srid retag' ELSE 'FAIL srid retag' END;
SELECT CASE WHEN st_srid(st_setsrid(st_setsrid(st_point(1,2), 4326), 0)) = 0
            THEN 'PASS srid clear' ELSE 'FAIL srid clear' END;
-- Tagged blobs still parse everywhere (tag is stripped at the trust boundary).
SELECT CASE WHEN st_astext(st_setsrid(st_point(1,2), 4326)) = 'POINT(1 2)'
            THEN 'PASS tagged astext' ELSE 'FAIL tagged astext' END;

-- ======================================================================
-- 2. SRID-aware constructors
-- ======================================================================
SELECT CASE WHEN st_srid(st_geomfromtext('POINT(1 2)', 4326)) = 4326
            THEN 'PASS geomfromtext srid' ELSE 'FAIL geomfromtext srid' END;
SELECT CASE WHEN st_srid(st_geometryfromtext('POINT(1 2)', 28356)) = 28356
            THEN 'PASS geometryfromtext srid' ELSE 'FAIL geometryfromtext srid' END;
SELECT CASE WHEN st_srid(st_geomfromwkb(st_asbinary(st_point(1,2)), 4326)) = 4326
            THEN 'PASS geomfromwkb srid' ELSE 'FAIL geomfromwkb srid' END;
SELECT CASE WHEN st_srid(st_geomfromewkt('SRID=3857;POINT(1 2)')) = 3857
            THEN 'PASS geomfromewkt srid' ELSE 'FAIL geomfromewkt srid' END;
SELECT CASE WHEN st_srid(st_geomfromewkt('POINT(1 2)')) = 0
            THEN 'PASS geomfromewkt untagged' ELSE 'FAIL geomfromewkt untagged' END;
-- ST_Polygon(linestring, srid) tags its result with the SRID argument.
SELECT CASE WHEN st_srid(st_polygon(st_geomfromtext('LINESTRING(0 0,1 0,1 1,0 0)'), 4326)) = 4326
            THEN 'PASS polygon srid' ELSE 'FAIL polygon srid' END;

-- ======================================================================
-- 3. EWKT output reads the tag
-- ======================================================================
SELECT CASE WHEN st_asewkt(st_setsrid(st_point(1,2), 4326)) = 'SRID=4326;POINT(1 2)'
            THEN 'PASS asewkt tagged' ELSE 'FAIL asewkt tagged' END;
SELECT CASE WHEN st_asewkt(st_point(1,2)) = 'POINT(1 2)'
            THEN 'PASS asewkt plain' ELSE 'FAIL asewkt plain' END;
-- EWKT round-trip: text → blob → text.
SELECT CASE WHEN st_asewkt(st_geomfromewkt('SRID=4326;POINT(1 2)')) = 'SRID=4326;POINT(1 2)'
            THEN 'PASS ewkt roundtrip' ELSE 'FAIL ewkt roundtrip' END;

-- ======================================================================
-- 4. SRID propagation through geometry-producing functions
-- ======================================================================
-- Local geo path (dispatch layer).
SELECT CASE WHEN st_srid(st_centroid(st_setsrid(st_geomfromtext('POLYGON((0 0,1 0,1 1,0 0))'), 4326))) = 4326
            THEN 'PASS propagate centroid' ELSE 'FAIL propagate centroid' END;
SELECT CASE WHEN st_srid(st_buffer(st_setsrid(st_point(1,2), 28356), 1.0)) = 28356
            THEN 'PASS propagate buffer' ELSE 'FAIL propagate buffer' END;
-- Binary local: result takes the first argument's SRID.
SELECT CASE WHEN st_srid(st_intersection(
                st_setsrid(st_geomfromtext('POLYGON((0 0,2 0,2 2,0 2,0 0))'), 4326),
                st_geomfromtext('POLYGON((1 1,3 1,3 3,1 3,1 1))'))) = 4326
            THEN 'PASS propagate intersection' ELSE 'FAIL propagate intersection' END;
-- Bridge-routed path (SedonaDB kernel; tag stripped for the kernel, re-tagged after).
SELECT CASE WHEN st_srid(st_reverse(st_setsrid(st_geomfromtext('LINESTRING(0 0,1 1)'), 4326))) = 4326
            THEN 'PASS propagate bridge reverse' ELSE 'FAIL propagate bridge reverse' END;
SELECT CASE WHEN st_srid(st_force3d(st_setsrid(st_point(1,2), 4326))) = 4326
            THEN 'PASS propagate bridge force3d' ELSE 'FAIL propagate bridge force3d' END;
-- GEOS-backed raw-WKB path.
SELECT CASE WHEN st_srid(st_makevalid(st_setsrid(st_geomfromtext('POLYGON((0 0,1 1,1 0,0 1,0 0))'), 4326))) = 4326
            THEN 'PASS propagate geos makevalid' ELSE 'FAIL propagate geos makevalid' END;
-- Untagged input stays untagged.
SELECT CASE WHEN st_srid(st_centroid(st_geomfromtext('POLYGON((0 0,1 0,1 1,0 0))'))) = 0
            THEN 'PASS no spurious tag' ELSE 'FAIL no spurious tag' END;

-- ======================================================================
-- 5. ST_Transform(geom, to_srid) — source CRS from the tag
-- ======================================================================
-- Melbourne lon/lat → WebMercator; x ≈ 16.1M metres.
SELECT CASE WHEN abs(st_x(st_transform(st_setsrid(st_point(144.96, -37.81), 4326), 3857)) - 16136873.0) < 10.0
            THEN 'PASS transform 2arg' ELSE 'FAIL transform 2arg' END;
SELECT CASE WHEN st_srid(st_transform(st_setsrid(st_point(144.96, -37.81), 4326), 3857)) = 3857
            THEN 'PASS transform output srid' ELSE 'FAIL transform output srid' END;
-- Untagged input has no source CRS: fail closed with NULL (PostGIS errors).
SELECT CASE WHEN st_transform(st_point(1,2), 3857) IS NULL
            THEN 'PASS transform untagged null' ELSE 'FAIL transform untagged null' END;
-- 3-arg form tags its output with to_srid.
SELECT CASE WHEN st_srid(st_transform(st_point(144.96, -37.81), 4326, 3857)) = 3857
            THEN 'PASS transform 3arg srid' ELSE 'FAIL transform 3arg srid' END;

-- ======================================================================
-- 6. Spheroid strings — SPHEROID["name",a,rf] (any ellipsoid)
-- ======================================================================
-- Explicit WGS84 string must equal the WGS84 default.
SELECT CASE WHEN st_distancespheroid(st_point(0,0), st_point(1,1),
                 'SPHEROID["WGS 84",6378137,298.257223563]')
             = st_distancespheroid(st_point(0,0), st_point(1,1))
            THEN 'PASS spheroid wgs84 str' ELSE 'FAIL spheroid wgs84 str' END;
-- Custom ellipsoid parameters must take effect.
SELECT CASE WHEN abs(st_distancespheroid(st_point(0,0), st_point(1,1),
                 'SPHEROID["Custom",6378000,298.0]')
             - st_distancespheroid(st_point(0,0), st_point(1,1))) > 1.0
            THEN 'PASS spheroid custom differs' ELSE 'FAIL spheroid custom differs' END;
-- rf = 0 means a sphere: quarter meridian of r=6371000 is ~10007543 m.
SELECT CASE WHEN abs(st_distancespheroid(st_point(0,0), st_point(0,90),
                 'SPHEROID["Sphere",6371000,0]') - 10007543.4) < 10.0
            THEN 'PASS spheroid sphere' ELSE 'FAIL spheroid sphere' END;
-- Length and area variants accept the string too.
SELECT CASE WHEN abs(st_lengthspheroid(st_geomfromtext('LINESTRING(0 0,1 0)'),
                 'SPHEROID["WGS 84",6378137,298.257223563]') - 111319.0) < 1.0
            THEN 'PASS lengthspheroid str' ELSE 'FAIL lengthspheroid str' END;
SELECT CASE WHEN st_areaspheroid(st_geomfromtext('POLYGON((0 0,1 0,1 1,0 1,0 0))'),
                 'SPHEROID["WGS 84",6378137,298.257223563]') > 1.2e10
            THEN 'PASS areaspheroid str' ELSE 'FAIL areaspheroid str' END;
-- Malformed spheroid string: NULL, never a silent wrong answer.
SELECT CASE WHEN st_distancespheroid(st_point(0,0), st_point(1,1), 'garbage') IS NULL
            THEN 'PASS spheroid garbage null' ELSE 'FAIL spheroid garbage null' END;
SELECT CASE WHEN st_distancespheroid(st_point(0,0), st_point(1,1), 'SPHEROID["x",-1,298]') IS NULL
            THEN 'PASS spheroid bad axis null' ELSE 'FAIL spheroid bad axis null' END;

-- ======================================================================
-- 7. Scalar ST_Collect → st_collect_scalar (+ AST rewriter mapping)
-- ======================================================================
SELECT CASE WHEN st_astext(st_collect_scalar(st_point(1,2), st_point(3,4)))
                 = 'MULTIPOINT((1 2),(3 4))'
            THEN 'PASS collect points' ELSE 'FAIL collect points' END;
SELECT CASE WHEN st_astext(st_collect_scalar(st_geomfromtext('LINESTRING(0 0,1 1)'),
                                             st_geomfromtext('LINESTRING(2 2,3 3)')))
                 = 'MULTILINESTRING((0 0,1 1),(2 2,3 3))'
            THEN 'PASS collect lines' ELSE 'FAIL collect lines' END;
SELECT CASE WHEN st_geometrytype(st_collect_scalar(st_geomfromtext('POLYGON((0 0,1 0,1 1,0 0))'),
                                                   st_geomfromtext('POLYGON((2 2,3 2,3 3,2 2))')))
                 = 'ST_MultiPolygon'
            THEN 'PASS collect polygons' ELSE 'FAIL collect polygons' END;
SELECT CASE WHEN st_geometrytype(st_collect_scalar(st_point(1,2), st_geomfromtext('LINESTRING(0 0,1 1)')))
                 = 'ST_GeometryCollection'
            THEN 'PASS collect mixed' ELSE 'FAIL collect mixed' END;
-- The rewriter maps 2-arg ST_Collect onto st_collect_scalar mechanically.
SELECT CASE WHEN sedonadb_rewrite_postgis('SELECT ST_Collect(a.geom, b.geom) FROM a, b')
                 LIKE '%st_collect_scalar(a.geom, b.geom)%'
            THEN 'PASS rewriter collect2' ELSE 'FAIL rewriter collect2' END;
-- 1-arg aggregate ST_Collect is left untouched by the rewriter.
SELECT CASE WHEN sedonadb_rewrite_postgis('SELECT ST_Collect(geom) FROM t')
                 NOT LIKE '%st_collect_scalar%'
            THEN 'PASS rewriter collect1 untouched' ELSE 'FAIL rewriter collect1 untouched' END;
-- Scalar collect propagates the SRID tag.
SELECT CASE WHEN st_srid(st_collect_scalar(st_setsrid(st_point(1,2), 4326), st_point(3,4))) = 4326
            THEN 'PASS collect srid' ELSE 'FAIL collect srid' END;
