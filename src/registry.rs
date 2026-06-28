// SPDX-License-Identifier: Apache-2.0
//
// The central maintenance registry.
//
// To add a spatial function you add exactly ONE line to one of the macro
// invocations below: `( "st_sql_name", functions::rust_fn )`. There is no
// boilerplate FFI callback to write — the `register_*!` macros mint a unique
// `unsafe extern "C" fn` per invocation (in its own block scope, so the names
// never collide) and forward to a monomorphized generic executor from
// `dispatch.rs`.
//
// This is the "declarative macro dispatch" architecture from the project
// brief, implemented against the real `quack-rs` C-API binding (where
// `ScalarFunctionBuilder::function` takes a function pointer, not a closure).

use libduckdb_sys::{duckdb_connection, duckdb_data_chunk, duckdb_function_info, duckdb_vector};
use quack_rs::aggregate::AggregateFunctionBuilder;
use quack_rs::prelude::{ExtensionError, NullHandling, ScalarFunctionBuilder, TableFunctionBuilder, TypeId};

use crate::{dispatch, functions};

/// Register every spatial function exposed by this extension.
///
/// Called once from the extension entry point with a live `duckdb_connection`.
/// Returns the first registration error, if any.
pub(crate) fn register_all(con: duckdb_connection) -> Result<(), ExtensionError> {
    // -- Geometry -> Geometry (BLOB -> BLOB) --------------------------------
    macro_rules! register_unary_geom {
        ($name:expr, $func:path) => {{
            // Unique per invocation thanks to the enclosing block scope.
            unsafe extern "C" fn cb(
                _info: duckdb_function_info,
                input: duckdb_data_chunk,
                output: duckdb_vector,
            ) {
                dispatch::unary_geom(input, output, $func);
            }
            // SAFETY: `con` is a valid, open connection provided by DuckDB.
            unsafe {
                ScalarFunctionBuilder::new($name)
                    .param(TypeId::Blob)
                    .returns(TypeId::Blob)
                    .null_handling(NullHandling::SpecialNullHandling)
                    .function(cb)
                    .register(con)?;
            }
        }};
    }

    // -- Geometry, Geometry -> Geometry (BLOB, BLOB -> BLOB) ----------------
    macro_rules! register_binary_geom {
        ($name:expr, $func:path) => {{
            unsafe extern "C" fn cb(
                _info: duckdb_function_info,
                input: duckdb_data_chunk,
                output: duckdb_vector,
            ) {
                dispatch::binary_geom(input, output, $func);
            }
            unsafe {
                ScalarFunctionBuilder::new($name)
                    .param(TypeId::Blob)
                    .param(TypeId::Blob)
                    .returns(TypeId::Blob)
                    .null_handling(NullHandling::SpecialNullHandling)
                    .function(cb)
                    .register(con)?;
            }
        }};
    }

    // -- Geometry, Geometry -> BOOLEAN (predicate) --------------------------
    macro_rules! register_predicate {
        ($name:expr, $func:path) => {{
            unsafe extern "C" fn cb(
                _info: duckdb_function_info,
                input: duckdb_data_chunk,
                output: duckdb_vector,
            ) {
                dispatch::binary_predicate(input, output, $func);
            }
            unsafe {
                ScalarFunctionBuilder::new($name)
                    .param(TypeId::Blob)
                    .param(TypeId::Blob)
                    .returns(TypeId::Boolean)
                    .null_handling(NullHandling::SpecialNullHandling)
                    .function(cb)
                    .register(con)?;
            }
        }};
    }

    // -- Geometry -> DOUBLE -------------------------------------------------
    macro_rules! register_geom_double {
        ($name:expr, $func:path) => {{
            unsafe extern "C" fn cb(
                _info: duckdb_function_info,
                input: duckdb_data_chunk,
                output: duckdb_vector,
            ) {
                dispatch::unary_geom_double(input, output, $func);
            }
            unsafe {
                ScalarFunctionBuilder::new($name)
                    .param(TypeId::Blob)
                    .returns(TypeId::Double)
                    .null_handling(NullHandling::SpecialNullHandling)
                    .function(cb)
                    .register(con)?;
            }
        }};
    }

    // -- Geometry -> VARCHAR ------------------------------------------------
    macro_rules! register_geom_varchar {
        ($name:expr, $func:path) => {{
            unsafe extern "C" fn cb(
                _info: duckdb_function_info,
                input: duckdb_data_chunk,
                output: duckdb_vector,
            ) {
                dispatch::unary_geom_varchar(input, output, $func);
            }
            unsafe {
                ScalarFunctionBuilder::new($name)
                    .param(TypeId::Blob)
                    .returns(TypeId::Varchar)
                    .null_handling(NullHandling::SpecialNullHandling)
                    .function(cb)
                    .register(con)?;
            }
        }};
    }

    // -- (Geometry, INTEGER, INTEGER) -> Geometry (ST_Transform) -----------
    macro_rules! register_geom_int2_to_geom {
        ($name:expr, $func:path) => {{
            unsafe extern "C" fn cb(
                _info: duckdb_function_info,
                input: duckdb_data_chunk,
                output: duckdb_vector,
            ) {
                dispatch::geom_int2_to_geom(input, output, $func);
            }
            unsafe {
                ScalarFunctionBuilder::new($name)
                    .param(TypeId::Blob)
                    .param(TypeId::Integer)
                    .param(TypeId::Integer)
                    .returns(TypeId::Blob)
                    .null_handling(NullHandling::SpecialNullHandling)
                    .function(cb)
                    .register(con)?;
            }
        }};
    }

    // -- (Geometry, INTEGER) -> VARCHAR (ST_AsEWKT) -----------------------
    macro_rules! register_geom_int_to_varchar {
        ($name:expr, $func:path) => {{
            unsafe extern "C" fn cb(
                _info: duckdb_function_info,
                input: duckdb_data_chunk,
                output: duckdb_vector,
            ) {
                dispatch::geom_int_to_varchar(input, output, $func);
            }
            unsafe {
                ScalarFunctionBuilder::new($name)
                    .param(TypeId::Blob)
                    .param(TypeId::Integer)
                    .returns(TypeId::Varchar)
                    .null_handling(NullHandling::SpecialNullHandling)
                    .function(cb)
                    .register(con)?;
            }
        }};
    }

    // -- (Geometry, INTEGER) -> Geometry (indexed accessors) --------------
    macro_rules! register_geom_int_to_geom {
        ($name:expr, $func:path) => {{
            unsafe extern "C" fn cb(
                _info: duckdb_function_info,
                input: duckdb_data_chunk,
                output: duckdb_vector,
            ) {
                dispatch::geom_int_to_geom(input, output, $func);
            }
            unsafe {
                ScalarFunctionBuilder::new($name)
                    .param(TypeId::Blob)
                    .param(TypeId::Integer)
                    .returns(TypeId::Blob)
                    .null_handling(NullHandling::SpecialNullHandling)
                    .function(cb)
                    .register(con)?;
            }
        }};
    }

    // -- (Geometry, Geometry, DOUBLE) -> Geometry --------------------------
    macro_rules! register_geom_int_geom_to_geom {
        ($name:expr, $func:path) => {{
            unsafe extern "C" fn cb(
                _info: duckdb_function_info,
                input: duckdb_data_chunk,
                output: duckdb_vector,
            ) {
                dispatch::geom_int_geom_to_geom(input, output, $func);
            }
            unsafe {
                ScalarFunctionBuilder::new($name)
                    .param(TypeId::Blob)
                    .param(TypeId::Integer)
                    .param(TypeId::Blob)
                    .returns(TypeId::Blob)
                    .null_handling(NullHandling::SpecialNullHandling)
                    .function(cb)
                    .register(con)?;
            }
        }};
    }

    // -- (Geometry, Geometry, DOUBLE) -> Geometry --------------------------
    macro_rules! register_geom_geom_double_to_geom {
        ($name:expr, $func:path) => {{
            unsafe extern "C" fn cb(
                _info: duckdb_function_info,
                input: duckdb_data_chunk,
                output: duckdb_vector,
            ) {
                dispatch::geom_geom_double_to_geom(input, output, $func);
            }
            unsafe {
                ScalarFunctionBuilder::new($name)
                    .param(TypeId::Blob)
                    .param(TypeId::Blob)
                    .param(TypeId::Double)
                    .returns(TypeId::Blob)
                    .null_handling(NullHandling::SpecialNullHandling)
                    .function(cb)
                    .register(con)?;
            }
        }};
    }

    // -- Geometry -> INTEGER ------------------------------------------------
    macro_rules! register_geom_int {
        ($name:expr, $func:path) => {{
            unsafe extern "C" fn cb(
                _info: duckdb_function_info,
                input: duckdb_data_chunk,
                output: duckdb_vector,
            ) {
                dispatch::unary_geom_int(input, output, $func);
            }
            unsafe {
                ScalarFunctionBuilder::new($name)
                    .param(TypeId::Blob)
                    .returns(TypeId::Integer)
                    .null_handling(NullHandling::SpecialNullHandling)
                    .function(cb)
                    .register(con)?;
            }
        }};
    }

    // -- VARCHAR -> Geometry (constructors from WKT) ------------------------
    macro_rules! register_str_geom {
        ($name:expr, $func:path) => {{
            unsafe extern "C" fn cb(
                _info: duckdb_function_info,
                input: duckdb_data_chunk,
                output: duckdb_vector,
            ) {
                dispatch::str_to_geom(input, output, $func);
            }
            unsafe {
                ScalarFunctionBuilder::new($name)
                    .param(TypeId::Varchar)
                    .returns(TypeId::Blob)
                    .null_handling(NullHandling::SpecialNullHandling)
                    .function(cb)
                    .register(con)?;
            }
        }};
    }

    // -- (Geometry, DOUBLE) -> Geometry (transforms) ------------------------
    macro_rules! register_geom_double_to_geom {
        ($name:expr, $func:path) => {{
            unsafe extern "C" fn cb(
                _info: duckdb_function_info,
                input: duckdb_data_chunk,
                output: duckdb_vector,
            ) {
                dispatch::geom_double_to_geom(input, output, $func);
            }
            unsafe {
                ScalarFunctionBuilder::new($name)
                    .param(TypeId::Blob)
                    .param(TypeId::Double)
                    .returns(TypeId::Blob)
                    .null_handling(NullHandling::SpecialNullHandling)
                    .function(cb)
                    .register(con)?;
            }
        }};
    }

    // -- (Geometry, DOUBLE, DOUBLE) -> Geometry (translate/scale) ----------
    macro_rules! register_geom_double2_to_geom {
        ($name:expr, $func:path) => {{
            unsafe extern "C" fn cb(
                _info: duckdb_function_info,
                input: duckdb_data_chunk,
                output: duckdb_vector,
            ) {
                dispatch::geom_double2_to_geom(input, output, $func);
            }
            unsafe {
                ScalarFunctionBuilder::new($name)
                    .param(TypeId::Blob)
                    .param(TypeId::Double)
                    .param(TypeId::Double)
                    .returns(TypeId::Blob)
                    .null_handling(NullHandling::SpecialNullHandling)
                    .function(cb)
                    .register(con)?;
            }
        }};
    }

    // -- (Geometry, DOUBLE×6) -> Geometry (ST_Affine 2D) ------------------
    macro_rules! register_geom_double6_to_geom {
        ($name:expr, $func:path) => {{
            unsafe extern "C" fn cb(
                _info: duckdb_function_info,
                input: duckdb_data_chunk,
                output: duckdb_vector,
            ) {
                dispatch::geom_double6_to_geom(input, output, $func);
            }
            unsafe {
                ScalarFunctionBuilder::new($name)
                    .param(TypeId::Blob)
                    .param(TypeId::Double)
                    .param(TypeId::Double)
                    .param(TypeId::Double)
                    .param(TypeId::Double)
                    .param(TypeId::Double)
                    .param(TypeId::Double)
                    .returns(TypeId::Blob)
                    .null_handling(NullHandling::SpecialNullHandling)
                    .function(cb)
                    .register(con)?;
            }
        }};
    }

    // -- (DOUBLE, DOUBLE) -> Geometry (point constructor) -------------------
    macro_rules! register_doubles2_geom {
        ($name:expr, $func:path) => {{
            unsafe extern "C" fn cb(
                _info: duckdb_function_info,
                input: duckdb_data_chunk,
                output: duckdb_vector,
            ) {
                dispatch::doubles2_to_geom(input, output, $func);
            }
            unsafe {
                ScalarFunctionBuilder::new($name)
                    .param(TypeId::Double)
                    .param(TypeId::Double)
                    .returns(TypeId::Blob)
                    .null_handling(NullHandling::SpecialNullHandling)
                    .function(cb)
                    .register(con)?;
            }
        }};
    }

    // -- (DOUBLE×4) -> Geometry (bbox constructor: ST_MakeEnvelope) --------
    macro_rules! register_doubles4_geom {
        ($name:expr, $func:path) => {{
            unsafe extern "C" fn cb(
                _info: duckdb_function_info,
                input: duckdb_data_chunk,
                output: duckdb_vector,
            ) {
                dispatch::doubles4_to_geom(input, output, $func);
            }
            unsafe {
                ScalarFunctionBuilder::new($name)
                    .param(TypeId::Double)
                    .param(TypeId::Double)
                    .param(TypeId::Double)
                    .param(TypeId::Double)
                    .returns(TypeId::Blob)
                    .null_handling(NullHandling::SpecialNullHandling)
                    .function(cb)
                    .register(con)?;
            }
        }};
    }

    // -- (Geometry, Geometry) -> DOUBLE (measurements) ----------------------
    macro_rules! register_binary_double {
        ($name:expr, $func:path) => {{
            unsafe extern "C" fn cb(
                _info: duckdb_function_info,
                input: duckdb_data_chunk,
                output: duckdb_vector,
            ) {
                dispatch::binary_geom_double(input, output, $func);
            }
            unsafe {
                ScalarFunctionBuilder::new($name)
                    .param(TypeId::Blob)
                    .param(TypeId::Blob)
                    .returns(TypeId::Double)
                    .null_handling(NullHandling::SpecialNullHandling)
                    .function(cb)
                    .register(con)?;
            }
        }};
    }

    // -- (Geometry, Geometry) -> VARCHAR (DE-9IM matrix: ST_Relate) --------
    macro_rules! register_binary_geom_varchar {
        ($name:expr, $func:path) => {{
            unsafe extern "C" fn cb(
                _info: duckdb_function_info,
                input: duckdb_data_chunk,
                output: duckdb_vector,
            ) {
                dispatch::binary_geom_varchar(input, output, $func);
            }
            unsafe {
                ScalarFunctionBuilder::new($name)
                    .param(TypeId::Blob)
                    .param(TypeId::Blob)
                    .returns(TypeId::Varchar)
                    .null_handling(NullHandling::SpecialNullHandling)
                    .function(cb)
                    .register(con)?;
            }
        }};
    }

    // -- (Geometry, Geometry, VARCHAR) -> BOOLEAN (ST_Relate pattern) ------
    macro_rules! register_geom_geom_str_predicate {
        ($name:expr, $func:path) => {{
            unsafe extern "C" fn cb(
                _info: duckdb_function_info,
                input: duckdb_data_chunk,
                output: duckdb_vector,
            ) {
                dispatch::geom_geom_str_predicate(input, output, $func);
            }
            unsafe {
                ScalarFunctionBuilder::new($name)
                    .param(TypeId::Blob)
                    .param(TypeId::Blob)
                    .param(TypeId::Varchar)
                    .returns(TypeId::Boolean)
                    .null_handling(NullHandling::SpecialNullHandling)
                    .function(cb)
                    .register(con)?;
            }
        }};
    }

    // -- Geometry -> BOOLEAN (unary predicates) -----------------------------
    macro_rules! register_geom_bool {
        ($name:expr, $func:path) => {{
            unsafe extern "C" fn cb(
                _info: duckdb_function_info,
                input: duckdb_data_chunk,
                output: duckdb_vector,
            ) {
                dispatch::unary_geom_bool(input, output, $func);
            }
            unsafe {
                ScalarFunctionBuilder::new($name)
                    .param(TypeId::Blob)
                    .returns(TypeId::Boolean)
                    .null_handling(NullHandling::SpecialNullHandling)
                    .function(cb)
                    .register(con)?;
            }
        }};
    }

    // ---------------------------------------------------------------------
    // THE CATALOG. Add new SedonaDB-backed operations by appending a line.
    // ---------------------------------------------------------------------
    register_unary_geom!("st_convexhull", functions::convex_hull);
    // st_envelope is now literal-backed — see the bridge batch below.
    register_unary_geom!("st_centroid", functions::centroid);
    register_unary_geom!("st_geomfromwkb", functions::geom_from_wkb);

    register_binary_geom!("st_intersection", functions::intersection);
    register_binary_geom!("st_union", functions::union);
    register_binary_geom!("st_difference", functions::difference);
    register_binary_geom!("st_symdifference", functions::sym_difference);
    // st_makeline is now literal-backed — see M4 bridge batch below.

    // --- GEOS-backed planar topology (PostGIS-grade fidelity) ---------------
    // These operate at the raw-WKB level (no geo_types round-trip) via a narrow
    // boundary in src/geos_backend.rs. Each fails closed to NULL on GEOS errors.
    {
        unsafe extern "C" fn st_node_cb(
            _: duckdb_function_info, input: duckdb_data_chunk, output: duckdb_vector,
        ) { dispatch::unary_wkb(input, output, crate::geos_backend::node); }
        unsafe {
            ScalarFunctionBuilder::new("st_node")
                .param(TypeId::Blob).returns(TypeId::Blob)
                .null_handling(NullHandling::SpecialNullHandling)
                .function(st_node_cb).register(con)?;
        }
    }
    {
        unsafe extern "C" fn st_polygonize_cb(
            _: duckdb_function_info, input: duckdb_data_chunk, output: duckdb_vector,
        ) { dispatch::unary_wkb(input, output, crate::geos_backend::polygonize); }
        unsafe {
            ScalarFunctionBuilder::new("st_polygonize")
                .param(TypeId::Blob).returns(TypeId::Blob)
                .null_handling(NullHandling::SpecialNullHandling)
                .function(st_polygonize_cb).register(con)?;
        }
    }
    {
        unsafe extern "C" fn st_buildarea_cb(
            _: duckdb_function_info, input: duckdb_data_chunk, output: duckdb_vector,
        ) { dispatch::unary_wkb(input, output, crate::geos_backend::build_area); }
        unsafe {
            ScalarFunctionBuilder::new("st_buildarea")
                .param(TypeId::Blob).returns(TypeId::Blob)
                .null_handling(NullHandling::SpecialNullHandling)
                .function(st_buildarea_cb).register(con)?;
        }
    }
    {
        unsafe extern "C" fn st_voronoipolygons_cb(
            _: duckdb_function_info, input: duckdb_data_chunk, output: duckdb_vector,
        ) {
            dispatch::unary_wkb(input, output, |wkb| {
                crate::geos_backend::voronoi_polygons(wkb, 0.0, None)
            });
        }
        unsafe {
            ScalarFunctionBuilder::new("st_voronoipolygons")
                .param(TypeId::Blob).returns(TypeId::Blob)
                .null_handling(NullHandling::SpecialNullHandling)
                .function(st_voronoipolygons_cb).register(con)?;
        }
        // ST_Snap (geom, geom, DOUBLE) -> geom — GEOS snap (canonical PostGIS engine)
        unsafe extern "C" fn st_snap_cb(
            _: duckdb_function_info, input: duckdb_data_chunk, output: duckdb_vector,
        ) {
            dispatch::binary_wkb_double(input, output, crate::geos_backend::snap);
        }
        unsafe {
            ScalarFunctionBuilder::new("st_snap")
                .param(TypeId::Blob).param(TypeId::Blob).param(TypeId::Double)
                .returns(TypeId::Blob)
                .null_handling(NullHandling::SpecialNullHandling)
                .function(st_snap_cb).register(con)?;
        }
        // ST_MakeValid (geom) -> geom — GEOS MakeValid (canonical PostGIS engine)
        unsafe extern "C" fn st_makevalid_cb(
            _: duckdb_function_info, input: duckdb_data_chunk, output: duckdb_vector,
        ) {
            dispatch::unary_wkb(input, output, crate::geos_backend::make_valid);
        }
        unsafe {
            ScalarFunctionBuilder::new("st_makevalid")
                .param(TypeId::Blob).returns(TypeId::Blob)
                .null_handling(NullHandling::SpecialNullHandling)
                .function(st_makevalid_cb).register(con)?;
        }
    }

    register_predicate!("st_intersects", functions::intersects);
    register_predicate!("st_contains", functions::contains);
    register_predicate!("st_within", functions::within);
    register_predicate!("st_disjoint", functions::disjoint);

    // -- (Geometry, Geometry, DOUBLE) -> BOOLEAN --------------------------
    {
        unsafe extern "C" fn cb(
            _info: duckdb_function_info, input: duckdb_data_chunk, output: duckdb_vector,
        ) {
            dispatch::geom_geom_double_bool(input, output, functions::dwithin);
        }
        unsafe {
            ScalarFunctionBuilder::new("st_dwithin")
                .param(TypeId::Blob).param(TypeId::Blob).param(TypeId::Double)
                .returns(TypeId::Boolean)
                .null_handling(NullHandling::SpecialNullHandling)
                .function(cb).register(con)?;
        }
    }

    register_geom_double!("st_area", functions::area);
    register_geom_double!("st_length", functions::length);
    register_geom_double!("st_perimeter", functions::perimeter);
    // st_x/y/z/m and bbox accessors (xmin..ymax) are now literal-backed — see
    // the bridge batch below (one SedonaDB kernel, two SQL entry points).

    // st_geometrytype / st_astext are now literal-backed (see bridge batch).
    register_geom_varchar!("st_asgeojson", functions::as_geojson);
    register_geom_int_to_varchar!("st_asewkt", functions::as_ewkt);
    register_geom_varchar!("st_assvg", functions::as_svg);
    register_geom_varchar!("st_askml", functions::as_kml);
    register_geom_varchar!("st_astwkb", functions::as_twkb);
    register_geom_varchar!("st_asmvt", functions::as_mvt);

    // st_dimension / st_numpoints / st_npoints / st_numgeometries are now
    // literal-backed — see the bridge batch below.
    register_geom_int!("st_numinteriorrings", functions::num_interior_rings);
    register_geom_int!("st_coorddim", functions::coord_dim);
    // st_zmflag / st_srid are now literal-backed — see bridge batch below.

    register_unary_geom!("st_exteriorring", functions::exterior_ring);
    // st_startpoint / st_endpoint are now literal-backed — see bridge batch.
    register_unary_geom!("st_pointonsurface", functions::point_on_surface);
    // st_asbinary is now literal-backed — see bridge batch.
    // st_makevalid is now GEOS-backed — see topology batch below.
    // st_force2d / st_reverse / st_flipcoordinates / st_points are now
    // literal-backed — see bridge batch.
    register_unary_geom!("st_removerepeatedpoints", functions::remove_repeated_points);
    register_unary_geom!("st_orientedenvelope", functions::oriented_envelope);
    // st_points moved to literal batch above.
    register_unary_geom!("st_boundary", functions::boundary);
    register_unary_geom!("st_forcepolygoncw", functions::force_polygon_cw);
    register_unary_geom!("st_delaunaytriangles", functions::delaunay_triangles);
    register_unary_geom!("st_voronoilines", functions::voronoi_lines);

    // --- Tier 1 remaining: ST_Subdivide, ST_Node ------------------
    // st_snap is now GEOS-backed — see topology batch below.
    register_geom_int_to_geom!("st_subdivide", functions::subdivide);
    // st_node is now GEOS-backed (canonical PostGIS-grade topology, line ~451).

    // --- Tier 1/1b parity batch: editing, transforms, measurements ---------
    // st_affine is now literal-backed — see M4 bridge batch below.
    // st_segmentize is now literal-backed — see bridge batch.
    // st_linesubstring now literal-backed — see M7 bridge batch.
    register_unary_geom!("st_linemerge", functions::line_merge);
    register_geom_int_to_geom!("st_collectionextract", functions::collection_extract);
    register_unary_geom!("st_forcepolygonccw", functions::force_polygon_ccw);
    register_unary_geom!("st_forcerhr", functions::force_rhr);
    register_unary_geom!("st_forcecollection", functions::force_collection);
    register_unary_geom!("st_normalize", functions::normalize);
    register_unary_geom!("st_multi", functions::multi);
    register_unary_geom!("st_triangulatepolygon", functions::triangulate_polygon);
    register_binary_double!("st_maxdistance", functions::max_distance);
    register_binary_geom!("st_longestline", functions::longest_line);
    register_binary_geom!("st_shortestline", functions::shortest_line);
    register_geom_int!("st_nrings", functions::n_rings);
    register_geom_int!("st_numinteriorring", functions::num_interior_rings); // PostGIS alias
    register_predicate!("st_orderingequals", functions::ordering_equals);
    register_geom_bool!("st_ispoint", functions::is_point);
    register_geom_bool!("st_islinestring", functions::is_linestring);
    register_geom_bool!("st_ispolygon", functions::is_polygon);
    // st_asewkb / st_geomfromewkb: asewkb is now literal-backed; geomfromewkb
    // stays local (it is a trust-boundary constructor, not a serialization).
    register_unary_geom!("st_geomfromewkb", functions::geom_from_wkb);
    register_geom_varchar!("st_ashexewkb", functions::as_hex_ewkb);

    // --- Tier 1/1b parity batch round 2: constructors, editing, measurement ---
    register_doubles4_geom!("st_makeenvelope", functions::make_envelope);
    register_unary_geom!("st_makepolygon", functions::make_polygon);
    // st_makepoint is now literal-backed — see M4 bridge batch (alias of st_point).
    {
        // ST_Polygon(linestring, srid) — result tagged with the SRID argument.
        unsafe extern "C" fn cb_polygon(_i: duckdb_function_info, input: duckdb_data_chunk, output: duckdb_vector) {
            dispatch::wkb_int_to_blob(input, output, functions::polygon_raw);
        }
        unsafe {
            ScalarFunctionBuilder::new("st_polygon")
                .param(TypeId::Blob).param(TypeId::Integer).returns(TypeId::Blob)
                .null_handling(NullHandling::SpecialNullHandling)
                .function(cb_polygon).register(con)?;
        }
    }
    register_geom_int_to_geom!("st_removepoint", functions::remove_point);
    register_geom_int_geom_to_geom!("st_setpoint", functions::set_point);
    register_binary_geom!("st_addpoint", functions::add_point);
    register_geom_double_to_geom!("st_simplifypreservetopology", functions::simplify_preserve_topology);
    register_geom_double!("st_minimumclearance", functions::minimum_clearance);
    register_unary_geom!("st_minimumclearanceline", functions::minimum_clearance_line);
    register_geom_double_to_geom!("st_minimumboundingcircle", functions::minimum_bounding_circle);
    register_geom_int_to_geom!("st_generatepoints", functions::generate_points);
    register_geom_varchar!("st_isvalidreason", functions::is_valid_reason);

    // (geom, int) -> geom
    // st_geometryn / st_pointn / st_interiorringn remain local for now (their
    // literal twins require the integer to be a non-constant column in some
    // edge cases). st_setsrid is now literal-backed — see bridge batch.
    register_geom_int_to_geom!("st_geometryn", functions::geometry_n);
    register_geom_int_to_geom!("st_pointn", functions::point_n);
    register_geom_int_to_geom!("st_interiorringn", functions::interior_ring_n);

    // (geom, int, int) -> geom  (CRS reprojection via PROJ)
    register_geom_int2_to_geom!("st_transform", functions::transform);

    register_geom_bool!("st_isvalid", functions::is_valid);
    // st_isempty / st_isclosed / st_iscollection / st_hasz / st_hasm are now
    // literal-backed — see the bridge batch below.
    register_geom_bool!("st_isring", functions::is_ring);

    // --- constructors & mixed-type --------------------------------------
    // st_linefromtext / st_pointfromtext / st_polygonfromtext are now
    // literal-backed — see M4 bridge batch below (SedonaDB typed constructors
    // add type validation matching PostGIS semantics).
    register_str_geom!("st_geomfromtext", functions::geom_from_text);
    register_str_geom!("st_geometryfromtext", functions::geom_from_text); // PostGIS alias

    // --- PostGIS SRID semantics (EWKB SRID tag on the blob) ---------------
    // ST_SetSRID writes the tag, ST_SRID reads it, constructors accept an
    // SRID argument, and the dispatch layer propagates tags through every
    // geometry-producing function. Kernels never see the tag (`from_wkb`
    // strips it at the trust boundary; the bridge strips it before SedonaDB).
    {
        // st_geomfromewkt: SRID=<n>;WKT input, SRID preserved as a tag.
        unsafe extern "C" fn cb_ewkt(_i: duckdb_function_info, input: duckdb_data_chunk, output: duckdb_vector) {
            dispatch::str_to_blob(input, output, functions::geom_from_ewkt_raw);
        }
        // st_geomfromtext(wkt, srid) / st_geometryfromtext(wkt, srid)
        unsafe extern "C" fn cb_text_srid(_i: duckdb_function_info, input: duckdb_data_chunk, output: duckdb_vector) {
            dispatch::str_int_to_blob(input, output, functions::geom_from_text_srid);
        }
        // st_geomfromwkb(wkb, srid)
        unsafe extern "C" fn cb_wkb_srid(_i: duckdb_function_info, input: duckdb_data_chunk, output: duckdb_vector) {
            dispatch::wkb_int_to_blob(input, output, functions::geom_from_wkb_srid);
        }
        // st_setsrid(geom, srid) — byte-level retag
        unsafe extern "C" fn cb_setsrid(_i: duckdb_function_info, input: duckdb_data_chunk, output: duckdb_vector) {
            dispatch::wkb_int_to_blob(input, output, functions::set_srid_raw);
        }
        // st_srid(geom) — read the tag
        unsafe extern "C" fn cb_srid(_i: duckdb_function_info, input: duckdb_data_chunk, output: duckdb_vector) {
            dispatch::wkb_to_int(input, output, functions::srid_raw);
        }
        // st_transform(geom, to_srid) — source CRS from the tag
        unsafe extern "C" fn cb_transform2(_i: duckdb_function_info, input: duckdb_data_chunk, output: duckdb_vector) {
            dispatch::wkb_int_to_blob(input, output, functions::transform_to_srid);
        }
        // st_asewkt(geom) — SRID=<n>;WKT when tagged, plain WKT when not
        unsafe extern "C" fn cb_asewkt1(_i: duckdb_function_info, input: duckdb_data_chunk, output: duckdb_vector) {
            dispatch::wkb_to_varchar(input, output, functions::as_ewkt_auto);
        }
        unsafe {
            ScalarFunctionBuilder::new("st_geomfromewkt")
                .param(TypeId::Varchar).returns(TypeId::Blob)
                .null_handling(NullHandling::SpecialNullHandling)
                .function(cb_ewkt).register(con)?;
            for name in ["st_geomfromtext", "st_geometryfromtext"] {
                ScalarFunctionBuilder::new(name)
                    .param(TypeId::Varchar).param(TypeId::Integer).returns(TypeId::Blob)
                    .null_handling(NullHandling::SpecialNullHandling)
                    .function(cb_text_srid).register(con)?;
            }
            ScalarFunctionBuilder::new("st_geomfromwkb")
                .param(TypeId::Blob).param(TypeId::Integer).returns(TypeId::Blob)
                .null_handling(NullHandling::SpecialNullHandling)
                .function(cb_wkb_srid).register(con)?;
            ScalarFunctionBuilder::new("st_setsrid")
                .param(TypeId::Blob).param(TypeId::Integer).returns(TypeId::Blob)
                .null_handling(NullHandling::SpecialNullHandling)
                .function(cb_setsrid).register(con)?;
            ScalarFunctionBuilder::new("st_srid")
                .param(TypeId::Blob).returns(TypeId::Integer)
                .null_handling(NullHandling::SpecialNullHandling)
                .function(cb_srid).register(con)?;
            ScalarFunctionBuilder::new("st_transform")
                .param(TypeId::Blob).param(TypeId::Integer).returns(TypeId::Blob)
                .null_handling(NullHandling::SpecialNullHandling)
                .function(cb_transform2).register(con)?;
            ScalarFunctionBuilder::new("st_asewkt")
                .param(TypeId::Blob).returns(TypeId::Varchar)
                .null_handling(NullHandling::SpecialNullHandling)
                .function(cb_asewkt1).register(con)?;
        }
    }
    // st_point is now literal-backed — see M4 bridge batch below.
    register_geom_double_to_geom!("st_buffer", functions::buffer);
    register_geom_double_to_geom!("st_simplify", functions::simplify);
    register_geom_double_to_geom!("st_simplifyvw", functions::simplify_vw);
    register_geom_double_to_geom!("st_concavehull", functions::concave_hull);
    // st_rotate is now literal-backed — see M4 bridge batch below.
    register_geom_double_to_geom!("st_lineinterpolatepoint", functions::line_interpolate_point);
    register_geom_double_to_geom!("st_snaptogrid", functions::snap_to_grid);
    // st_translate / st_scale are now literal-backed — see M4 bridge batch.
    register_geom_double2_to_geom!("st_project", functions::project);
    register_binary_double!("st_distance", functions::distance);
    // st_azimuth is now literal-backed — see M4 bridge batch.
    register_binary_double!("st_hausdorffdistance", functions::hausdorff_distance);
    register_binary_double!("st_frechetdistance", functions::frechet_distance);
    register_binary_double!("st_linelocatepoint", functions::line_locate_point);
    register_binary_double!("st_distancesphere", functions::distance_sphere);
    register_geom_double!("st_lengthsphere", functions::length_sphere);
    register_geom_double!("st_areasphere", functions::area_sphere);
    register_binary_geom!("st_closestpoint", functions::closest_point);
    // Geography distance threshold (metres)
    {
        unsafe extern "C" fn cb(_i: duckdb_function_info, input: duckdb_data_chunk, output: duckdb_vector) {
            dispatch::geom_geom_double_bool(input, output, functions::dwithin_sphere);
        }
        unsafe {
            ScalarFunctionBuilder::new("st_dwithinsphere")
                .param(TypeId::Blob).param(TypeId::Blob).param(TypeId::Double)
                .returns(TypeId::Boolean)
                .null_handling(NullHandling::SpecialNullHandling)
                .function(cb).register(con)?;
        }
    }

    // --- Spheroid geodesics (Karney / GeographicLib) -----------------------
    // Higher accuracy than the sphere variants; converges everywhere including
    // antipodal points. Distances in metres, area in m². The 1-arg/2-arg forms
    // use WGS84; the PostGIS `SPHEROID["name",a,rf]` string variants accept
    // any ellipsoid.
    register_binary_double!("st_distancespheroid", functions::distance_spheroid);
    register_geom_double!("st_lengthspheroid", functions::length_spheroid);
    register_geom_double!("st_areaspheroid", functions::area_spheroid);
    {
        unsafe extern "C" fn cb_dist(_i: duckdb_function_info, input: duckdb_data_chunk, output: duckdb_vector) {
            dispatch::geom_geom_str_to_double(input, output, functions::distance_spheroid_custom);
        }
        unsafe extern "C" fn cb_len(_i: duckdb_function_info, input: duckdb_data_chunk, output: duckdb_vector) {
            dispatch::geom_str_to_double(input, output, functions::length_spheroid_custom);
        }
        unsafe extern "C" fn cb_area(_i: duckdb_function_info, input: duckdb_data_chunk, output: duckdb_vector) {
            dispatch::geom_str_to_double(input, output, functions::area_spheroid_custom);
        }
        unsafe {
            ScalarFunctionBuilder::new("st_distancespheroid")
                .param(TypeId::Blob).param(TypeId::Blob).param(TypeId::Varchar)
                .returns(TypeId::Double)
                .null_handling(NullHandling::SpecialNullHandling)
                .function(cb_dist).register(con)?;
            ScalarFunctionBuilder::new("st_lengthspheroid")
                .param(TypeId::Blob).param(TypeId::Varchar)
                .returns(TypeId::Double)
                .null_handling(NullHandling::SpecialNullHandling)
                .function(cb_len).register(con)?;
            ScalarFunctionBuilder::new("st_areaspheroid")
                .param(TypeId::Blob).param(TypeId::Varchar)
                .returns(TypeId::Double)
                .null_handling(NullHandling::SpecialNullHandling)
                .function(cb_area).register(con)?;
        }
    }
    {
        unsafe extern "C" fn cb2(_i: duckdb_function_info, input: duckdb_data_chunk, output: duckdb_vector) {
            dispatch::geom_geom_double_bool(input, output, functions::dwithin_spheroid);
        }
        unsafe {
            ScalarFunctionBuilder::new("st_dwithinspheroid")
                .param(TypeId::Blob).param(TypeId::Blob).param(TypeId::Double)
                .returns(TypeId::Boolean)
                .null_handling(NullHandling::SpecialNullHandling)
                .function(cb2).register(con)?;
        }
    }

    // --- DE-9IM predicates (via geo::Relate) ----------------------------
    register_predicate!("st_equals", functions::equals);
    register_predicate!("st_touches", functions::touches);
    register_predicate!("st_crosses", functions::crosses);
    register_predicate!("st_overlaps", functions::overlaps);
    register_predicate!("st_covers", functions::covers);
    register_predicate!("st_coveredby", functions::covered_by);
    register_predicate!("st_containsproperly", functions::contains_properly);

    // --- DE-9IM relate matrix (via GEOS, the canonical PostGIS engine) ----
    register_binary_geom_varchar!("st_relate", functions::relate);
    register_geom_geom_str_predicate!("st_relate", functions::relate_pattern);

    // --- aggregate: ST_Collect(geom) -> GEOMETRYCOLLECTION ----------------
    // SAFETY: `con` is a valid open connection provided by DuckDB.
    unsafe {
        AggregateFunctionBuilder::new("st_collect")
            .param(TypeId::Blob)
            .returns(TypeId::Blob)
            .state_size(dispatch::collect_state_size)
            .init(dispatch::collect_state_init)
            .update(dispatch::collect_update)
            .combine(dispatch::collect_combine)
            .finalize(dispatch::collect_finalize)
            .destructor(dispatch::collect_destroy)
            .register(con)?;
    }

    // --- scalar: ST_Collect(g1, g2) — PostGIS pairwise collect -------------
    // DuckDB's C API rejects a *scalar* under the same catalog name as an
    // aggregate (verified: duckdb_register_scalar_function fails for
    // 'st_collect'), so the scalar form is registered as st_collect_scalar;
    // sedonadb_rewrite_postgis() maps 2-arg ST_Collect(a, b) onto it.
    register_binary_geom!("st_collect_scalar", functions::collect_two);

    // --- aggregate: ST_Envelope (bbox union) -----------------------------
    unsafe {
        AggregateFunctionBuilder::new("st_envelope_agg")
            .param(TypeId::Blob)
            .returns(TypeId::Blob)
            .state_size(dispatch::envelope_state_size)
            .init(dispatch::envelope_state_init)
            .update(dispatch::envelope_update)
            .combine(dispatch::envelope_combine)
            .finalize(dispatch::envelope_finalize)
            .destructor(dispatch::envelope_destroy)
            .register(con)?;
    }

    // --- aggregate: ST_Union (cascaded polygonal union) ------------------
    unsafe {
        AggregateFunctionBuilder::new("st_union_agg")
            .param(TypeId::Blob)
            .returns(TypeId::Blob)
            .state_size(dispatch::union_state_size)
            .init(dispatch::union_state_init)
            .update(dispatch::union_update)
            .combine(dispatch::union_combine)
            .finalize(dispatch::union_finalize)
            .destructor(dispatch::union_destroy)
            .register(con)?;
    }

    // --- aggregate: ST_Intersection (cascaded polygonal intersection) ----
    unsafe {
        AggregateFunctionBuilder::new("st_intersection_agg")
            .param(TypeId::Blob)
            .returns(TypeId::Blob)
            .state_size(dispatch::intersection_state_size)
            .init(dispatch::intersection_state_init)
            .update(dispatch::intersection_update)
            .combine(dispatch::intersection_combine)
            .finalize(dispatch::intersection_finalize)
            .destructor(dispatch::intersection_destroy)
            .register(con)?;
    }

    // --- aggregate: ST_MakeLine (points → LineString) --------------------
    unsafe {
        AggregateFunctionBuilder::new("st_makeline_agg")
            .param(TypeId::Blob)
            .returns(TypeId::Blob)
            .state_size(dispatch::make_line_state_size)
            .init(dispatch::make_line_state_init)
            .update(dispatch::make_line_update)
            .combine(dispatch::make_line_combine)
            .finalize(dispatch::make_line_finalize)
            .destructor(dispatch::make_line_destroy)
            .register(con)?;
    }

    // --- table function: sedona_join (R-tree spatial join over two parquet files)
    unsafe {
        TableFunctionBuilder::new("sedona_join")
            .param(TypeId::Varchar)
            .param(TypeId::Varchar)
            .param(TypeId::Varchar)
            .bind(crate::spatial_join::join_bind)
            .init(crate::spatial_join::join_init)
            .scan(crate::spatial_join::join_scan)
            .register(con)?;
    }

    // --- table functions: raster (GDAL) ---------------------------------
    unsafe {
        TableFunctionBuilder::new("st_raster_info")
            .param(TypeId::Varchar)
            .bind(crate::raster::raster_info_bind)
            .init(crate::raster::raster_info_init)
            .scan(crate::raster::raster_info_scan)
            .register(con)?;
        TableFunctionBuilder::new("st_raster_stats")
            .param(TypeId::Varchar)
            .param(TypeId::Integer)
            .bind(crate::raster::raster_stats_bind)
            .init(crate::raster::raster_stats_init)
            .scan(crate::raster::raster_stats_scan)
            .register(con)?;
        TableFunctionBuilder::new("st_pixeldata")
            .param(TypeId::Varchar)
            .param(TypeId::Integer)
            .bind(crate::raster::pixeldata_bind)
            .init(crate::raster::pixeldata_init)
            .scan(crate::raster::pixeldata_scan)
            .register(con)?;
        TableFunctionBuilder::new("st_raster_transform")
            .param(TypeId::Varchar)
            .bind(crate::raster::raster_transform_bind)
            .init(crate::raster::raster_transform_init)
            .scan(crate::raster::raster_transform_scan)
            .register(con)?;
    }

    // --- set-returning table functions: ST_Dump family -------------------
    unsafe {
        TableFunctionBuilder::new("st_dump")
            .param(TypeId::Blob)
            .bind(crate::dump::dump_bind)
            .init(crate::dump::dump_init)
            .scan(crate::dump::dump_scan)
            .register(con)?;
        TableFunctionBuilder::new("st_dumppoints")
            .param(TypeId::Blob)
            .bind(crate::dump::dump_points_bind)
            .init(crate::dump::dump_points_init)
            .scan(crate::dump::dump_points_scan)
            .register(con)?;
        TableFunctionBuilder::new("st_dumpsegments")
            .param(TypeId::Blob)
            .bind(crate::dump::dump_segments_bind)
            .init(crate::dump::dump_segments_init)
            .scan(crate::dump::dump_segments_scan)
            .register(con)?;
        TableFunctionBuilder::new("st_dumprings")
            .param(TypeId::Blob)
            .bind(crate::dump::dump_rings_bind)
            .init(crate::dump::dump_rings_init)
            .scan(crate::dump::dump_rings_scan)
            .register(con)?;
        TableFunctionBuilder::new("st_isvaliddetail")
            .param(TypeId::Blob)
            .bind(crate::dump::valid_detail_bind)
            .init(crate::dump::valid_detail_init)
            .scan(crate::dump::valid_detail_scan)
            .register(con)?;
    }

    // ---------------------------------------------------------------------
    // Literal Apache SedonaDB bridge.
    //
    // The functions below register the REAL `sedona-functions` DataFusion
    // scalar UDFs (linked from the apache/sedona-db workspace — see Cargo.toml
    // for the pinned rev) under a `sedona_` prefix, side-by-side with the
    // reimplemented functions above. Each line routes a DuckDB SQL name to a
    // SedonaDB kernel through the DuckDB-chunk ⇄ Arrow bridge in
    // `src/bridge.rs`. This makes the "SedonaDB superset" literal: the same
    // code SedonaDB itself runs is invoked on DuckDB vectors.
    //
    // Extend by appending lines; the entire `default_function_set()` is
    // reachable this way (only item-crs/struct-returning UDFs are omitted —
    // the extension's type system is plain WKB BLOB, not SedonaDB item-crs).
    // ---------------------------------------------------------------------
    macro_rules! register_sedona_blob_blob {
        ($sql_name:expr, $sedona_name:expr) => {{
            unsafe extern "C" fn cb(
                _i: duckdb_function_info, input: duckdb_data_chunk, output: duckdb_vector,
            ) {
                crate::bridge::unary_blob_to_blob($sedona_name, input, output);
            }
            unsafe {
                ScalarFunctionBuilder::new($sql_name)
                    .param(TypeId::Blob)
                    .returns(TypeId::Blob)
                    .null_handling(NullHandling::SpecialNullHandling)
                    .function(cb)
                    .register(con)?;
            }
        }};
    }
    macro_rules! register_sedona_blob_int {
        ($sql_name:expr, $sedona_name:expr) => {{
            unsafe extern "C" fn cb(
                _i: duckdb_function_info, input: duckdb_data_chunk, output: duckdb_vector,
            ) {
                crate::bridge::unary_blob_to_int($sedona_name, input, output);
            }
            unsafe {
                ScalarFunctionBuilder::new($sql_name)
                    .param(TypeId::Blob)
                    .returns(TypeId::Integer)
                    .null_handling(NullHandling::SpecialNullHandling)
                    .function(cb)
                    .register(con)?;
            }
        }};
    }
    macro_rules! register_sedona_blob_bool {
        ($sql_name:expr, $sedona_name:expr) => {{
            unsafe extern "C" fn cb(
                _i: duckdb_function_info, input: duckdb_data_chunk, output: duckdb_vector,
            ) {
                crate::bridge::unary_blob_to_bool($sedona_name, input, output);
            }
            unsafe {
                ScalarFunctionBuilder::new($sql_name)
                    .param(TypeId::Blob)
                    .returns(TypeId::Boolean)
                    .null_handling(NullHandling::SpecialNullHandling)
                    .function(cb)
                    .register(con)?;
            }
        }};
    }
    macro_rules! register_sedona_blob_varchar {
        ($sql_name:expr, $sedona_name:expr) => {{
            unsafe extern "C" fn cb(
                _i: duckdb_function_info, input: duckdb_data_chunk, output: duckdb_vector,
            ) {
                crate::bridge::unary_blob_to_varchar($sedona_name, input, output);
            }
            unsafe {
                ScalarFunctionBuilder::new($sql_name)
                    .param(TypeId::Blob)
                    .returns(TypeId::Varchar)
                    .null_handling(NullHandling::SpecialNullHandling)
                    .function(cb)
                    .register(con)?;
            }
        }};
    }
    macro_rules! register_sedona_blob_double {
        ($sql_name:expr, $sedona_name:expr) => {{
            unsafe extern "C" fn cb(
                _i: duckdb_function_info, input: duckdb_data_chunk, output: duckdb_vector,
            ) {
                crate::bridge::unary_blob_to_double($sedona_name, input, output);
            }
            unsafe {
                ScalarFunctionBuilder::new($sql_name)
                    .param(TypeId::Blob)
                    .returns(TypeId::Double)
                    .null_handling(NullHandling::SpecialNullHandling)
                    .function(cb)
                    .register(con)?;
            }
        }};
    }
    macro_rules! register_sedona_blob_int_blob {
        ($sql_name:expr, $sedona_name:expr) => {{
            unsafe extern "C" fn cb(
                _i: duckdb_function_info, input: duckdb_data_chunk, output: duckdb_vector,
            ) {
                crate::bridge::blob_int_to_blob($sedona_name, input, output);
            }
            unsafe {
                ScalarFunctionBuilder::new($sql_name)
                    .param(TypeId::Blob)
                    .param(TypeId::Integer)
                    .returns(TypeId::Blob)
                    .null_handling(NullHandling::SpecialNullHandling)
                    .function(cb)
                    .register(con)?;
            }
        }};
    }
    macro_rules! register_sedona_doubles2_blob {
        ($sql_name:expr, $sedona_name:expr) => {{
            unsafe extern "C" fn cb(
                _i: duckdb_function_info, input: duckdb_data_chunk, output: duckdb_vector,
            ) {
                crate::bridge::doubles2_to_blob($sedona_name, input, output);
            }
            unsafe {
                ScalarFunctionBuilder::new($sql_name)
                    .param(TypeId::Double)
                    .param(TypeId::Double)
                    .returns(TypeId::Blob)
                    .null_handling(NullHandling::SpecialNullHandling)
                    .function(cb)
                    .register(con)?;
            }
        }};
    }
    macro_rules! register_sedona_binary_blob {
        ($sql_name:expr, $sedona_name:expr) => {{
            unsafe extern "C" fn cb(
                _i: duckdb_function_info, input: duckdb_data_chunk, output: duckdb_vector,
            ) {
                crate::bridge::binary_to_blob($sedona_name, input, output);
            }
            unsafe {
                ScalarFunctionBuilder::new($sql_name)
                    .param(TypeId::Blob)
                    .returns(TypeId::Blob)
                    .null_handling(NullHandling::SpecialNullHandling)
                    .function(cb)
                    .register(con)?;
            }
        }};
    }
    macro_rules! register_sedona_blob_crs {
        ($sql_name:expr, $sedona_name:expr) => {{
            unsafe extern "C" fn cb(
                _i: duckdb_function_info, input: duckdb_data_chunk, output: duckdb_vector,
            ) {
                crate::bridge::unary_blob_extract_crs($sedona_name, input, output);
            }
            unsafe {
                ScalarFunctionBuilder::new($sql_name)
                    .param(TypeId::Blob)
                    .returns(TypeId::Varchar)
                    .null_handling(NullHandling::SpecialNullHandling)
                    .function(cb)
                    .register(con)?;
            }
        }};
    }
    macro_rules! register_sedona_blob_double6_blob {
        ($sql_name:expr, $sedona_name:expr) => {{
            unsafe extern "C" fn cb(
                _i: duckdb_function_info, input: duckdb_data_chunk, output: duckdb_vector,
            ) {
                crate::bridge::blob_double6_to_blob($sedona_name, input, output);
            }
            unsafe {
                ScalarFunctionBuilder::new($sql_name)
                    .param(TypeId::Blob)
                    .param(TypeId::Double).param(TypeId::Double).param(TypeId::Double)
                    .param(TypeId::Double).param(TypeId::Double).param(TypeId::Double)
                    .returns(TypeId::Blob)
                    .null_handling(NullHandling::SpecialNullHandling)
                    .function(cb)
                    .register(con)?;
            }
        }};
    }
    macro_rules! register_sedona_doubles3_blob {
        ($sql_name:expr, $sedona_name:expr) => {{
            unsafe extern "C" fn cb(
                _i: duckdb_function_info, input: duckdb_data_chunk, output: duckdb_vector,
            ) {
                crate::bridge::doubles3_to_blob($sedona_name, input, output);
            }
            unsafe {
                ScalarFunctionBuilder::new($sql_name)
                    .param(TypeId::Double)
                    .param(TypeId::Double)
                    .param(TypeId::Double)
                    .returns(TypeId::Blob)
                    .null_handling(NullHandling::SpecialNullHandling)
                    .function(cb)
                    .register(con)?;
            }
        }};
    }
    macro_rules! register_sedona_doubles4_blob {
        ($sql_name:expr, $sedona_name:expr) => {{
            unsafe extern "C" fn cb(
                _i: duckdb_function_info, input: duckdb_data_chunk, output: duckdb_vector,
            ) {
                crate::bridge::doubles4_to_blob($sedona_name, input, output);
            }
            unsafe {
                ScalarFunctionBuilder::new($sql_name)
                    .param(TypeId::Double)
                    .param(TypeId::Double)
                    .param(TypeId::Double)
                    .param(TypeId::Double)
                    .returns(TypeId::Blob)
                    .null_handling(NullHandling::SpecialNullHandling)
                    .function(cb)
                    .register(con)?;
            }
        }};
    }
    macro_rules! register_sedona_blob_double2_blob {
        ($sql_name:expr, $sedona_name:expr) => {{
            unsafe extern "C" fn cb(
                _i: duckdb_function_info, input: duckdb_data_chunk, output: duckdb_vector,
            ) {
                crate::bridge::blob_double2_to_blob($sedona_name, input, output);
            }
            unsafe {
                ScalarFunctionBuilder::new($sql_name)
                    .param(TypeId::Blob)
                    .param(TypeId::Double)
                    .param(TypeId::Double)
                    .returns(TypeId::Blob)
                    .null_handling(NullHandling::SpecialNullHandling)
                    .function(cb)
                    .register(con)?;
            }
        }};
    }
    macro_rules! register_sedona_blob_blob_blob {
        ($sql_name:expr, $sedona_name:expr) => {{
            unsafe extern "C" fn cb(
                _i: duckdb_function_info, input: duckdb_data_chunk, output: duckdb_vector,
            ) {
                crate::bridge::blob_blob_to_blob($sedona_name, input, output);
            }
            unsafe {
                ScalarFunctionBuilder::new($sql_name)
                    .param(TypeId::Blob)
                    .param(TypeId::Blob)
                    .returns(TypeId::Blob)
                    .null_handling(NullHandling::SpecialNullHandling)
                    .function(cb)
                    .register(con)?;
            }
        }};
    }
    macro_rules! register_sedona_blob_blob_double {
        ($sql_name:expr, $sedona_name:expr) => {{
            unsafe extern "C" fn cb(
                _i: duckdb_function_info, input: duckdb_data_chunk, output: duckdb_vector,
            ) {
                crate::bridge::blob_blob_to_double($sedona_name, input, output);
            }
            unsafe {
                ScalarFunctionBuilder::new($sql_name)
                    .param(TypeId::Blob)
                    .param(TypeId::Blob)
                    .returns(TypeId::Double)
                    .null_handling(NullHandling::SpecialNullHandling)
                    .function(cb)
                    .register(con)?;
            }
        }};
    }
    macro_rules! register_sedona_blob_int_crs {
        ($sql_name:expr, $sedona_name:expr) => {{
            unsafe extern "C" fn cb(
                _i: duckdb_function_info, input: duckdb_data_chunk, output: duckdb_vector,
            ) {
                crate::bridge::blob_int_extract_crs($sedona_name, input, output);
            }
            unsafe {
                ScalarFunctionBuilder::new($sql_name)
                    .param(TypeId::Blob)
                    .param(TypeId::Integer)
                    .returns(TypeId::Varchar)
                    .null_handling(NullHandling::SpecialNullHandling)
                    .function(cb)
                    .register(con)?;
            }
        }};
    }
    macro_rules! register_sedona_varchar_crs {
        ($sql_name:expr, $sedona_name:expr) => {{
            unsafe extern "C" fn cb(
                _i: duckdb_function_info, input: duckdb_data_chunk, output: duckdb_vector,
            ) {
                crate::bridge::varchar_extract_crs($sedona_name, input, output);
            }
            unsafe {
                ScalarFunctionBuilder::new($sql_name)
                    .param(TypeId::Varchar)
                    .returns(TypeId::Varchar)
                    .null_handling(NullHandling::SpecialNullHandling)
                    .function(cb)
                    .register(con)?;
            }
        }};
    }
    macro_rules! register_sedona_varchar_blob {
        ($sql_name:expr, $sedona_name:expr) => {{
            unsafe extern "C" fn cb(
                _i: duckdb_function_info, input: duckdb_data_chunk, output: duckdb_vector,
            ) {
                crate::bridge::varchar_to_blob($sedona_name, input, output);
            }
            unsafe {
                ScalarFunctionBuilder::new($sql_name)
                    .param(TypeId::Varchar)
                    .returns(TypeId::Blob)
                    .null_handling(NullHandling::SpecialNullHandling)
                    .function(cb)
                    .register(con)?;
            }
        }};
    }
    macro_rules! register_sedona_blob_double_blob {
        ($sql_name:expr, $sedona_name:expr) => {{
            unsafe extern "C" fn cb(
                _i: duckdb_function_info, input: duckdb_data_chunk, output: duckdb_vector,
            ) {
                crate::bridge::blob_double_to_blob($sedona_name, input, output);
            }
            unsafe {
                ScalarFunctionBuilder::new($sql_name)
                    .param(TypeId::Blob)
                    .param(TypeId::Double)
                    .returns(TypeId::Blob)
                    .null_handling(NullHandling::SpecialNullHandling)
                    .function(cb)
                    .register(con)?;
            }
        }};
    }

    // The literal batch — real SedonaDB kernels, one line each.
    register_sedona_blob_blob!("sedona_st_envelope", "st_envelope");
    register_sedona_blob_blob!("sedona_st_reverse", "st_reverse");
    register_sedona_blob_blob!("sedona_st_flipcoordinates", "st_flipcoordinates");
    register_sedona_blob_blob!("sedona_st_startpoint", "st_startpoint");
    register_sedona_blob_int!("sedona_st_dimension", "st_dimension");
    register_sedona_blob_int!("sedona_st_numpoints", "st_npoints");
    register_sedona_blob_bool!("sedona_st_isempty", "st_isempty");
    register_sedona_blob_bool!("sedona_st_isclosed", "st_isclosed");
    register_sedona_blob_varchar!("sedona_st_astext", "st_astext");
    register_sedona_blob_varchar!("sedona_st_geometrytype", "st_geometrytype");
    register_sedona_varchar_blob!("sedona_st_geomfromewkt", "st_geomfromewkt");
    register_sedona_blob_double_blob!("sedona_st_segmentize", "st_segmentize");

    // --- Expanded literal batch: ordinate accessors, predicates, accessors ---
    // (geom -> DOUBLE) — bbox/ordinate accessors (the prefilter-join surface)
    register_sedona_blob_double!("sedona_st_x", "st_x");
    register_sedona_blob_double!("sedona_st_y", "st_y");
    register_sedona_blob_double!("sedona_st_z", "st_z");
    register_sedona_blob_double!("sedona_st_m", "st_m");
    register_sedona_blob_double!("sedona_st_xmin", "st_xmin");
    register_sedona_blob_double!("sedona_st_xmax", "st_xmax");
    register_sedona_blob_double!("sedona_st_ymin", "st_ymin");
    register_sedona_blob_double!("sedona_st_ymax", "st_ymax");
    register_sedona_blob_double!("sedona_st_zmin", "st_zmin");
    register_sedona_blob_double!("sedona_st_zmax", "st_zmax");
    register_sedona_blob_double!("sedona_st_mmin", "st_mmin");
    register_sedona_blob_double!("sedona_st_mmax", "st_mmax");
    // (geom -> BOOLEAN)
    register_sedona_blob_bool!("sedona_st_iscollection", "st_iscollection");
    register_sedona_blob_bool!("sedona_st_hasz", "st_hasz");
    register_sedona_blob_bool!("sedona_st_hasm", "st_hasm");
    // (geom -> INTEGER)
    register_sedona_blob_int!("sedona_st_numgeometries", "st_numgeometries");
    // (geom -> geom)
    register_sedona_blob_blob!("sedona_st_force2d", "st_force2d");
    register_sedona_blob_blob!("sedona_st_points", "st_points");
    register_sedona_blob_blob!("sedona_st_endpoint", "st_endpoint");
    // (geom, INTEGER -> geom) — indexed accessors
    register_sedona_blob_int_blob!("sedona_st_geometryn", "st_geometryn");
    register_sedona_blob_int_blob!("sedona_st_pointn", "st_pointn");
    register_sedona_blob_int_blob!("sedona_st_interiorringn", "st_interiorringn");
    register_sedona_blob_int_blob!("sedona_st_setsrid", "st_setsrid");

    // --- Phase D: CRS sidecar access (item-crs struct → VARCHAR crs column) ---
    // sedona_st_geomfromewkt returns plain WKB (item); sedona_st_geomfromewkt_crs
    // extracts the CRS string SedonaDB parsed from the EWKT `SRID=...;` prefix,
    // so callers can read item-crs metadata without a DuckDB struct type model.
    // (st_setsrid sets the SRID at the type level and returns plain WKB, so it
    // has no extractable crs column — geomfromewkt is the CRS-bearing path.)
    register_sedona_varchar_crs!("sedona_st_geomfromewkt_crs", "st_geomfromewkt");

    // --- Phase C expanded batch: constructors, transforms, measurements ---
    // (DOUBLE, DOUBLE) -> geom
    register_sedona_doubles2_blob!("sedona_st_point", "st_point");
    register_sedona_doubles2_blob!("sedona_st_geogpoint", "st_geogpoint");
    // (geom, DOUBLE, DOUBLE) -> geom
    register_sedona_blob_double2_blob!("sedona_st_translate", "st_translate");
    register_sedona_blob_double2_blob!("sedona_st_scale", "st_scale");
    register_sedona_blob_double2_blob!("sedona_st_linesubstring", "st_linesubstring");
    // (geom, geom) -> geom
    register_sedona_blob_blob_blob!("sedona_st_makeline", "st_makeline");
    // (geom, geom) -> DOUBLE
    register_sedona_blob_blob_double!("sedona_st_azimuth", "st_azimuth");
    // (geom) -> BLOB (WKB serialization) / INTEGER (zm flag)
    register_sedona_blob_blob!("sedona_st_asbinary", "st_asbinary");
    register_sedona_blob_blob!("sedona_st_asewkb", "st_asewkb");
    register_sedona_blob_int!("sedona_st_zmflag", "st_zmflag");
    // (geom, DOUBLE) -> geom — numeric-tolerant rotates
    register_sedona_blob_double_blob!("sedona_st_rotate", "st_rotate");
    register_sedona_blob_double_blob!("sedona_st_rotate_x", "st_rotate_x");
    register_sedona_blob_double_blob!("sedona_st_rotate_y", "st_rotate_y");

    // --- P1: complete the literal SedonaDB scalar surface (literal-by-default) ---
    // WKB constructors (BLOB EWKB -> geom; struct return unwrapped to WKB).
    // Input typed as raw Binary — these kernels match is_binary(), not geometry.
    register_sedona_binary_blob!("sedona_st_geomfromwkb", "st_geomfromwkb");
    register_sedona_binary_blob!("sedona_st_geomfromewkb", "st_geomfromewkb");
    register_sedona_binary_blob!("sedona_st_geomfromwkbunchecked", "st_geomfromwkbunchecked");
    register_sedona_binary_blob!("sedona_st_geogfromwkb", "st_geogfromwkb");
    // WKT constructors (VARCHAR -> geom; struct return unwrapped to WKB). The
    // typed constructors mirror PostGIS names; SedonaDB accepts an optional SRID
    // as a second arg, which we do not expose here (use *_crs for CRS metadata).
    register_sedona_varchar_blob!("sedona_st_geomfromwkt", "st_geomfromwkt");
    register_sedona_varchar_blob!("sedona_st_geogfromwkt", "st_geogfromwkt");
    register_sedona_varchar_blob!("sedona_st_linefromtext", "st_linefromtext");
    register_sedona_varchar_blob!("sedona_st_pointfromtext", "st_pointfromtext");
    register_sedona_varchar_blob!("sedona_st_polygonfromtext", "st_polygonfromtext");
    register_sedona_varchar_blob!("sedona_st_mlinefromtext", "st_mlinefromtext");
    register_sedona_varchar_blob!("sedona_st_mpointfromtext", "st_mpointfromtext");
    register_sedona_varchar_blob!("sedona_st_mpolyfromtext", "st_mpolyfromtext");
    register_sedona_varchar_blob!("sedona_st_geomcollfromtext", "st_geomcollfromtext");
    // SRID accessor (geom -> INTEGER; returns the SRID SedonaDB tracks).
    register_sedona_blob_int!("sedona_st_srid", "st_srid");
    // CRS sidecar: ST_SetCRS sets the CRS at the type level (returns item-crs).
    register_sedona_blob_int_crs!("sedona_st_setcrs_crs", "st_set_crs");
    // Dimension forcing (geom [, z] [, m] -> geom); optional doubles default in-kernel.
    register_sedona_blob_double_blob!("sedona_st_force3d", "st_force3d");
    register_sedona_blob_double_blob!("sedona_st_force3dm", "st_force3dm");
    register_sedona_blob_double2_blob!("sedona_st_force4d", "st_force4d");
    // Z/M point constructors (DOUBLE×N -> geom).
    register_sedona_doubles3_blob!("sedona_st_pointz", "st_pointz");
    register_sedona_doubles3_blob!("sedona_st_pointm", "st_pointm");
    register_sedona_doubles4_blob!("sedona_st_pointzm", "st_pointzm");

    // --- P1b: route proven-equivalent st_* accessors to the literal SedonaDB
    // kernel (one implementation, two SQL entry points: st_* + sedona_st_*).
    // These are pure scalar reads (int/bool/double) with zero formatting
    // ambiguity; fidelity.sql + edge_cases.sql prove local == literal across
    // empty/collection/nested/large-coord/Z-dim inputs. The local function
    // bodies remain as dormant fallback but are no longer wired. ---
    // (geom -> INTEGER)
    // st_dimension uses local implementation (not bridge) to return -1 for EMPTY (PostGIS parity)
    register_geom_int!("st_dimension", functions::dimension);
    register_sedona_blob_int!("st_numpoints", "st_npoints");
    register_sedona_blob_int!("st_npoints", "st_npoints");
    register_sedona_blob_int!("st_numgeometries", "st_numgeometries");
    // (geom -> BOOLEAN)
    register_sedona_blob_bool!("st_isempty", "st_isempty");
    register_sedona_blob_bool!("st_isclosed", "st_isclosed");
    register_sedona_blob_bool!("st_iscollection", "st_iscollection");
    register_sedona_blob_bool!("st_hasz", "st_hasz");
    register_sedona_blob_bool!("st_hasm", "st_hasm");
    // (geom -> DOUBLE) — ordinates + bbox
    register_sedona_blob_double!("st_x", "st_x");
    register_sedona_blob_double!("st_y", "st_y");
    register_sedona_blob_double!("st_z", "st_z");
    register_sedona_blob_double!("st_m", "st_m");
    register_sedona_blob_double!("st_xmin", "st_xmin");
    register_sedona_blob_double!("st_xmax", "st_xmax");
    register_sedona_blob_double!("st_ymin", "st_ymin");
    register_sedona_blob_double!("st_ymax", "st_ymax");
    // (geom -> VARCHAR) — text representation + type name (proven equivalent
    // via fidelity.sql over the full corpus).
    register_sedona_blob_varchar!("st_astext", "st_astext");
    register_sedona_blob_varchar!("st_geometrytype", "st_geometrytype");
    // (geom -> geom) — bounding rectangle (compared by area in fidelity.sql;
    // ring winding may legitimately differ CCW/CW).
    register_sedona_blob_blob!("st_envelope", "st_envelope");

    // --- P1 round 2: route proven-equivalent transforms / serializers /
    // accessors to the literal kernel (fidelity.sql proves local == literal). ---
    // (geom -> geom) — geometry transforms
    register_sedona_blob_blob!("st_startpoint", "st_startpoint");
    register_sedona_blob_blob!("st_endpoint", "st_endpoint");
    register_sedona_blob_blob!("st_reverse", "st_reverse");
    register_sedona_blob_blob!("st_flipcoordinates", "st_flipcoordinates");
    register_sedona_blob_blob!("st_force2d", "st_force2d");
    register_sedona_blob_blob!("st_points", "st_points");
    register_sedona_blob_blob!("st_asbinary", "st_asbinary");
    register_sedona_blob_blob!("st_asewkb", "st_asewkb");
    // (geom -> INTEGER)
    register_sedona_blob_int!("st_zmflag", "st_zmflag");
    // st_srid / st_setsrid are LOCAL (EWKB SRID tag on the blob): the literal
    // kernel models SRID at the type level and cannot express it in plain WKB
    // output, so routing them would always return 0 / drop the tag. See the
    // "PostGIS SRID semantics" section below.
    // (geom, DOUBLE) -> geom — segmentize
    register_sedona_blob_double_blob!("st_segmentize", "st_segmentize");
    // (geom, DOUBLE×6) -> geom — ST_Affine 2D (a,b,d,e,xOff,yOff).
    register_sedona_blob_double6_blob!("sedona_st_affine", "st_affine");
    // (geom -> VARCHAR crs) — unary CRS extractor (ST_CRS / ST_SRID crs form).
    register_sedona_blob_crs!("sedona_st_crs_crs", "st_crs");

    // --- Month 1: route public st_* to literal SedonaDB kernel for Z/M
    // dimension forcing. The SedonaDB kernel handles Z/M WKB natively.
    // 2-arg form: st_force3d(geom, z) etc.
    register_sedona_blob_double_blob!("st_force3d", "st_force3d");
    register_sedona_blob_double_blob!("st_force3dz", "st_force3d"); // PostGIS alias
    register_sedona_blob_double_blob!("st_force3dm", "st_force3dm");
    register_sedona_blob_double2_blob!("st_force4d", "st_force4d");

    // 1-arg overloads: st_force3d(geom) etc. — PostGIS defaults z=0, m=0.
    // Delta closed (M-delta-retire): these match PostGIS signature exactly.
    {
        unsafe extern "C" fn force3d_1arg_cb(
            _i: duckdb_function_info, input: duckdb_data_chunk, output: duckdb_vector,
        ) {
            crate::bridge::blob_to_blob_with_default_double("st_force3d", 0.0, input, output);
        }
        unsafe extern "C" fn force3dz_1arg_cb(
            _i: duckdb_function_info, input: duckdb_data_chunk, output: duckdb_vector,
        ) {
            crate::bridge::blob_to_blob_with_default_double("st_force3d", 0.0, input, output);
        }
        unsafe extern "C" fn force3dm_1arg_cb(
            _i: duckdb_function_info, input: duckdb_data_chunk, output: duckdb_vector,
        ) {
            crate::bridge::blob_to_blob_with_default_double("st_force3dm", 0.0, input, output);
        }
        unsafe extern "C" fn force4d_1arg_cb(
            _i: duckdb_function_info, input: duckdb_data_chunk, output: duckdb_vector,
        ) {
            crate::bridge::blob_to_blob_with_default_doubles("st_force4d", 0.0, 0.0, input, output);
        }
        unsafe {
            ScalarFunctionBuilder::new("st_force3d")
                .param(TypeId::Blob)
                .returns(TypeId::Blob)
                .null_handling(NullHandling::SpecialNullHandling)
                .function(force3d_1arg_cb)
                .register(con)?;
            ScalarFunctionBuilder::new("st_force3dz")
                .param(TypeId::Blob)
                .returns(TypeId::Blob)
                .null_handling(NullHandling::SpecialNullHandling)
                .function(force3dz_1arg_cb)
                .register(con)?;
            ScalarFunctionBuilder::new("st_force3dm")
                .param(TypeId::Blob)
                .returns(TypeId::Blob)
                .null_handling(NullHandling::SpecialNullHandling)
                .function(force3dm_1arg_cb)
                .register(con)?;
            ScalarFunctionBuilder::new("st_force4d")
                .param(TypeId::Blob)
                .returns(TypeId::Blob)
                .null_handling(NullHandling::SpecialNullHandling)
                .function(force4d_1arg_cb)
                .register(con)?;
        }
    }

    // --- Month 4: route proven-equivalent st_* to the literal SedonaDB
    // kernel. Each function below was previously local-geo; parity fixtures
    // in tests/reference/m4_fixtures.sql verify st_* == sedona_st_*. ---
    // (DOUBLE, DOUBLE) -> geom — point constructor
    register_sedona_doubles2_blob!("st_point", "st_point");
    register_sedona_doubles2_blob!("st_makepoint", "st_point"); // PostGIS alias
    // (geom, geom) -> geom — line from two geometries
    register_sedona_blob_blob_blob!("st_makeline", "st_makeline");
    // (geom, geom) -> DOUBLE — azimuth between two points
    register_sedona_blob_blob_double!("st_azimuth", "st_azimuth");
    // (geom, DOUBLE×6) -> geom — 2D affine (a,b,d,e,xOff,yOff)
    register_sedona_blob_double6_blob!("st_affine", "st_affine");
    // (geom, DOUBLE) -> geom — rotate about origin
    register_sedona_blob_double_blob!("st_rotate", "st_rotate");
    // (geom, DOUBLE, DOUBLE) -> geom — translate / scale
    register_sedona_blob_double2_blob!("st_translate", "st_translate");
    register_sedona_blob_double2_blob!("st_scale", "st_scale");
    // (VARCHAR) -> geom — typed WKT constructors (SedonaDB kernels add
    // type validation matching PostGIS: ST_LineFromText rejects non-LineString)
    register_sedona_varchar_blob!("st_linefromtext", "st_linefromtext");
    register_sedona_varchar_blob!("st_pointfromtext", "st_pointfromtext");
    register_sedona_varchar_blob!("st_polygonfromtext", "st_polygonfromtext");

    // --- Milestone 7: compatibility evidence and namespace closure ---
    // Route st_linesubstring to literal (was local; COMPATIBILITY.md already
    // claimed routing — this fixes the drift). The bridge blob_double2_blob
    // pattern is proven by st_translate/st_scale.
    register_sedona_blob_double2_blob!("st_linesubstring", "st_linesubstring");
    // st_geomfromwkt: SedonaDB naming for WKT constructor (PostGIS alias for
    // ST_GeomFromText). Routed to literal kernel.
    register_sedona_varchar_blob!("st_geomfromwkt", "st_geomfromwkt");

    // --- Milestone 2: ST_Value — point sampling scalar via GDAL ---
    // st_value(path VARCHAR, band INTEGER, x DOUBLE, y DOUBLE) → DOUBLE
    // Opens the raster, inverts the GeoTransform, reads one pixel. Returns NULL
    // for out-of-bounds/nodata. For bulk sampling, use st_pixeldata + SQL join.
    {
        unsafe extern "C" fn st_value_dispatch(
            _i: duckdb_function_info, input: duckdb_data_chunk, output: duckdb_vector,
        ) {
            unsafe { crate::raster::st_value_cb(_i, input, output) };
        }
        unsafe {
            ScalarFunctionBuilder::new("st_value")
                .param(TypeId::Varchar)
                .param(TypeId::Integer)
                .param(TypeId::Double)
                .param(TypeId::Double)
                .returns(TypeId::Double)
                .null_handling(NullHandling::SpecialNullHandling)
                .function(st_value_dispatch)
                .register(con)?;
        }
    }

    // --- Milestone 9: spatial partition key primitives ---
    // Pure deterministic functions for DuckLake/Hive partitioning and pruning.
    // See ARCHITECTURE.md §L3 for the design and CRS contract.
    
    // ST_BBoxIntersects — cheap bbox-only predicate
    register_predicate!("st_bbox_intersects", crate::spatial_keys::bbox_intersects);
    // ST_QuadKey / ST_GeoHash — cell key strings
    register_geom_int_to_varchar!("st_quadkey", crate::spatial_keys::quadkey);
    register_geom_int_to_varchar!("st_geohash", crate::spatial_keys::geohash);
    
    // ST_Hilbert / ST_Morton — BIGINT sort keys (inline, no matching macro)
    // ST_TileEnvelope(z, x, y) → polygon BLOB
    {
        unsafe extern "C" fn hilbert_cb(
            _i: duckdb_function_info, input: duckdb_data_chunk, output: duckdb_vector,
        ) {
            dispatch::geom_int_to_i64(input, output, crate::spatial_keys::hilbert_key);
        }
        unsafe {
            ScalarFunctionBuilder::new("st_hilbert")
                .param(TypeId::Blob)
                .param(TypeId::Integer)
                .returns(TypeId::BigInt)
                .null_handling(NullHandling::SpecialNullHandling)
                .function(hilbert_cb)
                .register(con)?;
        }
        unsafe extern "C" fn morton_cb(
            _i: duckdb_function_info, input: duckdb_data_chunk, output: duckdb_vector,
        ) {
            dispatch::geom_int_to_i64(input, output, crate::spatial_keys::morton_key);
        }
        unsafe {
            ScalarFunctionBuilder::new("st_morton")
                .param(TypeId::Blob)
                .param(TypeId::Integer)
                .returns(TypeId::BigInt)
                .null_handling(NullHandling::SpecialNullHandling)
                .function(morton_cb)
                .register(con)?;
        }
        // ST_TileEnvelope(z, x, y) → polygon BLOB
        unsafe extern "C" fn tile_envelope_cb(
            _i: duckdb_function_info, input: duckdb_data_chunk, output: duckdb_vector,
        ) {
            dispatch::int3_to_geom(input, output, crate::spatial_keys::tile_envelope);
        }
        unsafe {
            ScalarFunctionBuilder::new("st_tileenvelope")
                .param(TypeId::Integer)
                .param(TypeId::Integer)
                .param(TypeId::Integer)
                .returns(TypeId::Blob)
                .null_handling(NullHandling::SpecialNullHandling)
                .function(tile_envelope_cb)
                .register(con)?;
        }
    }

    // --- Milestone 9: ST_CoveringQuadKeys table function ---
    unsafe {
        TableFunctionBuilder::new("st_covering_quadkeys")
            .param(TypeId::Blob)
            .param(TypeId::Integer)
            .param(TypeId::Integer)
            .bind(crate::dump::covering_quadkeys_bind)
            .init(crate::dump::covering_quadkeys_init)
            .scan(crate::dump::covering_quadkeys_scan)
            .register(con)?;
    }

    // --- Milestone 11: adaptive partitioning helpers ---
    // Pure math functions for estimating partition counts and recommending
    // zoom levels from data statistics.
    {
        // st_estimate_partition_count(total_rows BIGINT, avg_row_bytes INT,
        //   target_object_bytes BIGINT) → INT
        unsafe extern "C" fn est_part_cb(
            _i: duckdb_function_info, input: duckdb_data_chunk, output: duckdb_vector,
        ) {
            dispatch::i64_i32_i64_to_i32(
                input, output, crate::spatial_keys::estimate_partition_count,
            );
        }
        unsafe {
            ScalarFunctionBuilder::new("st_estimate_partition_count")
                .param(TypeId::BigInt)
                .param(TypeId::Integer)
                .param(TypeId::BigInt)
                .returns(TypeId::Integer)
                .null_handling(NullHandling::SpecialNullHandling)
                .function(est_part_cb)
                .register(con)?;
        }
        // st_recommend_zoom(n_partitions INT) → INT
        unsafe extern "C" fn rec_zoom_cb(
            _i: duckdb_function_info, input: duckdb_data_chunk, output: duckdb_vector,
        ) {
            dispatch::i32_to_i32(input, output, crate::spatial_keys::recommend_zoom);
        }
        unsafe {
            ScalarFunctionBuilder::new("st_recommend_zoom")
                .param(TypeId::Integer)
                .returns(TypeId::Integer)
                .null_handling(NullHandling::SpecialNullHandling)
                .function(rec_zoom_cb)
                .register(con)?;
        }
    }

    // ── PostGIS rewrite tool (M25) ─────────────────────────────────────
    {
        unsafe extern "C" fn rewrite_cb(
            _i: duckdb_function_info, input: duckdb_data_chunk, output: duckdb_vector,
        ) {
            let chunk = unsafe { quack_rs::data_chunk::DataChunk::from_raw(input) };
            let reader = unsafe { quack_rs::vector::VectorReader::new(chunk.as_raw(), 0) };
            let mut writer = unsafe { quack_rs::vector::VectorWriter::new(output) };
            for row in 0..reader.row_count() {
                if !unsafe { reader.is_valid(row) } {
                    unsafe { writer.set_null(row) };
                    continue;
                }
                let sql = unsafe { reader.read_str(row) };
                let rewritten = crate::rewriter::rewrite_postgis(sql);
                unsafe { writer.write_varchar(row, &rewritten) };
            }
        }
        unsafe {
            ScalarFunctionBuilder::new("sedonadb_rewrite_postgis")
                .param(TypeId::Varchar)
                .returns(TypeId::Varchar)
                .null_handling(NullHandling::SpecialNullHandling)
                .function(rewrite_cb)
                .register(con)?;
        }
    }

    Ok(())
}
