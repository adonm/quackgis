-- SPDX-License-Identifier: Apache-2.0
-- SQL regression for the LITERAL Apache SedonaDB bridge (src/bridge.rs).
-- These run the real sedona-functions DataFusion kernels inside DuckDB,
-- side-by-side with the extension's own reimplementation. Run with:
--   LD_LIBRARY_PATH=<gdal-lib> duckdb -unsigned -cmd "LOAD '<ext>';" < tests/sedona_bridge.sql
--
-- Each test prints 'PASS' or 'FAIL <details>'. Item-crs-returning UDFs are
-- expected to fail closed to NULL until the item-crs bridge lands (Phase 3).
.bail off
.mode list

-- === side-by-side: local vs literal SedonaDB must agree ===
SELECT CASE WHEN st_dimension(geom) = sedona_st_dimension(geom) THEN 'PASS' ELSE 'FAIL dimension mismatch' END
FROM (SELECT st_geomfromtext('POINT(1 2)') AS geom);

SELECT CASE WHEN st_astext(geom) = sedona_st_astext(geom) THEN 'PASS' ELSE 'FAIL astext mismatch' END
FROM (SELECT st_geomfromtext('POINT(1 2)') AS geom);

SELECT CASE WHEN st_isempty(geom) = sedona_st_isempty(geom) THEN 'PASS' ELSE 'FAIL isempty mismatch' END
FROM (SELECT st_geomfromtext('POINT(1 2)') AS geom);

-- === scalar accessors through the literal bridge ===
SELECT CASE WHEN sedona_st_dimension(geom) = 2 THEN 'PASS' ELSE 'FAIL sedona dimension' END
FROM (SELECT st_geomfromtext('POLYGON((0 0,4 0,4 4,0 4,0 0))') AS geom);

SELECT CASE WHEN sedona_st_numpoints(geom) = 5 THEN 'PASS' ELSE 'FAIL sedona numpoints' END
FROM (SELECT st_geomfromtext('POLYGON((0 0,4 0,4 4,0 4,0 0))') AS geom);

SELECT CASE WHEN sedona_st_isclosed(geom) = true THEN 'PASS' ELSE 'FAIL sedona isclosed' END
FROM (SELECT st_geomfromtext('POLYGON((0 0,4 0,4 4,0 4,0 0))') AS geom);

SELECT CASE WHEN sedona_st_geometrytype(geom) = 'ST_Polygon' THEN 'PASS' ELSE 'FAIL sedona geometrytype' END
FROM (SELECT st_geomfromtext('POLYGON((0 0,4 0,4 4,0 4,0 0))') AS geom);

-- === geometry-returning kernels (assert on area/npoints, robust to ring winding) ===
SELECT CASE WHEN st_area(sedona_st_envelope(geom)) = 16.0 THEN 'PASS' ELSE 'FAIL sedona envelope' END
FROM (SELECT st_geomfromtext('POLYGON((0 0,4 0,4 4,0 4,0 0))') AS geom);

SELECT CASE WHEN st_astext(sedona_st_flipcoordinates(st_geomfromtext('POINT(1 2)'))) = 'POINT(2 1)' THEN 'PASS' ELSE 'FAIL sedona flip' END;

SELECT CASE WHEN st_astext(sedona_st_startpoint(geom)) = 'POINT(0 0)' THEN 'PASS' ELSE 'FAIL sedona startpoint' END
FROM (SELECT st_geomfromtext('LINESTRING(0 0,1 1,2 2)') AS geom);

SELECT CASE WHEN st_astext(sedona_st_startpoint(sedona_st_reverse(geom))) = 'POINT(2 2)' THEN 'PASS' ELSE 'FAIL sedona reverse' END
FROM (SELECT st_geomfromtext('LINESTRING(0 0,1 1,2 2)') AS geom);

SELECT CASE WHEN sedona_st_numpoints(sedona_st_segmentize(geom, 3.0)) = 5 THEN 'PASS' ELSE 'FAIL sedona segmentize' END
FROM (SELECT st_geomfromtext('LINESTRING(0 0, 10 0)') AS geom);

-- === item-crs geometry returns: Struct<item,crs> is unwrapped to plain WKB
-- (Phase 3, Option B) — the geometry is preserved at the extension's native
-- SRID-less fidelity; the CRS sidecar is dropped (matches the extension's own
-- no-op ST_SetSRID model). ===
SELECT CASE WHEN st_astext(sedona_st_geomfromewkt('SRID=4326;POINT(1 2)')) = 'POINT(1 2)' THEN 'PASS' ELSE 'FAIL sedona geomfromewkt' END;
SELECT CASE WHEN st_astext(sedona_st_setsrid(st_geomfromtext('POINT(1 2)'), 4326)) = 'POINT(1 2)' THEN 'PASS' ELSE 'FAIL sedona setsrid' END;

-- === expanded batch (Phase 2): ordinate accessors / predicates / accessors ===
SELECT CASE WHEN sedona_st_x(pt) = 1.0 AND sedona_st_y(pt) = 2.0 THEN 'PASS' ELSE 'FAIL sedona x/y' END
FROM (SELECT st_geomfromtext('POINT(1 2)') AS pt);

SELECT CASE WHEN sedona_st_xmin(p) = 0.0 AND sedona_st_xmax(p) = 4.0 AND sedona_st_ymin(p) = 0.0 AND sedona_st_ymax(p) = 4.0 THEN 'PASS' ELSE 'FAIL sedona bbox accessors' END
FROM (SELECT st_geomfromtext('POLYGON((0 0,4 0,4 4,0 4,0 0))') AS p);

SELECT CASE WHEN sedona_st_iscollection(coll) = true AND sedona_st_hasz(pt) = false AND sedona_st_numgeometries(coll) = 2 THEN 'PASS' ELSE 'FAIL sedona predicates' END
FROM (SELECT st_geomfromtext('GEOMETRYCOLLECTION(POINT(1 2),LINESTRING(3 4,5 6))') AS coll, st_geomfromtext('POINT(1 2)') AS pt);

SELECT CASE WHEN sedona_st_numpoints(sedona_st_force2d(st_geomfromtext('POINT Z(1 2 3)'))) = 1 THEN 'PASS' ELSE 'FAIL sedona force2d' END;

SELECT CASE WHEN st_astext(sedona_st_endpoint(ln)) = 'POINT(3 3)' THEN 'PASS' ELSE 'FAIL sedona endpoint' END
FROM (SELECT st_geomfromtext('LINESTRING(0 0,1 1,2 2,3 3)') AS ln);

-- constant-scalar index fidelity (ST_PointN expects a scalar index)
SELECT CASE WHEN st_astext(sedona_st_pointn(ln, 2)) = 'POINT(1 1)' THEN 'PASS' ELSE 'FAIL sedona pointn' END
FROM (SELECT st_geomfromtext('LINESTRING(0 0,1 1,2 2,3 3)') AS ln);

SELECT CASE WHEN st_astext(sedona_st_geometryn(coll, 1)) = 'POINT(9 9)' THEN 'PASS' ELSE 'FAIL sedona geometryn' END
FROM (SELECT st_geomfromtext('GEOMETRYCOLLECTION(POINT(9 9),POINT(8 8))') AS coll);

SELECT CASE WHEN st_astext(sedona_st_interiorringn(p, 1)) = 'LINESTRING(1 1,2 1,2 2,1 2,1 1)' THEN 'PASS' ELSE 'FAIL sedona interiorringn' END
FROM (SELECT st_geomfromtext('POLYGON((0 0,4 0,4 4,0 4,0 0),(1 1,2 1,2 2,1 2,1 1))') AS p);

-- === Phase C: constructors, transforms, measurements ===
SELECT CASE WHEN st_astext(sedona_st_point(3, 4)) = 'POINT(3 4)' THEN 'PASS' ELSE 'FAIL sedona point' END;
SELECT CASE WHEN st_astext(sedona_st_translate(st_geomfromtext('POINT(1 2)'), 5.0, -1.0)) = 'POINT(6 1)' THEN 'PASS' ELSE 'FAIL sedona translate' END;
SELECT CASE WHEN st_astext(sedona_st_scale(st_geomfromtext('POINT(2 3)'), 2.0, 3.0)) = 'POINT(4 9)' THEN 'PASS' ELSE 'FAIL sedona scale' END;
SELECT CASE WHEN st_astext(sedona_st_linesubstring(st_geomfromtext('LINESTRING(0 0, 10 0)'), 0.0, 0.5)) = 'LINESTRING(0 0,5 0)' THEN 'PASS' ELSE 'FAIL sedona linesubstring' END;
SELECT CASE WHEN st_astext(sedona_st_makeline(st_geomfromtext('POINT(0 0)'), st_geomfromtext('POINT(1 1)'))) = 'LINESTRING(0 0,1 1)' THEN 'PASS' ELSE 'FAIL sedona makeline' END;
SELECT CASE WHEN sedona_st_azimuth(st_geomfromtext('POINT(0 0)'), st_geomfromtext('POINT(0 1)')) IS NOT NULL THEN 'PASS' ELSE 'FAIL sedona azimuth' END;
SELECT CASE WHEN sedona_st_zmflag(st_geomfromtext('POINT(1 2)')) = 0 THEN 'PASS' ELSE 'FAIL sedona zmflag' END;
SELECT CASE WHEN st_astext(sedona_st_rotate(st_geomfromtext('POINT(1 0)'), 1.5707963267948966)) LIKE 'POINT(%1)' THEN 'PASS' ELSE 'FAIL sedona rotate' END;

-- === Phase D: CRS sidecar (item-crs struct crs column → VARCHAR) ===
SELECT CASE WHEN sedona_st_geomfromewkt_crs('SRID=4326;POINT(1 2)') = 'OGC:CRS84' THEN 'PASS' ELSE 'FAIL sedona ewkt-crs 4326' END;
SELECT CASE WHEN sedona_st_geomfromewkt_crs('SRID=3857;POINT(1 2)') = 'EPSG:3857' THEN 'PASS' ELSE 'FAIL sedona ewkt-crs 3857' END;
SELECT CASE WHEN sedona_st_geomfromewkt_crs('POINT(1 2)') IS NULL THEN 'PASS' ELSE 'FAIL sedona ewkt-crs none' END;

-- === P1: complete the literal surface — WKT/WKB constructors, dimension
-- forcing, Z/M points, SRID. Struct-returning constructors are unwrapped to
-- plain WKB at the extension's native SRID-less fidelity. ===
-- WKT constructors round-trip through astext.
SELECT CASE WHEN st_astext(sedona_st_geomfromwkt('POINT(1 2)')) = 'POINT(1 2)' THEN 'PASS' ELSE 'FAIL sedona geomfromwkt' END;
SELECT CASE WHEN st_astext(sedona_st_linefromtext('LINESTRING(0 0,1 1)')) = 'LINESTRING(0 0,1 1)' THEN 'PASS' ELSE 'FAIL sedona linefromtext' END;
SELECT CASE WHEN st_astext(sedona_st_pointfromtext('POINT(3 4)')) = 'POINT(3 4)' THEN 'PASS' ELSE 'FAIL sedona pointfromtext' END;
SELECT CASE WHEN st_astext(sedona_st_polygonfromtext('POLYGON((0 0,1 0,1 1,0 0))')) = 'POLYGON((0 0,1 0,1 1,0 0))' THEN 'PASS' ELSE 'FAIL sedona polygonfromtext' END;
SELECT CASE WHEN st_astext(sedona_st_mlinefromtext('MULTILINESTRING((0 0,1 1))')) = 'MULTILINESTRING((0 0,1 1))' THEN 'PASS' ELSE 'FAIL sedona mlinefromtext' END;
SELECT CASE WHEN st_astext(sedona_st_mpointfromtext('MULTIPOINT((1 2))')) = 'MULTIPOINT((1 2))' THEN 'PASS' ELSE 'FAIL sedona mpointfromtext' END;
SELECT CASE WHEN st_astext(sedona_st_mpolyfromtext('MULTIPOLYGON(((0 0,1 0,1 1,0 0)))')) = 'MULTIPOLYGON(((0 0,1 0,1 1,0 0)))' THEN 'PASS' ELSE 'FAIL sedona mpolyfromtext' END;
SELECT CASE WHEN st_astext(sedona_st_geomcollfromtext('GEOMETRYCOLLECTION(POINT(1 2))')) = 'GEOMETRYCOLLECTION(POINT(1 2))' THEN 'PASS' ELSE 'FAIL sedona geomcollfromtext' END;
SELECT CASE WHEN st_astext(sedona_st_geogfromwkt('POINT(5 6)')) = 'POINT(5 6)' THEN 'PASS' ELSE 'FAIL sedona geogfromwkt' END;
-- WKB constructors (raw Binary input → geom).
SELECT CASE WHEN st_astext(sedona_st_geomfromwkb(st_asbinary(st_geomfromtext('POINT(7 8)')))) = 'POINT(7 8)' THEN 'PASS' ELSE 'FAIL sedona geomfromwkb' END;
SELECT CASE WHEN st_astext(sedona_st_geomfromewkb(st_asewkb(st_geomfromtext('POINT(9 10)')))) = 'POINT(9 10)' THEN 'PASS' ELSE 'FAIL sedona geomfromewkb' END;
SELECT CASE WHEN st_astext(sedona_st_geomfromwkbunchecked(st_asbinary(st_geomfromtext('LINESTRING(0 0,2 2)')))) = 'LINESTRING(0 0,2 2)' THEN 'PASS' ELSE 'FAIL sedona geomfromwkbunchecked' END;
-- SRID accessor (native WKB has no embedded SRID → 0).
SELECT CASE WHEN sedona_st_srid(sedona_st_geomfromewkt('SRID=4326;POINT(1 2)')) = 0 THEN 'PASS' ELSE 'FAIL sedona srid' END;
-- Dimension forcing sets the zm flag (0=XY, 1=XYM, 2=XYZ, 3=XYZM).
SELECT CASE WHEN sedona_st_zmflag(sedona_st_force3d(st_geomfromtext('POINT(1 2)'), 9.0)) = 2 THEN 'PASS' ELSE 'FAIL sedona force3d' END;
SELECT CASE WHEN sedona_st_zmflag(sedona_st_force3dm(st_geomfromtext('POINT(1 2)'), 9.0)) = 1 THEN 'PASS' ELSE 'FAIL sedona force3dm' END;
SELECT CASE WHEN sedona_st_zmflag(sedona_st_force4d(st_geomfromtext('POINT(1 2)'), 9.0, 8.0)) = 3 THEN 'PASS' ELSE 'FAIL sedona force4d' END;
-- Z/M point constructors. (Local st_astext is 2D-only and drops Z/M; the
-- literal sedona_st_astext and sedona_st_zmflag read the full ordinate set.)
SELECT CASE WHEN sedona_st_astext(sedona_st_pointz(1, 2, 3)) = 'POINT Z(1 2 3)' THEN 'PASS' ELSE 'FAIL sedona pointz' END;
SELECT CASE WHEN sedona_st_astext(sedona_st_pointm(1, 2, 3)) = 'POINT M(1 2 3)' THEN 'PASS' ELSE 'FAIL sedona pointm' END;
SELECT CASE WHEN sedona_st_astext(sedona_st_pointzm(1, 2, 3, 4)) = 'POINT ZM(1 2 3 4)' THEN 'PASS' ELSE 'FAIL sedona pointzm' END;

-- === Phase 1 cleanup: ST_Affine 2D (geom + 6 doubles) ===
-- a=2,b=0,d=0,e=3,xOff=1,yOff=1 on POINT(1,1) -> (2*1+0*1+1, 0*1+3*1+1) = (3,4)
SELECT CASE WHEN st_astext(sedona_st_affine(st_geomfromtext('POINT(1 1)'), 2,0,0,3,1,1)) = 'POINT(3 4)' THEN 'PASS' ELSE 'FAIL sedona affine' END;
