// SPDX-License-Identifier: Apache-2.0
//! QuackGIS SQL-to-DuckLake routing.
//!
//! datafusion-ducklake's writer API is the validated storage path. This hook
//! maps the SQL clients actually send (CTAS / INSERT) onto that writer API for
//! the `quackgis.main.<table>` catalog path.

use std::sync::Arc;

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use datafusion::arrow::array::{
    Array, ArrayRef, BinaryArray, BinaryViewArray, Int32Array, Int64Array, NullArray, StringArray,
    StringViewArray,
};
use datafusion::arrow::datatypes::{DataType, Field, Schema, SchemaRef};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::common::ParamValues;
use datafusion::logical_expr::LogicalPlan;
use datafusion::prelude::SessionContext;
use datafusion::sql::sqlparser::ast::{
    AssignmentTarget, ColumnDef, Expr, FromTable, ObjectName, TableFactor, TableWithJoins,
};
use datafusion_ducklake::{
    DuckLakeCatalog, DuckLakeTableWriter, MetadataWriter, SqliteMetadataProvider,
    SqliteMetadataWriter,
};
use datafusion_postgres::hooks::{HookClient, QueryHook};
use datafusion_postgres::pgwire::api::results::{Response, Tag};
use datafusion_postgres::pgwire::error::{PgWireError, PgWireResult};
use object_store::local::LocalFileSystem;

use crate::context::{DUCKLAKE_CATALOG, StoragePaths};

#[derive(Debug, Clone)]
pub struct DuckLakeSqlHook {
    paths: StoragePaths,
}

impl DuckLakeSqlHook {
    pub fn new(paths: StoragePaths) -> Self {
        Self { paths }
    }
}

#[async_trait]
impl QueryHook for DuckLakeSqlHook {
    async fn handle_simple_query(
        &self,
        statement: &datafusion::sql::sqlparser::ast::Statement,
        session_context: &SessionContext,
        _client: &mut dyn HookClient,
    ) -> Option<PgWireResult<Response>> {
        match statement {
            datafusion::sql::sqlparser::ast::Statement::CreateTable(ct)
                if table_name_parts(&ct.name).is_some() =>
            {
                Some(self.handle_create_table(ct, session_context).await)
            }
            datafusion::sql::sqlparser::ast::Statement::Insert(insert)
                if insert.source.is_some() && insert_target_parts(&insert.table).is_some() =>
            {
                Some(self.handle_insert(insert, session_context).await)
            }
            datafusion::sql::sqlparser::ast::Statement::Delete(delete)
                if delete_target_parts(delete).is_some() =>
            {
                Some(self.handle_delete(delete, session_context).await)
            }
            datafusion::sql::sqlparser::ast::Statement::Update(update)
                if update_target_parts(&update.table).is_some() =>
            {
                Some(
                    self.handle_update(
                        &update.table,
                        &update.assignments,
                        update.selection.as_ref(),
                        session_context,
                    )
                    .await,
                )
            }
            _ => None,
        }
    }

    async fn handle_extended_parse_query(
        &self,
        _statement: &datafusion::sql::sqlparser::ast::Statement,
        _session_context: &SessionContext,
        _client: &(dyn datafusion_postgres::pgwire::api::ClientInfo + Send + Sync),
    ) -> Option<PgWireResult<LogicalPlan>> {
        None
    }

    async fn handle_extended_query(
        &self,
        statement: &datafusion::sql::sqlparser::ast::Statement,
        _logical_plan: &LogicalPlan,
        _params: &ParamValues,
        session_context: &SessionContext,
        _client: &mut dyn HookClient,
    ) -> Option<PgWireResult<Response>> {
        // Route extended-protocol CTAS/INSERT too; clients differ in whether
        // they send DDL via simple or extended flow.
        match statement {
            datafusion::sql::sqlparser::ast::Statement::CreateTable(ct)
                if table_name_parts(&ct.name).is_some() =>
            {
                Some(self.handle_create_table(ct, session_context).await)
            }
            datafusion::sql::sqlparser::ast::Statement::Insert(insert)
                if insert.source.is_some() && insert_target_parts(&insert.table).is_some() =>
            {
                Some(self.handle_insert(insert, session_context).await)
            }
            datafusion::sql::sqlparser::ast::Statement::Delete(delete)
                if delete_target_parts(delete).is_some() =>
            {
                Some(self.handle_delete(delete, session_context).await)
            }
            datafusion::sql::sqlparser::ast::Statement::Update(update)
                if update_target_parts(&update.table).is_some() =>
            {
                Some(
                    self.handle_update(
                        &update.table,
                        &update.assignments,
                        update.selection.as_ref(),
                        session_context,
                    )
                    .await,
                )
            }
            _ => None,
        }
    }
}

impl DuckLakeSqlHook {
    async fn handle_create_table(
        &self,
        ct: &datafusion::sql::sqlparser::ast::CreateTable,
        session_context: &SessionContext,
    ) -> PgWireResult<Response> {
        let (schema, table) = table_name_parts(&ct.name).expect("guarded by caller");
        if let Some(query) = &ct.query {
            self.write_query(
                session_context,
                &query.to_string(),
                &schema,
                &table,
                WriteDisposition::Replace,
            )
            .await?;
        } else {
            self.create_empty_table(&schema, &table, &ct.columns)
                .await?;
            self.refresh_ducklake_catalog(session_context).await?;
        }
        Ok(Response::Execution(Tag::new("CREATE TABLE")))
    }

    async fn handle_insert(
        &self,
        insert: &datafusion::sql::sqlparser::ast::Insert,
        session_context: &SessionContext,
    ) -> PgWireResult<Response> {
        let (schema, table) = insert_target_parts(&insert.table).expect("guarded by caller");
        let source_query = insert
            .source
            .as_ref()
            .expect("guarded by caller")
            .to_string();
        let query = if insert.columns.is_empty()
            && !insert_source_is_values(insert.source.as_ref().expect("guarded by caller"))
        {
            source_query
        } else {
            self.insert_source_with_target_schema(
                session_context,
                &schema,
                &table,
                &insert.columns,
                &source_query,
            )
            .await?
        };
        let rows = self
            .write_query(
                session_context,
                &query,
                &schema,
                &table,
                WriteDisposition::Append,
            )
            .await?;
        Ok(Response::Execution(Tag::new(&format!("INSERT 0 {rows}"))))
    }

    async fn handle_delete(
        &self,
        delete: &datafusion::sql::sqlparser::ast::Delete,
        session_context: &SessionContext,
    ) -> PgWireResult<Response> {
        let (schema, table) = delete_target_parts(delete).expect("guarded by caller");
        let table_ref =
            format!("{DUCKLAKE_CATALOG}.{}.", quote_ident(&schema)) + &quote_ident(&table);
        let where_clause = delete
            .selection
            .as_ref()
            .map(|e| format!("NOT ({e})"))
            .unwrap_or_else(|| "FALSE".to_string());
        let query = format!("SELECT * FROM {table_ref} WHERE {where_clause}");
        let remaining = self
            .write_query(
                session_context,
                &query,
                &schema,
                &table,
                WriteDisposition::Replace,
            )
            .await?;
        Ok(Response::Execution(Tag::new(&format!(
            "DELETE {remaining}"
        ))))
    }

    async fn handle_update(
        &self,
        table: &TableWithJoins,
        assignments: &[datafusion::sql::sqlparser::ast::Assignment],
        selection: Option<&Expr>,
        session_context: &SessionContext,
    ) -> PgWireResult<Response> {
        let (schema, table_name) = update_target_parts(table).expect("guarded by caller");
        let table_ref =
            format!("{DUCKLAKE_CATALOG}.{}.", quote_ident(&schema)) + &quote_ident(&table_name);
        let schema_ref = self.table_schema(session_context, &table_ref).await?;
        let mut assignment_map = std::collections::HashMap::new();
        for assignment in assignments {
            let AssignmentTarget::ColumnName(name) = &assignment.target else {
                return Err(user_error(anyhow!(
                    "tuple UPDATE assignments are not supported yet"
                )));
            };
            let col = object_name_last(name)
                .ok_or_else(|| user_error(anyhow!("invalid UPDATE target")))?;
            assignment_map.insert(col, assignment.value.to_string());
        }
        let predicate = selection.map(|e| e.to_string());
        let mut select_items = Vec::new();
        for field in schema_ref.fields() {
            let col = field.name();
            let expr = if let Some(value) = assignment_map.get(col) {
                let sql_type = arrow_type_to_sql(field.data_type()).map_err(user_error)?;
                if let Some(pred) = &predicate {
                    format!(
                        "CAST(CASE WHEN {pred} THEN {value} ELSE {} END AS {sql_type}) AS {}",
                        quote_ident(col),
                        quote_ident(col)
                    )
                } else {
                    format!("CAST({value} AS {sql_type}) AS {}", quote_ident(col))
                }
            } else {
                quote_ident(col)
            };
            select_items.push(expr);
        }
        let query = format!("SELECT {} FROM {table_ref}", select_items.join(", "));
        let rows = self
            .write_query(
                session_context,
                &query,
                &schema,
                &table_name,
                WriteDisposition::Replace,
            )
            .await?;
        Ok(Response::Execution(Tag::new(&format!("UPDATE {rows}"))))
    }

    async fn create_empty_table(
        &self,
        schema: &str,
        table: &str,
        columns: &[ColumnDef],
    ) -> PgWireResult<()> {
        if columns.is_empty() {
            return Err(user_error(anyhow!(
                "CREATE TABLE requires at least one column"
            )));
        }
        let fields = columns
            .iter()
            .map(sql_type_to_arrow_field)
            .collect::<Result<Vec<_>>>()
            .map_err(user_error)?;
        let arrays = fields
            .iter()
            .map(|f| empty_array_for(f.data_type()))
            .collect::<Result<Vec<_>>>()
            .map_err(user_error)?;
        let batch = RecordBatch::try_new(Arc::new(Schema::new(fields)), arrays)
            .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
        self.write_batches(schema, table, &[batch], WriteDisposition::Replace)
            .await
            .map(|_| ())
    }

    async fn insert_source_with_target_schema(
        &self,
        session_context: &SessionContext,
        schema: &str,
        table: &str,
        insert_columns: &[ObjectName],
        source_query: &str,
    ) -> PgWireResult<String> {
        let table_ref =
            format!("{DUCKLAKE_CATALOG}.{}.", quote_ident(schema)) + &quote_ident(table);
        let schema_ref = self.table_schema(session_context, &table_ref).await?;
        let mut insert_positions = std::collections::HashMap::new();
        if insert_columns.is_empty() {
            // INSERT INTO table VALUES (...) yields DataFusion columns named
            // column1, column2, ... . Alias them back to the target table schema
            // so Parquet/DuckLake persists the real column names.
            for (idx, field) in schema_ref.fields().iter().enumerate() {
                insert_positions.insert(field.name().clone(), idx + 1);
            }
        } else {
            for (idx, name) in insert_columns.iter().enumerate() {
                let col = object_name_last(name)
                    .ok_or_else(|| user_error(anyhow!("invalid INSERT column")))?;
                insert_positions.insert(col, idx + 1); // VALUES columns are column1, column2, ...
            }
        }
        let mut items = Vec::new();
        for field in schema_ref.fields() {
            let col = field.name();
            let expr = if let Some(pos) = insert_positions.get(col) {
                format!(
                    "CAST(column{pos} AS {}) AS {}",
                    arrow_type_to_sql(field.data_type()).map_err(user_error)?,
                    quote_ident(col)
                )
            } else {
                format!(
                    "CAST(NULL AS {}) AS {}",
                    arrow_type_to_sql(field.data_type()).map_err(user_error)?,
                    quote_ident(col)
                )
            };
            items.push(expr);
        }
        Ok(format!(
            "SELECT {} FROM ({source_query}) AS v",
            items.join(", ")
        ))
    }

    async fn write_query(
        &self,
        session_context: &SessionContext,
        query: &str,
        schema: &str,
        table: &str,
        disposition: WriteDisposition,
    ) -> PgWireResult<usize> {
        let batches = session_context
            .sql(query)
            .await
            .map_err(|e| PgWireError::ApiError(Box::new(e)))?
            .collect()
            .await
            .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
        let rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        let batches = normalize_batches_for_ducklake(batches).map_err(user_error)?;

        self.write_batches(schema, table, &batches, disposition)
            .await?;
        self.refresh_ducklake_catalog(session_context).await?;
        Ok(rows)
    }

    async fn write_batches(
        &self,
        schema: &str,
        table: &str,
        batches: &[RecordBatch],
        disposition: WriteDisposition,
    ) -> PgWireResult<usize> {
        let rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        let writer = Arc::new(
            SqliteMetadataWriter::new_with_init(&self.paths.catalog_conn)
                .await
                .map_err(|e| PgWireError::ApiError(Box::new(e)))?,
        );
        writer
            .set_data_path(&self.paths.data_path)
            .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
        let snapshot = writer
            .create_snapshot()
            .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
        writer
            .get_or_create_schema(schema, None, snapshot)
            .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
        let object_store: Arc<dyn object_store::ObjectStore> = Arc::new(LocalFileSystem::new());
        let table_writer = DuckLakeTableWriter::new(writer, object_store)
            .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
        match disposition {
            WriteDisposition::Replace => table_writer
                .write_table(schema, table, batches)
                .await
                .map_err(|e| PgWireError::ApiError(Box::new(e)))?,
            WriteDisposition::Append => table_writer
                .append_table(schema, table, batches)
                .await
                .map_err(|e| PgWireError::ApiError(Box::new(e)))?,
        };
        Ok(rows)
    }

    async fn table_schema(
        &self,
        session_context: &SessionContext,
        table_ref: &str,
    ) -> PgWireResult<SchemaRef> {
        let df = session_context
            .sql(&format!("SELECT * FROM {table_ref} LIMIT 0"))
            .await
            .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
        Ok(Arc::new(df.schema().as_arrow().clone()))
    }

    async fn refresh_ducklake_catalog(&self, session_context: &SessionContext) -> PgWireResult<()> {
        let writer = Arc::new(
            SqliteMetadataWriter::new_with_init(&self.paths.catalog_conn)
                .await
                .map_err(|e| PgWireError::ApiError(Box::new(e)))?,
        );
        writer
            .set_data_path(&self.paths.data_path)
            .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
        let provider = SqliteMetadataProvider::new(&self.paths.catalog_conn)
            .await
            .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
        let ducklake = DuckLakeCatalog::with_writer(Arc::new(provider), writer)
            .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
        session_context.register_catalog(DUCKLAKE_CATALOG, Arc::new(ducklake));
        crate::public_schema::register_public_schema_alias(session_context)
            .map_err(|e| PgWireError::ApiError(Box::new(e)))?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
enum WriteDisposition {
    Replace,
    Append,
}

fn table_name_parts(
    name: &datafusion::sql::sqlparser::ast::ObjectName,
) -> Option<(String, String)> {
    let parts: Vec<String> = name
        .0
        .iter()
        .map(|p| p.to_string().trim_matches('"').to_string())
        .collect();
    match parts.as_slice() {
        [catalog, schema, table] if catalog == DUCKLAKE_CATALOG => {
            Some((schema.clone(), table.clone()))
        }
        [schema, table] if schema == "main" => Some((schema.clone(), table.clone())),
        _ => None,
    }
}

fn insert_target_parts(
    table: &datafusion::sql::sqlparser::ast::TableObject,
) -> Option<(String, String)> {
    match table {
        datafusion::sql::sqlparser::ast::TableObject::TableName(name) => table_name_parts(name),
        _ => None,
    }
}

fn insert_source_is_values(query: &datafusion::sql::sqlparser::ast::Query) -> bool {
    matches!(
        query.body.as_ref(),
        datafusion::sql::sqlparser::ast::SetExpr::Values(_)
    )
}

fn normalize_batches_for_ducklake(batches: Vec<RecordBatch>) -> Result<Vec<RecordBatch>> {
    batches
        .into_iter()
        .map(normalize_batch_for_ducklake)
        .collect()
}

fn normalize_batch_for_ducklake(batch: RecordBatch) -> Result<RecordBatch> {
    let fields = batch.schema().fields().iter().cloned().collect::<Vec<_>>();
    let mut changed = false;
    let mut new_fields = Vec::with_capacity(fields.len());
    let mut arrays: Vec<ArrayRef> = Vec::with_capacity(fields.len());

    for (field, arr) in fields.into_iter().zip(batch.columns()) {
        match field.data_type() {
            DataType::Utf8View => {
                let a = arr
                    .as_any()
                    .downcast_ref::<StringViewArray>()
                    .ok_or_else(|| anyhow!("expected StringViewArray for Utf8View"))?;
                let vals: Vec<Option<&str>> = (0..a.len())
                    .map(|i| if a.is_null(i) { None } else { Some(a.value(i)) })
                    .collect();
                arrays.push(Arc::new(StringArray::from(vals)));
                new_fields.push(Arc::new(Field::new(
                    field.name(),
                    DataType::Utf8,
                    field.is_nullable(),
                )));
                changed = true;
            }
            DataType::BinaryView => {
                let a = arr
                    .as_any()
                    .downcast_ref::<BinaryViewArray>()
                    .ok_or_else(|| anyhow!("expected BinaryViewArray for BinaryView"))?;
                let vals: Vec<Option<&[u8]>> = (0..a.len())
                    .map(|i| if a.is_null(i) { None } else { Some(a.value(i)) })
                    .collect();
                arrays.push(Arc::new(BinaryArray::from(vals)));
                new_fields.push(Arc::new(Field::new(
                    field.name(),
                    DataType::Binary,
                    field.is_nullable(),
                )));
                changed = true;
            }
            _ => {
                arrays.push(Arc::clone(arr));
                new_fields.push(field);
            }
        }
    }

    if !changed {
        return Ok(batch);
    }

    RecordBatch::try_new(Arc::new(Schema::new(new_fields)), arrays)
        .map_err(|e| anyhow!("normalizing RecordBatch for DuckLake: {e}"))
}

fn delete_target_parts(
    delete: &datafusion::sql::sqlparser::ast::Delete,
) -> Option<(String, String)> {
    let from = match &delete.from {
        FromTable::WithFromKeyword(t) | FromTable::WithoutKeyword(t) => t,
    };
    if from.len() != 1 || delete.using.is_some() || !delete.tables.is_empty() {
        return None;
    }
    table_factor_parts(&from[0].relation)
}

fn update_target_parts(table: &TableWithJoins) -> Option<(String, String)> {
    if !table.joins.is_empty() {
        return None;
    }
    table_factor_parts(&table.relation)
}

fn table_factor_parts(f: &TableFactor) -> Option<(String, String)> {
    match f {
        TableFactor::Table { name, .. } => table_name_parts(name),
        _ => None,
    }
}

fn object_name_last(name: &ObjectName) -> Option<String> {
    name.0
        .last()
        .map(|p| p.to_string().trim_matches('"').to_string())
}

fn quote_ident(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

fn sql_type_to_arrow_field(col: &ColumnDef) -> Result<Field> {
    use datafusion::sql::sqlparser::ast::DataType as SqlType;
    let dt = match &col.data_type {
        SqlType::Int(_) | SqlType::Integer(_) => DataType::Int32,
        SqlType::BigInt(_) => DataType::Int64,
        SqlType::Text | SqlType::String(_) | SqlType::Varchar(_) | SqlType::Char(_) => {
            DataType::Utf8
        }
        SqlType::Bytea | SqlType::Binary(_) | SqlType::Varbinary(_) => DataType::Binary,
        other => {
            return Err(anyhow!(
                "unsupported CREATE TABLE column type for {}: {other}",
                col.name
            ));
        }
    };
    Ok(Field::new(col.name.to_string(), dt, true))
}

fn arrow_type_to_sql(dt: &DataType) -> Result<&'static str> {
    match dt {
        DataType::Int32 => Ok("INT"),
        DataType::Int64 => Ok("BIGINT"),
        DataType::Utf8 => Ok("VARCHAR"),
        DataType::Binary => Ok("BYTEA"),
        other => Err(anyhow!("unsupported INSERT target column type: {other}")),
    }
}

fn empty_array_for(dt: &DataType) -> Result<ArrayRef> {
    match dt {
        DataType::Int32 => Ok(Arc::new(Int32Array::from(Vec::<i32>::new()))),
        DataType::Int64 => Ok(Arc::new(Int64Array::from(Vec::<i64>::new()))),
        DataType::Utf8 => Ok(Arc::new(StringArray::from(Vec::<String>::new()))),
        DataType::Binary => Ok(Arc::new(BinaryArray::from(Vec::<&[u8]>::new()))),
        _ => Ok(Arc::new(NullArray::new(0))),
    }
}

fn user_error(err: anyhow::Error) -> PgWireError {
    PgWireError::UserError(Box::new(
        datafusion_postgres::pgwire::error::ErrorInfo::new(
            "ERROR".to_string(),
            "22023".to_string(),
            err.to_string(),
        ),
    ))
}
