// SPDX-License-Identifier: Apache-2.0
//! Curated PostGIS expected-value fixture parsed by current DuckDB gates.
//!
//! This is intentionally small and explicit: it pins the PostGIS-compatible
//! functions QuackGIS currently classifies and gives the DuckDB CLI/pgwire gates
//! one stable expected-value source.

struct Case {
    name: &'static str,
    sql: &'static str,
    expected: &'static str,
}

const CASES: &[Case] = &[
    Case {
        name: "postgis_lib_version",
        sql: "SELECT postgis_lib_version()",
        expected: "3.4.0",
    },
    Case {
        name: "postgis_version_marker",
        sql: "SELECT postgis_version()",
        expected: "3.4.0 QUACKGIS",
    },
    Case {
        name: "geomfromtext_astext_point",
        sql: "SELECT ST_AsText(ST_GeomFromText('POINT(1 2)'))",
        expected: "POINT(1 2)",
    },
    Case {
        name: "geomfromewkt_astext_point",
        sql: "SELECT ST_AsText(ST_GeomFromEWKT('SRID=4326;POINT(1 2)'))",
        expected: "POINT(1 2)",
    },
    Case {
        name: "point_constructor",
        sql: "SELECT ST_AsText(ST_Point(3.0, 4.0))",
        expected: "POINT(3 4)",
    },
    Case {
        name: "point_constructor_srid",
        sql: "SELECT ST_AsEWKT(ST_Point(3.0, 4.0, 4326))",
        expected: "SRID=4326;POINT(3 4)",
    },
    Case {
        name: "makepoint_constructor",
        sql: "SELECT ST_AsText(ST_MakePoint(5.0, 6.0))",
        expected: "POINT(5 6)",
    },
    Case {
        name: "setsrid_srid",
        sql: "SELECT CAST(ST_SRID(ST_SetSRID(ST_GeomFromText('POINT(1 2)'), 4326)) AS TEXT)",
        expected: "4326",
    },
    Case {
        name: "asewkt_srid_point",
        sql: "SELECT ST_AsEWKT(ST_SetSRID(ST_GeomFromText('POINT(1 2)'), 4326))",
        expected: "SRID=4326;POINT(1 2)",
    },
    Case {
        name: "ashexewkb_point",
        sql: "SELECT ST_AsHEXEWKB(ST_GeomFromText('POINT(1 2)'))",
        expected: "0101000000000000000000F03F0000000000000040",
    },
    Case {
        name: "transform_sets_target_srid",
        sql: "SELECT CAST(ST_SRID(ST_Transform(ST_SetSRID(ST_GeomFromText('POINT(0 0)'), 4326), 3857)) AS TEXT)",
        expected: "3857",
    },
    Case {
        name: "makeenvelope_srid",
        sql: "SELECT CAST(ST_SRID(ST_MakeEnvelope(0.0, 0.0, 1.0, 1.0, 3857)) AS TEXT)",
        expected: "3857",
    },
    Case {
        name: "force2d_point_identity",
        sql: "SELECT ST_AsText(ST_Force2D(ST_GeomFromText('POINT(1 2)')))",
        expected: "POINT(1 2)",
    },
    Case {
        name: "curvetoline_point_identity",
        sql: "SELECT ST_AsText(ST_CurveToLine(ST_GeomFromText('POINT(1 2)')))",
        expected: "POINT(1 2)",
    },
    Case {
        name: "hasarc_point_false",
        sql: "SELECT CAST(ST_HasArc(ST_GeomFromText('POINT(1 2)')) AS TEXT)",
        expected: "false",
    },
    Case {
        name: "zmflag_2d_point",
        sql: "SELECT CAST(ST_Zmflag(ST_GeomFromText('POINT(1 2)')) AS TEXT)",
        expected: "0",
    },
    Case {
        name: "ndims_2d_point",
        sql: "SELECT CAST(ST_NDims(ST_GeomFromText('POINT(1 2)')) AS TEXT)",
        expected: "2",
    },
    Case {
        name: "coorddim_2d_point",
        sql: "SELECT CAST(ST_CoordDim(ST_GeomFromText('POINT(1 2)')) AS TEXT)",
        expected: "2",
    },
    Case {
        name: "dimension_polygon",
        sql: "SELECT CAST(ST_Dimension(ST_GeomFromText('POLYGON((0 0,4 0,4 4,0 4,0 0))')) AS TEXT)",
        expected: "2",
    },
    Case {
        name: "numinteriorrings_polygon",
        sql: "SELECT CAST(ST_NumInteriorRings(ST_GeomFromText('POLYGON((0 0,4 0,4 4,0 4,0 0),(1 1,2 1,2 2,1 1))')) AS TEXT)",
        expected: "1",
    },
    Case {
        name: "exteriorring_polygon",
        sql: "SELECT ST_AsText(ST_ExteriorRing(ST_GeomFromText('POLYGON((0 0,4 0,4 4,0 4,0 0),(1 1,2 1,2 2,1 1))')))",
        expected: "LINESTRING(0 0,4 0,4 4,0 4,0 0)",
    },
    Case {
        name: "interiorringn_polygon",
        sql: "SELECT ST_AsText(ST_InteriorRingN(ST_GeomFromText('POLYGON((0 0,4 0,4 4,0 4,0 0),(1 1,2 1,2 2,1 1))'), 1))",
        expected: "LINESTRING(1 1,2 1,2 2,1 1)",
    },
    Case {
        name: "isempty_point_false",
        sql: "SELECT CAST(ST_IsEmpty(ST_GeomFromText('POINT(1 2)')) AS TEXT)",
        expected: "false",
    },
    Case {
        name: "isvalid_point_true",
        sql: "SELECT CAST(ST_IsValid(ST_GeomFromText('POINT(1 2)')) AS TEXT)",
        expected: "true",
    },
    Case {
        name: "geometrytype_point",
        sql: "SELECT GeometryType(ST_GeomFromText('POINT(1 2)'))",
        expected: "POINT",
    },
    Case {
        name: "st_geometrytype_point",
        sql: "SELECT ST_GeometryType(ST_GeomFromText('POINT(1 2)'))",
        expected: "ST_Point",
    },
    Case {
        name: "st_geometrytype_multipoint",
        sql: "SELECT ST_GeometryType(ST_GeomFromText('MULTIPOINT((0 0),(1 1))'))",
        expected: "ST_MultiPoint",
    },
    Case {
        name: "intersects_point_in_polygon",
        sql: "SELECT CAST(ST_Intersects(ST_GeomFromText('POINT(1 1)'), ST_GeomFromText('POLYGON((0 0,4 0,4 4,0 4,0 0))')) AS TEXT)",
        expected: "true",
    },
    Case {
        name: "area_square",
        sql: "SELECT CAST(ST_Area(ST_GeomFromText('POLYGON((0 0,4 0,4 4,0 4,0 0))')) AS TEXT)",
        expected: "16.0",
    },
    Case {
        name: "distance_3_4_5",
        sql: "SELECT CAST(ST_Distance(ST_GeomFromText('POINT(0 0)'), ST_GeomFromText('POINT(3 4)')) AS TEXT)",
        expected: "5.0",
    },
    Case {
        name: "length_3_4_5",
        sql: "SELECT CAST(ST_Length(ST_GeomFromText('LINESTRING(0 0,3 4)')) AS TEXT)",
        expected: "5.0",
    },
    Case {
        name: "perimeter_square",
        sql: "SELECT CAST(ST_Perimeter(ST_GeomFromText('POLYGON((0 0,4 0,4 4,0 4,0 0))')) AS TEXT)",
        expected: "16.0",
    },
    Case {
        name: "npoints_linestring",
        sql: "SELECT CAST(ST_NPoints(ST_GeomFromText('LINESTRING(0 0,3 4,6 8)')) AS TEXT)",
        expected: "3",
    },
    Case {
        name: "numpoints_linestring_alias",
        sql: "SELECT CAST(ST_NumPoints(ST_GeomFromText('LINESTRING(0 0,3 4,6 8)')) AS TEXT)",
        expected: "3",
    },
    Case {
        name: "startpoint_linestring",
        sql: "SELECT ST_AsText(ST_StartPoint(ST_GeomFromText('LINESTRING(0 0,3 4,6 8)')))",
        expected: "POINT(0 0)",
    },
    Case {
        name: "endpoint_linestring",
        sql: "SELECT ST_AsText(ST_EndPoint(ST_GeomFromText('LINESTRING(0 0,3 4,6 8)')))",
        expected: "POINT(6 8)",
    },
    Case {
        name: "pointn_linestring",
        sql: "SELECT ST_AsText(ST_PointN(ST_GeomFromText('LINESTRING(0 0,3 4,6 8)'), 2))",
        expected: "POINT(3 4)",
    },
    Case {
        name: "isclosed_closed_linestring",
        sql: "SELECT CAST(ST_IsClosed(ST_GeomFromText('LINESTRING(0 0,3 0,0 0)')) AS TEXT)",
        expected: "true",
    },
    Case {
        name: "isring_square_linestring",
        sql: "SELECT CAST(ST_IsRing(ST_GeomFromText('LINESTRING(0 0,4 0,4 4,0 4,0 0)')) AS TEXT)",
        expected: "true",
    },
    Case {
        name: "reverse_linestring",
        sql: "SELECT ST_AsText(ST_Reverse(ST_GeomFromText('LINESTRING(0 0,3 4,6 8)')))",
        expected: "LINESTRING(6 8,3 4,0 0)",
    },
    Case {
        name: "flipcoordinates_point",
        sql: "SELECT ST_AsText(ST_FlipCoordinates(ST_GeomFromText('POINT(3 4)')))",
        expected: "POINT(4 3)",
    },
    Case {
        name: "translate_point",
        sql: "SELECT ST_AsText(ST_Translate(ST_GeomFromText('POINT(3 4)'), 2.0, -1.0))",
        expected: "POINT(5 3)",
    },
    Case {
        name: "scale_point",
        sql: "SELECT ST_AsText(ST_Scale(ST_GeomFromText('POINT(3 4)'), 2.0, 0.5))",
        expected: "POINT(6 2)",
    },
    Case {
        name: "numgeometries_point",
        sql: "SELECT CAST(ST_NumGeometries(ST_GeomFromText('POINT(3 4)')) AS TEXT)",
        expected: "1",
    },
    Case {
        name: "numgeometries_multipoint",
        sql: "SELECT CAST(ST_NumGeometries(ST_GeomFromText('MULTIPOINT((0 0),(1 1))')) AS TEXT)",
        expected: "2",
    },
    Case {
        name: "geometryn_simple_point",
        sql: "SELECT ST_AsText(ST_GeometryN(ST_GeomFromText('POINT(5 6)'), 1))",
        expected: "POINT(5 6)",
    },
    Case {
        name: "geometryn_multipoint",
        sql: "SELECT ST_AsText(ST_GeometryN(ST_GeomFromText('MULTIPOINT((0 0),(1 1))'), 2))",
        expected: "POINT(1 1)",
    },
    Case {
        name: "geometryn_multilinestring",
        sql: "SELECT ST_AsText(ST_GeometryN(ST_GeomFromText('MULTILINESTRING((0 0,1 1),(2 2,3 3))'), 2))",
        expected: "LINESTRING(2 2,3 3)",
    },
    Case {
        name: "x_point_accessor",
        sql: "SELECT CAST(ST_X(ST_GeomFromText('POINT(3 4)')) AS TEXT)",
        expected: "3.0",
    },
    Case {
        name: "y_point_accessor",
        sql: "SELECT CAST(ST_Y(ST_GeomFromText('POINT(3 4)')) AS TEXT)",
        expected: "4.0",
    },
    Case {
        name: "xmin_geometry_accessor",
        sql: "SELECT CAST(ST_XMin(ST_GeomFromText('LINESTRING(2 3,5 7)')) AS TEXT)",
        expected: "2.0",
    },
    Case {
        name: "ymin_geometry_accessor",
        sql: "SELECT CAST(ST_YMin(ST_GeomFromText('LINESTRING(2 3,5 7)')) AS TEXT)",
        expected: "3.0",
    },
    Case {
        name: "xmax_extent_accessor",
        sql: "SELECT CAST(ST_XMax(ST_Extent(geom)) AS TEXT) FROM public.postgis_regress_points",
        expected: "2.0",
    },
    Case {
        name: "ymax_extent_accessor",
        sql: "SELECT CAST(ST_YMax(ST_Extent(geom)) AS TEXT) FROM public.postgis_regress_points",
        expected: "3.0",
    },
    Case {
        name: "intersects_disjoint_points",
        sql: "SELECT CAST(ST_Intersects(ST_GeomFromText('POINT(0 0)'), ST_GeomFromText('POINT(2 2)')) AS TEXT)",
        expected: "false",
    },
    Case {
        name: "extent_points",
        sql: "SELECT ST_Extent(geom) FROM public.postgis_regress_points",
        expected: "BOX(0 0,2 3)",
    },
    Case {
        name: "find_srid_metadata",
        sql: "SELECT CAST(Find_SRID('public', 'postgis_regress_points', 'geom') AS TEXT)",
        expected: "0",
    },
];
