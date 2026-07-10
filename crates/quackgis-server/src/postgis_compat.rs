// SPDX-License-Identifier: Apache-2.0
//! PostgreSQL/PostGIS compatibility surface: metadata functions and views that
//! clients like Martin, QGIS, and GeoServer expect on connection.

use std::sync::Arc;

use datafusion::arrow::array::{
    Array, ArrayRef, BooleanArray, Int32Array, Int64Array, ListArray, ListBuilder, RecordBatch,
    StringArray, StringBuilder,
};
use datafusion::arrow::datatypes::{DataType, Field, Schema, SchemaRef};
use datafusion::common::Result as DFResult;
use datafusion::logical_expr::{
    ScalarFunctionArgs, ScalarUDF, ScalarUDFImpl, Signature, TypeSignature, Volatility,
};
use datafusion::physical_plan::ColumnarValue;
use datafusion::prelude::SessionContext;

use crate::auth::{AccessRole, AuthConfig, AuthMode, WriteTarget, parse_write_target};

const POSTGIS_VERSION: &str = "3.4.0";
const POSTGIS_VERSION_FULL: &str = "POSTGIS=\"3.4.0\" QUACKGIS";

pub fn register_postgis_compat(ctx: &SessionContext, auth: &AuthConfig) -> DFResult<()> {
    register_postgis_version_udfs(ctx)?;
    register_pg_recovery_udf(ctx)?;
    register_privilege_udfs(ctx, auth)?;
    register_pg_serial_sequence_udf(ctx)?;
    register_current_setting_udf(ctx)?;
    register_pg_array_search_path_udfs(ctx)?;
    register_find_srid_udf(ctx)?;
    register_regexp_matches_udf(ctx)?;
    register_jsonb_object_agg(ctx)?;
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

fn register_privilege_udfs(ctx: &SessionContext, auth: &AuthConfig) -> DFResult<()> {
    // GIS clients check table/schema/database/column editability via PostgreSQL
    // privilege helpers. The two-argument "current user" forms stay allow-all
    // for compatibility because DataFusion scalar UDFs do not receive pgwire
    // session identity. Explicit-user forms fail closed for configured read-only
    // users and unknown users in password mode; DuckLakeSqlHook remains the
    // authoritative write boundary.
    ctx.register_udf(make_privilege_udf(
        "has_table_privilege",
        PrivilegeObject::Table,
        auth,
    ));
    ctx.register_udf(make_privilege_udf(
        "has_schema_privilege",
        PrivilegeObject::Schema,
        auth,
    ));
    ctx.register_udf(make_privilege_udf(
        "has_database_privilege",
        PrivilegeObject::Database,
        auth,
    ));
    ctx.register_udf(make_privilege_udf(
        "has_column_privilege",
        PrivilegeObject::Column,
        auth,
    ));
    ctx.register_udf(make_privilege_udf(
        "has_any_column_privilege",
        PrivilegeObject::Column,
        auth,
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

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
enum PrivilegeObject {
    Database,
    Schema,
    Table,
    Column,
}

#[derive(Debug, Hash, PartialEq, Eq)]
struct PrivilegeUdf {
    signature: Signature,
    name: String,
    object: PrivilegeObject,
    password_mode: bool,
    roles: Vec<PrivilegeRole>,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct PrivilegeRole {
    name: String,
    role: AccessRole,
    write_targets: Option<Vec<WriteTarget>>,
}

fn make_privilege_udf(name: &str, object: PrivilegeObject, auth: &AuthConfig) -> ScalarUDF {
    let signatures = match object {
        PrivilegeObject::Column => vec![
            TypeSignature::Exact(vec![DataType::Utf8, DataType::Utf8, DataType::Utf8]),
            TypeSignature::Exact(vec![
                DataType::Utf8,
                DataType::Utf8,
                DataType::Utf8,
                DataType::Utf8,
            ]),
        ],
        PrivilegeObject::Database | PrivilegeObject::Schema | PrivilegeObject::Table => vec![
            TypeSignature::Exact(vec![DataType::Utf8, DataType::Utf8]),
            TypeSignature::Exact(vec![DataType::Utf8, DataType::Utf8, DataType::Utf8]),
        ],
    };
    let mut roles = auth
        .users()
        .map(|(name, user)| PrivilegeRole {
            name: name.to_string(),
            role: user.role,
            write_targets: user.write_targets().map(<[_]>::to_vec),
        })
        .collect::<Vec<_>>();
    roles.sort_by(|left, right| left.name.cmp(&right.name));
    PrivilegeUdf {
        signature: Signature::one_of(signatures, Volatility::Stable),
        name: name.to_string(),
        object,
        password_mode: auth.mode() == AuthMode::Password,
        roles,
    }
    .into_scalar_udf()
}

impl PrivilegeUdf {
    fn into_scalar_udf(self) -> ScalarUDF {
        ScalarUDF::new_from_impl(self)
    }

    fn role_for_explicit_user(&self, username: &str) -> Option<&PrivilegeRole> {
        self.roles
            .iter()
            .find(|candidate| candidate.name == username)
    }

    fn evaluate(
        &self,
        explicit_user: Option<&str>,
        object_name: Option<&str>,
        privilege: Option<&str>,
    ) -> Option<bool> {
        let Some(privilege) = privilege else {
            return Some(false);
        };

        // Trust mode and current-user compatibility forms preserve the existing
        // allow-all metadata surface. Write statements are still authorized by
        // DuckLakeSqlHook with the real pgwire session identity.
        if !self.password_mode || explicit_user.is_none() {
            return Some(true);
        }

        let Some(role) = explicit_user.and_then(|user| self.role_for_explicit_user(user)) else {
            return Some(false);
        };
        Some(match role.role {
            AccessRole::ReadWrite => self.readwrite_allows(role, object_name, privilege),
            AccessRole::ReadOnly => self.readonly_allows(privilege),
        })
    }

    fn readwrite_allows(
        &self,
        role: &PrivilegeRole,
        object_name: Option<&str>,
        privilege: &str,
    ) -> bool {
        let permissions = privilege_parts(privilege);
        if permissions.is_empty() || permissions.iter().any(|permission| permission == "GRANT") {
            return false;
        }
        if permissions
            .iter()
            .all(|permission| matches!(permission.as_str(), "SELECT" | "USAGE" | "CONNECT"))
        {
            return true;
        }
        let Some(write_targets) = &role.write_targets else {
            return true;
        };
        match self.object {
            PrivilegeObject::Table | PrivilegeObject::Column => object_name
                .and_then(|name| parse_write_target(name).ok())
                .is_some_and(|target| write_targets.iter().any(|allowed| allowed == &target)),
            PrivilegeObject::Database | PrivilegeObject::Schema => false,
        }
    }

    fn readonly_allows(&self, privilege: &str) -> bool {
        if privilege.to_ascii_uppercase().contains("GRANT") {
            return false;
        }
        let permissions = privilege_parts(privilege);
        if permissions.is_empty() {
            return false;
        }

        permissions.iter().all(|permission| match self.object {
            PrivilegeObject::Database => matches!(permission.as_str(), "CONNECT"),
            PrivilegeObject::Schema => matches!(permission.as_str(), "USAGE"),
            PrivilegeObject::Table | PrivilegeObject::Column => {
                matches!(permission.as_str(), "SELECT")
            }
        })
    }
}

fn privilege_parts(privilege: &str) -> Vec<String> {
    privilege
        .split(|ch: char| ch == ',' || ch.is_whitespace())
        .filter(|part| !part.is_empty())
        .map(str::to_ascii_uppercase)
        .collect()
}

impl ScalarUDFImpl for PrivilegeUdf {
    fn name(&self) -> &str {
        &self.name
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> DFResult<DataType> {
        Ok(DataType::Boolean)
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> DFResult<ColumnarValue> {
        let args = ColumnarValue::values_to_arrays(&args.args)?;
        let len = args.first().map(|arg| arg.len()).unwrap_or(1);
        let explicit_user_idx = explicit_user_index(self.object, args.len());
        let object_idx = privilege_object_index(self.object, args.len());
        let privilege_idx = args.len().saturating_sub(1);
        let mut builder = BooleanArray::builder(len);
        for row in 0..len {
            let explicit_user = explicit_user_idx.and_then(|idx| string_at(&args[idx], row));
            let object_name = object_idx.and_then(|idx| string_at(&args[idx], row));
            let privilege = string_at(&args[privilege_idx], row);
            builder.append_option(self.evaluate(explicit_user, object_name, privilege));
        }
        Ok(ColumnarValue::Array(Arc::new(builder.finish())))
    }
}

fn explicit_user_index(object: PrivilegeObject, arg_count: usize) -> Option<usize> {
    match (object, arg_count) {
        (PrivilegeObject::Column, 4) => Some(0),
        (PrivilegeObject::Database | PrivilegeObject::Schema | PrivilegeObject::Table, 3) => {
            Some(0)
        }
        _ => None,
    }
}

fn privilege_object_index(object: PrivilegeObject, arg_count: usize) -> Option<usize> {
    match (object, arg_count) {
        (PrivilegeObject::Column, 3) | (PrivilegeObject::Table, 2) => Some(0),
        (PrivilegeObject::Column, 4) | (PrivilegeObject::Table, 3) => Some(1),
        _ => None,
    }
}

fn string_at(array: &ArrayRef, row: usize) -> Option<&str> {
    let strings = array.as_any().downcast_ref::<StringArray>()?;
    (!strings.is_null(row)).then(|| strings.value(row))
}

fn register_pg_serial_sequence_udf(ctx: &SessionContext) -> DFResult<()> {
    // GeoTools/pgjdbc asks this while mapping generated columns. QuackGIS does
    // not synthesize PostgreSQL sequences for plain integer ids, so mirror
    // PostgreSQL's "no serial sequence" result with NULL text.
    ctx.register_udf(datafusion::logical_expr::create_udf(
        "pg_get_serial_sequence",
        vec![DataType::Utf8, DataType::Utf8],
        DataType::Utf8,
        Volatility::Stable,
        Arc::new(|_| Ok(datafusion::scalar::ScalarValue::Utf8(None).into())),
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

fn register_pg_array_search_path_udfs(ctx: &SessionContext) -> DFResult<()> {
    // pgjdbc's TypeInfoCache walks the PostgreSQL search path with:
    //   generate_series(1, array_upper(current_schemas(false), 1))
    // Keep that catalog probe inside QuackGIS instead of failing planning on
    // PostgreSQL array helpers that DataFusion does not provide natively.
    let text_list = DataType::List(Arc::new(Field::new_list_field(DataType::Utf8, true)));
    ctx.register_udf(datafusion::logical_expr::create_udf(
        "current_schemas",
        vec![DataType::Boolean],
        text_list.clone(),
        Volatility::Stable,
        Arc::new(|args| {
            let include_implicit = bool_arg_value(&args[0], 0)?.unwrap_or(false);
            let mut schemas = Vec::new();
            if include_implicit {
                schemas.push(datafusion::scalar::ScalarValue::Utf8(Some(
                    "pg_catalog".to_string(),
                )));
            }
            schemas.push(datafusion::scalar::ScalarValue::Utf8(Some(
                "public".to_string(),
            )));
            Ok(datafusion::scalar::ScalarValue::List(
                datafusion::scalar::ScalarValue::new_list_nullable(&schemas, &DataType::Utf8),
            )
            .into())
        }),
    ));
    ctx.register_udf(datafusion::logical_expr::create_udf(
        "array_upper",
        vec![text_list, DataType::Int64],
        DataType::Int64,
        Volatility::Immutable,
        Arc::new(array_upper_utf8_list),
    ));
    Ok(())
}

fn array_upper_utf8_list(args: &[ColumnarValue]) -> DFResult<ColumnarValue> {
    let row_count = args
        .iter()
        .find_map(|arg| match arg {
            ColumnarValue::Array(arr) => Some(arr.len()),
            ColumnarValue::Scalar(_) => None,
        })
        .unwrap_or(1);
    if row_count == 1
        && args
            .iter()
            .all(|arg| matches!(arg, ColumnarValue::Scalar(_)))
    {
        return Ok(datafusion::scalar::ScalarValue::Int64(array_upper_value(args, 0)?).into());
    }
    let values = (0..row_count)
        .map(|row| array_upper_value(args, row))
        .collect::<DFResult<Vec<_>>>()?;
    Ok(ColumnarValue::Array(Arc::new(Int64Array::from(values))))
}

fn array_upper_value(args: &[ColumnarValue], row: usize) -> DFResult<Option<i64>> {
    if int_arg_value(&args[1], row)? != Some(1) {
        return Ok(None);
    }
    list_arg_len(&args[0], row)
}

fn list_arg_len(arg: &ColumnarValue, row: usize) -> DFResult<Option<i64>> {
    let list = match arg {
        ColumnarValue::Scalar(datafusion::scalar::ScalarValue::List(list)) => list.as_ref(),
        ColumnarValue::Array(arr) => arr.as_any().downcast_ref::<ListArray>().ok_or_else(|| {
            datafusion::common::DataFusionError::Internal("expected Utf8 list".into())
        })?,
        _ => {
            return Err(datafusion::common::DataFusionError::Internal(
                "expected Utf8 list".into(),
            ));
        }
    };
    if list.is_null(row) {
        Ok(None)
    } else {
        Ok(Some(i64::from(list.value_length(row))))
    }
}

fn bool_arg_value(arg: &ColumnarValue, row: usize) -> DFResult<Option<bool>> {
    match arg {
        ColumnarValue::Scalar(datafusion::scalar::ScalarValue::Boolean(value)) => Ok(*value),
        ColumnarValue::Array(arr) => {
            let bools = arr
                .as_any()
                .downcast_ref::<datafusion::arrow::array::BooleanArray>()
                .ok_or_else(|| {
                    datafusion::common::DataFusionError::Internal("expected Boolean".into())
                })?;
            if bools.is_null(row) {
                Ok(None)
            } else {
                Ok(Some(bools.value(row)))
            }
        }
        _ => Err(datafusion::common::DataFusionError::Internal(
            "expected Boolean".into(),
        )),
    }
}

fn int_arg_value(arg: &ColumnarValue, row: usize) -> DFResult<Option<i64>> {
    match arg {
        ColumnarValue::Scalar(datafusion::scalar::ScalarValue::Int64(value)) => Ok(*value),
        ColumnarValue::Scalar(datafusion::scalar::ScalarValue::Int32(value)) => {
            Ok(value.map(i64::from))
        }
        ColumnarValue::Array(arr) => {
            if let Some(ints) = arr.as_any().downcast_ref::<Int64Array>() {
                return if ints.is_null(row) {
                    Ok(None)
                } else {
                    Ok(Some(ints.value(row)))
                };
            }
            if let Some(ints) = arr.as_any().downcast_ref::<Int32Array>() {
                return if ints.is_null(row) {
                    Ok(None)
                } else {
                    Ok(Some(i64::from(ints.value(row))))
                };
            }
            Err(datafusion::common::DataFusionError::Internal(
                "expected Int64".into(),
            ))
        }
        _ => Err(datafusion::common::DataFusionError::Internal(
            "expected Int64".into(),
        )),
    }
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

pub fn geography_columns_schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("f_table_catalog", DataType::Utf8, true),
        Field::new("f_table_schema", DataType::Utf8, true),
        Field::new("f_table_name", DataType::Utf8, true),
        Field::new("f_geography_column", DataType::Utf8, true),
        Field::new("coord_dimension", DataType::Int32, true),
        Field::new("srid", DataType::Int32, true),
        Field::new("type", DataType::Utf8, true),
    ]))
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
