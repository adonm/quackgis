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
    register_unary_geom!("st_envelope", functions::envelope);
    register_unary_geom!("st_centroid", functions::centroid);
    register_unary_geom!("st_geomfromwkb", functions::geom_from_wkb);

    register_binary_geom!("st_intersection", functions::intersection);
    register_binary_geom!("st_union", functions::union);
    register_binary_geom!("st_difference", functions::difference);
    register_binary_geom!("st_symdifference", functions::sym_difference);
    register_binary_geom!("st_makeline", functions::make_line);

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
    register_geom_double!("st_x", functions::x);
    register_geom_double!("st_y", functions::y);
    register_geom_double!("st_z", functions::z);
    register_geom_double!("st_m", functions::m);
    register_geom_double!("st_length", functions::length);
    register_geom_double!("st_perimeter", functions::perimeter);
    register_geom_double!("st_xmin", functions::xmin);
    register_geom_double!("st_xmax", functions::xmax);
    register_geom_double!("st_ymin", functions::ymin);
    register_geom_double!("st_ymax", functions::ymax);

    register_geom_varchar!("st_geometrytype", functions::geometry_type);
    register_geom_varchar!("st_astext", functions::as_text);
    register_geom_varchar!("st_asgeojson", functions::as_geojson);
    register_geom_int_to_varchar!("st_asewkt", functions::as_ewkt);

    register_geom_int!("st_dimension", functions::dimension);
    register_geom_int!("st_numpoints", functions::num_points);
    register_geom_int!("st_npoints", functions::num_points);
    register_geom_int!("st_numgeometries", functions::num_geometries);
    register_geom_int!("st_numinteriorrings", functions::num_interior_rings);
    register_geom_int!("st_coorddim", functions::coord_dim);
    register_geom_int!("st_zmflag", functions::zm_flag);
    register_geom_int!("st_srid", functions::srid);

    register_unary_geom!("st_exteriorring", functions::exterior_ring);
    register_unary_geom!("st_startpoint", functions::start_point);
    register_unary_geom!("st_endpoint", functions::end_point);
    register_unary_geom!("st_pointonsurface", functions::point_on_surface);
    register_unary_geom!("st_asbinary", functions::geom_from_wkb);
    register_unary_geom!("st_makevalid", functions::make_valid);
    register_unary_geom!("st_force2d", functions::force_2d);
    register_unary_geom!("st_reverse", functions::reverse_geom);
    register_unary_geom!("st_flipcoordinates", functions::flip_coordinates);
    register_unary_geom!("st_removerepeatedpoints", functions::remove_repeated_points);
    register_unary_geom!("st_orientedenvelope", functions::oriented_envelope);
    register_unary_geom!("st_points", functions::points);
    register_unary_geom!("st_boundary", functions::boundary);
    register_unary_geom!("st_forcepolygoncw", functions::force_polygon_cw);
    register_unary_geom!("st_delaunaytriangles", functions::delaunay_triangles);
    register_unary_geom!("st_voronoilines", functions::voronoi_lines);

    // --- Tier 1/1b parity batch: editing, transforms, measurements ---------
    register_geom_double6_to_geom!("st_affine", functions::affine);
    register_geom_double_to_geom!("st_segmentize", functions::segmentize);
    register_geom_double2_to_geom!("st_linesubstring", functions::line_substring);
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
    register_unary_geom!("st_asewkb", functions::geom_from_wkb); // SRID-less: EWKB == WKB
    register_unary_geom!("st_geomfromewkb", functions::geom_from_wkb); // EWKB-tolerant from_wkb
    register_geom_varchar!("st_ashexewkb", functions::as_hex_ewkb);

    // (geom, int) -> geom
    register_geom_int_to_geom!("st_geometryn", functions::geometry_n);
    register_geom_int_to_geom!("st_pointn", functions::point_n);
    register_geom_int_to_geom!("st_interiorringn", functions::interior_ring_n);
    register_geom_int_to_geom!("st_setsrid", functions::set_srid);

    // (geom, int, int) -> geom  (CRS reprojection via PROJ)
    register_geom_int2_to_geom!("st_transform", functions::transform);

    register_geom_bool!("st_isvalid", functions::is_valid);
    register_geom_bool!("st_isempty", functions::is_empty);
    register_geom_bool!("st_isclosed", functions::is_closed);
    register_geom_bool!("st_iscollection", functions::is_collection);
    register_geom_bool!("st_hasz", functions::has_z);
    register_geom_bool!("st_hasm", functions::has_m);
    register_geom_bool!("st_isring", functions::is_ring);

    // --- constructors & mixed-type --------------------------------------
    register_str_geom!("st_geomfromtext", functions::geom_from_text);
    register_str_geom!("st_geomfromewkt", functions::geom_from_ewkt);
    register_str_geom!("st_linefromtext", functions::geom_from_text);
    register_str_geom!("st_pointfromtext", functions::geom_from_text);
    register_str_geom!("st_polygonfromtext", functions::geom_from_text);
    register_doubles2_geom!("st_point", functions::point);
    register_geom_double_to_geom!("st_buffer", functions::buffer);
    register_geom_double_to_geom!("st_simplify", functions::simplify);
    register_geom_double_to_geom!("st_simplifyvw", functions::simplify_vw);
    register_geom_double_to_geom!("st_concavehull", functions::concave_hull);
    register_geom_double_to_geom!("st_rotate", functions::rotate);
    register_geom_double_to_geom!("st_lineinterpolatepoint", functions::line_interpolate_point);
    register_geom_double_to_geom!("st_snaptogrid", functions::snap_to_grid);
    register_geom_double2_to_geom!("st_translate", functions::translate);
    register_geom_double2_to_geom!("st_scale", functions::scale);
    register_geom_double2_to_geom!("st_project", functions::project);
    register_binary_double!("st_distance", functions::distance);
    register_binary_double!("st_azimuth", functions::azimuth);
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

    // --- DE-9IM predicates (via geo::Relate) ----------------------------
    register_predicate!("st_equals", functions::equals);
    register_predicate!("st_touches", functions::touches);
    register_predicate!("st_crosses", functions::crosses);
    register_predicate!("st_overlaps", functions::overlaps);
    register_predicate!("st_covers", functions::covers);
    register_predicate!("st_coveredby", functions::covered_by);

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
    }

    Ok(())
}
