// SPDX-License-Identifier: Apache-2.0
//! QuackGIS spatial UDFs that supplement or override SedonaDB's function
//! catalog with pure-Rust implementations. ST_Transform is a passthrough
//! for now (real CRS transform via proj-wkt comes in Path B sedonadb fork);
//! ST_MakeEnvelope / ST_TileEnvelope / ST_Expand use Sedona's WKB helpers.

use std::{collections::HashMap, sync::Arc};

use datafusion::arrow::array::{
    Array, ArrayRef, BinaryArray, BinaryViewArray, BooleanArray, Float64Array, Int32Array,
    Int64Array, StringArray,
};
use datafusion::arrow::datatypes::DataType;
use datafusion::common::{DataFusionError, Result as DFResult};
use datafusion::logical_expr::{
    AggregateUDF, AggregateUDFImpl, ScalarFunctionArgs, ScalarUDF, ScalarUDFImpl, Signature,
    TypeSignature, Volatility, create_udf,
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
    register_st_point_constructors(ctx)?;
    register_st_makeenvelope(ctx)?;
    register_st_tileenvelope(ctx)?;
    register_st_expand(ctx)?;
    register_st_envelope(ctx)?;
    register_st_curvetoline(ctx)?;
    register_st_force2d(ctx)?;
    register_st_hasarc(ctx)?;
    register_st_simplify(ctx)?;
    register_st_asmvtgeom(ctx)?;
    register_st_asmvt(ctx)?;
    register_st_extent(ctx)?;
    register_st_estimatedextent(ctx)?;
    register_st_astext(ctx)?;
    register_st_asbinary(ctx)?;
    register_st_measure_accessors(ctx)?;
    register_st_point_accessors(ctx)?;
    register_st_bbox_accessors(ctx)?;
    register_st_geometry_accessors(ctx)?;
    register_st_line_accessors(ctx)?;
    register_st_polygon_ring_accessors(ctx)?;
    register_st_editing_affine_helpers(ctx)?;
    register_st_transform_real(ctx)?;
    register_bbox_overlap(ctx)?;
    register_st_geomfromewkt(ctx)?;
    register_qgis_geometry_metadata(ctx)?;
    Ok(())
}

// ─── ST_Point/ST_MakePoint constructors ────────────────────────────────────
fn register_st_point_constructors(ctx: &SessionContext) -> DFResult<()> {
    ctx.register_udf(ScalarUDF::new_from_impl(STPointConstructor::new(
        "st_point", true,
    )));
    ctx.register_udf(ScalarUDF::new_from_impl(STPointConstructor::new(
        "st_makepoint",
        false,
    )));
    Ok(())
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct STPointConstructor {
    name: &'static str,
    allow_srid: bool,
    signature: Signature,
}

impl STPointConstructor {
    fn new(name: &'static str, allow_srid: bool) -> Self {
        let mut signatures = vec![TypeSignature::Exact(vec![
            DataType::Float64,
            DataType::Float64,
        ])];
        if allow_srid {
            signatures.push(TypeSignature::Exact(vec![
                DataType::Float64,
                DataType::Float64,
                DataType::Int64,
            ]));
        }
        Self {
            name,
            allow_srid,
            signature: Signature::one_of(signatures, Volatility::Immutable),
        }
    }
}

impl ScalarUDFImpl for STPointConstructor {
    fn name(&self) -> &str {
        self.name
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> DFResult<DataType> {
        Ok(DataType::Binary)
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> DFResult<ColumnarValue> {
        if args.args.len() != 2 && !(self.allow_srid && args.args.len() == 3) {
            return Err(DataFusionError::Plan(format!(
                "{} expected {} arguments, got {}",
                self.name,
                if self.allow_srid { "2 or 3" } else { "2" },
                args.args.len()
            )));
        }
        let arrays = columnar_values_to_arrays(&args.args)?;
        let x = as_f64_array_ref(&arrays[0])?;
        let y = as_f64_array_ref(&arrays[1])?;
        let srid = arrays.get(2).map(as_i64_array_ref).transpose()?;
        let out = (0..x.len())
            .map(|i| {
                if x.is_null(i) || y.is_null(i) || srid.is_some_and(|values| values.is_null(i)) {
                    return Ok(None);
                }
                let wkb = point_to_wkb((x.value(i), y.value(i)))?;
                match srid {
                    Some(values) => tag_wkb_srid_i64(&wkb, values.value(i)).map(Some),
                    None => Ok(Some(wkb)),
                }
            })
            .collect::<DFResult<Vec<_>>>()?;
        Ok(ColumnarValue::Array(Arc::new(BinaryArray::from_iter(
            out.iter().map(|v| v.as_deref()),
        ))))
    }
}

// ─── ST_Envelope(geom|box2d) — bbox polygon ────────────────────────────────
fn register_st_envelope(ctx: &SessionContext) -> DFResult<()> {
    ctx.register_udf(ScalarUDF::new_from_impl(STEnvelope::new()));
    Ok(())
}

#[derive(Debug, PartialEq, Hash)]
struct STEnvelope {
    signature: Signature,
}

impl Eq for STEnvelope {}

impl STEnvelope {
    fn new() -> Self {
        Self {
            signature: Signature::one_of(
                vec![
                    TypeSignature::Exact(vec![DataType::Binary]),
                    // GeoTools layer bounds uses ST_Envelope(ST_Extent(geom)).
                    // ST_Extent is represented here as PostGIS BOX text.
                    TypeSignature::Exact(vec![DataType::Utf8]),
                ],
                Volatility::Immutable,
            ),
        }
    }
}

impl ScalarUDFImpl for STEnvelope {
    fn name(&self) -> &str {
        "st_envelope"
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> DFResult<DataType> {
        Ok(DataType::Binary)
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> DFResult<ColumnarValue> {
        let arrays = columnar_values_to_arrays(&args.args)?;
        let input = arrays
            .first()
            .ok_or_else(|| DataFusionError::Internal("st_envelope expected one argument".into()))?;
        let out: Vec<Option<Vec<u8>>> = if let Some(geom) =
            input.as_any().downcast_ref::<BinaryArray>()
        {
            (0..geom.len())
                .map(|i| {
                    if geom.is_null(i) {
                        Ok(None)
                    } else {
                        rect_from_wkb(geom.value(i)).and_then(|rect| rect_to_wkb_option(rect, 0))
                    }
                })
                .collect::<DFResult<_>>()?
        } else if let Some(box_text) = input.as_any().downcast_ref::<StringArray>() {
            (0..box_text.len())
                .map(|i| {
                    if box_text.is_null(i) {
                        Ok(None)
                    } else {
                        parse_box2d(box_text.value(i)).and_then(|rect| rect_to_wkb_option(rect, 0))
                    }
                })
                .collect::<DFResult<_>>()?
        } else {
            return Err(DataFusionError::Internal(
                "st_envelope expected Binary geometry or Utf8 BOX text".into(),
            ));
        };

        Ok(ColumnarValue::Array(Arc::new(BinaryArray::from_iter(
            out.iter().map(|v| v.as_deref()),
        ))))
    }
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

// ─── ST_Force2D(geom) — identity for QuackGIS 2D WKB storage ───────────────
fn register_st_force2d(ctx: &SessionContext) -> DFResult<()> {
    ctx.register_udf(create_udf(
        "st_force2d",
        vec![DataType::Binary],
        DataType::Binary,
        Volatility::Immutable,
        Arc::new(|args| Ok(args[0].clone())),
    ));
    Ok(())
}

// ─── ST_HasArc(geom) — no curved geometries in WKB storage ─────────────────
fn register_st_hasarc(ctx: &SessionContext) -> DFResult<()> {
    ctx.register_udf(create_udf(
        "st_hasarc",
        vec![DataType::Binary],
        DataType::Boolean,
        Volatility::Immutable,
        Arc::new(|args| {
            let arrays = columnar_values_to_arrays(args)?;
            let geom = as_binary_array_ref(&arrays[0])?;
            let out = (0..geom.len()).map(|i| (!geom.is_null(i)).then_some(false));
            Ok(ColumnarValue::Array(Arc::new(BooleanArray::from_iter(out))))
        }),
    ));
    Ok(())
}

// ─── ST_Simplify(geom, tolerance) — identity fallback for renderer probes ──
fn register_st_simplify(ctx: &SessionContext) -> DFResult<()> {
    ctx.register_udf(ScalarUDF::new_from_impl(STSimplify::new()));
    Ok(())
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct STSimplify {
    signature: Signature,
}

impl STSimplify {
    fn new() -> Self {
        Self {
            signature: Signature::user_defined(Volatility::Immutable),
        }
    }
}

impl ScalarUDFImpl for STSimplify {
    fn name(&self) -> &str {
        "st_simplify"
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> DFResult<DataType> {
        Ok(DataType::Binary)
    }

    fn coerce_types(&self, arg_types: &[DataType]) -> DFResult<Vec<DataType>> {
        match arg_types.len() {
            2 => Ok(vec![DataType::Binary, DataType::Float64]),
            3 => Ok(vec![DataType::Binary, DataType::Float64, DataType::Boolean]),
            len => Err(DataFusionError::Plan(format!(
                "st_simplify expected 2 or 3 arguments, got {len}"
            ))),
        }
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> DFResult<ColumnarValue> {
        let arrays = columnar_values_to_arrays(&args.args)?;
        Ok(ColumnarValue::Array(arrays[0].clone()))
    }
}

// ─── ST_AsText(geom) — WKB → WKT ───────────────────────────────────────────
fn register_st_astext(ctx: &SessionContext) -> DFResult<()> {
    ctx.register_udf(ScalarUDF::new_from_impl(STAsText::new(
        "st_astext",
        TextOutputKind::Wkt,
    )));
    ctx.register_udf(ScalarUDF::new_from_impl(STAsText::new(
        "st_asewkt",
        TextOutputKind::Ewkt,
    )));
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum TextOutputKind {
    Wkt,
    Ewkt,
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct STAsText {
    name: &'static str,
    kind: TextOutputKind,
    signature: Signature,
}

impl STAsText {
    fn new(name: &'static str, kind: TextOutputKind) -> Self {
        Self {
            name,
            kind,
            signature: Signature::one_of(
                vec![
                    TypeSignature::Exact(vec![DataType::Binary]),
                    TypeSignature::Exact(vec![DataType::BinaryView]),
                ],
                Volatility::Immutable,
            ),
        }
    }
}

impl ScalarUDFImpl for STAsText {
    fn name(&self) -> &str {
        self.name
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> DFResult<DataType> {
        Ok(DataType::Utf8)
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> DFResult<ColumnarValue> {
        let arrays = columnar_values_to_arrays(&args.args)?;
        let geom = arrays.first().ok_or_else(|| {
            DataFusionError::Internal(format!("{} expected one argument", self.name))
        })?;
        let out = (0..geom.len())
            .map(|i| match binary_value_at(geom, i, self.name)? {
                Some(wkb) => wkb_to_text(wkb, self.kind, self.name).map(Some),
                None => Ok(None),
            })
            .collect::<DFResult<Vec<_>>>()?;
        Ok(ColumnarValue::Array(Arc::new(StringArray::from_iter(out))))
    }
}

// ─── ST_X/ST_Y(point) — point coordinate accessors ─────────────────────────
fn register_st_point_accessors(ctx: &SessionContext) -> DFResult<()> {
    ctx.register_udf(ScalarUDF::new_from_impl(STPointCoord::new("st_x", Axis::X)));
    ctx.register_udf(ScalarUDF::new_from_impl(STPointCoord::new("st_y", Axis::Y)));
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Axis {
    X,
    Y,
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct STPointCoord {
    name: &'static str,
    axis: Axis,
    signature: Signature,
}

impl STPointCoord {
    fn new(name: &'static str, axis: Axis) -> Self {
        Self {
            name,
            axis,
            signature: Signature::one_of(
                vec![
                    TypeSignature::Exact(vec![DataType::Binary]),
                    TypeSignature::Exact(vec![DataType::BinaryView]),
                ],
                Volatility::Immutable,
            ),
        }
    }
}

impl ScalarUDFImpl for STPointCoord {
    fn name(&self) -> &str {
        self.name
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> DFResult<DataType> {
        Ok(DataType::Float64)
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> DFResult<ColumnarValue> {
        let arrays = columnar_values_to_arrays(&args.args)?;
        let geom = arrays.first().ok_or_else(|| {
            DataFusionError::Internal(format!("{} expected one argument", self.name))
        })?;
        let out = (0..geom.len())
            .map(|i| match binary_value_at(geom, i, self.name)? {
                Some(wkb) => point_coord(wkb, self.axis, self.name).map(Some),
                None => Ok(None),
            })
            .collect::<DFResult<Vec<_>>>()?;
        Ok(ColumnarValue::Array(Arc::new(Float64Array::from_iter(out))))
    }
}

fn point_coord(wkb: &[u8], axis: Axis, function: &str) -> DFResult<f64> {
    let parsed = crate::mvt::parse_wkb(wkb).ok_or_else(|| {
        DataFusionError::Execution(format!("{function} expected valid 2D WKB/EWKB"))
    })?;
    if parsed.geom_type != crate::mvt::GeomType::Point {
        return Err(DataFusionError::Execution(format!(
            "{function} expected POINT geometry"
        )));
    }
    let (x, y) = parsed
        .rings
        .first()
        .and_then(|ring| ring.first())
        .copied()
        .ok_or_else(|| {
            DataFusionError::Execution(format!("{function} expected non-empty POINT"))
        })?;
    Ok(match axis {
        Axis::X => x,
        Axis::Y => y,
    })
}

// ─── ST_XMin/ST_YMin/ST_XMax/ST_YMax(geom|box2d) — bbox accessors ──────────
fn register_st_bbox_accessors(ctx: &SessionContext) -> DFResult<()> {
    ctx.register_udf(ScalarUDF::new_from_impl(STBboxCoord::new(
        "st_xmin",
        BboxAxis::XMin,
    )));
    ctx.register_udf(ScalarUDF::new_from_impl(STBboxCoord::new(
        "st_ymin",
        BboxAxis::YMin,
    )));
    ctx.register_udf(ScalarUDF::new_from_impl(STBboxCoord::new(
        "st_xmax",
        BboxAxis::XMax,
    )));
    ctx.register_udf(ScalarUDF::new_from_impl(STBboxCoord::new(
        "st_ymax",
        BboxAxis::YMax,
    )));
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum BboxAxis {
    XMin,
    YMin,
    XMax,
    YMax,
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct STBboxCoord {
    name: &'static str,
    axis: BboxAxis,
    signature: Signature,
}

impl STBboxCoord {
    fn new(name: &'static str, axis: BboxAxis) -> Self {
        Self {
            name,
            axis,
            signature: Signature::one_of(
                vec![
                    TypeSignature::Exact(vec![DataType::Binary]),
                    TypeSignature::Exact(vec![DataType::BinaryView]),
                    TypeSignature::Exact(vec![DataType::Utf8]),
                ],
                Volatility::Immutable,
            ),
        }
    }
}

impl ScalarUDFImpl for STBboxCoord {
    fn name(&self) -> &str {
        self.name
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> DFResult<DataType> {
        Ok(DataType::Float64)
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> DFResult<ColumnarValue> {
        let arrays = columnar_values_to_arrays(&args.args)?;
        let input = arrays.first().ok_or_else(|| {
            DataFusionError::Internal(format!("{} expected one argument", self.name))
        })?;
        let out = (0..input.len())
            .map(|i| bbox_coord_at(input, i, self.axis, self.name))
            .collect::<DFResult<Vec<_>>>()?;
        Ok(ColumnarValue::Array(Arc::new(Float64Array::from_iter(out))))
    }
}

fn bbox_coord_at(
    input: &ArrayRef,
    row: usize,
    axis: BboxAxis,
    function: &str,
) -> DFResult<Option<f64>> {
    let rect = if matches!(input.data_type(), DataType::Utf8) {
        let text = input
            .as_any()
            .downcast_ref::<StringArray>()
            .ok_or_else(|| DataFusionError::Internal(format!("{function} expected Utf8")))?;
        if text.is_null(row) {
            return Ok(None);
        }
        parse_box2d(text.value(row))?
    } else {
        let Some(wkb) = binary_value_at(input, row, function)? else {
            return Ok(None);
        };
        rect_from_wkb(wkb)?
    };
    Ok(rect.map(|rect| match axis {
        BboxAxis::XMin => rect.min_x,
        BboxAxis::YMin => rect.min_y,
        BboxAxis::XMax => rect.max_x,
        BboxAxis::YMax => rect.max_y,
    }))
}

// ─── ST_GeometryType/ST_NPoints/ST_NumGeometries metadata accessors ────────
fn register_st_geometry_accessors(ctx: &SessionContext) -> DFResult<()> {
    ctx.register_udf(ScalarUDF::new_from_impl(STGeometryType::new()));
    ctx.register_udf(ScalarUDF::new_from_impl(STGeometryCount::new(
        "st_npoints",
        GeometryCountKind::Points,
    )));
    ctx.register_udf(ScalarUDF::new_from_impl(STGeometryCount::new(
        "st_numpoints",
        GeometryCountKind::Points,
    )));
    ctx.register_udf(ScalarUDF::new_from_impl(STGeometryCount::new(
        "st_numgeometries",
        GeometryCountKind::Geometries,
    )));
    ctx.register_udf(ScalarUDF::new_from_impl(STGeometryN::new()));
    ctx.register_udf(ScalarUDF::new_from_impl(STGeometryCount::new(
        "st_ndims",
        GeometryCountKind::CoordinateDimensions,
    )));
    ctx.register_udf(ScalarUDF::new_from_impl(STGeometryCount::new(
        "st_coorddim",
        GeometryCountKind::CoordinateDimensions,
    )));
    ctx.register_udf(ScalarUDF::new_from_impl(STGeometryCount::new(
        "st_dimension",
        GeometryCountKind::TopologicalDimension,
    )));
    ctx.register_udf(ScalarUDF::new_from_impl(STGeometryBool::new(
        "st_isempty",
        GeometryBoolKind::Empty,
    )));
    ctx.register_udf(ScalarUDF::new_from_impl(STGeometryBool::new(
        "st_isvalid",
        GeometryBoolKind::Valid,
    )));
    ctx.register_udf(ScalarUDF::new_from_impl(STGeometryBool::new(
        "st_isclosed",
        GeometryBoolKind::Closed,
    )));
    ctx.register_udf(ScalarUDF::new_from_impl(STGeometryBool::new(
        "st_isring",
        GeometryBoolKind::Ring,
    )));
    Ok(())
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct STGeometryType {
    signature: Signature,
}

impl STGeometryType {
    fn new() -> Self {
        Self {
            signature: Signature::one_of(
                vec![
                    TypeSignature::Exact(vec![DataType::Binary]),
                    TypeSignature::Exact(vec![DataType::BinaryView]),
                ],
                Volatility::Immutable,
            ),
        }
    }
}

impl ScalarUDFImpl for STGeometryType {
    fn name(&self) -> &str {
        "st_geometrytype"
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> DFResult<DataType> {
        Ok(DataType::Utf8)
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> DFResult<ColumnarValue> {
        let arrays = columnar_values_to_arrays(&args.args)?;
        let input = arrays.first().ok_or_else(|| {
            DataFusionError::Internal("st_geometrytype expected one argument".into())
        })?;
        let out = (0..input.len())
            .map(|i| match binary_value_at(input, i, "st_geometrytype")? {
                Some(wkb) => Ok(wkb_geometry_type_name(wkb).map(postgis_geometry_type_name)),
                None => Ok(None),
            })
            .collect::<DFResult<Vec<_>>>()?;
        Ok(ColumnarValue::Array(Arc::new(StringArray::from_iter(out))))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum GeometryCountKind {
    Points,
    Geometries,
    CoordinateDimensions,
    TopologicalDimension,
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct STGeometryCount {
    name: &'static str,
    kind: GeometryCountKind,
    signature: Signature,
}

impl STGeometryCount {
    fn new(name: &'static str, kind: GeometryCountKind) -> Self {
        Self {
            name,
            kind,
            signature: Signature::one_of(
                vec![
                    TypeSignature::Exact(vec![DataType::Binary]),
                    TypeSignature::Exact(vec![DataType::BinaryView]),
                ],
                Volatility::Immutable,
            ),
        }
    }
}

impl ScalarUDFImpl for STGeometryCount {
    fn name(&self) -> &str {
        self.name
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> DFResult<DataType> {
        Ok(DataType::Int32)
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> DFResult<ColumnarValue> {
        let arrays = columnar_values_to_arrays(&args.args)?;
        let input = arrays.first().ok_or_else(|| {
            DataFusionError::Internal(format!("{} expected one argument", self.name))
        })?;
        let out = (0..input.len())
            .map(|i| match binary_value_at(input, i, self.name)? {
                Some(wkb) => geometry_count(wkb, self.kind, self.name).map(Some),
                None => Ok(None),
            })
            .collect::<DFResult<Vec<_>>>()?;
        Ok(ColumnarValue::Array(Arc::new(Int32Array::from_iter(out))))
    }
}

fn postgis_geometry_type_name(name: &str) -> String {
    match name {
        "POINT" => "ST_Point".to_string(),
        "LINESTRING" => "ST_LineString".to_string(),
        "POLYGON" => "ST_Polygon".to_string(),
        "MULTIPOINT" => "ST_MultiPoint".to_string(),
        "MULTILINESTRING" => "ST_MultiLineString".to_string(),
        "MULTIPOLYGON" => "ST_MultiPolygon".to_string(),
        "GEOMETRYCOLLECTION" => "ST_GeometryCollection".to_string(),
        _ => "ST_Geometry".to_string(),
    }
}

fn geometry_count(wkb: &[u8], kind: GeometryCountKind, function: &str) -> DFResult<i32> {
    let parsed = crate::mvt::parse_wkb(wkb).ok_or_else(|| {
        DataFusionError::Execution(format!("{function} expected valid 2D WKB/EWKB"))
    })?;
    let count = match kind {
        GeometryCountKind::Points => parsed.all_coords().count(),
        GeometryCountKind::Geometries => match parsed.geom_type {
            crate::mvt::GeomType::MultiPoint => parsed.rings.first().map_or(0, Vec::len),
            crate::mvt::GeomType::MultiLineString => parsed.rings.len(),
            crate::mvt::GeomType::MultiPolygon => {
                parsed.sub_geom_ring_counts.as_ref().map_or(0, Vec::len)
            }
            crate::mvt::GeomType::GeometryCollection => 0,
            crate::mvt::GeomType::Unknown(_) => 0,
            _ => usize::from(parsed.all_coords().next().is_some()),
        },
        GeometryCountKind::CoordinateDimensions => 2,
        GeometryCountKind::TopologicalDimension => match parsed.geom_type {
            crate::mvt::GeomType::Point | crate::mvt::GeomType::MultiPoint => 0,
            crate::mvt::GeomType::LineString | crate::mvt::GeomType::MultiLineString => 1,
            crate::mvt::GeomType::Polygon | crate::mvt::GeomType::MultiPolygon => 2,
            crate::mvt::GeomType::GeometryCollection | crate::mvt::GeomType::Unknown(_) => 0,
        },
    };
    i32::try_from(count).map_err(|_| {
        DataFusionError::Execution(format!("{function} result exceeds INT32 range: {count}"))
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum GeometryBoolKind {
    Empty,
    Valid,
    Closed,
    Ring,
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct STGeometryBool {
    name: &'static str,
    kind: GeometryBoolKind,
    signature: Signature,
}

impl STGeometryBool {
    fn new(name: &'static str, kind: GeometryBoolKind) -> Self {
        Self {
            name,
            kind,
            signature: Signature::one_of(
                vec![
                    TypeSignature::Exact(vec![DataType::Binary]),
                    TypeSignature::Exact(vec![DataType::BinaryView]),
                ],
                Volatility::Immutable,
            ),
        }
    }
}

impl ScalarUDFImpl for STGeometryBool {
    fn name(&self) -> &str {
        self.name
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> DFResult<DataType> {
        Ok(DataType::Boolean)
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> DFResult<ColumnarValue> {
        let arrays = columnar_values_to_arrays(&args.args)?;
        let input = arrays.first().ok_or_else(|| {
            DataFusionError::Internal(format!("{} expected one argument", self.name))
        })?;
        let out = (0..input.len())
            .map(|i| match binary_value_at(input, i, self.name)? {
                Some(wkb) => geometry_bool(wkb, self.kind, self.name),
                None => Ok(None),
            })
            .collect::<DFResult<Vec<_>>>()?;
        Ok(ColumnarValue::Array(Arc::new(BooleanArray::from_iter(out))))
    }
}

fn geometry_bool(wkb: &[u8], kind: GeometryBoolKind, function: &str) -> DFResult<Option<bool>> {
    let parsed = crate::mvt::parse_wkb(wkb);
    match kind {
        GeometryBoolKind::Valid => Ok(Some(parsed.is_some())),
        GeometryBoolKind::Empty => parsed
            .map(|geom| Some(geom.all_coords().next().is_none()))
            .ok_or_else(|| {
                DataFusionError::Execution("st_isempty expected valid 2D WKB/EWKB".into())
            }),
        GeometryBoolKind::Closed => parsed
            .map(|geom| Some(geometry_is_closed(&geom)))
            .ok_or_else(|| {
                DataFusionError::Execution(format!("{function} expected valid 2D WKB/EWKB"))
            }),
        GeometryBoolKind::Ring => {
            parsed
                .map(|geom| Some(geometry_is_ring(&geom)))
                .ok_or_else(|| {
                    DataFusionError::Execution(format!("{function} expected valid 2D WKB/EWKB"))
                })
        }
    }
}

fn geometry_is_closed(geom: &crate::mvt::ParsedGeom) -> bool {
    match geom.geom_type {
        crate::mvt::GeomType::LineString => {
            geom.rings.first().is_some_and(|ring| ring_is_closed(ring))
        }
        crate::mvt::GeomType::MultiLineString => {
            !geom.rings.is_empty() && geom.rings.iter().all(|ring| ring_is_closed(ring))
        }
        _ => false,
    }
}

fn geometry_is_ring(geom: &crate::mvt::ParsedGeom) -> bool {
    match geom.geom_type {
        crate::mvt::GeomType::LineString => geom
            .rings
            .first()
            .is_some_and(|ring| ring_is_closed(ring) && ring_is_simple(ring)),
        _ => false,
    }
}

fn ring_is_closed(ring: &[(f64, f64)]) -> bool {
    ring.first().zip(ring.last()).is_some_and(|(a, b)| a == b)
}

fn ring_is_simple(ring: &[(f64, f64)]) -> bool {
    if ring.len() < 4 {
        return false;
    }
    let segment_count = ring.len() - 1;
    for i in 0..segment_count {
        for j in (i + 1)..segment_count {
            if j == i + 1 || (i == 0 && j + 1 == segment_count) {
                continue;
            }
            if segments_intersect(ring[i], ring[i + 1], ring[j], ring[j + 1]) {
                return false;
            }
        }
    }
    true
}

fn segments_intersect(a: (f64, f64), b: (f64, f64), c: (f64, f64), d: (f64, f64)) -> bool {
    fn orientation(a: (f64, f64), b: (f64, f64), c: (f64, f64)) -> f64 {
        (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)
    }
    fn between(value: f64, start: f64, end: f64) -> bool {
        value >= start.min(end) && value <= start.max(end)
    }
    fn on_segment(a: (f64, f64), b: (f64, f64), p: (f64, f64)) -> bool {
        orientation(a, b, p) == 0.0 && between(p.0, a.0, b.0) && between(p.1, a.1, b.1)
    }

    let o1 = orientation(a, b, c);
    let o2 = orientation(a, b, d);
    let o3 = orientation(c, d, a);
    let o4 = orientation(c, d, b);

    if ((o1 > 0.0 && o2 < 0.0) || (o1 < 0.0 && o2 > 0.0))
        && ((o3 > 0.0 && o4 < 0.0) || (o3 < 0.0 && o4 > 0.0))
    {
        return true;
    }

    on_segment(a, b, c) || on_segment(a, b, d) || on_segment(c, d, a) || on_segment(c, d, b)
}

// ─── ST_Perimeter ──────────────────────────────────────────────────────────
fn register_st_measure_accessors(ctx: &SessionContext) -> DFResult<()> {
    ctx.register_udf(ScalarUDF::new_from_impl(STPerimeter::new()));
    Ok(())
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct STPerimeter {
    signature: Signature,
}

impl STPerimeter {
    fn new() -> Self {
        Self {
            signature: Signature::one_of(
                vec![
                    TypeSignature::Exact(vec![DataType::Binary]),
                    TypeSignature::Exact(vec![DataType::BinaryView]),
                ],
                Volatility::Immutable,
            ),
        }
    }
}

impl ScalarUDFImpl for STPerimeter {
    fn name(&self) -> &str {
        "st_perimeter"
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> DFResult<DataType> {
        Ok(DataType::Float64)
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> DFResult<ColumnarValue> {
        let arrays = columnar_values_to_arrays(&args.args)?;
        let input = arrays.first().ok_or_else(|| {
            DataFusionError::Internal("st_perimeter expected one argument".into())
        })?;
        let out = (0..input.len())
            .map(|i| match binary_value_at(input, i, "st_perimeter")? {
                Some(wkb) => geometry_perimeter(wkb).map(Some),
                None => Ok(None),
            })
            .collect::<DFResult<Vec<_>>>()?;
        Ok(ColumnarValue::Array(Arc::new(Float64Array::from_iter(out))))
    }
}

fn geometry_perimeter(wkb: &[u8]) -> DFResult<f64> {
    let parsed = crate::mvt::parse_wkb(wkb).ok_or_else(|| {
        DataFusionError::Execution("st_perimeter expected valid 2D WKB/EWKB".into())
    })?;
    Ok(match parsed.geom_type {
        crate::mvt::GeomType::Polygon | crate::mvt::GeomType::MultiPolygon => {
            parsed.rings.iter().map(|ring| line_length(ring)).sum()
        }
        _ => 0.0,
    })
}

fn line_length(points: &[(f64, f64)]) -> f64 {
    points
        .windows(2)
        .map(|window| (window[1].0 - window[0].0).hypot(window[1].1 - window[0].1))
        .sum()
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct STGeometryN {
    signature: Signature,
}

impl STGeometryN {
    fn new() -> Self {
        Self {
            signature: Signature::one_of(
                vec![
                    TypeSignature::Exact(vec![DataType::Binary, DataType::Int64]),
                    TypeSignature::Exact(vec![DataType::BinaryView, DataType::Int64]),
                ],
                Volatility::Immutable,
            ),
        }
    }
}

impl ScalarUDFImpl for STGeometryN {
    fn name(&self) -> &str {
        "st_geometryn"
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> DFResult<DataType> {
        Ok(DataType::Binary)
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> DFResult<ColumnarValue> {
        let arrays = columnar_values_to_arrays(&args.args)?;
        let geom = arrays
            .first()
            .ok_or_else(|| DataFusionError::Internal("st_geometryn expected geometry".into()))?;
        let n = arrays.get(1).ok_or_else(|| {
            DataFusionError::Internal("st_geometryn expected geometry index".into())
        })?;
        let n = as_i64_array_ref(n)?;
        let out = (0..geom.len())
            .map(|i| {
                if n.is_null(i) {
                    return Ok(None);
                }
                match binary_value_at(geom, i, "st_geometryn")? {
                    Some(wkb) => geometry_n_wkb(wkb, n.value(i)),
                    None => Ok(None),
                }
            })
            .collect::<DFResult<Vec<_>>>()?;
        Ok(ColumnarValue::Array(Arc::new(BinaryArray::from_iter(
            out.iter().map(|v| v.as_deref()),
        ))))
    }
}

fn geometry_n_wkb(wkb: &[u8], n: i64) -> DFResult<Option<Vec<u8>>> {
    if n <= 0 {
        return Ok(None);
    }
    let parsed = crate::mvt::parse_wkb(wkb).ok_or_else(|| {
        DataFusionError::Execution("st_geometryn expected valid 2D WKB/EWKB".into())
    })?;
    let Some(index) = usize::try_from(n - 1).ok() else {
        return Ok(None);
    };
    match parsed.geom_type {
        crate::mvt::GeomType::Point
        | crate::mvt::GeomType::LineString
        | crate::mvt::GeomType::Polygon => Ok((index == 0).then(|| parsed.to_wkb())),
        crate::mvt::GeomType::MultiPoint => Ok(parsed
            .rings
            .first()
            .and_then(|points| points.get(index).copied())
            .map(point_to_wkb)
            .transpose()?),
        crate::mvt::GeomType::MultiLineString => parsed
            .rings
            .get(index)
            .map(|ring| ring_to_linestring_wkb(ring))
            .transpose(),
        crate::mvt::GeomType::MultiPolygon => multipolygon_geometry_n_wkb(&parsed, index),
        crate::mvt::GeomType::GeometryCollection | crate::mvt::GeomType::Unknown(_) => Ok(None),
    }
}

fn multipolygon_geometry_n_wkb(
    parsed: &crate::mvt::ParsedGeom,
    index: usize,
) -> DFResult<Option<Vec<u8>>> {
    let Some(counts) = parsed.sub_geom_ring_counts.as_ref() else {
        return Ok(None);
    };
    let Some(ring_count) = counts.get(index).copied() else {
        return Ok(None);
    };
    let start = counts.iter().take(index).sum::<usize>();
    let end = start + ring_count;
    if end > parsed.rings.len() {
        return Ok(None);
    }
    encode_polygon(&parsed.rings[start..end])
        .map(Some)
        .map_err(DataFusionError::Execution)
}

// ─── ST_StartPoint/ST_EndPoint/ST_PointN line accessors ────────────────────
fn register_st_line_accessors(ctx: &SessionContext) -> DFResult<()> {
    ctx.register_udf(ScalarUDF::new_from_impl(STLineEndpoint::new(
        "st_startpoint",
        LineEndpoint::Start,
    )));
    ctx.register_udf(ScalarUDF::new_from_impl(STLineEndpoint::new(
        "st_endpoint",
        LineEndpoint::End,
    )));
    ctx.register_udf(ScalarUDF::new_from_impl(STPointN::new()));
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum LineEndpoint {
    Start,
    End,
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct STLineEndpoint {
    name: &'static str,
    endpoint: LineEndpoint,
    signature: Signature,
}

impl STLineEndpoint {
    fn new(name: &'static str, endpoint: LineEndpoint) -> Self {
        Self {
            name,
            endpoint,
            signature: Signature::one_of(
                vec![
                    TypeSignature::Exact(vec![DataType::Binary]),
                    TypeSignature::Exact(vec![DataType::BinaryView]),
                ],
                Volatility::Immutable,
            ),
        }
    }
}

impl ScalarUDFImpl for STLineEndpoint {
    fn name(&self) -> &str {
        self.name
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> DFResult<DataType> {
        Ok(DataType::Binary)
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> DFResult<ColumnarValue> {
        let arrays = columnar_values_to_arrays(&args.args)?;
        let input = arrays.first().ok_or_else(|| {
            DataFusionError::Internal(format!("{} expected one argument", self.name))
        })?;
        let out = (0..input.len())
            .map(|i| match binary_value_at(input, i, self.name)? {
                Some(wkb) => line_endpoint_wkb(wkb, self.endpoint),
                None => Ok(None),
            })
            .collect::<DFResult<Vec<_>>>()?;
        Ok(ColumnarValue::Array(Arc::new(BinaryArray::from_iter(
            out.iter().map(|v| v.as_deref()),
        ))))
    }
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct STPointN {
    signature: Signature,
}

impl STPointN {
    fn new() -> Self {
        Self {
            signature: Signature::one_of(
                vec![
                    TypeSignature::Exact(vec![DataType::Binary, DataType::Int64]),
                    TypeSignature::Exact(vec![DataType::BinaryView, DataType::Int64]),
                ],
                Volatility::Immutable,
            ),
        }
    }
}

impl ScalarUDFImpl for STPointN {
    fn name(&self) -> &str {
        "st_pointn"
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> DFResult<DataType> {
        Ok(DataType::Binary)
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> DFResult<ColumnarValue> {
        let arrays = columnar_values_to_arrays(&args.args)?;
        let geom = arrays
            .first()
            .ok_or_else(|| DataFusionError::Internal("st_pointn expected geometry".into()))?;
        let n = arrays
            .get(1)
            .ok_or_else(|| DataFusionError::Internal("st_pointn expected point index".into()))?;
        let n = as_i64_array_ref(n)?;
        let out = (0..geom.len())
            .map(|i| {
                if n.is_null(i) {
                    return Ok(None);
                }
                match binary_value_at(geom, i, "st_pointn")? {
                    Some(wkb) => line_point_n_wkb(wkb, n.value(i)),
                    None => Ok(None),
                }
            })
            .collect::<DFResult<Vec<_>>>()?;
        Ok(ColumnarValue::Array(Arc::new(BinaryArray::from_iter(
            out.iter().map(|v| v.as_deref()),
        ))))
    }
}

fn line_endpoint_wkb(wkb: &[u8], endpoint: LineEndpoint) -> DFResult<Option<Vec<u8>>> {
    let parsed = crate::mvt::parse_wkb(wkb).ok_or_else(|| {
        DataFusionError::Execution("line endpoint accessor expected valid 2D WKB/EWKB".into())
    })?;
    if parsed.geom_type != crate::mvt::GeomType::LineString {
        return Ok(None);
    }
    let point = match endpoint {
        LineEndpoint::Start => parsed.rings.first().and_then(|ring| ring.first()).copied(),
        LineEndpoint::End => parsed.rings.first().and_then(|ring| ring.last()).copied(),
    };
    point.map(point_to_wkb).transpose()
}

fn line_point_n_wkb(wkb: &[u8], n: i64) -> DFResult<Option<Vec<u8>>> {
    let parsed = crate::mvt::parse_wkb(wkb)
        .ok_or_else(|| DataFusionError::Execution("st_pointn expected valid 2D WKB/EWKB".into()))?;
    if parsed.geom_type != crate::mvt::GeomType::LineString || n == 0 {
        return Ok(None);
    }
    let Some(ring) = parsed.rings.first() else {
        return Ok(None);
    };
    let index = if n > 0 {
        usize::try_from(n - 1).ok()
    } else {
        let from_end = usize::try_from(-n).ok();
        from_end.and_then(|offset| ring.len().checked_sub(offset))
    };
    index
        .and_then(|idx| ring.get(idx).copied())
        .map(point_to_wkb)
        .transpose()
}

fn point_to_wkb(point: (f64, f64)) -> DFResult<Vec<u8>> {
    wkb_point(point).map_err(|e| DataFusionError::Execution(e.to_string()))
}

// ─── ST_ExteriorRing/ST_InteriorRingN/ST_NumInteriorRings ──────────────────
fn register_st_polygon_ring_accessors(ctx: &SessionContext) -> DFResult<()> {
    ctx.register_udf(ScalarUDF::new_from_impl(STNumInteriorRings::new()));
    ctx.register_udf(ScalarUDF::new_from_impl(STPolygonRing::new(
        "st_exteriorring",
        PolygonRingKind::Exterior,
    )));
    ctx.register_udf(ScalarUDF::new_from_impl(STInteriorRingN::new()));
    Ok(())
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct STNumInteriorRings {
    signature: Signature,
}

impl STNumInteriorRings {
    fn new() -> Self {
        Self {
            signature: Signature::one_of(
                vec![
                    TypeSignature::Exact(vec![DataType::Binary]),
                    TypeSignature::Exact(vec![DataType::BinaryView]),
                ],
                Volatility::Immutable,
            ),
        }
    }
}

impl ScalarUDFImpl for STNumInteriorRings {
    fn name(&self) -> &str {
        "st_numinteriorrings"
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> DFResult<DataType> {
        Ok(DataType::Int32)
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> DFResult<ColumnarValue> {
        let arrays = columnar_values_to_arrays(&args.args)?;
        let input = arrays.first().ok_or_else(|| {
            DataFusionError::Internal("st_numinteriorrings expected one argument".into())
        })?;
        let out = (0..input.len())
            .map(
                |i| match binary_value_at(input, i, "st_numinteriorrings")? {
                    Some(wkb) => polygon_interior_ring_count(wkb).map(Some),
                    None => Ok(None),
                },
            )
            .collect::<DFResult<Vec<_>>>()?;
        Ok(ColumnarValue::Array(Arc::new(Int32Array::from_iter(out))))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum PolygonRingKind {
    Exterior,
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct STPolygonRing {
    name: &'static str,
    kind: PolygonRingKind,
    signature: Signature,
}

impl STPolygonRing {
    fn new(name: &'static str, kind: PolygonRingKind) -> Self {
        Self {
            name,
            kind,
            signature: Signature::one_of(
                vec![
                    TypeSignature::Exact(vec![DataType::Binary]),
                    TypeSignature::Exact(vec![DataType::BinaryView]),
                ],
                Volatility::Immutable,
            ),
        }
    }
}

impl ScalarUDFImpl for STPolygonRing {
    fn name(&self) -> &str {
        self.name
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> DFResult<DataType> {
        Ok(DataType::Binary)
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> DFResult<ColumnarValue> {
        let arrays = columnar_values_to_arrays(&args.args)?;
        let input = arrays.first().ok_or_else(|| {
            DataFusionError::Internal(format!("{} expected one argument", self.name))
        })?;
        let out = (0..input.len())
            .map(|i| match binary_value_at(input, i, self.name)? {
                Some(wkb) => polygon_ring_wkb(wkb, self.kind),
                None => Ok(None),
            })
            .collect::<DFResult<Vec<_>>>()?;
        Ok(ColumnarValue::Array(Arc::new(BinaryArray::from_iter(
            out.iter().map(|v| v.as_deref()),
        ))))
    }
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct STInteriorRingN {
    signature: Signature,
}

impl STInteriorRingN {
    fn new() -> Self {
        Self {
            signature: Signature::one_of(
                vec![
                    TypeSignature::Exact(vec![DataType::Binary, DataType::Int64]),
                    TypeSignature::Exact(vec![DataType::BinaryView, DataType::Int64]),
                ],
                Volatility::Immutable,
            ),
        }
    }
}

impl ScalarUDFImpl for STInteriorRingN {
    fn name(&self) -> &str {
        "st_interiorringn"
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> DFResult<DataType> {
        Ok(DataType::Binary)
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> DFResult<ColumnarValue> {
        let arrays = columnar_values_to_arrays(&args.args)?;
        let geom = arrays.first().ok_or_else(|| {
            DataFusionError::Internal("st_interiorringn expected geometry".into())
        })?;
        let n = arrays.get(1).ok_or_else(|| {
            DataFusionError::Internal("st_interiorringn expected ring index".into())
        })?;
        let n = as_i64_array_ref(n)?;
        let out = (0..geom.len())
            .map(|i| {
                if n.is_null(i) {
                    return Ok(None);
                }
                match binary_value_at(geom, i, "st_interiorringn")? {
                    Some(wkb) => polygon_interior_ring_wkb(wkb, n.value(i)),
                    None => Ok(None),
                }
            })
            .collect::<DFResult<Vec<_>>>()?;
        Ok(ColumnarValue::Array(Arc::new(BinaryArray::from_iter(
            out.iter().map(|v| v.as_deref()),
        ))))
    }
}

fn polygon_interior_ring_count(wkb: &[u8]) -> DFResult<i32> {
    let parsed = parse_polygon_wkb(wkb, "st_numinteriorrings")?;
    i32::try_from(parsed.rings.len().saturating_sub(1)).map_err(|_| {
        DataFusionError::Execution("st_numinteriorrings result exceeds INT32 range".into())
    })
}

fn polygon_ring_wkb(wkb: &[u8], kind: PolygonRingKind) -> DFResult<Option<Vec<u8>>> {
    let parsed = parse_polygon_wkb(wkb, "st_exteriorring")?;
    let ring = match kind {
        PolygonRingKind::Exterior => parsed.rings.first(),
    };
    ring.map(|ring| ring_to_linestring_wkb(ring)).transpose()
}

fn polygon_interior_ring_wkb(wkb: &[u8], n: i64) -> DFResult<Option<Vec<u8>>> {
    if n <= 0 {
        return Ok(None);
    }
    let parsed = parse_polygon_wkb(wkb, "st_interiorringn")?;
    let Some(index) = usize::try_from(n).ok() else {
        return Ok(None);
    };
    parsed
        .rings
        .get(index)
        .map(|ring| ring_to_linestring_wkb(ring))
        .transpose()
}

fn parse_polygon_wkb(wkb: &[u8], function: &str) -> DFResult<crate::mvt::ParsedGeom> {
    let parsed = crate::mvt::parse_wkb(wkb).ok_or_else(|| {
        DataFusionError::Execution(format!("{function} expected valid 2D WKB/EWKB"))
    })?;
    if parsed.geom_type != crate::mvt::GeomType::Polygon {
        return Err(DataFusionError::Execution(format!(
            "{function} expected POLYGON geometry"
        )));
    }
    Ok(parsed)
}

fn ring_to_linestring_wkb(ring: &[(f64, f64)]) -> DFResult<Vec<u8>> {
    wkb_linestring(ring.iter().copied()).map_err(|e| DataFusionError::Execution(e.to_string()))
}

// ─── ST_Reverse/ST_FlipCoordinates/ST_Translate/ST_Scale ───────────────────
fn register_st_editing_affine_helpers(ctx: &SessionContext) -> DFResult<()> {
    ctx.register_udf(ScalarUDF::new_from_impl(STUnaryGeomEdit::new(
        "st_reverse",
        UnaryGeomEditKind::Reverse,
    )));
    ctx.register_udf(ScalarUDF::new_from_impl(STUnaryGeomEdit::new(
        "st_flipcoordinates",
        UnaryGeomEditKind::FlipCoordinates,
    )));
    ctx.register_udf(ScalarUDF::new_from_impl(STAffine2D::new(
        "st_translate",
        Affine2DKind::Translate,
    )));
    ctx.register_udf(ScalarUDF::new_from_impl(STAffine2D::new(
        "st_scale",
        Affine2DKind::Scale,
    )));
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum UnaryGeomEditKind {
    Reverse,
    FlipCoordinates,
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct STUnaryGeomEdit {
    name: &'static str,
    kind: UnaryGeomEditKind,
    signature: Signature,
}

impl STUnaryGeomEdit {
    fn new(name: &'static str, kind: UnaryGeomEditKind) -> Self {
        Self {
            name,
            kind,
            signature: Signature::one_of(
                vec![
                    TypeSignature::Exact(vec![DataType::Binary]),
                    TypeSignature::Exact(vec![DataType::BinaryView]),
                ],
                Volatility::Immutable,
            ),
        }
    }
}

impl ScalarUDFImpl for STUnaryGeomEdit {
    fn name(&self) -> &str {
        self.name
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> DFResult<DataType> {
        Ok(DataType::Binary)
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> DFResult<ColumnarValue> {
        let arrays = columnar_values_to_arrays(&args.args)?;
        let input = arrays.first().ok_or_else(|| {
            DataFusionError::Internal(format!("{} expected one argument", self.name))
        })?;
        let out = (0..input.len())
            .map(|i| match binary_value_at(input, i, self.name)? {
                Some(wkb) => unary_geom_edit_wkb(wkb, self.kind, self.name),
                None => Ok(None),
            })
            .collect::<DFResult<Vec<_>>>()?;
        Ok(ColumnarValue::Array(Arc::new(BinaryArray::from_iter(
            out.iter().map(|v| v.as_deref()),
        ))))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Affine2DKind {
    Translate,
    Scale,
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct STAffine2D {
    name: &'static str,
    kind: Affine2DKind,
    signature: Signature,
}

impl STAffine2D {
    fn new(name: &'static str, kind: Affine2DKind) -> Self {
        Self {
            name,
            kind,
            signature: Signature::one_of(
                vec![
                    TypeSignature::Exact(vec![
                        DataType::Binary,
                        DataType::Float64,
                        DataType::Float64,
                    ]),
                    TypeSignature::Exact(vec![
                        DataType::BinaryView,
                        DataType::Float64,
                        DataType::Float64,
                    ]),
                ],
                Volatility::Immutable,
            ),
        }
    }
}

impl ScalarUDFImpl for STAffine2D {
    fn name(&self) -> &str {
        self.name
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> DFResult<DataType> {
        Ok(DataType::Binary)
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> DFResult<ColumnarValue> {
        let arrays = columnar_values_to_arrays(&args.args)?;
        let geom = arrays
            .first()
            .ok_or_else(|| DataFusionError::Internal(format!("{} expected geometry", self.name)))?;
        let x = arrays.get(1).ok_or_else(|| {
            DataFusionError::Internal(format!("{} expected x argument", self.name))
        })?;
        let y = arrays.get(2).ok_or_else(|| {
            DataFusionError::Internal(format!("{} expected y argument", self.name))
        })?;
        let x = as_f64_array_ref(x)?;
        let y = as_f64_array_ref(y)?;
        let out = (0..geom.len())
            .map(|i| {
                if x.is_null(i) || y.is_null(i) {
                    return Ok(None);
                }
                match binary_value_at(geom, i, self.name)? {
                    Some(wkb) => affine_2d_wkb(wkb, self.kind, x.value(i), y.value(i), self.name),
                    None => Ok(None),
                }
            })
            .collect::<DFResult<Vec<_>>>()?;
        Ok(ColumnarValue::Array(Arc::new(BinaryArray::from_iter(
            out.iter().map(|v| v.as_deref()),
        ))))
    }
}

fn unary_geom_edit_wkb(
    wkb: &[u8],
    kind: UnaryGeomEditKind,
    function: &str,
) -> DFResult<Option<Vec<u8>>> {
    let parsed = crate::mvt::parse_wkb(wkb).ok_or_else(|| {
        DataFusionError::Execution(format!("{function} expected valid 2D WKB/EWKB"))
    })?;
    let edited = match kind {
        UnaryGeomEditKind::Reverse => crate::mvt::ParsedGeom {
            geom_type: parsed.geom_type.clone(),
            rings: parsed
                .rings
                .iter()
                .map(|ring| ring.iter().rev().copied().collect())
                .collect(),
            sub_geom_ring_counts: parsed.sub_geom_ring_counts.clone(),
            srid: parsed.srid,
        },
        UnaryGeomEditKind::FlipCoordinates => parsed.map_coords(|x, y| (y, x)),
    };
    parsed_geom_to_wkb(&edited).map(Some)
}

fn affine_2d_wkb(
    wkb: &[u8],
    kind: Affine2DKind,
    x_arg: f64,
    y_arg: f64,
    function: &str,
) -> DFResult<Option<Vec<u8>>> {
    let parsed = crate::mvt::parse_wkb(wkb).ok_or_else(|| {
        DataFusionError::Execution(format!("{function} expected valid 2D WKB/EWKB"))
    })?;
    let edited = match kind {
        Affine2DKind::Translate => parsed.map_coords(|x, y| (x + x_arg, y + y_arg)),
        Affine2DKind::Scale => parsed.map_coords(|x, y| (x * x_arg, y * y_arg)),
    };
    parsed_geom_to_wkb(&edited).map(Some)
}

fn parsed_geom_to_wkb(parsed: &crate::mvt::ParsedGeom) -> DFResult<Vec<u8>> {
    let wkb = parsed.to_wkb();
    match parsed.srid {
        Some(srid) => tag_wkb_srid(&wkb, srid)
            .ok_or_else(|| DataFusionError::Execution("failed to preserve EWKB SRID".into())),
        None => Ok(wkb),
    }
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

fn rect_wkb(min_x: f64, min_y: f64, max_x: f64, max_y: f64, srid: i64) -> DFResult<Vec<u8>> {
    let wkb = wkb_rect(min_x, min_y, max_x, max_y).map_err(|e| {
        DataFusionError::Execution(format!("failed to write WKB rectangle with Sedona: {e}"))
    })?;
    tag_wkb_srid_i64(&wkb, srid)
}

fn rect_to_wkb_option(rect: Option<Rect>, srid: i64) -> DFResult<Option<Vec<u8>>> {
    rect.map(|rect| rect_wkb(rect.min_x, rect.min_y, rect.max_x, rect.max_y, srid))
        .transpose()
}

fn parse_box2d(value: &str) -> DFResult<Option<Rect>> {
    let trimmed = value.trim();
    let Some(rest) = trimmed
        .strip_prefix("BOX(")
        .or_else(|| trimmed.strip_prefix("box("))
    else {
        return Err(DataFusionError::Execution(format!(
            "st_envelope expected BOX text, got {trimmed:?}"
        )));
    };
    let Some(inner) = rest.strip_suffix(')') else {
        return Err(DataFusionError::Execution(format!(
            "st_envelope expected BOX text, got {trimmed:?}"
        )));
    };
    let (min, max) = inner.split_once(',').ok_or_else(|| {
        DataFusionError::Execution(format!("st_envelope expected BOX text, got {trimmed:?}"))
    })?;
    let mut min_parts = min.split_whitespace();
    let min_x = parse_box_coord(min_parts.next(), trimmed)?;
    let min_y = parse_box_coord(min_parts.next(), trimmed)?;
    if min_parts.next().is_some() {
        return Err(DataFusionError::Execution(format!(
            "st_envelope expected BOX text, got {trimmed:?}"
        )));
    }
    let mut max_parts = max.split_whitespace();
    let max_x = parse_box_coord(max_parts.next(), trimmed)?;
    let max_y = parse_box_coord(max_parts.next(), trimmed)?;
    if max_parts.next().is_some() {
        return Err(DataFusionError::Execution(format!(
            "st_envelope expected BOX text, got {trimmed:?}"
        )));
    }
    Ok(Some(Rect {
        min_x,
        min_y,
        max_x,
        max_y,
    }))
}

fn parse_box_coord(value: Option<&str>, original: &str) -> DFResult<f64> {
    let value = value.ok_or_else(|| {
        DataFusionError::Execution(format!("st_envelope expected BOX text, got {original:?}"))
    })?;
    value.parse::<f64>().map_err(|_| {
        DataFusionError::Execution(format!("st_envelope expected BOX text, got {original:?}"))
    })
}

fn wkb_to_text(wkb: &[u8], kind: TextOutputKind, function: &str) -> DFResult<String> {
    let parsed = crate::mvt::parse_wkb(wkb).ok_or_else(|| {
        DataFusionError::Execution(format!("{function} expected valid 2D WKB/EWKB"))
    })?;
    let wkt = format_wkt(&parsed);
    match kind {
        TextOutputKind::Wkt => Ok(wkt),
        TextOutputKind::Ewkt => match parsed.srid {
            Some(srid) if srid != 0 => Ok(format!("SRID={srid};{wkt}")),
            _ => Ok(wkt),
        },
    }
}

fn format_wkt(geom: &crate::mvt::ParsedGeom) -> String {
    match geom.geom_type {
        crate::mvt::GeomType::Point => geom
            .rings
            .first()
            .and_then(|ring| ring.first())
            .map(|point| format!("POINT({})", format_wkt_coord(*point)))
            .unwrap_or_else(|| "POINT EMPTY".to_string()),
        crate::mvt::GeomType::LineString => geom
            .rings
            .first()
            .map(|ring| format!("LINESTRING({})", format_wkt_coords(ring)))
            .unwrap_or_else(|| "LINESTRING EMPTY".to_string()),
        crate::mvt::GeomType::Polygon => format_wkt_polygon(&geom.rings),
        crate::mvt::GeomType::MultiPoint => geom
            .rings
            .first()
            .map(|points| {
                let parts: Vec<String> = points
                    .iter()
                    .map(|point| format!("({})", format_wkt_coord(*point)))
                    .collect();
                format!("MULTIPOINT({})", parts.join(","))
            })
            .unwrap_or_else(|| "MULTIPOINT EMPTY".to_string()),
        crate::mvt::GeomType::MultiLineString => {
            if geom.rings.is_empty() {
                "MULTILINESTRING EMPTY".to_string()
            } else {
                let parts: Vec<String> = geom
                    .rings
                    .iter()
                    .map(|ring| format!("({})", format_wkt_coords(ring)))
                    .collect();
                format!("MULTILINESTRING({})", parts.join(","))
            }
        }
        crate::mvt::GeomType::MultiPolygon => format_wkt_multipolygon(geom),
        crate::mvt::GeomType::GeometryCollection | crate::mvt::GeomType::Unknown(_) => {
            "GEOMETRYCOLLECTION EMPTY".to_string()
        }
    }
}

fn format_wkt_polygon(rings: &[Vec<(f64, f64)>]) -> String {
    if rings.is_empty() {
        return "POLYGON EMPTY".to_string();
    }
    format!("POLYGON{}", format_wkt_polygon_body(rings))
}

fn format_wkt_multipolygon(geom: &crate::mvt::ParsedGeom) -> String {
    if geom.rings.is_empty() {
        return "MULTIPOLYGON EMPTY".to_string();
    }
    let counts = geom
        .sub_geom_ring_counts
        .clone()
        .unwrap_or_else(|| vec![geom.rings.len()]);
    let mut ring_idx = 0usize;
    let mut polygons = Vec::with_capacity(counts.len());
    for count in counts {
        let end = (ring_idx + count).min(geom.rings.len());
        polygons.push(format_wkt_polygon_body(&geom.rings[ring_idx..end]));
        ring_idx = end;
    }
    format!("MULTIPOLYGON({})", polygons.join(","))
}

fn format_wkt_polygon_body(rings: &[Vec<(f64, f64)>]) -> String {
    let parts: Vec<String> = rings
        .iter()
        .map(|ring| format!("({})", format_wkt_coords(ring)))
        .collect();
    format!("({})", parts.join(","))
}

fn format_wkt_coords(coords: &[(f64, f64)]) -> String {
    coords
        .iter()
        .map(|coord| format_wkt_coord(*coord))
        .collect::<Vec<_>>()
        .join(",")
}

fn format_wkt_coord((x, y): (f64, f64)) -> String {
    format!("{} {}", format_wkt_number(x), format_wkt_number(y))
}

fn format_wkt_number(value: f64) -> String {
    if value == 0.0 {
        "0".to_string()
    } else {
        value.to_string()
    }
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

fn binary_value_at<'a>(
    arr: &'a ArrayRef,
    row: usize,
    function: &str,
) -> DFResult<Option<&'a [u8]>> {
    if let Some(binary) = arr.as_any().downcast_ref::<BinaryArray>() {
        return Ok((!binary.is_null(row)).then(|| binary.value(row)));
    }
    if let Some(binary_view) = arr.as_any().downcast_ref::<BinaryViewArray>() {
        return Ok((!binary_view.is_null(row)).then(|| binary_view.value(row)));
    }
    Err(datafusion::common::DataFusionError::Internal(format!(
        "{function} expected Binary or BinaryView"
    )))
}

fn as_i64_array_ref(arr: &ArrayRef) -> DFResult<&Int64Array> {
    arr.as_any()
        .downcast_ref::<Int64Array>()
        .ok_or_else(|| datafusion::common::DataFusionError::Internal("expected Int64".into()))
}

fn as_string_array_ref(arr: &ArrayRef) -> DFResult<&StringArray> {
    arr.as_any()
        .downcast_ref::<StringArray>()
        .ok_or_else(|| datafusion::common::DataFusionError::Internal("expected Utf8".into()))
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
        "st_setsrid",
        vec![DataType::Binary, DataType::Int64],
        DataType::Binary,
        Volatility::Immutable,
        Arc::new(|args| {
            let arrays = columnar_values_to_arrays(args)?;
            let geom = as_binary_array_ref(&arrays[0])?;
            let srid = as_i64_array_ref(&arrays[1])?;
            let out: Vec<Option<Vec<u8>>> = (0..geom.len())
                .map(|i| {
                    if geom.is_null(i) || srid.is_null(i) {
                        None
                    } else {
                        tag_wkb_srid_i64(geom.value(i), srid.value(i)).ok()
                    }
                })
                .collect();
            Ok(ColumnarValue::Array(Arc::new(BinaryArray::from_iter(
                out.iter().map(|v| v.as_deref()),
            ))))
        }),
    ));

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

fn tag_wkb_srid_i64(wkb: &[u8], srid: i64) -> DFResult<Vec<u8>> {
    let srid = i32::try_from(srid).map_err(|_| {
        DataFusionError::Execution(format!("SRID {srid} is outside the supported i32 range"))
    })?;
    tag_wkb_srid(wkb, srid).ok_or_else(|| {
        DataFusionError::Execution("failed to write EWKB SRID tag on invalid WKB".into())
    })
}

fn tag_wkb_srid(wkb: &[u8], srid: i32) -> Option<Vec<u8>> {
    if wkb.len() < 5 {
        return None;
    }
    let raw = wkb_type_id(wkb)?;
    let has_srid = (raw & 0x2000_0000) != 0;
    let body_offset = if has_srid { 9 } else { 5 };
    if wkb.len() < body_offset {
        return None;
    }

    let tagged_type = if srid == 0 {
        raw & !0x2000_0000
    } else {
        raw | 0x2000_0000
    };
    let mut out = Vec::with_capacity(wkb.len() + if srid == 0 || has_srid { 0 } else { 4 });
    out.push(wkb[0]);
    write_u32_endian(&mut out, tagged_type, wkb[0])?;
    if srid != 0 {
        write_u32_endian(&mut out, srid as u32, wkb[0])?;
    }
    out.extend_from_slice(&wkb[body_offset..]);
    Some(out)
}

fn write_u32_endian(out: &mut Vec<u8>, value: u32, byte_order: u8) -> Option<()> {
    match byte_order {
        0 => out.extend_from_slice(&value.to_be_bytes()),
        1 => out.extend_from_slice(&value.to_le_bytes()),
        _ => return None,
    }
    Some(())
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
    ctx.register_udaf(AggregateUDF::from(STAsMVT::new()));
    Ok(())
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct STAsMVT {
    signature: Signature,
}

impl STAsMVT {
    fn new() -> Self {
        Self {
            signature: Signature::user_defined(Volatility::Immutable),
        }
    }
}

impl AggregateUDFImpl for STAsMVT {
    fn name(&self) -> &str {
        "st_asmvt"
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> DFResult<DataType> {
        Ok(DataType::Binary)
    }

    fn coerce_types(&self, arg_types: &[DataType]) -> DFResult<Vec<DataType>> {
        if arg_types.is_empty() {
            return Err(DataFusionError::Plan(
                "st_asmvt expected at least one geometry argument".into(),
            ));
        }
        if !matches!(
            arg_types[0],
            DataType::Binary | DataType::BinaryView | DataType::LargeBinary
        ) {
            return Err(DataFusionError::Plan(format!(
                "st_asmvt first argument must be Binary geometry, got {}",
                arg_types[0]
            )));
        }

        match arg_types.len() {
            // ST_AsMVT(geom)
            1 => Ok(vec![DataType::Binary]),
            // ST_AsMVT(geom, layer_name)
            2 => Ok(vec![DataType::Binary, DataType::Utf8]),
            // ST_AsMVT(geom, layer_name, extent, attr1, attr2, ...)
            n => {
                let mut coerced = Vec::with_capacity(n);
                coerced.push(DataType::Binary);
                coerced.push(DataType::Utf8);
                coerced.push(DataType::Int64);
                coerced.extend(std::iter::repeat_n(DataType::Utf8, n - 3));
                Ok(coerced)
            }
        }
    }

    fn accumulator(
        &self,
        acc_args: datafusion_functions_aggregate_common::accumulator::AccumulatorArgs,
    ) -> DFResult<Box<dyn datafusion::logical_expr::Accumulator>> {
        let attr_names = acc_args
            .expr_fields
            .iter()
            .enumerate()
            .skip(3)
            .map(|(idx, field)| mvt_attr_key_name(field.name(), idx - 3))
            .collect();
        Ok(Box::new(MvtAccumulator::new(attr_names)))
    }
}

fn mvt_attr_key_name(raw: &str, attr_index: usize) -> String {
    let name = raw
        .rsplit('.')
        .next()
        .unwrap_or(raw)
        .trim()
        .trim_matches('"')
        .to_string();
    if name.is_empty() {
        format!("attr{}", attr_index + 1)
    } else {
        name
    }
}

// ─── ST_Extent — aggregate bbox as PostGIS BOX text ────────────────────────

fn register_st_extent(ctx: &SessionContext) -> DFResult<()> {
    use datafusion_functions_aggregate_common::accumulator::AccumulatorArgs;
    use datafusion_functions_aggregate_common::accumulator::AccumulatorFactoryFunction;

    let accumulator: AccumulatorFactoryFunction = Arc::new(|_args: AccumulatorArgs| {
        Ok(Box::<ExtentAccumulator>::default()
            as Box<
                dyn datafusion_expr_common::accumulator::Accumulator,
            >)
    });

    ctx.register_udaf(datafusion::logical_expr::create_udaf(
        "st_extent",
        vec![DataType::Binary],
        Arc::new(DataType::Utf8),
        Volatility::Immutable,
        accumulator,
        Arc::new(vec![
            DataType::Float64,
            DataType::Float64,
            DataType::Float64,
            DataType::Float64,
        ]),
    ));
    Ok(())
}

// ─── ST_EstimatedExtent — no statistics yet; NULL triggers exact fallback ──

fn register_st_estimatedextent(ctx: &SessionContext) -> DFResult<()> {
    ctx.register_udf(ScalarUDF::new_from_impl(STEstimatedExtent::new()));
    Ok(())
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct STEstimatedExtent {
    signature: Signature,
}

impl STEstimatedExtent {
    fn new() -> Self {
        Self {
            signature: Signature::one_of(
                vec![
                    TypeSignature::Exact(vec![DataType::Utf8, DataType::Utf8]),
                    TypeSignature::Exact(vec![DataType::Utf8, DataType::Utf8, DataType::Utf8]),
                    TypeSignature::Exact(vec![
                        DataType::Utf8,
                        DataType::Utf8,
                        DataType::Utf8,
                        DataType::Boolean,
                    ]),
                ],
                Volatility::Immutable,
            ),
        }
    }
}

impl ScalarUDFImpl for STEstimatedExtent {
    fn name(&self) -> &str {
        "st_estimatedextent"
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> DFResult<DataType> {
        Ok(DataType::Utf8)
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> DFResult<ColumnarValue> {
        let arrays = columnar_values_to_arrays(&args.args)?;
        let len = arrays.first().map_or(1, |array| array.len());
        Ok(ColumnarValue::Array(Arc::new(StringArray::from_iter(
            std::iter::repeat_n(None::<&str>, len),
        ))))
    }
}

// ─── ST_AsBinary(geom [, endian]) — WKB passthrough ───────────────────────

fn register_st_asbinary(ctx: &SessionContext) -> DFResult<()> {
    ctx.register_udf(ScalarUDF::new_from_impl(STAsBinary::new("st_asbinary")));
    ctx.register_udf(ScalarUDF::new_from_impl(STAsBinary::new("st_asewkb")));
    ctx.register_udf(ScalarUDF::new_from_impl(STAsHexEwkb::new()));
    Ok(())
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct STAsBinary {
    name: &'static str,
    signature: Signature,
}

impl STAsBinary {
    fn new(name: &'static str) -> Self {
        Self {
            name,
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
        self.name
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
        // ST_AsBinary(geom, 'NDR') and GeoTools asks for ST_AsEWKB(geom), so
        // this is a byte-preserving pgwire compatibility shim until full endian
        // conversion is needed.
        Ok(args.args[0].clone())
    }
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct STAsHexEwkb {
    signature: Signature,
}

impl STAsHexEwkb {
    fn new() -> Self {
        Self {
            signature: Signature::one_of(
                vec![
                    TypeSignature::Exact(vec![DataType::Binary]),
                    TypeSignature::Exact(vec![DataType::BinaryView]),
                ],
                Volatility::Immutable,
            ),
        }
    }
}

impl ScalarUDFImpl for STAsHexEwkb {
    fn name(&self) -> &str {
        "st_ashexewkb"
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> DFResult<DataType> {
        Ok(DataType::Utf8)
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> DFResult<ColumnarValue> {
        let arrays = columnar_values_to_arrays(&args.args)?;
        let geom = arrays.first().ok_or_else(|| {
            DataFusionError::Internal("st_ashexewkb expected one argument".into())
        })?;
        let out = (0..geom.len())
            .map(|i| match binary_value_at(geom, i, "st_ashexewkb")? {
                Some(wkb) => Ok(Some(hex_upper(wkb))),
                None => Ok(None),
            })
            .collect::<DFResult<Vec<_>>>()?;
        Ok(ColumnarValue::Array(Arc::new(StringArray::from_iter(out))))
    }
}

fn hex_upper(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

#[derive(Debug)]
struct MvtAccumulator {
    layer_name: String,
    extent: u32,
    keys: Vec<String>,
    key_index: HashMap<String, u32>,
    values: Vec<String>,
    value_index: HashMap<String, u32>,
    features: Vec<crate::mvt::MvtFeature>,
}

impl MvtAccumulator {
    fn new(keys: Vec<String>) -> Self {
        let key_index = keys
            .iter()
            .enumerate()
            .map(|(idx, key)| (key.clone(), idx as u32))
            .collect();
        Self {
            layer_name: "layer".to_string(),
            extent: 4096,
            keys,
            key_index,
            values: Vec::new(),
            value_index: HashMap::new(),
            features: Vec::new(),
        }
    }

    fn ensure_key(&mut self, key: &str) -> u32 {
        if let Some(index) = self.key_index.get(key) {
            return *index;
        }
        let index = self.keys.len() as u32;
        self.keys.push(key.to_string());
        self.key_index.insert(key.to_string(), index);
        index
    }

    fn ensure_value(&mut self, value: &str) -> u32 {
        if let Some(index) = self.value_index.get(value) {
            return *index;
        }
        let index = self.values.len() as u32;
        self.values.push(value.to_string());
        self.value_index.insert(value.to_string(), index);
        index
    }

    fn merge_decoded_state(&mut self, state: MvtAccumulatorState) -> DFResult<()> {
        if self.features.is_empty() {
            self.layer_name = state.layer_name;
            self.extent = state.extent;
        }

        for feature in state.features {
            let mut tags = Vec::with_capacity(feature.tags.len());
            for pair in feature.tags.chunks(2) {
                if pair.len() != 2 {
                    return Err(DataFusionError::Internal(
                        "st_asmvt state contained an odd tag dictionary index count".into(),
                    ));
                }
                let key = state.keys.get(pair[0] as usize).ok_or_else(|| {
                    DataFusionError::Internal("st_asmvt state key index out of bounds".into())
                })?;
                let value = state.values.get(pair[1] as usize).ok_or_else(|| {
                    DataFusionError::Internal("st_asmvt state value index out of bounds".into())
                })?;
                let key_idx = self.ensure_key(key);
                let value_idx = self.ensure_value(value);
                tags.push(key_idx);
                tags.push(value_idx);
            }
            self.features
                .push(crate::mvt::MvtFeature { tags, ..feature });
        }
        Ok(())
    }
}

#[derive(Debug)]
struct MvtAccumulatorState {
    layer_name: String,
    extent: u32,
    keys: Vec<String>,
    values: Vec<String>,
    features: Vec<crate::mvt::MvtFeature>,
}

const MVT_STATE_MAGIC: &[u8] = b"QGMVT2\0";

fn encode_mvt_state(acc: &MvtAccumulator) -> Vec<u8> {
    let mut blob = Vec::new();
    blob.extend_from_slice(MVT_STATE_MAGIC);
    push_state_string(&mut blob, &acc.layer_name);
    push_state_u32(&mut blob, acc.extent);
    push_state_strings(&mut blob, &acc.keys);
    push_state_strings(&mut blob, &acc.values);
    push_state_u32(&mut blob, acc.features.len() as u32);
    for feature in &acc.features {
        push_state_u32(&mut blob, feature.geom_type);
        match feature.id {
            Some(id) => {
                blob.push(1);
                blob.extend_from_slice(&id.to_le_bytes());
            }
            None => blob.push(0),
        }
        push_state_u32_slice(&mut blob, &feature.tags);
        push_state_u32_slice(&mut blob, &feature.commands);
    }
    blob
}

fn decode_mvt_state(buf: &[u8]) -> DFResult<MvtAccumulatorState> {
    let mut offset = 0;
    if read_state_bytes(buf, &mut offset, MVT_STATE_MAGIC.len())? != MVT_STATE_MAGIC {
        return Err(DataFusionError::Internal(
            "st_asmvt state had an unknown format".into(),
        ));
    }
    let layer_name = read_state_string(buf, &mut offset)?;
    let extent = read_state_u32(buf, &mut offset)?;
    let keys = read_state_strings(buf, &mut offset)?;
    let values = read_state_strings(buf, &mut offset)?;
    let feature_count = read_state_u32(buf, &mut offset)? as usize;
    let mut features = Vec::with_capacity(feature_count);
    for _ in 0..feature_count {
        let geom_type = read_state_u32(buf, &mut offset)?;
        let id = match read_state_u8(buf, &mut offset)? {
            0 => None,
            1 => Some(read_state_u64(buf, &mut offset)?),
            other => {
                return Err(DataFusionError::Internal(format!(
                    "st_asmvt state had invalid feature id marker {other}"
                )));
            }
        };
        let tags = read_state_u32_vec(buf, &mut offset)?;
        let commands = read_state_u32_vec(buf, &mut offset)?;
        features.push(crate::mvt::MvtFeature {
            geom_type,
            commands,
            tags,
            id,
        });
    }
    Ok(MvtAccumulatorState {
        layer_name,
        extent,
        keys,
        values,
        features,
    })
}

fn push_state_u32(buf: &mut Vec<u8>, value: u32) {
    buf.extend_from_slice(&value.to_le_bytes());
}

fn push_state_u32_slice(buf: &mut Vec<u8>, values: &[u32]) {
    push_state_u32(buf, values.len() as u32);
    for value in values {
        push_state_u32(buf, *value);
    }
}

fn push_state_string(buf: &mut Vec<u8>, value: &str) {
    push_state_u32(buf, value.len() as u32);
    buf.extend_from_slice(value.as_bytes());
}

fn push_state_strings(buf: &mut Vec<u8>, values: &[String]) {
    push_state_u32(buf, values.len() as u32);
    for value in values {
        push_state_string(buf, value);
    }
}

fn read_state_bytes<'a>(buf: &'a [u8], offset: &mut usize, len: usize) -> DFResult<&'a [u8]> {
    let end = offset
        .checked_add(len)
        .ok_or_else(|| DataFusionError::Internal("st_asmvt state offset overflowed".into()))?;
    if end > buf.len() {
        return Err(DataFusionError::Internal(
            "st_asmvt state ended unexpectedly".into(),
        ));
    }
    let bytes = &buf[*offset..end];
    *offset = end;
    Ok(bytes)
}

fn read_state_u8(buf: &[u8], offset: &mut usize) -> DFResult<u8> {
    Ok(read_state_bytes(buf, offset, 1)?[0])
}

fn read_state_u32(buf: &[u8], offset: &mut usize) -> DFResult<u32> {
    let bytes: [u8; 4] = read_state_bytes(buf, offset, 4)?
        .try_into()
        .expect("read_state_bytes returned exactly four bytes");
    Ok(u32::from_le_bytes(bytes))
}

fn read_state_u64(buf: &[u8], offset: &mut usize) -> DFResult<u64> {
    let bytes: [u8; 8] = read_state_bytes(buf, offset, 8)?
        .try_into()
        .expect("read_state_bytes returned exactly eight bytes");
    Ok(u64::from_le_bytes(bytes))
}

fn read_state_u32_vec(buf: &[u8], offset: &mut usize) -> DFResult<Vec<u32>> {
    let len = read_state_u32(buf, offset)? as usize;
    let mut values = Vec::with_capacity(len);
    for _ in 0..len {
        values.push(read_state_u32(buf, offset)?);
    }
    Ok(values)
}

fn read_state_string(buf: &[u8], offset: &mut usize) -> DFResult<String> {
    let len = read_state_u32(buf, offset)? as usize;
    let bytes = read_state_bytes(buf, offset, len)?;
    std::str::from_utf8(bytes)
        .map(str::to_string)
        .map_err(|e| DataFusionError::Internal(format!("st_asmvt state string was not UTF-8: {e}")))
}

fn read_state_strings(buf: &[u8], offset: &mut usize) -> DFResult<Vec<String>> {
    let len = read_state_u32(buf, offset)? as usize;
    let mut values = Vec::with_capacity(len);
    for _ in 0..len {
        values.push(read_state_string(buf, offset)?);
    }
    Ok(values)
}

impl datafusion::logical_expr::Accumulator for MvtAccumulator {
    fn update_batch(&mut self, values: &[ArrayRef]) -> DFResult<()> {
        let geom = as_binary_array_ref(&values[0])?;
        let layer = values.get(1).map(as_string_array_ref).transpose()?;
        let extent = values.get(2).map(as_i64_array_ref).transpose()?;
        let attrs = values
            .iter()
            .skip(3)
            .map(as_string_array_ref)
            .collect::<DFResult<Vec<_>>>()?;
        let attr_keys = self.keys.clone();

        for i in 0..geom.len() {
            if let Some(layer) = layer
                && !layer.is_null(i)
            {
                self.layer_name = layer.value(i).to_string();
            }
            if let Some(extent) = extent
                && !extent.is_null(i)
            {
                let value = extent.value(i);
                if !(1..=u32::MAX as i64).contains(&value) {
                    return Err(DataFusionError::Execution(format!(
                        "st_asmvt extent must be in the u32 range, got {value}"
                    )));
                }
                self.extent = value as u32;
            }

            if geom.is_null(i) {
                continue;
            }
            if let Some(parsed) = crate::mvt::parse_wkb(geom.value(i))
                && let Some((geom_type, commands)) = crate::mvt::encode_mvt_geometry(&parsed)
            {
                let mut tags = Vec::new();
                for (attr_idx, attr) in attrs.iter().enumerate() {
                    if attr.is_null(i) {
                        continue;
                    }
                    let key = attr_keys
                        .get(attr_idx)
                        .cloned()
                        .unwrap_or_else(|| format!("attr{}", attr_idx + 1));
                    let key_idx = self.ensure_key(&key);
                    let value_idx = self.ensure_value(attr.value(i));
                    tags.push(key_idx);
                    tags.push(value_idx);
                }
                self.features.push(crate::mvt::MvtFeature {
                    geom_type,
                    commands,
                    tags,
                    id: None,
                });
            }
        }
        Ok(())
    }

    fn merge_batch(&mut self, states: &[ArrayRef]) -> DFResult<()> {
        if states.len() != 1 {
            return Err(DataFusionError::Internal(format!(
                "st_asmvt expected one state array, got {}",
                states.len()
            )));
        }
        let state = &states[0];
        for row in 0..state.len() {
            let Some(bytes) = binary_value_at(state, row, "st_asmvt state")? else {
                continue;
            };
            let decoded = decode_mvt_state(bytes)?;
            self.merge_decoded_state(decoded)?;
        }
        Ok(())
    }

    fn evaluate(&mut self) -> DFResult<ScalarValue> {
        let tile = crate::mvt::build_mvt_tile_with_dictionary(
            &self.layer_name,
            self.extent,
            &self.keys,
            &self.values,
            &self.features,
        );
        Ok(ScalarValue::Binary(Some(tile)))
    }

    fn size(&self) -> usize {
        self.features
            .iter()
            .map(|f| (f.commands.len() + f.tags.len()) * 4)
            .sum::<usize>()
            + self.keys.iter().map(String::len).sum::<usize>()
            + self.values.iter().map(String::len).sum::<usize>()
    }

    fn state(&mut self) -> DFResult<Vec<ScalarValue>> {
        Ok(vec![ScalarValue::Binary(Some(encode_mvt_state(self)))])
    }
}

#[derive(Debug, Default)]
struct ExtentAccumulator {
    extent: Option<Rect>,
}

impl ExtentAccumulator {
    fn update_wkb(&mut self, wkb: &[u8]) -> DFResult<()> {
        if let Some(rect) = rect_from_wkb(wkb)? {
            self.update_rect(rect);
        }
        Ok(())
    }

    fn update_rect(&mut self, rect: Rect) {
        self.extent = Some(match self.extent {
            Some(current) => Rect {
                min_x: current.min_x.min(rect.min_x),
                min_y: current.min_y.min(rect.min_y),
                max_x: current.max_x.max(rect.max_x),
                max_y: current.max_y.max(rect.max_y),
            },
            None => rect,
        });
    }
}

impl datafusion::logical_expr::Accumulator for ExtentAccumulator {
    fn update_batch(&mut self, values: &[ArrayRef]) -> DFResult<()> {
        let geom = values.first().ok_or_else(|| {
            datafusion::common::DataFusionError::Internal(
                "st_extent expected one geometry argument".into(),
            )
        })?;
        if let Some(binary) = geom.as_any().downcast_ref::<BinaryArray>() {
            for row in 0..binary.len() {
                if !binary.is_null(row) {
                    self.update_wkb(binary.value(row))?;
                }
            }
            return Ok(());
        }
        if let Some(binary_view) = geom.as_any().downcast_ref::<BinaryViewArray>() {
            for row in 0..binary_view.len() {
                if !binary_view.is_null(row) {
                    self.update_wkb(binary_view.value(row))?;
                }
            }
            return Ok(());
        }
        Err(datafusion::common::DataFusionError::Internal(
            "st_extent expected Binary geometry".into(),
        ))
    }

    fn merge_batch(&mut self, states: &[ArrayRef]) -> DFResult<()> {
        if states.len() != 4 {
            return Err(datafusion::common::DataFusionError::Internal(format!(
                "st_extent expected four state arrays, got {}",
                states.len()
            )));
        }
        let min_x = as_f64_array_ref(&states[0])?;
        let min_y = as_f64_array_ref(&states[1])?;
        let max_x = as_f64_array_ref(&states[2])?;
        let max_y = as_f64_array_ref(&states[3])?;
        for row in 0..min_x.len() {
            if min_x.is_null(row) || min_y.is_null(row) || max_x.is_null(row) || max_y.is_null(row)
            {
                continue;
            }
            self.update_rect(Rect {
                min_x: min_x.value(row),
                min_y: min_y.value(row),
                max_x: max_x.value(row),
                max_y: max_y.value(row),
            });
        }
        Ok(())
    }

    fn evaluate(&mut self) -> DFResult<ScalarValue> {
        Ok(ScalarValue::Utf8(self.extent.map(format_box2d)))
    }

    fn size(&self) -> usize {
        std::mem::size_of::<Self>()
    }

    fn state(&mut self) -> DFResult<Vec<ScalarValue>> {
        Ok(match self.extent {
            Some(rect) => vec![
                ScalarValue::Float64(Some(rect.min_x)),
                ScalarValue::Float64(Some(rect.min_y)),
                ScalarValue::Float64(Some(rect.max_x)),
                ScalarValue::Float64(Some(rect.max_y)),
            ],
            None => vec![
                ScalarValue::Float64(None),
                ScalarValue::Float64(None),
                ScalarValue::Float64(None),
                ScalarValue::Float64(None),
            ],
        })
    }
}

fn format_box2d(rect: Rect) -> String {
    format!(
        "BOX({} {},{} {})",
        format_box_coord(rect.min_x),
        format_box_coord(rect.min_y),
        format_box_coord(rect.max_x),
        format_box_coord(rect.max_y)
    )
}

fn format_box_coord(value: f64) -> String {
    if value == 0.0 {
        "0".to_string()
    } else {
        value.to_string()
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
        return tag_wkb_srid(wkb, target_srid).ok_or_else(|| "failed to tag SRID".to_string());
    }
    let source = format!("EPSG:{source_srid}");
    let target = format!("EPSG:{target_srid}");
    let transform = proj_wkt::transform_from_crs_strings(&source, &target)
        .map_err(|e| format!("CRS transform {source}→{target}: {e}"))?;
    let transformed = parsed.map_coords(|x, y| match transform.convert((x, y)) {
        Ok((nx, ny)) => (nx, ny),
        Err(_) => (x, y),
    });
    tag_wkb_srid(&transformed.to_wkb(), target_srid)
        .ok_or_else(|| "failed to tag transformed SRID".to_string())
}

// ─── && operator (bbox overlap) as function ────────────────────────────────

fn register_bbox_overlap(ctx: &SessionContext) -> DFResult<()> {
    ctx.register_udf(ScalarUDF::new_from_impl(STOverlapsBBox::new()));
    Ok(())
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct STOverlapsBBox {
    signature: Signature,
}

impl STOverlapsBBox {
    fn new() -> Self {
        let binary = DataType::Binary;
        let binary_view = DataType::BinaryView;
        Self {
            signature: Signature::one_of(
                vec![
                    TypeSignature::Exact(vec![binary.clone(), binary.clone()]),
                    TypeSignature::Exact(vec![binary.clone(), binary_view.clone()]),
                    TypeSignature::Exact(vec![binary_view.clone(), binary.clone()]),
                    TypeSignature::Exact(vec![binary_view.clone(), binary_view]),
                ],
                Volatility::Immutable,
            ),
        }
    }
}

impl ScalarUDFImpl for STOverlapsBBox {
    fn name(&self) -> &str {
        "st_overlaps_bbox"
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> DFResult<DataType> {
        Ok(DataType::Boolean)
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> DFResult<ColumnarValue> {
        let arrays = columnar_values_to_arrays(&args.args)?;
        let n = arrays[0].len();
        let mut out = BooleanArray::builder(n);
        for i in 0..n {
            let a = binary_value_at(&arrays[0], i, "st_overlaps_bbox")?;
            let b = binary_value_at(&arrays[1], i, "st_overlaps_bbox")?;
            match (a, b) {
                (Some(a), Some(b)) => {
                    let bbox_a = crate::mvt::parse_wkb(a).and_then(|g| g.bbox());
                    let bbox_b = crate::mvt::parse_wkb(b).and_then(|g| g.bbox());
                    match (bbox_a, bbox_b) {
                        (Some((ax1, ay1, ax2, ay2)), Some((bx1, by1, bx2, by2))) => {
                            out.append_value(ax1 <= bx2 && ax2 >= bx1 && ay1 <= by2 && ay2 >= by1);
                        }
                        _ => out.append_null(),
                    }
                }
                _ => out.append_null(),
            }
        }
        Ok(ColumnarValue::Array(Arc::new(out.finish())))
    }
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
    let (srid, wkt) = if let Some(rest) = ewkt.strip_prefix("SRID=") {
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
    let wkb = encode_wkt(&type_kw.to_uppercase(), &body)?;
    if let Some(srid) = srid {
        tag_wkb_srid(&wkb, srid).ok_or_else(|| "failed to write EWKB SRID tag".to_string())
    } else {
        Ok(wkb)
    }
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
