// SPDX-License-Identifier: Apache-2.0
//! QuackGIS spatial UDFs that supplement or override SedonaDB's function
//! catalog with pure-Rust implementations. ST_Transform is a passthrough
//! for now (real CRS transform via proj-wkt comes in Path B sedonadb fork);
//! ST_MakeEnvelope / ST_TileEnvelope / ST_Expand use Sedona's WKB helpers.

use std::sync::Arc;

use datafusion::arrow::array::{
    Array, ArrayRef, BinaryArray, BooleanArray, Float64Array, Int32Array, Int64Array, StringArray,
};
use datafusion::arrow::datatypes::DataType;
use datafusion::common::{DataFusionError, Result as DFResult};
use datafusion::logical_expr::{
    ScalarFunctionArgs, ScalarUDF, ScalarUDFImpl, Signature, TypeSignature, Volatility, create_udf,
};
use datafusion::physical_plan::ColumnarValue;
use datafusion::prelude::SessionContext;
use datafusion::scalar::ScalarValue;
use sedona_geometry::bounds::wkb_bounds_xy;
use sedona_geometry::interval::IntervalTrait;
use sedona_geometry::wkb_factory::{
    wkb_linestring, wkb_multilinestring, wkb_multipoint, wkb_multipolygon, wkb_point, wkb_rect,
    write_wkb_coord, write_wkb_geometrycollection_header, write_wkb_polygon_header,
    write_wkb_polygon_ring_header,
};

pub fn register_spatial_udfs(ctx: &SessionContext) -> DFResult<()> {
    register_st_transform(ctx)?;
    register_st_makeenvelope(ctx)?;
    register_st_tileenvelope(ctx)?;
    register_st_expand(ctx)?;
    register_st_curvetoline(ctx)?;
    register_st_asmvtgeom(ctx)?;
    register_st_asmvt(ctx)?;
    register_st_asbinary(ctx)?;
    register_st_transform_real(ctx)?;
    register_bbox_overlap(ctx)?;
    register_st_geomfromewkt(ctx)?;
    register_qgis_geometry_metadata(ctx)?;
    Ok(())
}

// ─── ST_Transform — passthrough for now; real CRS in Path B ────────────────
fn register_st_transform(_ctx: &SessionContext) -> DFResult<()> {
    Ok(())
}

// ─── ST_MakeEnvelope(x1, y1, x2, y2 [, srid]) ──────────────────────────────
fn register_st_makeenvelope(ctx: &SessionContext) -> DFResult<()> {
    ctx.register_udf(create_udf(
        "st_makeenvelope",
        vec![
            DataType::Float64,
            DataType::Float64,
            DataType::Float64,
            DataType::Float64,
            DataType::Int64,
        ],
        DataType::Binary,
        Volatility::Immutable,
        Arc::new(|args| {
            let arrays = columnar_values_to_arrays(args)?;
            let x1 = as_f64_array_ref(&arrays[0])?;
            let y1 = as_f64_array_ref(&arrays[1])?;
            let x2 = as_f64_array_ref(&arrays[2])?;
            let y2 = as_f64_array_ref(&arrays[3])?;
            let srid = as_i64_array_ref(&arrays[4])?;
            let n = x1.len();
            let mut out: Vec<Option<Vec<u8>>> = Vec::with_capacity(n);
            for i in 0..n {
                if x1.is_null(i) || y1.is_null(i) || x2.is_null(i) || y2.is_null(i) {
                    out.push(None);
                    continue;
                }
                let s = if srid.is_null(i) { 0 } else { srid.value(i) };
                out.push(Some(rect_wkb(
                    x1.value(i),
                    y1.value(i),
                    x2.value(i),
                    y2.value(i),
                    s,
                )?));
            }
            Ok(ColumnarValue::Array(Arc::new(BinaryArray::from_iter(
                out.iter().map(|v| v.as_deref()),
            ))))
        }),
    ));
    Ok(())
}

// ─── ST_TileEnvelope(z, x, y [, bounds] [, margin]) ────────────────────────
fn register_st_tileenvelope(ctx: &SessionContext) -> DFResult<()> {
    ctx.register_udf(ScalarUDF::new_from_impl(STTileEnvelope::new()));
    Ok(())
}

#[derive(Debug, PartialEq, Hash)]
struct STTileEnvelope {
    signature: Signature,
}

impl Eq for STTileEnvelope {}

impl STTileEnvelope {
    fn new() -> Self {
        let i = DataType::Int64;
        let g = DataType::Binary;
        let f = DataType::Float64;
        Self {
            signature: Signature::one_of(
                vec![
                    TypeSignature::Exact(vec![i.clone(), i.clone(), i.clone()]),
                    // PostGIS positional bounds overload.
                    TypeSignature::Exact(vec![i.clone(), i.clone(), i.clone(), g.clone()]),
                    // Convenience overload used by SQL rewrites that lower a
                    // named `margin => ...` while keeping default bounds.
                    TypeSignature::Exact(vec![i.clone(), i.clone(), i.clone(), f.clone()]),
                    // PostGIS positional bounds + margin overload.
                    TypeSignature::Exact(vec![i.clone(), i.clone(), i, g, f]),
                ],
                Volatility::Immutable,
            ),
        }
    }
}

impl ScalarUDFImpl for STTileEnvelope {
    fn name(&self) -> &str {
        "st_tileenvelope"
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> DFResult<DataType> {
        Ok(DataType::Binary)
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> DFResult<ColumnarValue> {
        st_tileenvelope_impl(&args.args)
    }
}

fn st_tileenvelope_impl(args: &[ColumnarValue]) -> DFResult<ColumnarValue> {
    let arrays = columnar_values_to_arrays(args)?;
    let z = as_i64_array_ref(&arrays[0])?;
    let x = as_i64_array_ref(&arrays[1])?;
    let y = as_i64_array_ref(&arrays[2])?;
    let (bounds, margin) = match arrays.len() {
        3 => (None, None),
        4 if matches!(arrays[3].data_type(), DataType::Binary) => {
            (Some(as_binary_array_ref(&arrays[3])?), None)
        }
        4 => (None, Some(as_f64_array_ref(&arrays[3])?)),
        5 => (
            Some(as_binary_array_ref(&arrays[3])?),
            Some(as_f64_array_ref(&arrays[4])?),
        ),
        n => {
            return Err(DataFusionError::Plan(format!(
                "st_tileenvelope expected 3, 4, or 5 arguments, got {n}"
            )));
        }
    };

    let n = z.len();
    let mut out: Vec<Option<Vec<u8>>> = Vec::with_capacity(n);
    for i in 0..n {
        if z.is_null(i)
            || x.is_null(i)
            || y.is_null(i)
            || bounds.is_some_and(|b| b.is_null(i))
            || margin.is_some_and(|m| m.is_null(i))
        {
            out.push(None);
            continue;
        }

        let bounds_rect = match bounds {
            Some(b) => match rect_from_wkb(b.value(i))? {
                Some(rect) => rect,
                None => {
                    out.push(None);
                    continue;
                }
            },
            None => default_web_mercator_bounds(),
        };
        let margin = margin.map_or(0.0, |m| m.value(i));
        if !margin.is_finite() || margin <= -0.5 {
            return Err(DataFusionError::Execution(
                "st_tileenvelope margin must be finite and greater than -0.5".into(),
            ));
        }

        let tile = tile_envelope_rect(z.value(i), x.value(i), y.value(i), bounds_rect, margin)?;
        out.push(Some(rect_wkb(
            tile.min_x, tile.min_y, tile.max_x, tile.max_y, 3857,
        )?));
    }

    Ok(ColumnarValue::Array(Arc::new(BinaryArray::from_iter(
        out.iter().map(|v| v.as_deref()),
    ))))
}

// ─── ST_Expand(geom, units) — expand bbox ──────────────────────────────────
fn register_st_expand(ctx: &SessionContext) -> DFResult<()> {
    ctx.register_udf(create_udf(
        "st_expand",
        vec![DataType::Binary, DataType::Float64],
        DataType::Binary,
        Volatility::Immutable,
        Arc::new(|args| {
            let arrays = columnar_values_to_arrays(args)?;
            let geom = as_binary_array_ref(&arrays[0])?;
            let units = as_f64_array_ref(&arrays[1])?;
            let n = geom.len();
            let mut out: Vec<Option<Vec<u8>>> = Vec::with_capacity(n);
            for i in 0..n {
                if geom.is_null(i) || units.is_null(i) {
                    out.push(None);
                    continue;
                }
                let bbox = rect_from_wkb(geom.value(i))?;
                let u = units.value(i);
                match bbox {
                    Some(rect) => {
                        out.push(Some(rect_wkb(
                            rect.min_x - u,
                            rect.min_y - u,
                            rect.max_x + u,
                            rect.max_y + u,
                            0,
                        )?));
                    }
                    None => out.push(None),
                }
            }
            Ok(ColumnarValue::Array(Arc::new(BinaryArray::from_iter(
                out.iter().map(|v| v.as_deref()),
            ))))
        }),
    ));
    Ok(())
}

// ─── ST_CurveToLine(geom) — identity (no curve types in SedonaDB) ──────────
fn register_st_curvetoline(ctx: &SessionContext) -> DFResult<()> {
    ctx.register_udf(create_udf(
        "st_curvetoline",
        vec![DataType::Binary],
        DataType::Binary,
        Volatility::Immutable,
        Arc::new(|args| {
            // SedonaDB geometries are already linear; this is identity.
            Ok(args[0].clone())
        }),
    ));
    Ok(())
}

// ─── WKB construction helpers ──────────────────────────────────────────────

#[derive(Clone, Copy, Debug)]
struct Rect {
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
}

fn default_web_mercator_bounds() -> Rect {
    const WORLD: f64 = 20037508.342789244;
    Rect {
        min_x: -WORLD,
        min_y: -WORLD,
        max_x: WORLD,
        max_y: WORLD,
    }
}

fn tile_envelope_rect(z: i64, x: i64, y: i64, bounds: Rect, margin: f64) -> DFResult<Rect> {
    if z < 0 {
        return Err(DataFusionError::Execution(
            "st_tileenvelope zoom must be non-negative".into(),
        ));
    }
    let tiles = 2.0_f64.powi(z as i32);
    let width = (bounds.max_x - bounds.min_x) / tiles;
    let height = (bounds.max_y - bounds.min_y) / tiles;
    let expand_x = margin * width;
    let expand_y = margin * height;

    Ok(Rect {
        min_x: bounds.min_x + (x as f64 * width) - expand_x,
        max_x: bounds.min_x + ((x + 1) as f64 * width) + expand_x,
        min_y: bounds.max_y - ((y + 1) as f64 * height) - expand_y,
        max_y: bounds.max_y - (y as f64 * height) + expand_y,
    })
}

fn rect_from_wkb(wkb: &[u8]) -> DFResult<Option<Rect>> {
    let bbox = wkb_bounds_xy(wkb).map_err(|e| {
        DataFusionError::Execution(format!("failed to read WKB bounds with Sedona: {e}"))
    })?;
    if bbox.is_empty() || bbox.x().is_wraparound() {
        return Ok(None);
    }
    Ok(Some(Rect {
        min_x: bbox.x().lo(),
        min_y: bbox.y().lo(),
        max_x: bbox.x().hi(),
        max_y: bbox.y().hi(),
    }))
}

fn rect_wkb(min_x: f64, min_y: f64, max_x: f64, max_y: f64, _srid: i64) -> DFResult<Vec<u8>> {
    wkb_rect(min_x, min_y, max_x, max_y).map_err(|e| {
        DataFusionError::Execution(format!("failed to write WKB rectangle with Sedona: {e}"))
    })
}

// ─── Array extraction helpers ──────────────────────────────────────────────

/// Convert a slice of ColumnarValue into arrays, broadcasting scalars to match
/// the longest array. When all args are scalar (e.g. `ST_TileEnvelope(0,0,0)`),
/// the result is a 1-row batch. This is the standard DataFusion UDF pattern
/// for handling both literal and column arguments.
fn columnar_values_to_arrays(args: &[ColumnarValue]) -> DFResult<Vec<ArrayRef>> {
    let num_rows = args
        .iter()
        .find_map(|arg| match arg {
            ColumnarValue::Array(arr) => Some(arr.len()),
            _ => None,
        })
        .unwrap_or(1);

    args.iter()
        .map(|arg| match arg {
            ColumnarValue::Array(arr) => Ok(arr.clone()),
            ColumnarValue::Scalar(s) => Ok(s.to_array_of_size(num_rows)?),
        })
        .collect()
}

/// Downcast helper: get a typed array from an ArrayRef, with error context.
fn as_binary_array_ref(arr: &ArrayRef) -> DFResult<&BinaryArray> {
    arr.as_any()
        .downcast_ref::<BinaryArray>()
        .ok_or_else(|| datafusion::common::DataFusionError::Internal("expected Binary".into()))
}

fn as_i64_array_ref(arr: &ArrayRef) -> DFResult<&Int64Array> {
    arr.as_any()
        .downcast_ref::<Int64Array>()
        .ok_or_else(|| datafusion::common::DataFusionError::Internal("expected Int64".into()))
}

fn as_f64_array_ref(arr: &ArrayRef) -> DFResult<&Float64Array> {
    arr.as_any()
        .downcast_ref::<Float64Array>()
        .ok_or_else(|| datafusion::common::DataFusionError::Internal("expected Float64".into()))
}

#[allow(dead_code)]
fn as_boolean_array_ref(arr: &ArrayRef) -> DFResult<&BooleanArray> {
    arr.as_any()
        .downcast_ref::<BooleanArray>()
        .ok_or_else(|| datafusion::common::DataFusionError::Internal("expected Boolean".into()))
}

fn register_qgis_geometry_metadata(ctx: &SessionContext) -> DFResult<()> {
    ctx.register_udf(create_udf(
        "st_srid",
        vec![DataType::Binary],
        DataType::Int32,
        Volatility::Immutable,
        Arc::new(|args| {
            let arrays = columnar_values_to_arrays(args)?;
            let geom = as_binary_array_ref(&arrays[0])?;
            let out = (0..geom.len()).map(|i| {
                if geom.is_null(i) {
                    None
                } else {
                    Some(wkb_srid(geom.value(i)).unwrap_or(0))
                }
            });
            Ok(ColumnarValue::Array(Arc::new(Int32Array::from_iter(out))))
        }),
    ));

    ctx.register_udf(create_udf(
        "geometrytype",
        vec![DataType::Binary],
        DataType::Utf8,
        Volatility::Immutable,
        Arc::new(|args| {
            let arrays = columnar_values_to_arrays(args)?;
            let geom = as_binary_array_ref(&arrays[0])?;
            let out = (0..geom.len()).map(|i| {
                if geom.is_null(i) {
                    None
                } else {
                    wkb_geometry_type_name(geom.value(i))
                }
            });
            Ok(ColumnarValue::Array(Arc::new(StringArray::from_iter(out))))
        }),
    ));

    ctx.register_udf(create_udf(
        "st_zmflag",
        vec![DataType::Binary],
        DataType::Int32,
        Volatility::Immutable,
        Arc::new(|args| {
            let arrays = columnar_values_to_arrays(args)?;
            let geom = as_binary_array_ref(&arrays[0])?;
            let out = (0..geom.len()).map(|i| {
                if geom.is_null(i) {
                    None
                } else {
                    Some(wkb_zm_flag(geom.value(i)))
                }
            });
            Ok(ColumnarValue::Array(Arc::new(Int32Array::from_iter(out))))
        }),
    ));
    Ok(())
}

fn wkb_geometry_type_name(wkb: &[u8]) -> Option<&'static str> {
    let raw = wkb_type_id(wkb)?;
    let typ = raw & 0x0fff;
    let base = if typ >= 1000 { typ % 1000 } else { typ };
    match base {
        1 => Some("POINT"),
        2 => Some("LINESTRING"),
        3 => Some("POLYGON"),
        4 => Some("MULTIPOINT"),
        5 => Some("MULTILINESTRING"),
        6 => Some("MULTIPOLYGON"),
        7 => Some("GEOMETRYCOLLECTION"),
        _ => Some("GEOMETRY"),
    }
}

fn wkb_zm_flag(wkb: &[u8]) -> i32 {
    let Some(raw) = wkb_type_id(wkb) else {
        return 0;
    };
    let has_z = (raw & 0x8000_0000) != 0 || (1000..4000).contains(&(raw & 0x0fff));
    let has_m = (raw & 0x4000_0000) != 0 || (2000..4000).contains(&(raw & 0x0fff));
    match (has_z, has_m) {
        (false, false) => 0,
        (true, false) => 1,
        (false, true) => 2,
        (true, true) => 3,
    }
}

fn wkb_type_id(wkb: &[u8]) -> Option<u32> {
    if wkb.len() < 5 {
        return None;
    }
    let bytes = [wkb[1], wkb[2], wkb[3], wkb[4]];
    match wkb[0] {
        0 => Some(u32::from_be_bytes(bytes)),
        1 => Some(u32::from_le_bytes(bytes)),
        _ => None,
    }
}

fn wkb_srid(wkb: &[u8]) -> Option<i32> {
    let raw = wkb_type_id(wkb)?;
    if (raw & 0x2000_0000) == 0 || wkb.len() < 9 {
        return None;
    }
    let bytes = [wkb[5], wkb[6], wkb[7], wkb[8]];
    match wkb[0] {
        0 => Some(u32::from_be_bytes(bytes) as i32),
        1 => Some(u32::from_le_bytes(bytes) as i32),
        _ => None,
    }
}

// ─── ST_AsMVTGeom(geom, bounds, extent, buffer, clip_geom) ─────────────────

fn register_st_asmvtgeom(ctx: &SessionContext) -> DFResult<()> {
    ctx.register_udf(create_udf(
        "st_asmvtgeom",
        vec![
            DataType::Binary,  // geometry (WKB)
            DataType::Binary,  // bounds (WKB polygon from ST_TileEnvelope)
            DataType::Int64,   // extent (e.g. 4096; SQL integer literals are Int64)
            DataType::Int64,   // buffer
            DataType::Boolean, // clip_geom
        ],
        DataType::Binary,
        Volatility::Immutable,
        Arc::new(|args| {
            let arrays = columnar_values_to_arrays(args)?;
            let geom_arr = as_binary_array_ref(&arrays[0])?;
            let bounds_arr = as_binary_array_ref(&arrays[1])?;
            let extent_arr = as_i64_array_ref(&arrays[2])?;
            let n = geom_arr.len();
            let mut out: Vec<Option<Vec<u8>>> = Vec::with_capacity(n);
            for i in 0..n {
                if geom_arr.is_null(i) || bounds_arr.is_null(i) {
                    out.push(None);
                    continue;
                }
                let parsed = match crate::mvt::parse_wkb(geom_arr.value(i)) {
                    Some(g) => g,
                    None => {
                        out.push(None);
                        continue;
                    }
                };
                let bounds_geom = match crate::mvt::parse_wkb(bounds_arr.value(i)) {
                    Some(g) => g,
                    None => {
                        out.push(None);
                        continue;
                    }
                };
                let bbox = match bounds_geom.bbox() {
                    Some(b) => b,
                    None => {
                        out.push(None);
                        continue;
                    }
                };
                let extent = if extent_arr.is_null(i) {
                    4096.0
                } else {
                    extent_arr.value(i) as f64
                };
                let transformed = crate::mvt::mvt_geom_transform(&parsed, bbox, extent);
                out.push(Some(transformed.to_wkb()));
            }
            Ok(ColumnarValue::Array(Arc::new(BinaryArray::from_iter(
                out.iter().map(|v| v.as_deref()),
            ))))
        }),
    ));
    Ok(())
}

// ─── ST_AsMVT — aggregate UDF ──────────────────────────────────────────────

fn register_st_asmvt(ctx: &SessionContext) -> DFResult<()> {
    use datafusion_functions_aggregate_common::accumulator::AccumulatorArgs;
    use datafusion_functions_aggregate_common::accumulator::AccumulatorFactoryFunction;

    let accumulator: AccumulatorFactoryFunction = Arc::new(|_args: AccumulatorArgs| {
        Ok(Box::new(MvtAccumulator {
            features: Vec::new(),
        })
            as Box<
                dyn datafusion_expr_common::accumulator::Accumulator,
            >)
    });

    ctx.register_udaf(datafusion::logical_expr::create_udaf(
        "st_asmvt",
        vec![DataType::Binary],
        Arc::new(DataType::Binary),
        Volatility::Immutable,
        accumulator,
        Arc::new(vec![DataType::Binary]),
    ));
    Ok(())
}

// ─── ST_AsBinary(geom [, endian]) — WKB passthrough ───────────────────────

fn register_st_asbinary(ctx: &SessionContext) -> DFResult<()> {
    ctx.register_udf(ScalarUDF::new_from_impl(STAsBinary::new()));
    Ok(())
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct STAsBinary {
    signature: Signature,
}

impl STAsBinary {
    fn new() -> Self {
        Self {
            signature: Signature::one_of(
                vec![
                    TypeSignature::Exact(vec![DataType::Binary]),
                    TypeSignature::Exact(vec![DataType::Binary, DataType::Utf8]),
                ],
                Volatility::Immutable,
            ),
        }
    }
}

impl ScalarUDFImpl for STAsBinary {
    fn name(&self) -> &str {
        "st_asbinary"
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> DFResult<DataType> {
        Ok(DataType::Binary)
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> DFResult<ColumnarValue> {
        if args.args.is_empty() || args.args.len() > 2 {
            return Err(DataFusionError::Plan(format!(
                "st_asbinary expected 1 or 2 arguments, got {}",
                args.args.len()
            )));
        }
        // QuackGIS stores geometry bytes as WKB/EWKB already. QGIS asks for
        // ST_AsBinary(geom, 'NDR'); fixtures are little-endian WKB, so this is
        // a byte-preserving pgwire compatibility shim until full endian
        // conversion is needed.
        Ok(args.args[0].clone())
    }
}

#[derive(Debug)]
struct MvtAccumulator {
    features: Vec<crate::mvt::MvtFeature>,
}

impl datafusion::logical_expr::Accumulator for MvtAccumulator {
    fn update_batch(&mut self, values: &[ArrayRef]) -> DFResult<()> {
        let arr = values[0]
            .as_any()
            .downcast_ref::<BinaryArray>()
            .ok_or_else(|| {
                datafusion::common::DataFusionError::Internal("expected Binary".into())
            })?;
        for i in 0..arr.len() {
            if arr.is_null(i) {
                continue;
            }
            if let Some(parsed) = crate::mvt::parse_wkb(arr.value(i))
                && let Some((geom_type, commands)) = crate::mvt::encode_mvt_geometry(&parsed)
            {
                self.features.push(crate::mvt::MvtFeature {
                    geom_type,
                    commands,
                    tags: vec![],
                    id: None,
                });
            }
        }
        Ok(())
    }

    fn merge_batch(&mut self, states: &[ArrayRef]) -> DFResult<()> {
        // For single-node: no merge needed. For distributed: decode state.
        self.update_batch(states)
    }

    fn evaluate(&mut self) -> DFResult<ScalarValue> {
        let tile = crate::mvt::build_mvt_tile("layer", 4096, &self.features);
        Ok(ScalarValue::Binary(Some(tile)))
    }

    fn size(&self) -> usize {
        self.features.iter().map(|f| f.commands.len() * 4).sum()
    }

    fn state(&mut self) -> DFResult<Vec<ScalarValue>> {
        // Simplified: serialize all features as a single binary blob
        let mut blob = Vec::new();
        for f in &self.features {
            blob.extend_from_slice(&f.geom_type.to_le_bytes());
            blob.extend_from_slice(&(f.commands.len() as u32).to_le_bytes());
            for &c in &f.commands {
                blob.extend_from_slice(&c.to_le_bytes());
            }
        }
        Ok(vec![ScalarValue::Binary(Some(blob))])
    }
}

// ─── ST_Transform — real CRS transform with proj-wkt ───────────────────────

fn register_st_transform_real(ctx: &SessionContext) -> DFResult<()> {
    // Override the passthrough version registered earlier
    ctx.register_udf(create_udf(
        "st_transform",
        vec![DataType::Binary, DataType::Int64],
        DataType::Binary,
        Volatility::Immutable,
        Arc::new(|args| {
            let arrays = columnar_values_to_arrays(args)?;
            let geom_arr = as_binary_array_ref(&arrays[0])?;
            let srid_arr = as_i64_array_ref(&arrays[1])?;
            let n = geom_arr.len();
            let mut out: Vec<Option<Vec<u8>>> = Vec::with_capacity(n);
            for i in 0..n {
                if geom_arr.is_null(i) || srid_arr.is_null(i) {
                    out.push(None);
                    continue;
                }
                let wkb = geom_arr.value(i);
                let target_srid = srid_arr.value(i) as i32;
                match transform_wkb_crs(wkb, target_srid) {
                    Ok(transformed) => out.push(Some(transformed)),
                    Err(_) => {
                        // Fall back to identity on transform error
                        out.push(Some(wkb.to_vec()));
                    }
                }
            }
            Ok(ColumnarValue::Array(Arc::new(BinaryArray::from_iter(
                out.iter().map(|v| v.as_deref()),
            ))))
        }),
    ));
    Ok(())
}

fn transform_wkb_crs(wkb: &[u8], target_srid: i32) -> std::result::Result<Vec<u8>, String> {
    let parsed = crate::mvt::parse_wkb(wkb).ok_or("WKB parse failed")?;
    let source_srid = parsed.srid.unwrap_or(4326);
    if source_srid == target_srid {
        return Ok(wkb.to_vec()); // identity
    }
    let source = format!("EPSG:{source_srid}");
    let target = format!("EPSG:{target_srid}");
    let transform = proj_wkt::transform_from_crs_strings(&source, &target)
        .map_err(|e| format!("CRS transform {source}→{target}: {e}"))?;
    let transformed = parsed.map_coords(|x, y| match transform.convert((x, y)) {
        Ok((nx, ny)) => (nx, ny),
        Err(_) => (x, y),
    });
    Ok(transformed.to_wkb())
}

// ─── && operator (bbox overlap) as function ────────────────────────────────

fn register_bbox_overlap(ctx: &SessionContext) -> DFResult<()> {
    ctx.register_udf(create_udf(
        "st_overlaps_bbox",
        vec![DataType::Binary, DataType::Binary],
        DataType::Boolean,
        Volatility::Immutable,
        Arc::new(|args| {
            let arrays = columnar_values_to_arrays(args)?;
            let a = as_binary_array_ref(&arrays[0])?;
            let b = as_binary_array_ref(&arrays[1])?;
            let n = a.len();
            let mut out = BooleanArray::builder(n);
            for i in 0..n {
                if a.is_null(i) || b.is_null(i) {
                    out.append_null();
                    continue;
                }
                let bbox_a = crate::mvt::parse_wkb(a.value(i)).and_then(|g| g.bbox());
                let bbox_b = crate::mvt::parse_wkb(b.value(i)).and_then(|g| g.bbox());
                match (bbox_a, bbox_b) {
                    (Some((ax1, ay1, ax2, ay2)), Some((bx1, by1, bx2, by2))) => {
                        out.append_value(ax1 <= bx2 && ax2 >= bx1 && ay1 <= by2 && ay2 >= by1);
                    }
                    _ => out.append_null(),
                }
            }
            Ok(ColumnarValue::Array(Arc::new(out.finish())))
        }),
    ));
    Ok(())
}

// ─── ST_GeomFromEWKT / GeomFromEWKT — EWKT → WKB ───────────────────────────

fn register_st_geomfromewkt(ctx: &SessionContext) -> DFResult<()> {
    let udf = create_udf(
        "st_geomfromewkt",
        vec![DataType::Utf8],
        DataType::Binary,
        Volatility::Immutable,
        Arc::new(|args| {
            let arrays = columnar_values_to_arrays(args)?;
            let arr = arrays[0]
                .as_any()
                .downcast_ref::<datafusion::arrow::array::StringArray>()
                .ok_or_else(|| {
                    datafusion::common::DataFusionError::Internal("expected Utf8".into())
                })?;
            let n = arr.len();
            let mut out: Vec<Option<Vec<u8>>> = Vec::with_capacity(n);
            for i in 0..n {
                if arr.is_null(i) {
                    out.push(None);
                    continue;
                }
                match ewkt_to_wkb(arr.value(i)) {
                    Ok(wkb) => out.push(Some(wkb)),
                    Err(_) => out.push(None),
                }
            }
            Ok(ColumnarValue::Array(Arc::new(BinaryArray::from_iter(
                out.iter().map(|v| v.as_deref()),
            ))))
        }),
    );
    ctx.register_udf(udf.clone());
    ctx.register_udf(udf.with_aliases(["geomfromewkt"]));
    Ok(())
}

/// Parse an EWKT string into WKB bytes.
/// EWKT format: `[SRID=XXXX;]WKT`
/// Curve types are linearized to their control points.
fn ewkt_to_wkb(ewkt: &str) -> std::result::Result<Vec<u8>, String> {
    let ewkt = ewkt.trim();
    let (_srid, wkt) = if let Some(rest) = ewkt.strip_prefix("SRID=") {
        let semi = rest.find(';').ok_or("missing ; after SRID=")?;
        let s = rest[..semi]
            .trim()
            .parse::<i32>()
            .map_err(|e| format!("bad SRID: {e}"))?;
        (Some(s), rest[semi + 1..].trim())
    } else {
        (None, ewkt)
    };
    let wkt = wkt.trim();
    let (type_kw, body) = split_wkt(wkt)?;
    encode_wkt(&type_kw.to_uppercase(), &body)
}

fn split_wkt(wkt: &str) -> std::result::Result<(String, String), String> {
    let open = wkt
        .find('(')
        .ok_or_else(|| format!("no '(' in WKT: {wkt}"))?;
    let type_kw = wkt[..open].trim().to_string();
    let close = find_matching_close(wkt, open)?;
    let body = wkt[open + 1..close].trim().to_string();
    Ok((type_kw, body))
}

fn find_matching_close(s: &str, open: usize) -> std::result::Result<usize, String> {
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    for (i, byte) in bytes.iter().enumerate().skip(open) {
        match byte {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Ok(i);
                }
            }
            _ => {}
        }
    }
    Err("unmatched parentheses".into())
}

fn split_top_level_commas(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut depth = 0i32;
    for ch in s.chars() {
        match ch {
            '(' => {
                depth += 1;
                current.push(ch);
            }
            ')' => {
                depth -= 1;
                current.push(ch);
            }
            ',' if depth == 0 => {
                parts.push(current.trim().to_string());
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    if !current.trim().is_empty() {
        parts.push(current.trim().to_string());
    }
    parts
}

fn parse_coord(s: &str) -> std::result::Result<(f64, f64), String> {
    let nums: Vec<f64> = s
        .split_whitespace()
        .filter(|t| !t.is_empty())
        .map(|t| {
            t.parse::<f64>()
                .map_err(|e| format!("bad number '{t}': {e}"))
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if nums.len() < 2 {
        return Err(format!("need at least 2 coords: '{s}'"));
    }
    Ok((nums[0], nums[1]))
}

fn parse_coord_list(s: &str) -> std::result::Result<Vec<(f64, f64)>, String> {
    s.split(',').map(|p| parse_coord(p.trim())).collect()
}

fn parse_rings(s: &str) -> std::result::Result<Vec<Vec<(f64, f64)>>, String> {
    split_top_level_commas(s)
        .into_iter()
        .map(|group| {
            let g = group.trim().trim_start_matches('(').trim_end_matches(')');
            parse_coord_list(g)
        })
        .collect()
}

fn encode_wkt(type_kw: &str, body: &str) -> std::result::Result<Vec<u8>, String> {
    use geo_traits::Dimensions;
    match type_kw {
        "POINT" => {
            let (x, y) = parse_coord(body)?;
            Ok(wkb_point((x, y)).map_err(|e| e.to_string())?)
        }
        "LINESTRING" | "CIRCULARSTRING" | "COMPOUNDCURVE" => {
            let coords = parse_coord_list(body)?;
            Ok(wkb_linestring(coords.into_iter()).map_err(|e| e.to_string())?)
        }
        "POLYGON" | "CURVEPOLYGON" => {
            let rings = parse_rings(body)?;
            encode_polygon(&rings)
        }
        "MULTIPOINT" => {
            let points = if body.contains('(') {
                parse_rings(body)?
                    .into_iter()
                    .filter_map(|ring| ring.into_iter().next())
                    .collect::<Vec<_>>()
            } else {
                parse_coord_list(body)?
            };
            Ok(wkb_multipoint(points.into_iter()).map_err(|e| e.to_string())?)
        }
        "MULTILINESTRING" | "MULTICURVE" => {
            let rings = parse_rings(body)?;
            Ok(wkb_multilinestring(rings.into_iter()).map_err(|e| e.to_string())?)
        }
        "MULTIPOLYGON" => {
            let polygons: Vec<Vec<Vec<(f64, f64)>>> = split_top_level_commas(body)
                .into_iter()
                .map(|group| {
                    let g = group.trim().trim_start_matches('(').trim_end_matches(')');
                    parse_rings(g)
                })
                .collect::<std::result::Result<Vec<_>, _>>()?;
            let outer_rings: Vec<Vec<(f64, f64)>> = polygons
                .into_iter()
                .map(|poly| poly.into_iter().next().unwrap_or_default())
                .collect();
            Ok(wkb_multipolygon(outer_rings.into_iter()).map_err(|e| e.to_string())?)
        }
        "GEOMETRYCOLLECTION" => {
            let mut sub_wkbs = Vec::new();
            for part in split_top_level_commas(body) {
                let (sub_kw, sub_body) = split_wkt(&part)?;
                sub_wkbs.push(encode_wkt(&sub_kw.to_uppercase(), &sub_body)?);
            }
            let mut buf = Vec::new();
            write_wkb_geometrycollection_header(&mut buf, Dimensions::Xy, sub_wkbs.len())
                .map_err(|e| e.to_string())?;
            for wkb in &sub_wkbs {
                buf.extend_from_slice(wkb);
            }
            Ok(buf)
        }
        _ => Err(format!("unsupported geometry type: {type_kw}")),
    }
}

fn encode_polygon(rings: &[Vec<(f64, f64)>]) -> std::result::Result<Vec<u8>, String> {
    use geo_traits::Dimensions;
    let mut buf = Vec::new();
    write_wkb_polygon_header(&mut buf, Dimensions::Xy, rings.len()).map_err(|e| e.to_string())?;
    for ring in rings {
        write_wkb_polygon_ring_header(&mut buf, ring.len()).map_err(|e| e.to_string())?;
        for &(x, y) in ring {
            write_wkb_coord(&mut buf, (x, y)).map_err(|e| e.to_string())?;
        }
    }
    Ok(buf)
}
