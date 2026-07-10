// SPDX-License-Identifier: Apache-2.0
//! Catalog-surface query classification for client compatibility hooks.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CatalogSurface {
    OgrSystemMetadataTableExists,
    OgrSystemMetadataRead,
    OgrSystemMetadataPrivilegeProbe,
    OgrSystemMetadataSuperuserProbe,
    OgrSystemMetadataEventTriggerProbe,
    OgrSystemMetadataNoopCommand,
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
    PgDescriptionRegclass,
    PgIndexPrimaryKeyProbe,
    PgIndexKeyColumn,
    PgIndexForTable,
    PgIndexIndkey,
    PgGetIndexdef,
    PgClassRelkindRegclass,
    PgAttributeRegclassIdentity,
    PgAttributeRegclassName,
}

pub(super) fn classify_catalog_surface(sql: &str) -> Option<CatalogSurface> {
    let select_end = sql.find(" from ").unwrap_or(sql.len());
    let select_list = &sql[..select_end];
    let sql_no_ws = sql.split_whitespace().collect::<String>();
    let select_list_no_ws = select_list.split_whitespace().collect::<String>();
    if is_ogr_system_metadata_table_exists(&sql_no_ws, &select_list_no_ws) {
        return Some(CatalogSurface::OgrSystemMetadataTableExists);
    }
    if is_ogr_system_metadata_read(&sql_no_ws, &select_list_no_ws) {
        return Some(CatalogSurface::OgrSystemMetadataRead);
    }
    if is_ogr_system_metadata_privilege_probe(&sql_no_ws) {
        return Some(CatalogSurface::OgrSystemMetadataPrivilegeProbe);
    }
    if sql_no_ws.contains("pg_user") && sql_no_ws.contains("usesuper") {
        return Some(CatalogSurface::OgrSystemMetadataSuperuserProbe);
    }
    if sql_no_ws.contains("pg_event_trigger")
        && sql_no_ws.contains("ogr_system_tables_event_trigger_for_metadata")
    {
        return Some(CatalogSurface::OgrSystemMetadataEventTriggerProbe);
    }
    if is_ogr_system_metadata_noop_command(&sql_no_ws) {
        return Some(CatalogSurface::OgrSystemMetadataNoopCommand);
    }
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

fn is_ogr_system_metadata_table_exists(sql: &str, select_list: &str) -> bool {
    sql.contains("pg_class")
        && sql.contains("pg_namespace")
        && select_list.contains("c.oid")
        && sql.contains("relname='metadata'")
        && sql.contains("nspname='ogr_system_tables'")
}

fn is_ogr_system_metadata_read(sql: &str, select_list: &str) -> bool {
    (sql.contains("fromogr_system_tables.metadata")
        || sql.contains("from\"ogr_system_tables\".\"metadata\""))
        && select_list.contains("metadata")
}

fn is_ogr_system_metadata_privilege_probe(sql: &str) -> bool {
    (sql.contains("has_database_privilege")
        || sql.contains("has_schema_privilege")
        || sql.contains("has_table_privilege"))
        && sql.contains("ogr_system_tables")
}

fn is_ogr_system_metadata_noop_command(sql: &str) -> bool {
    (sql.starts_with("createschema") && sql.contains("ogr_system_tables"))
        || (sql.starts_with("createtable") && sql.contains("ogr_system_tables.metadata"))
        || (sql.starts_with("deletefrom") && sql.contains("ogr_system_tables.metadata"))
        || (sql.starts_with("insertinto") && sql.contains("ogr_system_tables.metadata"))
        || (sql.starts_with("dropfunction") && sql.contains("ogr_system_tables"))
        || (sql.starts_with("createfunction") && sql.contains("ogr_system_tables"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_surface(sql: &str, expected: CatalogSurface) {
        assert_eq!(
            classify_catalog_surface(&sql.to_ascii_lowercase()),
            Some(expected),
            "unexpected surface for {sql}"
        );
    }

    #[test]
    fn classifies_pgjdbc_metadata_surfaces() {
        assert_surface(
            "SELECT current_database() AS TABLE_CAT, n.nspname AS TABLE_SCHEM, \
                    c.relname AS TABLE_NAME, 'TABLE' AS TABLE_TYPE \
             FROM pg_catalog.pg_namespace n, pg_catalog.pg_class c \
             WHERE c.relnamespace = n.oid",
            CatalogSurface::PgJdbcTableListing,
        );
        assert_surface(
            "SELECT result.TABLE_CAT, result.TABLE_SCHEM, result.TABLE_NAME, \
                    result.COLUMN_NAME, result.KEY_SEQ, result.PK_NAME \
             FROM (SELECT (information_schema._pg_expandarray(i.indkey)).n AS KEY_SEQ, \
                          a.attname AS COLUMN_NAME, ci.relname AS PK_NAME \
                   FROM pg_catalog.pg_index i JOIN pg_catalog.pg_attribute a ON true) result",
            CatalogSurface::PgJdbcPrimaryKeys,
        );
        assert_surface(
            "SELECT a.attname, a.atttypid, nullif(a.attidentity, '') AS attidentity, \
                    nullif(a.attgenerated, '') AS attgenerated, t.typbasetype, t.typtype \
             FROM pg_catalog.pg_attribute a JOIN pg_catalog.pg_type t ON a.atttypid = t.oid",
            CatalogSurface::PgJdbcColumns,
        );
    }

    #[test]
    fn classifies_index_and_type_surfaces() {
        assert_surface(
            "SELECT oid, typname, typtype, typelem FROM pg_type WHERE oid IN (90001)",
            CatalogSurface::PgTypePostgisProbe,
        );
        assert_surface(
            "SELECT indexrelid FROM pg_index WHERE indrelid='\"public\".\"points\"'::regclass",
            CatalogSurface::PgIndexForTable,
        );
        assert_surface(
            "SELECT indkey FROM pg_index WHERE indexrelid=90101",
            CatalogSurface::PgIndexIndkey,
        );
        assert_surface(
            "SELECT pg_get_indexdef(90101)",
            CatalogSurface::PgGetIndexdef,
        );
        assert_surface(
            "SELECT relkind FROM pg_class WHERE oid = 'public.points'::regclass",
            CatalogSurface::PgClassRelkindRegclass,
        );
        assert_surface(
            "SELECT attidentity FROM pg_attribute WHERE attrelid = 'public.points'::regclass",
            CatalogSurface::PgAttributeRegclassIdentity,
        );
        assert_surface(
            "SELECT attname FROM pg_attribute WHERE attrelid = 'public.points'::regclass",
            CatalogSurface::PgAttributeRegclassName,
        );
        assert_eq!(
            classify_catalog_surface(
                "select t.typname from pg_attribute a join pg_type t on a.atttypid = t.oid \
                 where a.attname = 'geom'"
            ),
            None,
            "a column name alone must not force a geometry type"
        );
    }

    #[test]
    fn classifies_ogr_system_metadata_surfaces() {
        assert_surface(
            "SELECT has_schema_privilege('ogr_system_tables', 'USAGE')",
            CatalogSurface::OgrSystemMetadataPrivilegeProbe,
        );
        assert_surface(
            "SELECT c.oid FROM pg_class c JOIN pg_namespace n ON c.relnamespace = n.oid \
             WHERE c.relname='metadata' AND n.nspname='ogr_system_tables'",
            CatalogSurface::OgrSystemMetadataTableExists,
        );
        assert_surface(
            "SELECT metadata FROM \"ogr_system_tables\".\"metadata\"",
            CatalogSurface::OgrSystemMetadataRead,
        );
    }
}
