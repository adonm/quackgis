// SPDX-License-Identifier: Apache-2.0
//! PostGIS compatibility surface: metadata functions and views that clients
//! like Martin, QGIS, and GeoServer expect on connection.

use std::sync::Arc;

use datafusion::arrow::array::{
    Array, Int32Array, ListBuilder, RecordBatch, StringArray, StringBuilder,
};
use datafusion::arrow::datatypes::{DataType, Field, Schema, SchemaRef};
use datafusion::common::Result as DFResult;
use datafusion::logical_expr::Volatility;
use datafusion::physical_plan::ColumnarValue;
use datafusion::prelude::SessionContext;

const POSTGIS_VERSION: &str = "3.4.0";
const POSTGIS_VERSION_FULL: &str = "POSTGIS=\"3.4.0\" QUACKGIS";

pub fn register_postgis_compat(ctx: &SessionContext) -> DFResult<()> {
    register_postgis_version_udfs(ctx)?;
    register_pg_recovery_udf(ctx)?;
    register_privilege_udfs(ctx)?;
    register_current_setting_udf(ctx)?;
    register_find_srid_udf(ctx)?;
    register_regexp_matches_udf(ctx)?;
    register_jsonb_object_agg(ctx)?;
    register_geography_columns(ctx)?;
    register_spatial_ref_sys(ctx)?;
    Ok(())
}

fn register_postgis_version_udfs(ctx: &SessionContext) -> DFResult<()> {
    ctx.register_udf(make_const_string_udf(
        "postgis_lib_version",
        POSTGIS_VERSION.to_string(),
    ));
    ctx.register_udf(make_const_string_udf(
        "postgis_version",
        format!("{POSTGIS_VERSION} QUACKGIS"),
    ));
    ctx.register_udf(make_const_string_udf(
        "postgis_full_version",
        POSTGIS_VERSION_FULL.to_string(),
    ));
    ctx.register_udf(make_const_string_udf(
        "postgis_extensions_versions",
        POSTGIS_VERSION_FULL.to_string(),
    ));
    ctx.register_udf(make_const_string_udf(
        "postgis_geos_version",
        "3.13.0-CAPI-1.19.0".to_string(),
    ));
    ctx.register_udf(make_const_string_udf(
        "postgis_proj_version",
        "9.6.0".to_string(),
    ));
    Ok(())
}

fn make_const_string_udf(name: &str, value: String) -> datafusion::logical_expr::ScalarUDF {
    datafusion::logical_expr::create_udf(
        name,
        vec![],
        DataType::Utf8,
        Volatility::Immutable,
        Arc::new(move |_| Ok(datafusion::scalar::ScalarValue::Utf8(Some(value.clone())).into())),
    )
}

fn register_privilege_udfs(ctx: &SessionContext) -> DFResult<()> {
    // QGIS checks column-level editability via PostgreSQL privilege helpers.
    // QuackGIS has no RBAC yet, so the dev/read-write posture is allow-all.
    ctx.register_udf(datafusion::logical_expr::create_udf(
        "has_column_privilege",
        vec![DataType::Utf8, DataType::Utf8, DataType::Utf8],
        DataType::Boolean,
        Volatility::Stable,
        Arc::new(|_| Ok(datafusion::scalar::ScalarValue::Boolean(Some(true)).into())),
    ));
    ctx.register_udf(datafusion::logical_expr::create_udf(
        "pg_has_role",
        vec![DataType::Int32, DataType::Utf8],
        DataType::Boolean,
        Volatility::Stable,
        Arc::new(|_| Ok(datafusion::scalar::ScalarValue::Boolean(Some(true)).into())),
    ));
    Ok(())
}

fn register_pg_recovery_udf(ctx: &SessionContext) -> DFResult<()> {
    ctx.register_udf(datafusion::logical_expr::create_udf(
        "pg_is_in_recovery",
        vec![],
        DataType::Boolean,
        Volatility::Stable,
        Arc::new(|_| Ok(datafusion::scalar::ScalarValue::Boolean(Some(false)).into())),
    ));
    Ok(())
}

fn register_current_setting_udf(ctx: &SessionContext) -> DFResult<()> {
    ctx.register_udf(datafusion::logical_expr::create_udf(
        "current_setting",
        vec![DataType::Utf8],
        DataType::Utf8,
        Volatility::Stable,
        Arc::new(|args| {
            let name = match &args[0] {
                ColumnarValue::Scalar(datafusion::scalar::ScalarValue::Utf8(Some(s))) => s.clone(),
                _ => return Ok(ColumnarValue::Scalar(datafusion::scalar::ScalarValue::Null)),
            };
            let val = match name.to_lowercase().as_str() {
                "server_version" => "16.0 (QuackGIS)".to_string(),
                "server_version_num" => "160000".to_string(),
                "standard_conforming_strings" => "on".to_string(),
                "client_encoding" => "UTF8".to_string(),
                "application_name" => String::new(),
                "bytea_output" => "hex".to_string(),
                "intervalstyle" => "postgres".to_string(),
                "datestyle" => "ISO, MDY".to_string(),
                "timezone" => "UTC".to_string(),
                "search_path" => "\"public\"".to_string(),
                "enable_seqscan" => "on".to_string(),
                "default_transaction_isolation" => "read committed".to_string(),
                _ => String::new(),
            };
            Ok(datafusion::scalar::ScalarValue::Utf8(Some(val)).into())
        }),
    ));
    Ok(())
}

fn register_find_srid_udf(ctx: &SessionContext) -> DFResult<()> {
    // PostGIS Find_SRID(schema, table, column) resolves typmod/catalog metadata.
    // QuackGIS stores geometry as WKB bytes and currently exposes unknown SRID
    // as 0 in geometry_columns, so mirror that catalog value. Clients use this
    // as metadata discovery; exact CRS tagging remains per-row EWKB.
    ctx.register_udf(datafusion::logical_expr::create_udf(
        "find_srid",
        vec![DataType::Utf8, DataType::Utf8, DataType::Utf8],
        DataType::Int32,
        Volatility::Stable,
        Arc::new(|args| {
            let n = args
                .iter()
                .find_map(|arg| match arg {
                    ColumnarValue::Array(arr) => Some(arr.len()),
                    ColumnarValue::Scalar(_) => None,
                })
                .unwrap_or(1);
            let values = (0..n).map(|row| {
                match (
                    string_arg_value(&args[0], row)?,
                    string_arg_value(&args[1], row)?,
                    string_arg_value(&args[2], row)?,
                ) {
                    (Some(_), Some(_), Some(_)) => Ok(Some(0_i32)),
                    _ => Ok(None),
                }
            });
            let values = values.collect::<DFResult<Vec<_>>>()?;
            Ok(ColumnarValue::Array(Arc::new(Int32Array::from(values))))
        }),
    ));
    Ok(())
}

fn register_regexp_matches_udf(ctx: &SessionContext) -> DFResult<()> {
    // Minimal PostgreSQL-compatible regexp_matches(text, pattern, flags)
    // implementation. Martin uses:
    //   (regexp_matches(current_setting('server_version'), '^(\d+\.\d+)', 'g'))[1]
    // so returning capture groups as a 1-based text[] is enough for startup.
    ctx.register_udf(datafusion::logical_expr::create_udf(
        "regexp_matches",
        vec![DataType::Utf8, DataType::Utf8, DataType::Utf8],
        DataType::List(Arc::new(Field::new("item", DataType::Utf8, true))),
        Volatility::Immutable,
        Arc::new(|args| {
            let n = args
                .iter()
                .find_map(|arg| match arg {
                    ColumnarValue::Array(arr) => Some(arr.len()),
                    ColumnarValue::Scalar(_) => None,
                })
                .unwrap_or(1);
            let mut builder = ListBuilder::new(StringBuilder::new());
            for row in 0..n {
                let text = string_arg_value(&args[0], row)?;
                let pattern = string_arg_value(&args[1], row)?;
                if let (Some(text), Some(pattern)) = (text, pattern) {
                    match regex::Regex::new(pattern) {
                        Ok(re) => {
                            if let Some(caps) = re.captures(text) {
                                for i in 1..caps.len() {
                                    if let Some(m) = caps.get(i) {
                                        builder.values().append_value(m.as_str());
                                    } else {
                                        builder.values().append_null();
                                    }
                                }
                                builder.append(true);
                            } else {
                                builder.append(false);
                            }
                        }
                        Err(_) => builder.append(false),
                    }
                } else {
                    builder.append(false);
                }
            }
            Ok(ColumnarValue::Array(Arc::new(builder.finish())))
        }),
    ));
    Ok(())
}

fn register_jsonb_object_agg(ctx: &SessionContext) -> DFResult<()> {
    use datafusion_functions_aggregate_common::accumulator::{
        AccumulatorArgs, AccumulatorFactoryFunction,
    };

    let accumulator: AccumulatorFactoryFunction = Arc::new(|_args: AccumulatorArgs| {
        Ok(Box::<EmptyJsonObjectAccumulator>::default()
            as Box<
                dyn datafusion_expr_common::accumulator::Accumulator,
            >)
    });

    ctx.register_udaf(datafusion::logical_expr::create_udaf(
        "jsonb_object_agg",
        vec![DataType::Utf8, DataType::Utf8],
        Arc::new(DataType::Utf8),
        Volatility::Immutable,
        accumulator,
        Arc::new(vec![DataType::Utf8]),
    ));
    Ok(())
}

#[derive(Debug, Default)]
struct EmptyJsonObjectAccumulator;

impl datafusion::logical_expr::Accumulator for EmptyJsonObjectAccumulator {
    fn update_batch(&mut self, _values: &[datafusion::arrow::array::ArrayRef]) -> DFResult<()> {
        Ok(())
    }

    fn merge_batch(&mut self, _states: &[datafusion::arrow::array::ArrayRef]) -> DFResult<()> {
        Ok(())
    }

    fn evaluate(&mut self) -> DFResult<datafusion::scalar::ScalarValue> {
        Ok(datafusion::scalar::ScalarValue::Utf8(Some(
            "{}".to_string(),
        )))
    }

    fn size(&self) -> usize {
        0
    }

    fn state(&mut self) -> DFResult<Vec<datafusion::scalar::ScalarValue>> {
        Ok(vec![datafusion::scalar::ScalarValue::Utf8(Some(
            "{}".to_string(),
        ))])
    }
}

fn string_arg_value(arg: &ColumnarValue, row: usize) -> DFResult<Option<&str>> {
    match arg {
        ColumnarValue::Scalar(datafusion::scalar::ScalarValue::Utf8(value)) => Ok(value.as_deref()),
        ColumnarValue::Array(arr) => {
            let strings = arr.as_any().downcast_ref::<StringArray>().ok_or_else(|| {
                datafusion::common::DataFusionError::Internal("expected Utf8".into())
            })?;
            if strings.is_null(row) {
                Ok(None)
            } else {
                Ok(Some(strings.value(row)))
            }
        }
        _ => Err(datafusion::common::DataFusionError::Internal(
            "expected Utf8".into(),
        )),
    }
}

pub fn geometry_columns_schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("f_table_catalog", DataType::Utf8, true),
        Field::new("f_table_schema", DataType::Utf8, true),
        Field::new("f_table_name", DataType::Utf8, true),
        Field::new("f_geometry_column", DataType::Utf8, true),
        Field::new("coord_dimension", DataType::Int32, true),
        Field::new("srid", DataType::Int32, true),
        Field::new("type", DataType::Utf8, true),
    ]))
}

fn register_geography_columns(ctx: &SessionContext) -> DFResult<()> {
    // Martin's discovery query UNIONs geometry_columns and geography_columns.
    // QuackGIS currently exposes geometry only, but the table must exist.
    let schema = Arc::new(Schema::new(vec![
        Field::new("f_table_catalog", DataType::Utf8, true),
        Field::new("f_table_schema", DataType::Utf8, true),
        Field::new("f_table_name", DataType::Utf8, true),
        Field::new("f_geography_column", DataType::Utf8, true),
        Field::new("coord_dimension", DataType::Int32, true),
        Field::new("srid", DataType::Int32, true),
        Field::new("type", DataType::Utf8, true),
    ]));
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(Vec::<Option<&str>>::new())),
            Arc::new(StringArray::from(Vec::<Option<&str>>::new())),
            Arc::new(StringArray::from(Vec::<Option<&str>>::new())),
            Arc::new(StringArray::from(Vec::<Option<&str>>::new())),
            Arc::new(Int32Array::from(Vec::<Option<i32>>::new())),
            Arc::new(Int32Array::from(Vec::<Option<i32>>::new())),
            Arc::new(StringArray::from(Vec::<Option<&str>>::new())),
        ],
    )?;
    ctx.register_batch("geography_columns", batch)?;
    Ok(())
}

fn register_spatial_ref_sys(ctx: &SessionContext) -> DFResult<()> {
    let refs = common_spatial_refs();
    let schema = Arc::new(Schema::new(vec![
        Field::new("srid", DataType::Int32, false),
        Field::new("auth_name", DataType::Utf8, true),
        Field::new("auth_srid", DataType::Int32, true),
        Field::new("srtext", DataType::Utf8, true),
        Field::new("proj4text", DataType::Utf8, true),
    ]));

    let srids: Vec<i32> = refs.iter().map(|r| r.0).collect();
    let auth_names: Vec<Option<&str>> = refs.iter().map(|r| Some(r.1)).collect();
    let auth_srids: Vec<i32> = refs.iter().map(|r| r.2).collect();
    let srtexts: Vec<Option<&str>> = refs.iter().map(|r| Some(r.3)).collect();
    let proj4texts: Vec<Option<&str>> = refs.iter().map(|r| Some(r.4)).collect();

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Int32Array::from(srids)),
            Arc::new(StringArray::from(auth_names)),
            Arc::new(Int32Array::from(auth_srids)),
            Arc::new(StringArray::from(srtexts)),
            Arc::new(StringArray::from(proj4texts)),
        ],
    )?;
    ctx.register_batch("spatial_ref_sys", batch)?;
    Ok(())
}

fn common_spatial_refs() -> Vec<(i32, &'static str, i32, &'static str, &'static str)> {
    vec![
        (
            4326,
            "EPSG",
            4326,
            "GEOGCS[\"WGS 84\",DATUM[\"WGS_1984\",SPHEROID[\"WGS 84\",6378137,298.257223563]],PRIMEM[\"Greenwich\",0],UNIT[\"degree\",0.0174532925199433]]",
            "+proj=longlat +datum=WGS84 +no_defs",
        ),
        (
            3857,
            "EPSG",
            3857,
            "PROJCS[\"WGS 84 / Pseudo-Mercator\",GEOGCS[\"WGS 84\",DATUM[\"WGS_1984\",SPHEROID[\"WGS 84\",6378137,298.257223563]],PRIMEM[\"Greenwich\",0],UNIT[\"degree\",0.0174532925199433]],PROJECTION[\"Mercator_1SP\"],UNIT[\"metre\",1]]",
            "+proj=merc +a=6378137 +b=6378137 +lat_ts=0 +lon_0=0 +x_0=0 +y_0=0 +k=1 +units=m +nadgrids=@null +wktext +no_defs",
        ),
        (
            4269,
            "EPSG",
            4269,
            "GEOGCS[\"NAD83\",DATUM[\"North_American_Datum_1983\",SPHEROID[\"GRS 1980\",6378137,298.257222101]],PRIMEM[\"Greenwich\",0],UNIT[\"degree\",0.0174532925199433]]",
            "+proj=longlat +datum=NAD83 +no_defs",
        ),
        (
            27700,
            "EPSG",
            27700,
            "PROJCS[\"OSGB36 / British National Grid\",GEOGCS[\"OSGB36\",DATUM[\"Ordnance_Survey_of_Great_Britain_1936\",SPHEROID[\"Airy 1830\",6377563.396,299.3249646]],PRIMEM[\"Greenwich\",0],UNIT[\"degree\",0.0174532925199433]],UNIT[\"metre\",1]]",
            "+proj=tmerc +lat_0=49 +lon_0=-2 +k=0.9996012717 +x_0=400000 +y_0=-100000 +ellps=airy +datum=OSGB36 +units=m +no_defs",
        ),
    ]
}
