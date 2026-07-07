// SPDX-License-Identifier: Apache-2.0
//! Catalog-surface query classification for client compatibility hooks.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CatalogSurface {
    PgTypePostgisProbe,
    StyleTableExists,
    PgJdbcTableListing,
    PgJdbcPrimaryKeys,
    PgJdbcColumns,
    PgClassTableListing,
    PgClassOidLookup,
    PgInheritsRelname,
    PgInheritsCount,
    PgAttributeColumnListing,
    GeographyColumnsProbe,
    PgDescriptionRegclass,
    PgIndexPrimaryKeyProbe,
    PgIndexKeyColumn,
    PgIndexForTable,
    PgIndexIndkey,
    PgGetIndexdef,
    PgClassRelkindRegclass,
    PgAttributeRegclassIdentity,
    PgAttributeRegclassName,
    PgAttributeGeomTypeName,
}

pub(super) fn classify_catalog_surface(sql: &str) -> Option<CatalogSurface> {
    let select_end = sql.find(" from ").unwrap_or(sql.len());
    let select_list = &sql[..select_end];
    if sql.contains("pg_type")
        && sql.contains("oid")
        && sql.contains("typname")
        && sql.contains("typtype")
    {
        return Some(CatalogSurface::PgTypePostgisProbe);
    }
    if (sql.contains("qgis_editor_widget_styles") || sql.contains("layer_styles"))
        && sql.contains("exists")
    {
        return Some(CatalogSurface::StyleTableExists);
    }
    if sql.contains("table_cat")
        && sql.contains("table_schem")
        && sql.contains("table_name")
        && sql.contains("table_type")
        && sql.contains("pg_class")
        && sql.contains("pg_namespace")
    {
        return Some(CatalogSurface::PgJdbcTableListing);
    }
    if is_pgjdbc_primary_keys_query(sql) {
        return Some(CatalogSurface::PgJdbcPrimaryKeys);
    }
    if is_pgjdbc_columns_query(sql) {
        return Some(CatalogSurface::PgJdbcColumns);
    }
    if sql.contains("pg_class")
        && sql.contains("pg_namespace")
        && sql.contains("pg_description")
        && sql.contains("d.classoid")
        && sql.contains("d.objsubid")
        && sql.contains("relkind")
        && select_list.contains("c.relname")
        && select_list.contains("n.nspname")
    {
        return Some(CatalogSurface::PgClassTableListing);
    }
    if is_ogr_pg_class_oid_lookup(sql) {
        return Some(CatalogSurface::PgClassOidLookup);
    }
    if sql.contains("pg_inherits") && sql.contains("inhparent") && sql.contains("relname") {
        return Some(CatalogSurface::PgInheritsRelname);
    }
    if sql.contains("pg_inherits") && sql.contains("inhparent") {
        return Some(CatalogSurface::PgInheritsCount);
    }
    if sql.contains("pg_attribute")
        && sql.contains("pg_type")
        && sql.contains("format_type")
        && sql.contains("pg_attrdef")
        && sql.contains("pg_index")
        && sql.contains("pg_description")
        && sql.contains("attnotnull")
        && sql.contains("indisunique")
    {
        return Some(CatalogSurface::PgAttributeColumnListing);
    }
    if sql.contains("from geography_columns")
        && sql.contains("type")
        && sql.contains("coord_dimension")
        && sql.contains("srid")
    {
        return Some(CatalogSurface::GeographyColumnsProbe);
    }
    if sql.contains("pg_description")
        && sql.contains("description")
        && (sql.contains("classoid") || sql.contains("pg_class"))
        && sql.contains("objsubid")
    {
        return Some(CatalogSurface::PgDescriptionRegclass);
    }
    if sql.contains("pg_attribute")
        && sql.contains("pg_type")
        && sql.contains("pg_index")
        && sql.contains("attnum")
        && sql.contains("typname")
        && sql.contains("isfid")
        && sql.contains("indisprimary")
    {
        return Some(CatalogSurface::PgIndexPrimaryKeyProbe);
    }
    if sql.contains("pg_index")
        && sql.contains("pg_attribute")
        && sql.contains("attname")
        && sql.contains("attnotnull")
        && sql.contains("indexrelid")
    {
        return Some(CatalogSurface::PgIndexKeyColumn);
    }
    if sql.contains("pg_index") && sql.contains("indrelid") {
        return Some(CatalogSurface::PgIndexForTable);
    }
    if sql.contains("pg_index") && sql.contains("indkey") {
        return Some(CatalogSurface::PgIndexIndkey);
    }
    if sql.contains("pg_get_indexdef") {
        return Some(CatalogSurface::PgGetIndexdef);
    }
    if sql.contains("relkind") && sql.contains("pg_class") && sql.contains("regclass") {
        return Some(CatalogSurface::PgClassRelkindRegclass);
    }
    if sql.contains("pg_attribute") && sql.contains("regclass") && sql.contains("attidentity") {
        return Some(CatalogSurface::PgAttributeRegclassIdentity);
    }
    if sql.contains("pg_attribute") && sql.contains("regclass") && sql.contains("attname") {
        return Some(CatalogSurface::PgAttributeRegclassName);
    }
    if sql.contains("pg_attribute")
        && sql.contains("pg_type")
        && sql.contains("t.typname")
        && sql.contains("a.attname = 'geom'")
    {
        return Some(CatalogSurface::PgAttributeGeomTypeName);
    }
    None
}

pub(super) fn is_pgjdbc_typeinfo_sqltype_query(sql: &str) -> bool {
    sql.contains("typinput")
        && sql.contains("array_in")
        && sql.contains("typtype")
        && sql.contains("typname")
        && sql.contains("pg_type.oid")
        && sql.contains("array_upper(current_schemas")
}

pub(super) fn is_pgjdbc_typeinfo_name_query(sql: &str) -> bool {
    sql.contains("current_schemas")
        && sql.contains("nspname")
        && sql.contains("t.typname")
        && sql.contains("pg_catalog.pg_type")
        && sql.contains("pg_catalog.pg_namespace")
}

pub(super) fn is_pgjdbc_primary_keys_query(sql: &str) -> bool {
    sql.contains("_pg_expandarray")
        && sql.contains("key_seq")
        && sql.contains("pk_name")
        && sql.contains("column_name")
        && sql.contains("pg_index")
}

pub(super) fn is_pgjdbc_columns_query(sql: &str) -> bool {
    sql.contains("pg_attribute")
        && sql.contains("pg_type")
        && sql.contains("atttypid")
        && sql.contains("attidentity")
        && sql.contains("attgenerated")
        && sql.contains("typbasetype")
        && sql.contains("typtype")
}

pub(super) fn is_ogr_pg_class_oid_lookup(sql: &str) -> bool {
    let select_end = sql.find(" from ").unwrap_or(sql.len());
    let select_list = &sql[..select_end];
    sql.contains("pg_class")
        && sql.contains("c.oid")
        && sql.contains("c.relname")
        && select_list.contains("c.oid")
        && sql.contains("relname")
        && (sql.contains("relname ~")
            || sql.contains("relname op")
            || sql.contains("relname =")
            || sql.contains("relname="))
}
