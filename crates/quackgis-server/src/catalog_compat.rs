// SPDX-License-Identifier: Apache-2.0
//! PostgreSQL/PostGIS catalog and cursor compatibility shims found by
//! client-trace probing.
//!
//! This module is organized around PostgreSQL/PostGIS surfaces rather than
//! individual clients. QGIS, GDAL/OGR, Martin, and similar clients mostly probe
//! the same boundary: `pg_type`, `pg_class`, `pg_attribute`, `pg_index`,
//! `geometry_columns`, and cursor flow. Helpers below are named for those
//! server surfaces; test names record the client trace that motivated each
//! query shape.

use std::sync::Arc;

use async_trait::async_trait;
use datafusion::common::ParamValues;
use datafusion::logical_expr::LogicalPlan;
use datafusion::prelude::SessionContext;
use datafusion::sql::sqlparser::ast::Statement;
use datafusion_postgres::hooks::{HookClient, QueryHook};
use datafusion_postgres::pgwire::api::ClientInfo;
use datafusion_postgres::pgwire::api::Type;
use datafusion_postgres::pgwire::api::results::{FieldFormat, FieldInfo, QueryResponse, Response};
use datafusion_postgres::pgwire::error::PgWireResult;

mod cursors;
mod encoding;
mod geotools;
mod params;
mod pg_attribute;
mod pg_class;
mod pg_index;
mod pg_type;
mod pgjdbc;
mod sql_parse;
mod surfaces;

use encoding::{
    current_portal_result_format, empty_response, single_bool_row, single_i64_row, single_text_row,
};
use sql_parse::count_positional_placeholders;
use surfaces::{
    CatalogSurface, classify_catalog_surface, is_ogr_pg_class_oid_lookup,
    is_pgjdbc_primary_keys_query, is_pgjdbc_typeinfo_name_query, is_pgjdbc_typeinfo_sqltype_query,
};

pub(crate) const SYNTHETIC_ROWID_COLUMN: &str = "_quackgis_rowid";

#[derive(Debug)]
pub struct CatalogCompatHook;

/// Backward-compatible alias for older call sites. The hook now contains
/// PostgreSQL/PostGIS catalog shims used by multiple clients, not only QGIS.
pub type QgisCatalogHook = CatalogCompatHook;

#[async_trait]
impl QueryHook for CatalogCompatHook {
    async fn handle_simple_query(
        &self,
        statement: &Statement,
        session_context: &SessionContext,
        _client: &mut dyn HookClient,
    ) -> Option<PgWireResult<Response>> {
        catalog_query_response(statement, session_context).await
    }

    async fn handle_extended_parse_query(
        &self,
        statement: &Statement,
        session_context: &SessionContext,
        _client: &(dyn ClientInfo + Send + Sync),
    ) -> Option<PgWireResult<LogicalPlan>> {
        let sql = statement.to_string();
        let sql_lower = sql.to_lowercase();
        let param_count = count_positional_placeholders(&sql);
        if sql.to_uppercase().contains("OGRPGLAYERREADER") {
            if let Some(plan) =
                cursors::postgres_driver_fetch_logical_plan(&sql, session_context).await
            {
                return Some(plan);
            }
            return Some(cursors::dummy_logical_plan(session_context).await);
        }
        if is_pgjdbc_typeinfo_sqltype_query(&sql_lower) {
            return Some(
                pg_type::pgjdbc_typeinfo_sqltype_logical_plan(session_context, param_count).await,
            );
        }
        if is_pgjdbc_typeinfo_name_query(&sql_lower) {
            return Some(
                pg_type::pgjdbc_typeinfo_name_logical_plan(session_context, param_count).await,
            );
        }
        if matches!(
            classify_catalog_surface(&sql_lower),
            Some(CatalogSurface::PgTypePostgisProbe)
        ) {
            return Some(pg_type::oid_typname_logical_plan(session_context, param_count).await);
        }
        if is_ogr_pg_class_oid_lookup(&sql_lower) {
            return Some(pg_class::ogr_oid_lookup_logical_plan(&sql, session_context).await);
        }
        if geotools::is_binary_geometry_query(&sql_lower) {
            return Some(
                geotools::binary_geometry_describe_plan(&sql, statement, session_context).await,
            );
        }
        if is_pgjdbc_primary_keys_query(&sql_lower) {
            return Some(
                pgjdbc::primary_keys_logical_plan(session_context, param_count.max(2)).await,
            );
        }
        None
    }

    async fn handle_extended_query(
        &self,
        statement: &Statement,
        _logical_plan: &LogicalPlan,
        params: &ParamValues,
        session_context: &SessionContext,
        _client: &mut dyn HookClient,
    ) -> Option<PgWireResult<Response>> {
        if geotools::is_binary_geometry_query(&statement.to_string().to_lowercase()) {
            return Some(
                geotools::st_asewkb_response(statement, params, session_context, _client).await,
            );
        }
        let sql = statement.to_string().to_lowercase();
        if is_ogr_pg_class_oid_lookup(&sql) {
            return Some(
                pg_class::oid_lookup_response(&sql, current_portal_result_format(_client))
                    .map(Response::Query),
            );
        }
        if let Some(response) =
            pgjdbc::primary_keys_extended_response(statement, params, session_context, _client)
                .await
        {
            return Some(response);
        }
        if let Some(response) =
            pgjdbc::columns_extended_response(statement, params, session_context, _client).await
        {
            return Some(response);
        }
        if let Some(response) = pg_type::extended_info_response(statement, params, _client) {
            return Some(response);
        }
        if let Some(response) = catalog_query_response(statement, session_context).await {
            return Some(response);
        }
        cursors::postgres_driver_cursor_response(statement, session_context).await
    }
}

async fn catalog_query_response(
    statement: &Statement,
    session_context: &SessionContext,
) -> Option<PgWireResult<Response>> {
    let sql = statement.to_string().to_lowercase();
    if is_instance_id_query(&sql) {
        return Some(single_text_row("quackgis_instance_id", &instance_id()).map(Response::Query));
    }
    if let Some(response) = pg_type::oid_in_response(&sql) {
        return Some(response.map(Response::Query));
    }

    let surface = classify_catalog_surface(&sql)?;
    match surface {
        CatalogSurface::PgTypePostgisProbe => {
            Some(pg_type::oid_typname_probe_response(&sql).map(Response::Query))
        }
        CatalogSurface::StyleTableExists => {
            Some(single_bool_row("exists", false).map(Response::Query))
        }
        CatalogSurface::PgJdbcTableListing => {
            Some(pgjdbc::table_listing_response(session_context).map(Response::Query))
        }
        CatalogSurface::PgJdbcPrimaryKeys => Some(
            pgjdbc::primary_keys_response(session_context, None, FieldFormat::Text)
                .await
                .map(Response::Query),
        ),
        CatalogSurface::PgJdbcColumns => Some(
            pgjdbc::columns_response(session_context, None, None, None, FieldFormat::Text)
                .await
                .map(Response::Query),
        ),
        CatalogSurface::PgClassTableListing => {
            Some(pg_class::table_listing_response(session_context).map(Response::Query))
        }
        CatalogSurface::PgClassOidLookup => {
            Some(pg_class::oid_lookup_response(&sql, FieldFormat::Text).map(Response::Query))
        }
        CatalogSurface::PgInheritsRelname => {
            Some(empty_response("relname", Type::VARCHAR).map(Response::Query))
        }
        CatalogSurface::PgInheritsCount => Some(single_i64_row("count", 0).map(Response::Query)),
        CatalogSurface::PgAttributeColumnListing => Some(
            pg_attribute::column_listing_response(&sql, session_context)
                .await
                .map(Response::Query),
        ),
        CatalogSurface::GeographyColumnsProbe => {
            Some(geography_columns_probe_response().map(Response::Query))
        }
        CatalogSurface::PgDescriptionRegclass => {
            Some(empty_response("description", Type::VARCHAR).map(Response::Query))
        }
        CatalogSurface::PgIndexPrimaryKeyProbe => {
            Some(pg_index::primary_key_probe_response().map(Response::Query))
        }
        CatalogSurface::PgIndexKeyColumn => {
            Some(pg_index::key_column_response(&sql).map(Response::Query))
        }
        CatalogSurface::PgIndexForTable => Some(
            pg_index::for_table_response(&sql, session_context)
                .await
                .map(Response::Query),
        ),
        CatalogSurface::PgIndexIndkey => Some(pg_index::indkey_response(&sql).map(Response::Query)),
        CatalogSurface::PgGetIndexdef => {
            Some(pg_index::get_indexdef_response(&sql).map(Response::Query))
        }
        CatalogSurface::PgClassRelkindRegclass => {
            Some(single_text_row("relkind", "r").map(Response::Query))
        }
        CatalogSurface::PgAttributeRegclassIdentity => {
            Some(empty_response("attidentity", Type::VARCHAR).map(Response::Query))
        }
        CatalogSurface::PgAttributeRegclassName => {
            Some(empty_response("attname", Type::VARCHAR).map(Response::Query))
        }
        CatalogSurface::PgAttributeGeomTypeName => {
            Some(single_text_row("typname", "geometry").map(Response::Query))
        }
    }
}

fn is_instance_id_query(sql: &str) -> bool {
    let compact = sql.split_whitespace().collect::<Vec<_>>().join(" ");
    matches!(
        compact.as_str(),
        "select quackgis_instance_id()" | "select quackgis_instance_id() as quackgis_instance_id"
    )
}

fn instance_id() -> String {
    std::env::var("QUACKGIS_INSTANCE_ID")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "unknown".to_string())
}

fn geography_columns_probe_response() -> PgWireResult<QueryResponse> {
    let fields = vec![
        FieldInfo::new(
            "type".to_string(),
            None,
            None,
            Type::VARCHAR,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "coord_dimension".to_string(),
            None,
            None,
            Type::INT4,
            FieldFormat::Text,
        ),
        FieldInfo::new(
            "srid".to_string(),
            None,
            None,
            Type::INT4,
            FieldFormat::Text,
        ),
    ];
    let row_stream = futures::stream::empty();
    Ok(QueryResponse::new(Arc::new(fields), Box::pin(row_stream)))
}
