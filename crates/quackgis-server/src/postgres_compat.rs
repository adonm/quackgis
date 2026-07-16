// SPDX-License-Identifier: Apache-2.0
//! PostgreSQL-facing catalog constants projected into the private DuckDB control catalog.

use arrow_pg::datatypes::{GEOGRAPHY_OID, GEOMETRY_OID};

pub const POSTGRESQL_COMPATIBILITY_VERSION: &str = "18.4";
pub const POSTGRESQL_COMPATIBILITY_VERSION_NUM: &str = "180004";
pub const POSTGRESQL_COMPATIBILITY_VERSION_STRING: &str = concat!(
    "PostgreSQL 18.4 compatible QuackGIS ",
    env!("CARGO_PKG_VERSION"),
    " (DuckDB 1.5.4)"
);
pub const PG_CATALOG_NAMESPACE_OID: u32 = 11;
pub const PUBLIC_NAMESPACE_OID: u32 = 2_200;
pub const BOOTSTRAP_OWNER_OID: u32 = 10;
pub const QUACKGIS_DATABASE_OID: u32 = 16_384;
pub const PG_CLASS_RELATION_OID: u32 = 1_259;
pub const GEOMETRY_ARRAY_OID: u32 = 90_003;
pub const GEOGRAPHY_ARRAY_OID: u32 = 90_004;
pub const POSTGIS_LIB_VERSION_PROC_OID: u32 = 90_005;
pub const POSTGIS_VERSION_PROC_OID: u32 = 90_006;
pub const POSTGIS_GEOS_VERSION_PROC_OID: u32 = 90_007;
pub const POSTGIS_PROJ_VERSION_PROC_OID: u32 = 90_008;
pub const DYNAMIC_OBJECT_OID_START: u32 = 100_000;
pub const INTERNAL_SCHEMA: &str = "_quackgis";

const PG_PROC_ROWS: [(u32, &str); 4] = [
    (POSTGIS_LIB_VERSION_PROC_OID, "postgis_lib_version"),
    (POSTGIS_VERSION_PROC_OID, "postgis_version"),
    (POSTGIS_GEOS_VERSION_PROC_OID, "postgis_geos_version"),
    (POSTGIS_PROJ_VERSION_PROC_OID, "postgis_proj_version"),
];

#[derive(Clone, Copy)]
struct PgTypeRow {
    oid: u32,
    name: &'static str,
    namespace: u32,
    len: i16,
    by_value: bool,
    category: char,
    preferred: bool,
    delimiter: char,
    element: u32,
    array: u32,
    collation: u32,
}

const fn builtin_scalar(
    oid: u32,
    name: &'static str,
    len: i16,
    category: char,
    array: u32,
) -> PgTypeRow {
    PgTypeRow {
        oid,
        name,
        namespace: PG_CATALOG_NAMESPACE_OID,
        len,
        by_value: false,
        category,
        preferred: false,
        delimiter: ',',
        element: 0,
        array,
        collation: 0,
    }
}

impl PgTypeRow {
    const fn by_value(mut self) -> Self {
        self.by_value = true;
        self
    }

    const fn preferred(mut self) -> Self {
        self.preferred = true;
        self
    }

    const fn element(mut self, element: u32) -> Self {
        self.element = element;
        self
    }

    const fn collation(mut self, collation: u32) -> Self {
        self.collation = collation;
        self
    }
}

const fn builtin_array(oid: u32, name: &'static str, element: u32, collation: u32) -> PgTypeRow {
    builtin_scalar(oid, name, -1, 'A', 0)
        .element(element)
        .collation(collation)
}

const fn spatial_type(oid: u32, name: &'static str, array: u32) -> PgTypeRow {
    PgTypeRow {
        oid,
        name,
        namespace: PUBLIC_NAMESPACE_OID,
        len: -1,
        by_value: false,
        category: 'U',
        preferred: false,
        delimiter: ':',
        element: 0,
        array,
        collation: 0,
    }
}

const fn spatial_array(oid: u32, name: &'static str, element: u32) -> PgTypeRow {
    PgTypeRow {
        oid,
        name,
        namespace: PUBLIC_NAMESPACE_OID,
        len: -1,
        by_value: false,
        category: 'A',
        preferred: false,
        delimiter: ':',
        element,
        array: 0,
        collation: 0,
    }
}

// PostgreSQL 18.4 oracle rows required by pg18-column-core-v1 and the captured
// QGIS 3.44 field-type query. Array partners are included so every published
// nonzero type reference resolves inside the maintained projection.
const TYPE_ROWS: &[PgTypeRow] = &[
    builtin_scalar(16, "bool", 1, 'B', 1000)
        .by_value()
        .preferred(),
    builtin_scalar(18, "char", 1, 'Z', 1002).by_value(),
    builtin_scalar(19, "name", 64, 'S', 1003)
        .element(18)
        .collation(950),
    builtin_scalar(20, "int8", 8, 'N', 1016).by_value(),
    builtin_scalar(21, "int2", 2, 'N', 1005).by_value(),
    builtin_scalar(23, "int4", 4, 'N', 1007).by_value(),
    builtin_scalar(25, "text", -1, 'S', 1009)
        .preferred()
        .collation(100),
    builtin_scalar(26, "oid", 4, 'N', 1028)
        .by_value()
        .preferred(),
    builtin_scalar(27, "tid", 6, 'U', 1010),
    builtin_scalar(28, "xid", 4, 'U', 1011).by_value(),
    builtin_scalar(29, "cid", 4, 'U', 1012).by_value(),
    builtin_array(1000, "_bool", 16, 0),
    builtin_array(1002, "_char", 18, 0),
    builtin_array(1003, "_name", 19, 950),
    builtin_array(1005, "_int2", 21, 0),
    builtin_array(1007, "_int4", 23, 0),
    builtin_array(1009, "_text", 25, 100),
    builtin_array(1010, "_tid", 27, 0),
    builtin_array(1011, "_xid", 28, 0),
    builtin_array(1012, "_cid", 29, 0),
    builtin_array(1015, "_varchar", 1043, 100),
    builtin_array(1016, "_int8", 20, 0),
    builtin_array(1028, "_oid", 26, 0),
    builtin_scalar(1043, "varchar", -1, 'S', 1015).collation(100),
    spatial_type(GEOMETRY_OID, "geometry", GEOMETRY_ARRAY_OID),
    spatial_type(GEOGRAPHY_OID, "geography", GEOGRAPHY_ARRAY_OID),
    spatial_array(GEOMETRY_ARRAY_OID, "_geometry", GEOMETRY_OID),
    spatial_array(GEOGRAPHY_ARRAY_OID, "_geography", GEOGRAPHY_OID),
];

// Scalar and array rows needed only once user-column catalogs are enabled.
// Keeping these out of the baseline projection preserves the frozen
// pg18-column-core-v1 oracle until the upstream DuckLake identity API ships in
// an official bundle.
const IDENTITY_TYPE_ROWS: &[PgTypeRow] = &[
    builtin_scalar(17, "bytea", -1, 'U', 1001),
    builtin_scalar(22, "int2vector", -1, 'A', 1006).element(21),
    builtin_scalar(194, "pg_node_tree", -1, 'S', 0).collation(100),
    builtin_array(1001, "_bytea", 17, 0),
    builtin_array(1006, "_int2vector", 22, 0),
    builtin_scalar(700, "float4", 4, 'N', 1021).by_value(),
    builtin_scalar(701, "float8", 8, 'N', 1022)
        .by_value()
        .preferred(),
    builtin_array(1021, "_float4", 700, 0),
    builtin_array(1022, "_float8", 701, 0),
    builtin_scalar(1082, "date", 4, 'D', 1182).by_value(),
    builtin_scalar(1083, "time", 8, 'D', 1183).by_value(),
    builtin_array(1115, "_timestamp", 1114, 0),
    builtin_scalar(1114, "timestamp", 8, 'D', 1115).by_value(),
    builtin_array(1182, "_date", 1082, 0),
    builtin_array(1183, "_time", 1083, 0),
    builtin_array(1185, "_timestamptz", 1184, 0),
    builtin_scalar(1184, "timestamptz", 8, 'D', 1185)
        .by_value()
        .preferred(),
    builtin_scalar(1186, "interval", 16, 'T', 1187).preferred(),
    builtin_array(1187, "_interval", 1186, 0),
    builtin_array(1231, "_numeric", 1700, 0),
    builtin_scalar(1700, "numeric", -1, 'N', 1231),
    builtin_scalar(3802, "jsonb", -1, 'U', 3807),
    builtin_array(3807, "_jsonb", 3802, 0),
    builtin_scalar(2205, "regclass", 4, 'N', 2210).by_value(),
    builtin_array(2210, "_regclass", 2205, 0),
    builtin_scalar(2206, "regtype", 4, 'N', 2211).by_value(),
    builtin_array(2211, "_regtype", 2206, 0),
    builtin_scalar(4089, "regnamespace", 4, 'N', 4090).by_value(),
    builtin_array(4090, "_regnamespace", 4089, 0),
    builtin_scalar(4096, "regrole", 4, 'N', 4097).by_value(),
    builtin_array(4097, "_regrole", 4096, 0),
];

fn render_type_rows<'a>(rows: impl IntoIterator<Item = &'a PgTypeRow>) -> String {
    rows.into_iter()
        .map(|row| {
            format!(
                "({}::UINTEGER, '{}'::VARCHAR, {}::UINTEGER, {}::SMALLINT, {}, \
                 'b'::VARCHAR, '{}'::VARCHAR, {}, true, '{}'::VARCHAR, 0::UINTEGER, \
                 {}::UINTEGER, {}::UINTEGER, false, 0::UINTEGER, -1::INTEGER, \
                 0::INTEGER, {}::UINTEGER)",
                row.oid,
                row.name,
                row.namespace,
                row.len,
                row.by_value,
                row.category,
                row.preferred,
                row.delimiter,
                row.element,
                row.array,
                row.collation,
            )
        })
        .collect::<Vec<_>>()
        .join(",\n")
}

fn type_rows_sql() -> String {
    render_type_rows(TYPE_ROWS)
}

fn identity_type_rows_sql() -> String {
    render_type_rows(TYPE_ROWS.iter().chain(IDENTITY_TYPE_ROWS))
}

fn pg_proc_rows_sql() -> String {
    PG_PROC_ROWS
        .iter()
        .map(|(oid, name)| {
            format!(
                "({oid}::UINTEGER, '{}'::VARCHAR, {PUBLIC_NAMESPACE_OID}::UINTEGER)",
                name
            )
        })
        .collect::<Vec<_>>()
        .join(",\n")
}

/// Create the first relational PostgreSQL compatibility catalog.
///
/// These views live only in DuckDB's process-local control database. Client SQL
/// reaches them through a structural `pg_catalog` rewrite; they are not user data
/// and are rebuilt at startup from constants. Wider user-object catalogs will be
/// derived from DuckDB/DuckLake metadata after the durable identity registry lands.
pub fn duckdb_catalog_bootstrap_sql() -> String {
    let type_rows = type_rows_sql();
    let pg_proc_rows = pg_proc_rows_sql();
    format!(
        "CREATE SCHEMA IF NOT EXISTS quackgis_pg_catalog;\n\
         CREATE OR REPLACE VIEW quackgis_pg_catalog.pg_namespace AS\n\
         SELECT * FROM (VALUES\n\
           ({PG_CATALOG_NAMESPACE_OID}::UINTEGER, 'pg_catalog'::VARCHAR, {BOOTSTRAP_OWNER_OID}::UINTEGER),\n\
           ({PUBLIC_NAMESPACE_OID}::UINTEGER, 'public'::VARCHAR, {BOOTSTRAP_OWNER_OID}::UINTEGER)\n\
         ) AS n(oid, nspname, nspowner);\n\
         CREATE OR REPLACE VIEW quackgis_pg_catalog.pg_database AS\n\
         SELECT {QUACKGIS_DATABASE_OID}::UINTEGER AS oid,\n\
                'quackgis'::VARCHAR AS datname,\n\
                {BOOTSTRAP_OWNER_OID}::UINTEGER AS datdba;\n\
         CREATE OR REPLACE VIEW quackgis_pg_catalog.pg_proc AS\n\
         SELECT * FROM (VALUES\n{pg_proc_rows}\n\
         ) AS p(oid, proname, pronamespace);\n\
         CREATE OR REPLACE VIEW quackgis_pg_catalog.pg_type AS\n\
         SELECT * FROM (VALUES\n{type_rows}\n\
         ) AS t(oid, typname, typnamespace, typlen, typbyval, typtype, typcategory,\n\
                typispreferred, typisdefined, typdelim, typrelid, typelem, typarray,\n\
                typnotnull, typbasetype, typtypmod, typndims, typcollation);\n\
         CREATE OR REPLACE VIEW quackgis_pg_catalog.pg_range AS\n\
         SELECT NULL::UINTEGER AS rngtypid, NULL::UINTEGER AS rngsubtype WHERE false;\n\
         CREATE OR REPLACE VIEW quackgis_pg_catalog.pg_collation AS\n\
         SELECT * FROM (VALUES\n\
           (100::UINTEGER, 'default'::VARCHAR, {PG_CATALOG_NAMESPACE_OID}::UINTEGER,\n\
            {BOOTSTRAP_OWNER_OID}::UINTEGER, 'd'::VARCHAR, true, -1::INTEGER,\n\
            NULL::VARCHAR, NULL::VARCHAR, NULL::VARCHAR, NULL::VARCHAR, NULL::VARCHAR),\n\
           (950::UINTEGER, 'C'::VARCHAR, {PG_CATALOG_NAMESPACE_OID}::UINTEGER,\n\
            {BOOTSTRAP_OWNER_OID}::UINTEGER, 'c'::VARCHAR, true, -1::INTEGER,\n\
            'C'::VARCHAR, 'C'::VARCHAR, NULL::VARCHAR, NULL::VARCHAR, NULL::VARCHAR)\n\
         ) AS c(oid, collname, collnamespace, collowner, collprovider,\n\
                collisdeterministic, collencoding, collcollate, collctype,\n\
                colllocale, collicurules, collversion);\n\
         CREATE OR REPLACE VIEW quackgis_pg_catalog.pg_roles AS\n\
         SELECT * FROM (VALUES\n\
           ({BOOTSTRAP_OWNER_OID}::UINTEGER, 'quackgis_owner'::VARCHAR, false, true, false, false,\n\
            false, false, -1::INTEGER, NULL::VARCHAR, NULL::TIMESTAMP WITH TIME ZONE,\n\
            false, NULL::VARCHAR[])\n\
         ) AS r(oid, rolname, rolsuper, rolinherit, rolcreaterole, rolcreatedb,\n\
                rolcanlogin, rolreplication, rolconnlimit, rolpassword, rolvaliduntil,\n\
                rolbypassrls, rolconfig);\n\
         CREATE OR REPLACE VIEW quackgis_pg_catalog.pg_auth_members AS\n\
         SELECT NULL::UINTEGER AS oid, NULL::UINTEGER AS roleid, NULL::UINTEGER AS member,\n\
                NULL::UINTEGER AS grantor, false AS admin_option, false AS inherit_option,\n\
                false AS set_option WHERE false;\n\
         CREATE OR REPLACE VIEW quackgis_pg_catalog.pg_table_owners AS\n\
         SELECT NULL::VARCHAR AS schema_name, NULL::VARCHAR AS table_name,\n\
                NULL::UINTEGER AS role_oid WHERE false;\n\
         CREATE OR REPLACE MACRO quackgis_current_database() AS 'quackgis';\n\
         CREATE OR REPLACE MACRO quackgis_current_schema() AS 'public';\n\
         CREATE OR REPLACE MACRO quackgis_current_schemas(include_implicit) AS\n\
           CASE WHEN CAST(include_implicit AS BOOLEAN)\n\
                THEN ['pg_catalog', 'public'] ELSE ['public'] END;\n\
         CREATE OR REPLACE MACRO quackgis_pg_is_in_recovery() AS false;\n\
         CREATE OR REPLACE MACRO quackgis_pg_version() AS {version_string};",
        version_string = quote_literal(POSTGRESQL_COMPATIBILITY_VERSION_STRING),
    )
}

/// Replace bootstrap role catalogs with one immutable configured role graph.
pub fn duckdb_role_catalog_sql(
    catalog: &crate::role::RoleCatalog,
    auth: &crate::auth::AuthConfig,
    catalog_identity_enabled: bool,
) -> String {
    let mut role_rows = vec![format!(
        "({BOOTSTRAP_OWNER_OID}::UINTEGER, 'quackgis_owner'::VARCHAR, false, true, false, false, \
         false, false, -1::INTEGER, NULL::VARCHAR, NULL::TIMESTAMP WITH TIME ZONE, \
         false, NULL::VARCHAR[])"
    )];
    role_rows.extend(catalog.roles().iter().map(|role| {
        format!(
            "({}::UINTEGER, {}::VARCHAR, false, {}, false, false, {}, false, -1::INTEGER, \
             NULL::VARCHAR, NULL::TIMESTAMP WITH TIME ZONE, false, NULL::VARCHAR[])",
            role.oid,
            quote_literal(&role.name),
            role.inherit,
            role.login,
        )
    }));
    let membership_sql = if catalog.memberships().is_empty() {
        "SELECT NULL::UINTEGER AS oid, NULL::UINTEGER AS roleid, NULL::UINTEGER AS member, \
         NULL::UINTEGER AS grantor, false AS admin_option, false AS inherit_option, \
         false AS set_option WHERE false"
            .to_owned()
    } else {
        let rows = catalog
            .memberships()
            .iter()
            .map(|membership| {
                let role = catalog
                    .role(&membership.role)
                    .expect("validated membership role");
                let member = catalog
                    .role(&membership.member)
                    .expect("validated membership member");
                format!(
                    "({}::UINTEGER, {}::UINTEGER, {}::UINTEGER, \
                     {BOOTSTRAP_OWNER_OID}::UINTEGER, {}, {}, {})",
                    membership.oid,
                    role.oid,
                    member.oid,
                    membership.admin_option,
                    membership.inherit_option,
                    membership.set_option,
                )
            })
            .collect::<Vec<_>>()
            .join(",\n");
        format!(
            "SELECT * FROM (VALUES\n{rows}\n) AS m(oid, roleid, member, grantor, \
             admin_option, inherit_option, set_option)"
        )
    };
    let owner_sql = if catalog.table_owners().is_empty() {
        "SELECT NULL::VARCHAR AS schema_name, NULL::VARCHAR AS table_name, \
         NULL::UINTEGER AS role_oid WHERE false"
            .to_owned()
    } else {
        let rows = catalog
            .table_owners()
            .iter()
            .map(|owner| {
                let role = catalog
                    .role(&owner.role)
                    .expect("validated table owner role");
                format!(
                    "({}::VARCHAR, {}::VARCHAR, {}::UINTEGER)",
                    quote_literal(&owner.schema),
                    quote_literal(&owner.table),
                    role.oid,
                )
            })
            .collect::<Vec<_>>()
            .join(",\n");
        format!("SELECT * FROM (VALUES\n{rows}\n) AS o(schema_name, table_name, role_oid)")
    };
    let information_schema_sql =
        duckdb_role_information_schema_sql(catalog, auth, catalog_identity_enabled);
    let structural_catalog_sql = if catalog_identity_enabled {
        duckdb_role_structural_catalog_sql()
    } else {
        ""
    };
    format!(
        "CREATE OR REPLACE VIEW quackgis_pg_catalog.pg_roles AS\n\
         SELECT * FROM (VALUES\n{}\n\
         ) AS r(oid, rolname, rolsuper, rolinherit, rolcreaterole, rolcreatedb,\n\
                rolcanlogin, rolreplication, rolconnlimit, rolpassword, rolvaliduntil,\n\
                rolbypassrls, rolconfig);\n\
         CREATE OR REPLACE VIEW quackgis_pg_catalog.pg_auth_members AS\n\
         {membership_sql};\n\
         CREATE OR REPLACE VIEW quackgis_pg_catalog.pg_table_owners AS\n\
         {owner_sql};\n\
         {information_schema_sql}\n\
         {structural_catalog_sql}",
        role_rows.join(",\n"),
    )
}

fn duckdb_role_structural_catalog_sql() -> &'static str {
    r#"CREATE OR REPLACE MACRO quackgis_pg_visible_relations(effective_role, session_identity) AS TABLE
           SELECT DISTINCT columns.relation_oid
           FROM quackgis_pg_catalog._current_columns columns
           JOIN quackgis_pg_catalog.information_schema_table_visibility visibility
             ON visibility.schema_name = columns.schema_name
            AND lower(visibility.table_name) = lower(columns.table_name)
           JOIN quackgis_pg_catalog.information_schema_legacy_visibility legacy
             ON legacy.schema_name = visibility.schema_name
            AND lower(legacy.table_name) = lower(visibility.table_name)
           WHERE visibility.role_name = CAST(effective_role AS VARCHAR)
             AND legacy.session_user = CAST(session_identity AS VARCHAR)
             AND (visibility.can_select AND legacy.can_read
                  OR (visibility.can_insert OR visibility.can_update OR visibility.can_delete)
                     AND legacy.can_write);
         CREATE OR REPLACE MACRO quackgis_pg_attrdef_visible(effective_role, session_identity) AS TABLE
           SELECT defaults.* FROM quackgis_pg_catalog.pg_attrdef defaults
           JOIN quackgis_pg_visible_relations(effective_role, session_identity) visible
             ON visible.relation_oid = defaults.adrelid;
         CREATE OR REPLACE MACRO quackgis_pg_description_visible(effective_role, session_identity) AS TABLE
           SELECT descriptions.* FROM quackgis_pg_catalog.pg_description descriptions
           JOIN quackgis_pg_visible_relations(effective_role, session_identity) visible
             ON visible.relation_oid = descriptions.objoid;
         CREATE OR REPLACE MACRO quackgis_pg_constraint_visible(effective_role, session_identity) AS TABLE
           SELECT constraints.* FROM quackgis_pg_catalog.pg_constraint constraints
           JOIN quackgis_pg_visible_relations(effective_role, session_identity) visible
             ON visible.relation_oid = constraints.conrelid;
         CREATE OR REPLACE MACRO quackgis_pg_index_visible(effective_role, session_identity) AS TABLE
           SELECT indexes.* FROM quackgis_pg_catalog.pg_index indexes
           JOIN quackgis_pg_visible_relations(effective_role, session_identity) visible
             ON visible.relation_oid = indexes.indrelid;
         CREATE OR REPLACE MACRO quackgis_pg_geometry_columns_visible(effective_role, session_identity) AS TABLE
           SELECT geometry_columns.f_table_catalog, geometry_columns.f_table_schema,
                  geometry_columns.f_table_name, geometry_columns.f_geometry_column,
                  geometry_columns.coord_dimension, geometry_columns.srid,
                  geometry_columns.type
           FROM quackgis_pg_catalog.geometry_columns geometry_columns
           JOIN quackgis_pg_visible_relations(effective_role, session_identity) visible
             ON visible.relation_oid = geometry_columns._qg_relation_oid;
         CREATE OR REPLACE MACRO quackgis_pg_col_description_visible(
           relation_value, column_value, effective_role, session_identity
         ) AS (SELECT description
               FROM quackgis_pg_description_visible(effective_role, session_identity)
               WHERE classoid = 1259::UINTEGER
                 AND objoid = try_cast(relation_value AS UINTEGER)
                 AND objsubid = try_cast(column_value AS INTEGER)
               LIMIT 1);
         CREATE OR REPLACE MACRO quackgis_pg_obj_description_visible(
           object_value, catalog_name := NULL, effective_role := NULL, session_identity := NULL
         ) AS CASE
           WHEN object_value IS NULL THEN NULL
           WHEN catalog_name IS NOT NULL AND lower(CAST(catalog_name AS VARCHAR)) <> 'pg_class'
             THEN error('PostgreSQL object description catalog is not maintained')
           ELSE (SELECT description
                 FROM quackgis_pg_description_visible(effective_role, session_identity)
                 WHERE classoid = 1259::UINTEGER
                   AND objoid = try_cast(object_value AS UINTEGER) AND objsubid = 0
                 LIMIT 1) END;
         CREATE OR REPLACE MACRO quackgis_pg_get_constraintdef_visible(
           constraint_value, pretty := false, effective_role := NULL, session_identity := NULL
         ) AS CASE
           WHEN constraint_value IS NULL THEN NULL
           WHEN pretty IS NOT NULL AND try_cast(pretty AS BOOLEAN) IS NULL
             THEN error('PostgreSQL pg_get_constraintdef pretty flag must be boolean')
           ELSE (SELECT 'NOT NULL ' || quackgis_pg_quote_identifier(attributes.attname)
                 FROM quackgis_pg_constraint_visible(effective_role, session_identity) constraints
                 JOIN quackgis_pg_catalog.pg_attribute attributes
                   ON attributes.attrelid = constraints.conrelid
                  AND attributes.attnum = constraints.conkey[1]
                 WHERE constraints.oid = try_cast(constraint_value AS UINTEGER)
                   AND constraints.contype = 'n'
                 LIMIT 1) END;
         CREATE OR REPLACE MACRO quackgis_pg_get_indexdef_visible(
           index_value, column_number := 0, pretty := false,
           effective_role := NULL, session_identity := NULL
         ) AS CASE
           WHEN index_value IS NULL THEN NULL
           WHEN try_cast(column_number AS INTEGER) IS NULL
             OR (pretty IS NOT NULL AND try_cast(pretty AS BOOLEAN) IS NULL)
             THEN error('PostgreSQL pg_get_indexdef arguments are invalid')
           ELSE (SELECT NULL::VARCHAR
                 FROM quackgis_pg_index_visible(effective_role, session_identity) indexes
                 WHERE indexes.indexrelid = try_cast(index_value AS UINTEGER)
                 LIMIT 1) END;"#
}

fn duckdb_role_information_schema_sql(
    catalog: &crate::role::RoleCatalog,
    auth: &crate::auth::AuthConfig,
    catalog_identity_enabled: bool,
) -> String {
    use crate::role::{RolePrivilege, SchemaPrivilege, TablePrivilege};
    use std::collections::BTreeSet;

    let schemas = catalog
        .schema_grants()
        .iter()
        .map(|grant| grant.schema.as_str())
        .collect::<BTreeSet<_>>();
    let schema_rows = catalog
        .roles()
        .iter()
        .flat_map(|role| {
            schemas
                .iter()
                .filter(move |schema| {
                    catalog.has_schema_privilege(&role.name, schema, SchemaPrivilege::Usage)
                })
                .map(move |schema| {
                    format!(
                        "({}::VARCHAR, {}::VARCHAR)",
                        quote_literal(&role.name),
                        quote_literal(schema),
                    )
                })
        })
        .collect::<Vec<_>>();
    let schema_visibility = values_or_empty(
        &schema_rows,
        "v(role_name, schema_name)",
        "SELECT NULL::VARCHAR AS role_name, NULL::VARCHAR AS schema_name WHERE false",
    );

    let tables = catalog
        .table_owners()
        .iter()
        .map(|owner| (owner.schema.as_str(), owner.table.as_str()))
        .chain(
            catalog
                .table_grants()
                .iter()
                .map(|grant| (grant.schema.as_str(), grant.table.as_str())),
        )
        .collect::<BTreeSet<_>>();
    let table_rows = catalog
        .roles()
        .iter()
        .flat_map(|role| {
            tables.iter().filter_map(move |(schema, table)| {
                let allowed = |privilege| {
                    catalog.allows_table_operation(&role.name, schema, table, privilege)
                };
                let select = allowed(TablePrivilege::Select);
                let insert = allowed(TablePrivilege::Insert);
                let update = allowed(TablePrivilege::Update);
                let delete = allowed(TablePrivilege::Delete);
                if select || insert || update || delete {
                    Some(format!(
                        "({}::VARCHAR, {}::VARCHAR, {}::VARCHAR, {}, {}, {}, {})",
                        quote_literal(&role.name),
                        quote_literal(schema),
                        quote_literal(table),
                        select,
                        insert,
                        update,
                        delete,
                    ))
                } else {
                    None
                }
            })
        })
        .collect::<Vec<_>>();
    let table_visibility = values_or_empty(
        &table_rows,
        "v(role_name, schema_name, table_name, can_select, can_insert, can_update, can_delete)",
        "SELECT NULL::VARCHAR AS role_name, NULL::VARCHAR AS schema_name, \
         NULL::VARCHAR AS table_name, false AS can_select, false AS can_insert, \
         false AS can_update, false AS can_delete WHERE false",
    );
    let legacy_rows = catalog
        .roles()
        .iter()
        .filter(|role| role.login)
        .flat_map(|role| {
            tables.iter().map(move |(schema, table)| {
                format!(
                    "({}::VARCHAR, {}::VARCHAR, {}::VARCHAR, {}, {})",
                    quote_literal(&role.name),
                    quote_literal(schema),
                    quote_literal(table),
                    auth.allows_read(Some(&role.name), (schema, table)),
                    auth.allows_write(Some(&role.name), Some((schema, table))),
                )
            })
        })
        .collect::<Vec<_>>();
    let legacy_visibility = values_or_empty(
        &legacy_rows,
        "v(session_user, schema_name, table_name, can_read, can_write)",
        "SELECT NULL::VARCHAR AS session_user, NULL::VARCHAR AS schema_name, \
         NULL::VARCHAR AS table_name, false AS can_read, false AS can_write WHERE false",
    );

    let mut grants = BTreeSet::new();
    for owner in catalog.table_owners() {
        for privilege in ["SELECT", "INSERT", "UPDATE", "DELETE"] {
            grants.insert((
                owner.schema.as_str(),
                owner.table.as_str(),
                owner.role.as_str(),
                owner.role.as_str(),
                privilege,
            ));
        }
    }
    for grant in catalog.table_grants() {
        let grantor = catalog
            .table_owners()
            .iter()
            .find(|owner| {
                owner.schema.eq_ignore_ascii_case(&grant.schema)
                    && owner.table.eq_ignore_ascii_case(&grant.table)
            })
            .map_or("quackgis_owner", |owner| owner.role.as_str());
        let grantee = grant.role.as_deref().unwrap_or("PUBLIC");
        for privilege in &grant.privileges {
            let privilege = match privilege {
                TablePrivilege::Select => "SELECT",
                TablePrivilege::Insert => "INSERT",
                TablePrivilege::Update => "UPDATE",
                TablePrivilege::Delete => "DELETE",
                TablePrivilege::Maintain => continue,
            };
            grants.insert((
                grant.schema.as_str(),
                grant.table.as_str(),
                grantor,
                grantee,
                privilege,
            ));
        }
    }
    let privilege_rows = catalog
        .roles()
        .iter()
        .flat_map(|observer| {
            grants
                .iter()
                .filter_map(move |(schema, table, grantor, grantee, privilege)| {
                    let grantee_enabled = *grantee == "PUBLIC"
                        || catalog.has_role_privilege(
                            &observer.name,
                            grantee,
                            RolePrivilege::Usage,
                        );
                    let grantor_enabled = catalog.role(grantor).is_some()
                        && catalog.has_role_privilege(
                            &observer.name,
                            grantor,
                            RolePrivilege::Usage,
                        );
                    if grantee_enabled || grantor_enabled {
                        Some(format!(
                            "({}::VARCHAR, {}::VARCHAR, {}::VARCHAR, {}::VARCHAR, \
                         {}::VARCHAR, {}::VARCHAR)",
                            quote_literal(&observer.name),
                            quote_literal(grantor),
                            quote_literal(grantee),
                            quote_literal(schema),
                            quote_literal(table),
                            quote_literal(privilege),
                        ))
                    } else {
                        None
                    }
                })
        })
        .collect::<Vec<_>>();
    let table_privileges = values_or_empty(
        &privilege_rows,
        "p(observer_role, grantor, grantee, schema_name, table_name, privilege_type)",
        "SELECT NULL::VARCHAR AS observer_role, NULL::VARCHAR AS grantor, \
         NULL::VARCHAR AS grantee, NULL::VARCHAR AS schema_name, \
         NULL::VARCHAR AS table_name, NULL::VARCHAR AS privilege_type WHERE false",
    );
    let data_type = duckdb_information_schema_data_type_sql();
    let udt_schema = duckdb_information_schema_udt_schema_sql();
    let udt_name = duckdb_information_schema_udt_name_sql();
    let spatial_table_rows = if catalog_identity_enabled {
        " UNION ALL\n\
           SELECT 'quackgis'::VARCHAR, 'public'::VARCHAR, metadata.table_name,\n\
                  'VIEW'::VARCHAR, NULL::VARCHAR, NULL::VARCHAR, NULL::VARCHAR,\n\
                  NULL::VARCHAR, NULL::VARCHAR, 'NO'::VARCHAR, 'NO'::VARCHAR,\n\
                  NULL::VARCHAR\n\
           FROM (VALUES ('geometry_columns'::VARCHAR), ('spatial_ref_sys'::VARCHAR))\n\
                metadata(table_name)"
    } else {
        ""
    };

    format!(
        "CREATE OR REPLACE VIEW quackgis_pg_catalog.information_schema_schema_visibility AS\n\
         {schema_visibility};\n\
         CREATE OR REPLACE VIEW quackgis_pg_catalog.information_schema_table_visibility AS\n\
         {table_visibility};\n\
         CREATE OR REPLACE VIEW quackgis_pg_catalog.information_schema_legacy_visibility AS\n\
         {legacy_visibility};\n\
         CREATE OR REPLACE VIEW quackgis_pg_catalog.information_schema_table_privileges AS\n\
         {table_privileges};\n\
         CREATE OR REPLACE MACRO quackgis_information_schema_schemata(effective_role, session_identity) AS TABLE\n\
           SELECT 'quackgis'::VARCHAR AS catalog_name,\n\
                  CASE WHEN s.schema_name = 'main' THEN 'public' ELSE s.schema_name END\n\
                    AS schema_name,\n\
                  'quackgis_owner'::VARCHAR AS schema_owner,\n\
                  NULL::VARCHAR AS default_character_set_catalog,\n\
                  NULL::VARCHAR AS default_character_set_schema,\n\
                  NULL::VARCHAR AS default_character_set_name,\n\
                  NULL::VARCHAR AS sql_path\n\
           FROM information_schema.schemata s\n\
           JOIN quackgis_pg_catalog.information_schema_schema_visibility v\n\
             ON v.schema_name = s.schema_name\n\
           WHERE v.role_name = CAST(effective_role AS VARCHAR)\n\
             AND s.catalog_name = 'quackgis';\n\
         CREATE OR REPLACE MACRO quackgis_information_schema_tables(effective_role, session_identity) AS TABLE\n\
           SELECT 'quackgis'::VARCHAR AS table_catalog,\n\
                  CASE WHEN t.table_schema = 'main' THEN 'public' ELSE t.table_schema END\n\
                    AS table_schema,\n\
                  t.table_name::VARCHAR AS table_name, t.table_type::VARCHAR AS table_type,\n\
                  NULL::VARCHAR AS self_referencing_column_name,\n\
                  NULL::VARCHAR AS reference_generation,\n\
                  NULL::VARCHAR AS user_defined_type_catalog,\n\
                  NULL::VARCHAR AS user_defined_type_schema,\n\
                  NULL::VARCHAR AS user_defined_type_name,\n\
                  t.is_insertable_into::VARCHAR AS is_insertable_into,\n\
                  t.is_typed::VARCHAR AS is_typed, NULL::VARCHAR AS commit_action\n\
           FROM information_schema.tables t\n\
           JOIN quackgis_pg_catalog.information_schema_table_visibility v\n\
             ON v.schema_name = t.table_schema AND lower(v.table_name) = lower(t.table_name)\n\
           JOIN quackgis_pg_catalog.information_schema_legacy_visibility legacy\n\
             ON legacy.schema_name = v.schema_name AND lower(legacy.table_name) = lower(v.table_name)\n\
           WHERE v.role_name = CAST(effective_role AS VARCHAR)\n\
             AND legacy.session_user = CAST(session_identity AS VARCHAR)\n\
             AND (v.can_select AND legacy.can_read\n\
                  OR (v.can_insert OR v.can_update OR v.can_delete) AND legacy.can_write)\n\
             AND t.table_catalog = 'quackgis'\n\
           {spatial_table_rows};\n\
         CREATE OR REPLACE MACRO quackgis_information_schema_columns(effective_role, session_identity) AS TABLE\n\
           SELECT 'quackgis'::VARCHAR AS table_catalog,\n\
                  CASE WHEN c.table_schema = 'main' THEN 'public' ELSE c.table_schema END\n\
                    AS table_schema,\n\
                  c.table_name::VARCHAR AS table_name, c.column_name::VARCHAR AS column_name,\n\
                  c.ordinal_position::INTEGER AS ordinal_position,\n\
                  nullif(c.column_default, 'NULL')::VARCHAR AS column_default,\n\
                  c.is_nullable::VARCHAR AS is_nullable, ({data_type})::VARCHAR AS data_type,\n\
                  ({udt_schema})::VARCHAR AS udt_schema, ({udt_name})::VARCHAR AS udt_name,\n\
                  CASE WHEN c.is_identity THEN 'YES' ELSE 'NO' END::VARCHAR AS is_identity,\n\
                  c.is_generated::VARCHAR AS is_generated\n\
           FROM information_schema.columns c\n\
           JOIN quackgis_pg_catalog.information_schema_table_visibility v\n\
             ON v.schema_name = c.table_schema AND lower(v.table_name) = lower(c.table_name)\n\
           JOIN quackgis_pg_catalog.information_schema_legacy_visibility legacy\n\
             ON legacy.schema_name = v.schema_name AND lower(legacy.table_name) = lower(v.table_name)\n\
           WHERE v.role_name = CAST(effective_role AS VARCHAR)\n\
             AND legacy.session_user = CAST(session_identity AS VARCHAR)\n\
             AND (v.can_select AND legacy.can_read\n\
                  OR (v.can_insert OR v.can_update OR v.can_delete) AND legacy.can_write)\n\
             AND c.table_catalog = 'quackgis';\n\
         CREATE OR REPLACE MACRO quackgis_information_schema_table_privileges(effective_role, session_identity) AS TABLE\n\
           SELECT p.grantor, p.grantee, 'quackgis'::VARCHAR AS table_catalog,\n\
                  CASE WHEN p.schema_name = 'main' THEN 'public' ELSE p.schema_name END\n\
                    AS table_schema,\n\
                  p.table_name, p.privilege_type, 'NO'::VARCHAR AS is_grantable,\n\
                  CASE WHEN p.privilege_type = 'SELECT' THEN 'YES' ELSE 'NO' END::VARCHAR\n\
                    AS with_hierarchy\n\
           FROM quackgis_pg_catalog.information_schema_table_privileges p\n\
           JOIN information_schema.tables t\n\
             ON t.table_catalog = 'quackgis' AND t.table_schema = p.schema_name\n\
            AND lower(t.table_name) = lower(p.table_name)\n\
           JOIN quackgis_pg_catalog.information_schema_legacy_visibility legacy\n\
             ON legacy.schema_name = p.schema_name AND lower(legacy.table_name) = lower(p.table_name)\n\
           WHERE p.observer_role = CAST(effective_role AS VARCHAR)\n\
             AND legacy.session_user = CAST(session_identity AS VARCHAR)\n\
             AND (p.privilege_type = 'SELECT' AND legacy.can_read\n\
                  OR p.privilege_type <> 'SELECT' AND legacy.can_write);\n\
         CREATE OR REPLACE MACRO quackgis_information_schema_role_table_grants(effective_role, session_identity) AS TABLE\n\
           SELECT * FROM quackgis_information_schema_table_privileges(effective_role, session_identity)\n\
           WHERE grantee <> 'PUBLIC';\n\
         CREATE OR REPLACE MACRO quackgis_information_schema_column_privileges(effective_role, session_identity) AS TABLE\n\
           SELECT p.grantor, p.grantee, p.table_catalog, p.table_schema, p.table_name,\n\
                  c.column_name::VARCHAR AS column_name, p.privilege_type, p.is_grantable\n\
           FROM quackgis_information_schema_table_privileges(effective_role, session_identity) p\n\
           JOIN information_schema.columns c\n\
             ON c.table_catalog = 'quackgis'\n\
            AND c.table_schema = CASE WHEN p.table_schema = 'public' THEN 'main' ELSE p.table_schema END\n\
            AND lower(c.table_name) = lower(p.table_name)\n\
           WHERE p.privilege_type IN ('SELECT', 'INSERT', 'UPDATE');\n\
         CREATE OR REPLACE MACRO quackgis_information_schema_role_column_grants(effective_role, session_identity) AS TABLE\n\
           SELECT * FROM quackgis_information_schema_column_privileges(effective_role, session_identity)\n\
           WHERE grantee <> 'PUBLIC';"
    )
}

fn values_or_empty(rows: &[String], columns: &str, empty: &str) -> String {
    if rows.is_empty() {
        empty.to_owned()
    } else {
        format!(
            "SELECT * FROM (VALUES\n{}\n) AS {columns}",
            rows.join(",\n")
        )
    }
}

fn duckdb_information_schema_data_type_sql() -> &'static str {
    "CASE
       WHEN c.data_type LIKE '%[]' THEN 'ARRAY'
       WHEN c.data_type = 'BOOLEAN' THEN 'boolean'
       WHEN c.data_type = 'TINYINT' THEN '\"char\"'
       WHEN c.data_type IN ('SMALLINT', 'UTINYINT') THEN 'smallint'
       WHEN c.data_type IN ('INTEGER', 'USMALLINT') THEN 'integer'
       WHEN c.data_type IN ('BIGINT', 'UINTEGER') THEN 'bigint'
       WHEN c.data_type IN ('HUGEINT', 'UHUGEINT', 'UBIGINT')
         OR (starts_with(c.data_type, 'DECIMAL(') AND NOT ends_with(c.data_type, '[]'))
         THEN 'numeric'
       WHEN c.data_type = 'FLOAT' THEN 'real'
       WHEN c.data_type = 'DOUBLE' THEN 'double precision'
       WHEN c.data_type = 'DATE' THEN 'date'
       WHEN c.data_type = 'TIME' THEN 'time without time zone'
       WHEN c.data_type IN ('TIMESTAMP', 'TIMESTAMP_S', 'TIMESTAMP_MS', 'TIMESTAMP_NS')
         THEN 'timestamp without time zone'
       WHEN c.data_type = 'TIMESTAMP WITH TIME ZONE' THEN 'timestamp with time zone'
       WHEN c.data_type = 'INTERVAL' THEN 'interval'
       WHEN c.data_type = 'JSON' AND lower(c.column_name) = 'properties' THEN 'jsonb'
       WHEN c.data_type IN ('VARCHAR', 'JSON') THEN 'text'
       WHEN c.data_type = 'GEOMETRY' THEN 'USER-DEFINED'
       WHEN c.data_type = 'BLOB' AND lower(c.column_name) IN
         ('geog', 'geography', 'the_geog', 'geom', 'geometry', 'the_geom',
          'wkb_geometry', 'wkb_geom', 'geom_wkb', 'shape', 'footprint', 'way')
         THEN 'USER-DEFINED'
       WHEN c.data_type = 'BLOB' THEN 'bytea'
       ELSE error('unsupported DuckLake column type in PostgreSQL information schema')
     END"
}

fn duckdb_information_schema_udt_schema_sql() -> &'static str {
    "CASE WHEN c.data_type = 'GEOMETRY' OR c.data_type = 'BLOB' AND lower(c.column_name) IN
       ('geog', 'geography', 'the_geog', 'geom', 'geometry', 'the_geom',
        'wkb_geometry', 'wkb_geom', 'geom_wkb', 'shape', 'footprint', 'way')
       THEN 'public' ELSE 'pg_catalog' END"
}

fn duckdb_information_schema_udt_name_sql() -> &'static str {
    "CASE
       WHEN c.data_type = 'BOOLEAN' THEN 'bool'
       WHEN c.data_type = 'TINYINT' THEN 'char'
       WHEN c.data_type IN ('SMALLINT', 'UTINYINT') THEN 'int2'
       WHEN c.data_type IN ('INTEGER', 'USMALLINT') THEN 'int4'
       WHEN c.data_type IN ('BIGINT', 'UINTEGER') THEN 'int8'
       WHEN c.data_type IN ('HUGEINT', 'UHUGEINT', 'UBIGINT')
         OR (starts_with(c.data_type, 'DECIMAL(') AND NOT ends_with(c.data_type, '[]'))
         THEN 'numeric'
       WHEN c.data_type = 'FLOAT' THEN 'float4'
       WHEN c.data_type = 'DOUBLE' THEN 'float8'
       WHEN c.data_type = 'DATE' THEN 'date'
       WHEN c.data_type = 'TIME' THEN 'time'
       WHEN c.data_type IN ('TIMESTAMP', 'TIMESTAMP_S', 'TIMESTAMP_MS', 'TIMESTAMP_NS')
         THEN 'timestamp'
       WHEN c.data_type = 'TIMESTAMP WITH TIME ZONE' THEN 'timestamptz'
       WHEN c.data_type = 'INTERVAL' THEN 'interval'
       WHEN c.data_type = 'JSON' AND lower(c.column_name) = 'properties' THEN 'jsonb'
       WHEN c.data_type IN ('VARCHAR', 'JSON') THEN 'text'
       WHEN c.data_type = 'GEOMETRY' THEN 'geometry'
       WHEN c.data_type = 'BLOB' AND lower(c.column_name) IN
         ('geog', 'geography', 'the_geog') THEN 'geography'
       WHEN c.data_type = 'BLOB' AND lower(c.column_name) IN
         ('geom', 'geometry', 'the_geom', 'wkb_geometry', 'wkb_geom',
          'geom_wkb', 'shape', 'footprint', 'way') THEN 'geometry'
       WHEN c.data_type = 'BLOB' THEN 'bytea'
       WHEN c.data_type = 'BOOLEAN[]' THEN '_bool'
       WHEN c.data_type IN ('TINYINT[]', 'SMALLINT[]', 'UTINYINT[]') THEN '_int2'
       WHEN c.data_type IN ('INTEGER[]', 'USMALLINT[]') THEN '_int4'
       WHEN c.data_type IN ('BIGINT[]', 'UINTEGER[]') THEN '_int8'
       WHEN c.data_type IN ('HUGEINT[]', 'UHUGEINT[]', 'UBIGINT[]')
         OR (starts_with(c.data_type, 'DECIMAL(') AND ends_with(c.data_type, '[]'))
         THEN '_numeric'
       WHEN c.data_type = 'FLOAT[]' THEN '_float4'
       WHEN c.data_type = 'DOUBLE[]' THEN '_float8'
       WHEN c.data_type = 'DATE[]' THEN '_date'
       WHEN c.data_type = 'TIME[]' THEN '_time'
       WHEN c.data_type IN ('TIMESTAMP[]', 'TIMESTAMP_S[]', 'TIMESTAMP_MS[]', 'TIMESTAMP_NS[]')
         THEN '_timestamp'
       WHEN c.data_type = 'TIMESTAMP WITH TIME ZONE[]' THEN '_timestamptz'
       WHEN c.data_type = 'INTERVAL[]' THEN '_interval'
       WHEN c.data_type IN ('VARCHAR[]', 'JSON[]') THEN '_text'
       WHEN c.data_type = 'BLOB[]' THEN '_bytea'
       ELSE error('unsupported DuckLake column type in PostgreSQL information schema')
     END"
}

fn duckdb_column_type_oid_sql() -> &'static str {
    "CASE
       WHEN data_type = 'GEOMETRY' THEN 90001
       WHEN data_type = 'BLOB' AND lower(column_name) IN
         ('geog', 'geography', 'the_geog') THEN 90002
       WHEN data_type = 'BLOB' AND lower(column_name) IN
         ('geom', 'geometry', 'the_geom', 'wkb_geometry', 'wkb_geom',
          'geom_wkb', 'shape', 'footprint', 'way') THEN 90001
       WHEN data_type IN ('VARCHAR', 'JSON') AND lower(column_name) = 'properties' THEN 3802
       WHEN data_type IN ('VARCHAR', 'JSON') THEN 25
       WHEN data_type = 'BOOLEAN' THEN 16
       WHEN data_type = 'TINYINT' THEN 18
       WHEN data_type IN ('SMALLINT', 'UTINYINT') THEN 21
       WHEN data_type IN ('INTEGER', 'USMALLINT') THEN 23
       WHEN data_type IN ('BIGINT', 'UINTEGER') THEN 20
       WHEN data_type IN ('HUGEINT', 'UHUGEINT', 'UBIGINT')
         OR (starts_with(data_type, 'DECIMAL(') AND NOT ends_with(data_type, '[]')) THEN 1700
       WHEN data_type = 'FLOAT' THEN 700
       WHEN data_type = 'DOUBLE' THEN 701
       WHEN data_type = 'DATE' THEN 1082
       WHEN data_type = 'TIME' THEN 1083
       WHEN data_type IN ('TIMESTAMP', 'TIMESTAMP_S', 'TIMESTAMP_MS', 'TIMESTAMP_NS') THEN 1114
       WHEN data_type = 'TIMESTAMP WITH TIME ZONE' THEN 1184
       WHEN data_type = 'INTERVAL' THEN 1186
       WHEN data_type = 'BLOB' THEN 17
       WHEN data_type = 'BOOLEAN[]' THEN 1000
       WHEN data_type IN ('TINYINT[]', 'SMALLINT[]', 'UTINYINT[]') THEN 1005
       WHEN data_type IN ('INTEGER[]', 'USMALLINT[]') THEN 1007
       WHEN data_type IN ('BIGINT[]', 'UINTEGER[]') THEN 1016
       WHEN data_type IN ('HUGEINT[]', 'UHUGEINT[]', 'UBIGINT[]')
         OR (starts_with(data_type, 'DECIMAL(') AND ends_with(data_type, '[]')) THEN 1231
       WHEN data_type = 'FLOAT[]' THEN 1021
       WHEN data_type = 'DOUBLE[]' THEN 1022
       WHEN data_type = 'DATE[]' THEN 1182
       WHEN data_type = 'TIME[]' THEN 1183
       WHEN data_type IN ('TIMESTAMP[]', 'TIMESTAMP_S[]', 'TIMESTAMP_MS[]', 'TIMESTAMP_NS[]') THEN 1115
       WHEN data_type = 'TIMESTAMP WITH TIME ZONE[]' THEN 1185
       WHEN data_type = 'INTERVAL[]' THEN 1187
       WHEN data_type IN ('VARCHAR[]', 'JSON[]') THEN 1009
       WHEN data_type = 'BLOB[]' THEN 1001
       ELSE error('unsupported DuckLake column type in PostgreSQL catalog projection')
     END"
}

fn identity_catalog_macros_sql() -> &'static str {
    r#"CREATE OR REPLACE MACRO quackgis_pg_name_first(value) AS
           regexp_extract(trim(CAST(value AS VARCHAR)),
             '^("(?:[^"]|"")*"|[A-Za-z_][A-Za-z0-9_$]*)(\s*\.\s*("(?:[^"]|"")*"|[A-Za-z_][A-Za-z0-9_$]*))?$', 1);
         CREATE OR REPLACE MACRO quackgis_pg_name_second(value) AS
           regexp_extract(trim(CAST(value AS VARCHAR)),
             '^("(?:[^"]|"")*"|[A-Za-z_][A-Za-z0-9_$]*)(\s*\.\s*("(?:[^"]|"")*"|[A-Za-z_][A-Za-z0-9_$]*))?$', 3);
         CREATE OR REPLACE MACRO quackgis_pg_normalize_identifier(part) AS
           CASE WHEN starts_with(part, '"')
                THEN replace(substr(part, 2, length(part) - 2), '""', '"')
                ELSE lower(part) END;
         CREATE OR REPLACE MACRO quackgis_pg_name_schema(value) AS
           CASE WHEN quackgis_pg_name_second(value) = '' THEN NULL
                ELSE quackgis_pg_normalize_identifier(quackgis_pg_name_first(value)) END;
         CREATE OR REPLACE MACRO quackgis_pg_name_object(value) AS
           CASE WHEN quackgis_pg_name_second(value) = ''
                THEN quackgis_pg_normalize_identifier(quackgis_pg_name_first(value))
                ELSE quackgis_pg_normalize_identifier(quackgis_pg_name_second(value)) END;
         CREATE OR REPLACE MACRO quackgis_pg_type_is_array(value) AS
           regexp_full_match(trim(CAST(value AS VARCHAR)), '.*\[\]\s*$');
         CREATE OR REPLACE MACRO quackgis_pg_type_clean(value) AS
           trim(regexp_replace(regexp_replace(trim(CAST(value AS VARCHAR)),
             '\[\]\s*$', ''), '\([^)]*\)\s*$', ''));
         CREATE OR REPLACE MACRO quackgis_pg_type_object(value) AS
           CASE lower(quackgis_pg_type_clean(value))
             WHEN 'boolean' THEN 'bool'
             WHEN 'smallint' THEN 'int2'
             WHEN 'integer' THEN 'int4'
             WHEN 'int' THEN 'int4'
             WHEN 'bigint' THEN 'int8'
             WHEN 'real' THEN 'float4'
             WHEN 'double precision' THEN 'float8'
             WHEN 'decimal' THEN 'numeric'
             WHEN 'character varying' THEN 'varchar'
             WHEN 'timestamp without time zone' THEN 'timestamp'
             WHEN 'timestamp with time zone' THEN 'timestamptz'
             WHEN 'time without time zone' THEN 'time'
             ELSE quackgis_pg_name_object(quackgis_pg_type_clean(value)) END;
         CREATE OR REPLACE MACRO quackgis_pg_to_regclass(value) AS
           CASE
             WHEN try_cast(value AS UINTEGER) = 1259 THEN 1259::UINTEGER
             WHEN try_cast(value AS UINTEGER) IS NULL
              AND quackgis_pg_name_object(value) = 'pg_class'
              AND coalesce(quackgis_pg_name_schema(value), 'pg_catalog') = 'pg_catalog'
               THEN 1259::UINTEGER
             ELSE (SELECT __quackgis_class.oid
                   FROM quackgis_pg_catalog.pg_class __quackgis_class
                   JOIN quackgis_pg_catalog.pg_namespace __quackgis_namespace
                     ON __quackgis_namespace.oid = __quackgis_class.relnamespace
                   WHERE (try_cast(value AS UINTEGER) IS NOT NULL
                          AND __quackgis_class.oid = try_cast(value AS UINTEGER))
                      OR (try_cast(value AS UINTEGER) IS NULL
                          AND __quackgis_class.relname = quackgis_pg_name_object(value)
                          AND __quackgis_namespace.nspname =
                              coalesce(quackgis_pg_name_schema(value), 'public'))
                   LIMIT 1) END;
         CREATE OR REPLACE MACRO quackgis_pg_regclass(value) AS
           CASE WHEN value IS NULL THEN NULL
                ELSE coalesce(quackgis_pg_to_regclass(value),
                     error('PostgreSQL relation does not exist')) END;
         CREATE OR REPLACE MACRO quackgis_pg_to_regtype(value) AS
           (SELECT CASE WHEN quackgis_pg_type_is_array(value)
                        THEN nullif(__quackgis_type.typarray, 0)
                        ELSE __quackgis_type.oid END
            FROM quackgis_pg_catalog.pg_type __quackgis_type
            JOIN quackgis_pg_catalog.pg_namespace __quackgis_namespace
              ON __quackgis_namespace.oid = __quackgis_type.typnamespace
            WHERE (try_cast(value AS UINTEGER) IS NOT NULL
                   AND __quackgis_type.oid = try_cast(value AS UINTEGER))
               OR (try_cast(value AS UINTEGER) IS NULL
                   AND __quackgis_type.typname = quackgis_pg_type_object(value)
                   AND (quackgis_pg_name_schema(quackgis_pg_type_clean(value)) IS NULL
                        AND __quackgis_namespace.nspname IN ('pg_catalog', 'public')
                     OR __quackgis_namespace.nspname =
                        quackgis_pg_name_schema(quackgis_pg_type_clean(value))))
            ORDER BY CASE __quackgis_namespace.nspname
              WHEN 'pg_catalog' THEN 0 WHEN 'public' THEN 1 ELSE 2 END
            LIMIT 1);
         CREATE OR REPLACE MACRO quackgis_pg_regtype(value) AS
           CASE WHEN value IS NULL THEN NULL
                ELSE coalesce(quackgis_pg_to_regtype(value),
                     error('PostgreSQL type does not exist')) END;
         CREATE OR REPLACE MACRO quackgis_pg_to_regnamespace(value) AS
           (SELECT __quackgis_namespace.oid
            FROM quackgis_pg_catalog.pg_namespace __quackgis_namespace
            WHERE (try_cast(value AS UINTEGER) IS NOT NULL
                   AND __quackgis_namespace.oid = try_cast(value AS UINTEGER))
               OR (try_cast(value AS UINTEGER) IS NULL
                   AND quackgis_pg_name_schema(value) IS NULL
                   AND __quackgis_namespace.nspname = quackgis_pg_name_object(value))
            LIMIT 1);
         CREATE OR REPLACE MACRO quackgis_pg_regnamespace(value) AS
           CASE WHEN value IS NULL THEN NULL
                ELSE coalesce(quackgis_pg_to_regnamespace(value),
                     error('PostgreSQL schema does not exist')) END;
         CREATE OR REPLACE MACRO quackgis_pg_to_regrole(value) AS
           (SELECT __quackgis_role.oid FROM quackgis_pg_catalog.pg_roles __quackgis_role
            WHERE (try_cast(value AS UINTEGER) IS NOT NULL
                   AND __quackgis_role.oid = try_cast(value AS UINTEGER))
               OR (try_cast(value AS UINTEGER) IS NULL
                   AND quackgis_pg_name_schema(value) IS NULL
                   AND __quackgis_role.rolname = quackgis_pg_name_object(value))
            LIMIT 1);
         CREATE OR REPLACE MACRO quackgis_pg_regrole(value) AS
           CASE WHEN value IS NULL THEN NULL
                ELSE coalesce(quackgis_pg_to_regrole(value),
                     error('PostgreSQL role does not exist')) END;
         CREATE OR REPLACE MACRO quackgis_pg_attribute_exists(relation_value, column_value) AS
           CASE WHEN relation_value IS NULL OR column_value IS NULL THEN NULL
                ELSE EXISTS (
                  SELECT 1 FROM quackgis_pg_catalog.pg_attribute __quackgis_attribute
                  WHERE __quackgis_attribute.attrelid = quackgis_pg_regclass(relation_value)
                    AND __quackgis_attribute.attnum > 0
                    AND NOT __quackgis_attribute.attisdropped
                    AND ((try_cast(column_value AS SMALLINT) IS NOT NULL
                          AND __quackgis_attribute.attnum = try_cast(column_value AS SMALLINT))
                      OR (try_cast(column_value AS SMALLINT) IS NULL
                          AND quackgis_pg_name_schema(column_value) IS NULL
                          AND __quackgis_attribute.attname =
                              quackgis_pg_name_object(column_value)))
                ) END;
         CREATE OR REPLACE MACRO quackgis_pg_quote_identifier(value) AS
           CASE WHEN regexp_full_match(value, '[a-z_][a-z0-9_$]*') THEN value
                ELSE '"' || replace(value, '"', '""') || '"' END;
         CREATE OR REPLACE MACRO quackgis_pg_regclass_text(value) AS
           CASE WHEN try_cast(value AS UINTEGER) = 1259 THEN 'pg_class'
           ELSE coalesce((SELECT CASE WHEN __quackgis_namespace.nspname = 'public'
                    THEN quackgis_pg_quote_identifier(__quackgis_class.relname)
                    ELSE quackgis_pg_quote_identifier(__quackgis_namespace.nspname) || '.' ||
                         quackgis_pg_quote_identifier(__quackgis_class.relname) END
             FROM quackgis_pg_catalog.pg_class __quackgis_class
             JOIN quackgis_pg_catalog.pg_namespace __quackgis_namespace
               ON __quackgis_namespace.oid = __quackgis_class.relnamespace
             WHERE __quackgis_class.oid = try_cast(value AS UINTEGER)),
             CAST(value AS VARCHAR)) END;
         CREATE OR REPLACE MACRO quackgis_pg_regnamespace_text(value) AS
           coalesce((SELECT quackgis_pg_quote_identifier(__quackgis_namespace.nspname)
             FROM quackgis_pg_catalog.pg_namespace __quackgis_namespace
             WHERE __quackgis_namespace.oid = try_cast(value AS UINTEGER)),
             CAST(value AS VARCHAR));
         CREATE OR REPLACE MACRO quackgis_pg_regrole_text(value) AS
           coalesce((SELECT quackgis_pg_quote_identifier(__quackgis_role.rolname)
             FROM quackgis_pg_catalog.pg_roles __quackgis_role
             WHERE __quackgis_role.oid = try_cast(value AS UINTEGER)),
             CAST(value AS VARCHAR));
         CREATE OR REPLACE MACRO quackgis_pg_base_type_display(
           type_oid, type_name, namespace_name, type_modifier
         ) AS CASE type_oid
           WHEN 16 THEN 'boolean' WHEN 17 THEN 'bytea' WHEN 18 THEN '"char"'
           WHEN 19 THEN 'name' WHEN 20 THEN 'bigint' WHEN 21 THEN 'smallint'
           WHEN 23 THEN 'integer' WHEN 25 THEN 'text' WHEN 26 THEN 'oid'
           WHEN 700 THEN 'real' WHEN 701 THEN 'double precision'
           WHEN 1043 THEN 'character varying' ||
             CASE WHEN type_modifier > 4 THEN '(' || CAST(type_modifier - 4 AS VARCHAR) || ')'
                  ELSE '' END
           WHEN 1082 THEN 'date'
           WHEN 1083 THEN 'time' ||
             CASE WHEN type_modifier >= 0 THEN '(' || CAST(type_modifier AS VARCHAR) || ')'
                  ELSE '' END || ' without time zone'
           WHEN 1114 THEN 'timestamp' ||
             CASE WHEN type_modifier >= 0 THEN '(' || CAST(type_modifier AS VARCHAR) || ')'
                  ELSE '' END || ' without time zone'
           WHEN 1184 THEN 'timestamp' ||
             CASE WHEN type_modifier >= 0 THEN '(' || CAST(type_modifier AS VARCHAR) || ')'
                  ELSE '' END || ' with time zone'
           WHEN 1186 THEN 'interval'
           WHEN 1700 THEN 'numeric' || CASE WHEN type_modifier >= 4
             THEN '(' || CAST(floor((type_modifier - 4) / 65536) AS BIGINT)::VARCHAR || ',' ||
                  CAST((type_modifier - 4) % 65536 AS BIGINT)::VARCHAR || ')' ELSE '' END
           WHEN 2205 THEN 'regclass' WHEN 2206 THEN 'regtype'
           WHEN 3802 THEN 'jsonb' WHEN 4089 THEN 'regnamespace'
           WHEN 4096 THEN 'regrole' WHEN 90001 THEN 'geometry'
           WHEN 90002 THEN 'geography'
           ELSE CASE WHEN namespace_name IN ('pg_catalog', 'public')
                     THEN quackgis_pg_quote_identifier(type_name)
                     ELSE quackgis_pg_quote_identifier(namespace_name) || '.' ||
                          quackgis_pg_quote_identifier(type_name) END END;
         CREATE OR REPLACE MACRO quackgis_pg_format_type(type_oid, type_modifier) AS
           CASE WHEN type_oid IS NULL THEN NULL ELSE coalesce((
             SELECT CASE WHEN __quackgis_type.typelem <> 0
                    THEN quackgis_pg_base_type_display(
                           __quackgis_element.oid, __quackgis_element.typname,
                           __quackgis_element_namespace.nspname, type_modifier) || '[]'
                    ELSE quackgis_pg_base_type_display(
                           __quackgis_type.oid, __quackgis_type.typname,
                           __quackgis_namespace.nspname, type_modifier) END
             FROM quackgis_pg_catalog.pg_type __quackgis_type
             JOIN quackgis_pg_catalog.pg_namespace __quackgis_namespace
               ON __quackgis_namespace.oid = __quackgis_type.typnamespace
             LEFT JOIN quackgis_pg_catalog.pg_type __quackgis_element
               ON __quackgis_element.oid = __quackgis_type.typelem
             LEFT JOIN quackgis_pg_catalog.pg_namespace __quackgis_element_namespace
               ON __quackgis_element_namespace.oid = __quackgis_element.typnamespace
             WHERE __quackgis_type.oid = CAST(type_oid AS UINTEGER)
           ), '???') END;
         CREATE OR REPLACE MACRO quackgis_pg_regtype_text(value) AS
           quackgis_pg_format_type(value, -1);
         CREATE OR REPLACE MACRO quackgis_pg_get_expr(
           expression_value, relation_value, pretty := false
         ) AS CASE
           WHEN expression_value IS NULL OR relation_value IS NULL THEN NULL
           WHEN pretty IS NOT NULL AND try_cast(pretty AS BOOLEAN) IS NULL
             THEN error('PostgreSQL pg_get_expr pretty flag must be boolean')
           WHEN quackgis_pg_regclass(relation_value) IS NOT NULL
             THEN CAST(expression_value AS VARCHAR)
           ELSE NULL END;
         CREATE OR REPLACE MACRO quackgis_pg_col_description(relation_value, column_value) AS
           (SELECT description FROM quackgis_pg_catalog.pg_description
            WHERE classoid = 1259::UINTEGER
              AND objoid = try_cast(relation_value AS UINTEGER)
              AND objsubid = try_cast(column_value AS INTEGER)
            LIMIT 1);
         CREATE OR REPLACE MACRO quackgis_pg_obj_description(
           object_value, catalog_name := NULL
         ) AS CASE
           WHEN object_value IS NULL THEN NULL
           WHEN catalog_name IS NOT NULL AND lower(CAST(catalog_name AS VARCHAR)) <> 'pg_class'
             THEN error('PostgreSQL object description catalog is not maintained')
           ELSE (SELECT description FROM quackgis_pg_catalog.pg_description
                 WHERE classoid = 1259::UINTEGER
                   AND objoid = try_cast(object_value AS UINTEGER) AND objsubid = 0
                 LIMIT 1) END;
         CREATE OR REPLACE MACRO quackgis_pg_get_constraintdef(
           constraint_value, pretty := false
         ) AS CASE
           WHEN constraint_value IS NULL THEN NULL
           WHEN pretty IS NOT NULL AND try_cast(pretty AS BOOLEAN) IS NULL
             THEN error('PostgreSQL pg_get_constraintdef pretty flag must be boolean')
           ELSE (SELECT 'NOT NULL ' || quackgis_pg_quote_identifier(attributes.attname)
                 FROM quackgis_pg_catalog.pg_constraint constraints
                 JOIN quackgis_pg_catalog.pg_attribute attributes
                   ON attributes.attrelid = constraints.conrelid
                  AND attributes.attnum = constraints.conkey[1]
                 WHERE constraints.oid = try_cast(constraint_value AS UINTEGER)
                   AND constraints.contype = 'n'
                 LIMIT 1) END;
         CREATE OR REPLACE MACRO quackgis_pg_get_indexdef(
           index_value, column_number := 0, pretty := false
         ) AS CASE
           WHEN index_value IS NULL THEN NULL
           WHEN try_cast(column_number AS INTEGER) IS NULL
             OR (pretty IS NOT NULL AND try_cast(pretty AS BOOLEAN) IS NULL)
             THEN error('PostgreSQL pg_get_indexdef arguments are invalid')
           ELSE (SELECT NULL::VARCHAR FROM quackgis_pg_catalog.pg_index indexes
                 WHERE indexes.indexrelid = try_cast(index_value AS UINTEGER)
                 LIMIT 1) END;"#
}

/// Replace the baseline catalog views with registry-backed user-object views.
///
/// This SQL is executed only after the supported pinned DuckLake
/// identity function and registry have both been validated. `_current_columns`
/// fails closed if a reader lands between the user commit and the separately
/// serialized registry reconciliation transaction.
pub fn duckdb_identity_catalog_bootstrap_sql(catalog: &str) -> String {
    let catalog_identifier = quote_identifier(catalog);
    let catalog_literal = quote_literal(catalog);
    let schema = quote_identifier(INTERNAL_SCHEMA);
    let type_rows = identity_type_rows_sql();
    let type_oid = duckdb_column_type_oid_sql();
    let macros = identity_catalog_macros_sql();
    format!(
        "CREATE OR REPLACE VIEW quackgis_pg_catalog._current_columns AS\n\
         WITH identity AS (\n\
           SELECT identity.*, columns.data_type AS data_type,\n\
                  nullif(columns.column_default, 'NULL') AS column_default,\n\
                  columns.comment AS column_comment, tables.comment AS table_comment,\n\
                  NOT columns.is_nullable AS column_not_null,\n\
                  constraints.constraint_name AS not_null_constraint_name\n\
           FROM ducklake_column_info({catalog_literal}) identity\n\
           JOIN duckdb_columns() columns\n\
             ON columns.database_name = {catalog_literal}\n\
            AND columns.schema_name = identity.schema_name\n\
            AND columns.table_name = identity.table_name\n\
            AND columns.column_name = identity.column_name\n\
           JOIN duckdb_tables() tables\n\
             ON tables.database_name = {catalog_literal}\n\
            AND tables.schema_name = identity.schema_name\n\
            AND tables.table_name = identity.table_name\n\
           LEFT JOIN duckdb_constraints() constraints\n\
             ON constraints.database_name = {catalog_literal}\n\
            AND constraints.schema_name = identity.schema_name\n\
            AND constraints.table_name = identity.table_name\n\
            AND constraints.constraint_type = 'NOT NULL'\n\
            AND len(constraints.constraint_column_names) = 1\n\
            AND constraints.constraint_column_names[1] = identity.column_name\n\
           WHERE identity.schema_name <> {internal_schema_literal}\n\
         ), identity_fingerprint AS (\n\
           SELECT sha256(coalesce(CAST(to_json(list(struct_pack(\n\
             schema_uuid := schema_uuid, schema_name := schema_name,\n\
             table_uuid := table_uuid, table_name := table_name,\n\
             column_id := column_id, column_name := column_name,\n\
             data_type := data_type,\n\
             column_default := column_default, column_comment := column_comment,\n\
             table_comment := table_comment, column_not_null := column_not_null,\n\
             not_null_constraint_name := not_null_constraint_name)\n\
             ORDER BY schema_uuid, table_uuid, column_id)) AS VARCHAR), '[]'))\n\
             AS fingerprint\n\
           FROM identity\n\
         ), valid AS (\n\
           SELECT CASE WHEN s.identity_fingerprint = f.fingerprint\n\
             AND NOT EXISTS (\n\
               SELECT 1 FROM identity i\n\
               LEFT JOIN {catalog_identifier}.{schema}.namespace_oid n USING (schema_uuid)\n\
               LEFT JOIN {catalog_identifier}.{schema}.relation_oid r USING (table_uuid)\n\
               LEFT JOIN {catalog_identifier}.{schema}.attribute_number a\n\
                 USING (table_uuid, column_id)\n\
               LEFT JOIN {catalog_identifier}.{schema}.not_null_constraint_oid constraint_oid\n\
                 ON constraint_oid.table_uuid = i.table_uuid\n\
                AND constraint_oid.column_id = i.column_id AND constraint_oid.active\n\
               LEFT JOIN information_schema.columns c\n\
                 ON c.table_catalog = {catalog_literal}\n\
                AND c.table_schema = i.schema_name\n\
                AND c.table_name = i.table_name\n\
                AND c.column_name = i.column_name\n\
               WHERE i.schema_name IN ('public', 'pg_catalog', 'information_schema',\n\
                                       'quackgis_pg_catalog')\n\
                  OR (i.schema_name = 'main' AND lower(i.table_name) IN\n\
                      ('geometry', 'geography', '_geometry', '_geography',\n\
                       'geometry_columns', 'spatial_ref_sys'))\n\
                  OR n.oid IS NULL OR r.oid IS NULL OR a.attnum IS NULL\n\
                  OR c.column_name IS NULL OR r.namespace_oid <> n.oid\n\
                  OR i.column_not_null <> (constraint_oid.oid IS NOT NULL)\n\
                  OR (i.column_not_null AND i.not_null_constraint_name IS NULL))\n\
             THEN true ELSE error('PostgreSQL catalog identity snapshot is not reconciled') END AS ok\n\
           FROM identity_fingerprint f, {catalog_identifier}.{schema}.catalog_state s\n\
           WHERE s.singleton\n\
         )\n\
         SELECT i.schema_name, i.schema_uuid, i.table_name, i.table_uuid,\n\
                i.column_name, i.column_id, n.oid AS namespace_oid,\n\
                r.oid AS relation_oid, r.row_type_oid, a.attnum,\n\
                c.ordinal_position, c.data_type, c.is_nullable,\n\
                c.numeric_precision, c.numeric_scale, i.column_default,\n\
                i.column_comment, i.table_comment, i.column_not_null,\n\
                i.not_null_constraint_name, constraint_oid.oid AS not_null_constraint_oid\n\
         FROM identity i\n\
         JOIN {catalog_identifier}.{schema}.namespace_oid n USING (schema_uuid)\n\
         JOIN {catalog_identifier}.{schema}.relation_oid r USING (table_uuid)\n\
         JOIN {catalog_identifier}.{schema}.attribute_number a USING (table_uuid, column_id)\n\
         LEFT JOIN {catalog_identifier}.{schema}.not_null_constraint_oid constraint_oid\n\
           ON constraint_oid.table_uuid = i.table_uuid\n\
          AND constraint_oid.column_id = i.column_id AND constraint_oid.active\n\
         JOIN information_schema.columns c\n\
           ON c.table_catalog = {catalog_literal}\n\
          AND c.table_schema = i.schema_name\n\
          AND c.table_name = i.table_name\n\
          AND c.column_name = i.column_name\n\
         CROSS JOIN valid v WHERE v.ok;\n\
         CREATE OR REPLACE VIEW quackgis_pg_catalog.pg_namespace AS\n\
         SELECT * FROM (VALUES\n\
           ({PG_CATALOG_NAMESPACE_OID}::UINTEGER, 'pg_catalog'::VARCHAR, {BOOTSTRAP_OWNER_OID}::UINTEGER),\n\
           ({PUBLIC_NAMESPACE_OID}::UINTEGER, 'public'::VARCHAR, {BOOTSTRAP_OWNER_OID}::UINTEGER)\n\
         ) AS n(oid, nspname, nspowner)\n\
         UNION ALL\n\
         SELECT DISTINCT namespace_oid, schema_name, {BOOTSTRAP_OWNER_OID}::UINTEGER\n\
         FROM quackgis_pg_catalog._current_columns\n\
         WHERE schema_name <> 'main';\n\
         CREATE OR REPLACE VIEW quackgis_pg_catalog.pg_type AS\n\
         SELECT * FROM (VALUES\n{type_rows}\n\
         ) AS t(oid, typname, typnamespace, typlen, typbyval, typtype, typcategory,\n\
                typispreferred, typisdefined, typdelim, typrelid, typelem, typarray,\n\
                typnotnull, typbasetype, typtypmod, typndims, typcollation)\n\
         UNION ALL\n\
         SELECT DISTINCT row_type_oid, table_name, namespace_oid, -1::SMALLINT, false,\n\
                'c'::VARCHAR, 'C'::VARCHAR, false, true, ','::VARCHAR, relation_oid,\n\
                0::UINTEGER, 0::UINTEGER, false, 0::UINTEGER, -1::INTEGER,\n\
                0::INTEGER, 0::UINTEGER\n\
         FROM quackgis_pg_catalog._current_columns;\n\
         CREATE OR REPLACE VIEW quackgis_pg_catalog.pg_class AS\n\
         SELECT columns.relation_oid AS oid, columns.table_name AS relname,\n\
                columns.namespace_oid AS relnamespace, columns.row_type_oid AS reltype,\n\
                coalesce(max(owner.role_oid), {BOOTSTRAP_OWNER_OID}::UINTEGER) AS relowner,\n\
                'r'::VARCHAR AS relkind, CAST(max(columns.attnum) AS SMALLINT) AS relnatts,\n\
                false AS relrowsecurity\n\
         FROM quackgis_pg_catalog._current_columns columns\n\
         LEFT JOIN quackgis_pg_catalog.pg_table_owners owner\n\
           ON owner.schema_name = columns.schema_name\n\
          AND lower(owner.table_name) = lower(columns.table_name)\n\
         GROUP BY columns.relation_oid, columns.table_name, columns.namespace_oid,\n\
                  columns.row_type_oid;\n\
         CREATE OR REPLACE VIEW quackgis_pg_catalog.pg_attribute AS\n\
         WITH typed AS (\n\
           SELECT *, CAST(({type_oid}) AS UINTEGER) AS atttypid,\n\
                  CASE WHEN starts_with(data_type, 'DECIMAL(') AND NOT ends_with(data_type, '[]')\n\
                       THEN CAST(numeric_precision * 65536 + numeric_scale + 4 AS INTEGER)\n\
                       ELSE -1::INTEGER END AS atttypmod\n\
           FROM quackgis_pg_catalog._current_columns\n\
         )\n\
         SELECT relation_oid AS attrelid, column_name AS attname, typed.atttypid,\n\
                t.typlen AS attlen, attnum, atttypmod, is_nullable = 'NO' AS attnotnull,\n\
                column_default IS NOT NULL AS atthasdef,\n\
                t.typcollation AS attcollation,\n\
                ''::VARCHAR AS attidentity, ''::VARCHAR AS attgenerated,\n\
                CASE WHEN t.typlen = -1 THEN 'x' ELSE 'p' END::VARCHAR AS attstorage,\n\
                ''::VARCHAR AS attcompression, -1::SMALLINT AS attstattarget,\n\
                false AS attisdropped\n\
         FROM typed JOIN quackgis_pg_catalog.pg_type t ON t.oid = typed.atttypid;\n\
         CREATE OR REPLACE VIEW quackgis_pg_catalog.pg_attrdef AS\n\
         SELECT relation_oid AS adrelid, attnum AS adnum, column_default AS adbin\n\
         FROM quackgis_pg_catalog._current_columns WHERE column_default IS NOT NULL;\n\
         CREATE OR REPLACE VIEW quackgis_pg_catalog.pg_description AS\n\
         SELECT DISTINCT relation_oid AS objoid, {PG_CLASS_RELATION_OID}::UINTEGER AS classoid,\n\
                0::INTEGER AS objsubid, table_comment AS description\n\
         FROM quackgis_pg_catalog._current_columns WHERE table_comment IS NOT NULL\n\
         UNION ALL\n\
         SELECT relation_oid, {PG_CLASS_RELATION_OID}::UINTEGER, CAST(attnum AS INTEGER),\n\
                column_comment\n\
         FROM quackgis_pg_catalog._current_columns WHERE column_comment IS NOT NULL;\n\
         CREATE OR REPLACE VIEW quackgis_pg_catalog.pg_constraint AS\n\
         SELECT not_null_constraint_oid AS oid, not_null_constraint_name AS conname,\n\
                namespace_oid AS connamespace, 'n'::VARCHAR AS contype,\n\
                false AS condeferrable, false AS condeferred, true AS convalidated,\n\
                relation_oid AS conrelid, 0::UINTEGER AS conindid,\n\
                0::UINTEGER AS conparentid, 0::UINTEGER AS confrelid,\n\
                true AS conislocal, 0::SMALLINT AS coninhcount,\n\
                true AS connoinherit, false AS conperiod,\n\
                [attnum]::SMALLINT[] AS conkey\n\
         FROM quackgis_pg_catalog._current_columns WHERE column_not_null;\n\
         CREATE OR REPLACE VIEW quackgis_pg_catalog.pg_index AS\n\
         SELECT NULL::UINTEGER AS indexrelid, NULL::UINTEGER AS indrelid,\n\
                0::SMALLINT AS indnatts, 0::SMALLINT AS indnkeyatts,\n\
                false AS indisunique, false AS indisnullsnotdistinct,\n\
                false AS indisprimary, false AS indisexclusion,\n\
                false AS indimmediate, false AS indisclustered, false AS indisvalid,\n\
                false AS indcheckxmin, false AS indisready, false AS indislive,\n\
                false AS indisreplident, NULL::SMALLINT[] AS indkey WHERE false;\n\
         CREATE OR REPLACE VIEW quackgis_pg_catalog.geometry_columns AS\n\
         SELECT 'quackgis'::VARCHAR AS f_table_catalog,\n\
                CASE WHEN schema_name = 'main' THEN 'public' ELSE schema_name END::VARCHAR\n\
                  AS f_table_schema,\n\
                table_name::VARCHAR AS f_table_name,\n\
                column_name::VARCHAR AS f_geometry_column,\n\
                2::INTEGER AS coord_dimension, 0::INTEGER AS srid,\n\
                'GEOMETRY'::VARCHAR AS type, relation_oid AS _qg_relation_oid\n\
         FROM quackgis_pg_catalog._current_columns\n\
         WHERE data_type = 'GEOMETRY' OR data_type = 'BLOB' AND lower(column_name) IN\n\
           ('geom', 'geometry', 'the_geom', 'wkb_geometry', 'wkb_geom',\n\
            'geom_wkb', 'shape', 'footprint', 'way');\n\
         CREATE OR REPLACE VIEW quackgis_pg_catalog.spatial_ref_sys AS\n\
         SELECT NULL::INTEGER AS srid, NULL::VARCHAR AS auth_name,\n\
                NULL::INTEGER AS auth_srid, NULL::VARCHAR AS srtext,\n\
                NULL::VARCHAR AS proj4text WHERE false;\n\
         {macros};\n\
         CREATE OR REPLACE MACRO quackgis_pg_schema_epoch() AS\n\
           (SELECT CAST(schema_epoch AS BIGINT)\n\
            FROM {catalog_identifier}.{schema}.catalog_state WHERE singleton);\n\
         CREATE OR REPLACE MACRO quackgis_pg_security_epoch() AS\n\
           (SELECT CAST(security_epoch AS BIGINT)\n\
            FROM {catalog_identifier}.{schema}.catalog_state WHERE singleton)",
        internal_schema_literal = quote_literal(INTERNAL_SCHEMA),
    )
}

fn quote_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn quote_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// Create the durable compatibility identity registry inside the attached
/// DuckLake catalog. DuckLake 1.5 does not support uniqueness constraints, so
/// every reconciliation validates the equivalent invariants explicitly.
pub fn ducklake_identity_registry_bootstrap_sql(catalog: &str) -> String {
    let catalog = quote_identifier(catalog);
    let schema = quote_identifier(INTERNAL_SCHEMA);
    format!(
        "CREATE SCHEMA IF NOT EXISTS {catalog}.{schema};\n\
         CREATE TABLE IF NOT EXISTS {catalog}.{schema}.catalog_state(\n\
           singleton BOOLEAN NOT NULL, format_version USMALLINT NOT NULL,\n\
           next_oid UINTEGER NOT NULL, schema_epoch UBIGINT NOT NULL,\n\
           identity_fingerprint VARCHAR, security_epoch UBIGINT NOT NULL,\n\
           security_fingerprint VARCHAR);\n\
         ALTER TABLE {catalog}.{schema}.catalog_state\n\
           ADD COLUMN IF NOT EXISTS security_epoch UBIGINT DEFAULT 0;\n\
         ALTER TABLE {catalog}.{schema}.catalog_state\n\
           ADD COLUMN IF NOT EXISTS security_fingerprint VARCHAR;\n\
         UPDATE {catalog}.{schema}.catalog_state SET format_version = 2\n\
           WHERE format_version = 1;\n\
         INSERT INTO {catalog}.{schema}.catalog_state\n\
           SELECT true, 2::USMALLINT, {DYNAMIC_OBJECT_OID_START}::UINTEGER,\n\
                  0::UBIGINT, NULL::VARCHAR, 0::UBIGINT, NULL::VARCHAR\n\
           WHERE NOT EXISTS (SELECT 1 FROM {catalog}.{schema}.catalog_state);\n\
         CREATE TABLE IF NOT EXISTS {catalog}.{schema}.namespace_oid(\n\
           schema_uuid UUID NOT NULL, schema_id BIGINT NOT NULL, oid UINTEGER NOT NULL);\n\
         CREATE TABLE IF NOT EXISTS {catalog}.{schema}.relation_oid(\n\
           table_uuid UUID NOT NULL, table_id BIGINT NOT NULL,\n\
           namespace_oid UINTEGER NOT NULL, oid UINTEGER NOT NULL,\n\
           row_type_oid UINTEGER NOT NULL);\n\
         CREATE TABLE IF NOT EXISTS {catalog}.{schema}.attribute_number(\n\
           table_uuid UUID NOT NULL, column_id BIGINT NOT NULL, attnum SMALLINT NOT NULL);\n\
         CREATE TABLE IF NOT EXISTS {catalog}.{schema}.not_null_constraint_oid(\n\
           table_uuid UUID NOT NULL, column_id BIGINT NOT NULL,\n\
           oid UINTEGER NOT NULL, active BOOLEAN NOT NULL);"
    )
}

pub fn ducklake_security_epoch_reconcile_sql(catalog: &str, fingerprint: &str) -> String {
    let catalog = quote_identifier(catalog);
    let schema = quote_identifier(INTERNAL_SCHEMA);
    let fingerprint = quote_literal(fingerprint);
    format!(
        "UPDATE {catalog}.{schema}.catalog_state\n\
         SET security_epoch = security_epoch + 1, security_fingerprint = {fingerprint}\n\
         WHERE singleton AND security_fingerprint IS DISTINCT FROM {fingerprint}"
    )
}

pub fn ducklake_security_epoch_prepare_sql(catalog: &str, fingerprint: &str) -> String {
    let catalog = quote_identifier(catalog);
    let schema = quote_identifier(INTERNAL_SCHEMA);
    let fingerprint = quote_literal(fingerprint);
    format!(
        "UPDATE {catalog}.{schema}.catalog_state\n\
         SET security_fingerprint = NULL\n\
         WHERE singleton AND security_fingerprint IS DISTINCT FROM {fingerprint}"
    )
}

/// Reconcile committed DuckLake identities into durable PostgreSQL-compatible
/// OID and attribute-number mappings. The caller owns one native transaction.
pub fn ducklake_identity_registry_reconcile_sql(catalog: &str) -> String {
    let catalog_identifier = quote_identifier(catalog);
    let catalog_literal = quote_literal(catalog);
    let schema = quote_identifier(INTERNAL_SCHEMA);
    format!(
        "CREATE OR REPLACE TEMP TABLE __quackgis_identity AS\n\
           SELECT identity.*, columns.data_type AS data_type,\n\
                  nullif(columns.column_default, 'NULL') AS column_default,\n\
                  columns.comment AS column_comment, tables.comment AS table_comment,\n\
                  NOT columns.is_nullable AS column_not_null,\n\
                  constraints.constraint_name AS not_null_constraint_name\n\
           FROM ducklake_column_info({catalog_literal}) identity\n\
           JOIN duckdb_columns() columns\n\
             ON columns.database_name = {catalog_literal}\n\
            AND columns.schema_name = identity.schema_name\n\
            AND columns.table_name = identity.table_name\n\
            AND columns.column_name = identity.column_name\n\
           JOIN duckdb_tables() tables\n\
             ON tables.database_name = {catalog_literal}\n\
            AND tables.schema_name = identity.schema_name\n\
            AND tables.table_name = identity.table_name\n\
           LEFT JOIN duckdb_constraints() constraints\n\
             ON constraints.database_name = {catalog_literal}\n\
            AND constraints.schema_name = identity.schema_name\n\
            AND constraints.table_name = identity.table_name\n\
            AND constraints.constraint_type = 'NOT NULL'\n\
            AND len(constraints.constraint_column_names) = 1\n\
            AND constraints.constraint_column_names[1] = identity.column_name\n\
           WHERE identity.schema_name <> {internal_schema_literal};\n\
         CREATE OR REPLACE TEMP TABLE __quackgis_identity_fingerprint AS\n\
           SELECT sha256(coalesce(CAST(to_json(list(struct_pack(\n\
             schema_uuid := schema_uuid, schema_name := schema_name,\n\
             table_uuid := table_uuid, table_name := table_name,\n\
             column_id := column_id, column_name := column_name,\n\
             data_type := data_type,\n\
             column_default := column_default, column_comment := column_comment,\n\
             table_comment := table_comment, column_not_null := column_not_null,\n\
             not_null_constraint_name := not_null_constraint_name)\n\
             ORDER BY schema_uuid, table_uuid, column_id)) AS VARCHAR), '[]'))\n\
             AS fingerprint\n\
           FROM __quackgis_identity;\n\
         CREATE OR REPLACE TEMP TABLE __quackgis_new_public_namespace AS\n\
           SELECT schema_uuid, min(schema_id) AS schema_id\n\
           FROM __quackgis_identity i\n\
           WHERE schema_name = 'main'\n\
             AND NOT EXISTS (\n\
               SELECT 1 FROM {catalog_identifier}.{schema}.namespace_oid n\n\
               WHERE n.schema_uuid = i.schema_uuid)\n\
           GROUP BY schema_uuid;\n\
         INSERT INTO {catalog_identifier}.{schema}.namespace_oid\n\
           SELECT schema_uuid, schema_id, {PUBLIC_NAMESPACE_OID}::UINTEGER\n\
           FROM __quackgis_new_public_namespace;\n\
         CREATE OR REPLACE TEMP TABLE __quackgis_new_namespaces AS\n\
           SELECT schema_uuid, schema_id,\n\
                  row_number() OVER (ORDER BY schema_uuid) AS ordinal\n\
           FROM (\n\
             SELECT schema_uuid, min(schema_id) AS schema_id\n\
             FROM __quackgis_identity i\n\
             WHERE schema_name <> 'main'\n\
               AND NOT EXISTS (\n\
                 SELECT 1 FROM {catalog_identifier}.{schema}.namespace_oid n\n\
                 WHERE n.schema_uuid = i.schema_uuid)\n\
             GROUP BY schema_uuid\n\
           ) missing;\n\
         INSERT INTO {catalog_identifier}.{schema}.namespace_oid\n\
           SELECT n.schema_uuid, n.schema_id,\n\
                  CAST(s.next_oid + n.ordinal - 1 AS UINTEGER)\n\
           FROM __quackgis_new_namespaces n,\n\
                {catalog_identifier}.{schema}.catalog_state s;\n\
         UPDATE {catalog_identifier}.{schema}.catalog_state\n\
           SET next_oid = next_oid + (SELECT count(*) FROM __quackgis_new_namespaces)\n\
           WHERE singleton\n\
             AND (SELECT count(*) FROM __quackgis_new_namespaces) > 0;\n\
         CREATE OR REPLACE TEMP TABLE __quackgis_new_relations AS\n\
           SELECT table_uuid, table_id, namespace_oid,\n\
                  row_number() OVER (ORDER BY table_uuid) AS ordinal\n\
           FROM (\n\
             SELECT i.table_uuid, min(i.table_id) AS table_id,\n\
                    min(n.oid) AS namespace_oid\n\
             FROM __quackgis_identity i\n\
             JOIN {catalog_identifier}.{schema}.namespace_oid n USING (schema_uuid)\n\
             WHERE NOT EXISTS (\n\
               SELECT 1 FROM {catalog_identifier}.{schema}.relation_oid r\n\
               WHERE r.table_uuid = i.table_uuid)\n\
             GROUP BY i.table_uuid\n\
           ) missing;\n\
         INSERT INTO {catalog_identifier}.{schema}.relation_oid\n\
           SELECT r.table_uuid, r.table_id, r.namespace_oid,\n\
                  CAST(s.next_oid + ((r.ordinal - 1) * 2) AS UINTEGER),\n\
                  CAST(s.next_oid + ((r.ordinal - 1) * 2) + 1 AS UINTEGER)\n\
           FROM __quackgis_new_relations r,\n\
                {catalog_identifier}.{schema}.catalog_state s;\n\
         UPDATE {catalog_identifier}.{schema}.catalog_state\n\
           SET next_oid = next_oid + (2 * (SELECT count(*) FROM __quackgis_new_relations))\n\
           WHERE singleton\n\
             AND (SELECT count(*) FROM __quackgis_new_relations) > 0;\n\
         CREATE OR REPLACE TEMP TABLE __quackgis_new_attributes AS\n\
           SELECT i.table_uuid, i.column_id,\n\
                  CAST(coalesce(m.max_attnum, 0) + row_number() OVER (\n\
                    PARTITION BY i.table_uuid ORDER BY i.column_id) AS SMALLINT) AS attnum\n\
           FROM __quackgis_identity i\n\
           LEFT JOIN (\n\
             SELECT table_uuid, max(attnum) AS max_attnum\n\
             FROM {catalog_identifier}.{schema}.attribute_number\n\
             GROUP BY table_uuid\n\
           ) m USING (table_uuid)\n\
           WHERE NOT EXISTS (\n\
             SELECT 1 FROM {catalog_identifier}.{schema}.attribute_number a\n\
             WHERE a.table_uuid = i.table_uuid AND a.column_id = i.column_id);\n\
         INSERT INTO {catalog_identifier}.{schema}.attribute_number\n\
           SELECT * FROM __quackgis_new_attributes;\n\
         UPDATE {catalog_identifier}.{schema}.not_null_constraint_oid constraints\n\
           SET active = false\n\
           WHERE constraints.active AND NOT EXISTS (\n\
             SELECT 1 FROM __quackgis_identity identity\n\
             WHERE identity.table_uuid = constraints.table_uuid\n\
               AND identity.column_id = constraints.column_id\n\
               AND identity.column_not_null);\n\
         CREATE OR REPLACE TEMP TABLE __quackgis_new_not_null_constraints AS\n\
           SELECT identity.table_uuid, identity.column_id,\n\
                  row_number() OVER (ORDER BY identity.table_uuid, identity.column_id) AS ordinal\n\
           FROM __quackgis_identity identity\n\
           WHERE identity.column_not_null AND NOT EXISTS (\n\
             SELECT 1 FROM {catalog_identifier}.{schema}.not_null_constraint_oid constraints\n\
             WHERE constraints.table_uuid = identity.table_uuid\n\
               AND constraints.column_id = identity.column_id AND constraints.active);\n\
         INSERT INTO {catalog_identifier}.{schema}.not_null_constraint_oid\n\
           SELECT constraints.table_uuid, constraints.column_id,\n\
                  CAST(state.next_oid + constraints.ordinal - 1 AS UINTEGER), true\n\
           FROM __quackgis_new_not_null_constraints constraints,\n\
                {catalog_identifier}.{schema}.catalog_state state;\n\
         UPDATE {catalog_identifier}.{schema}.catalog_state\n\
           SET next_oid = next_oid +\n\
               (SELECT count(*) FROM __quackgis_new_not_null_constraints)\n\
           WHERE singleton\n\
             AND (SELECT count(*) FROM __quackgis_new_not_null_constraints) > 0;\n\
         UPDATE {catalog_identifier}.{schema}.catalog_state s\n\
           SET schema_epoch = schema_epoch + CASE\n\
                 WHEN s.identity_fingerprint IS NULL\n\
                  AND ({change_count}) = 0 THEN 0 ELSE 1 END,\n\
               identity_fingerprint = f.fingerprint\n\
           FROM __quackgis_identity_fingerprint f\n\
           WHERE s.singleton\n\
             AND (s.identity_fingerprint IS DISTINCT FROM f.fingerprint\n\
                  OR ({change_count}) > 0);",
        internal_schema_literal = quote_literal(INTERNAL_SCHEMA),
        change_count = "(SELECT count(*) FROM __quackgis_new_public_namespace) + \
                        (SELECT count(*) FROM __quackgis_new_namespaces) + \
                        (SELECT count(*) FROM __quackgis_new_relations) + \
                        (SELECT count(*) FROM __quackgis_new_attributes) + \
                        (SELECT count(*) FROM __quackgis_new_not_null_constraints)",
    )
}

/// Return a query that fails inside DuckDB unless the unconstrained DuckLake
/// registry satisfies all uniqueness, allocation, and reference invariants.
pub fn ducklake_identity_registry_validation_sql(catalog: &str) -> String {
    let catalog = quote_identifier(catalog);
    let schema = quote_identifier(INTERNAL_SCHEMA);
    format!(
        "SELECT CASE WHEN\n\
           (SELECT count(*) FROM {catalog}.{schema}.catalog_state) = 1\n\
           AND (SELECT count(*) FROM {catalog}.{schema}.catalog_state\n\
                WHERE singleton AND format_version = 2\n\
                  AND next_oid >= {DYNAMIC_OBJECT_OID_START}\n\
                  AND security_epoch IS NOT NULL) = 1\n\
           AND NOT EXISTS (\n\
             SELECT schema_uuid FROM {catalog}.{schema}.namespace_oid\n\
             GROUP BY schema_uuid HAVING count(*) <> 1)\n\
           AND NOT EXISTS (\n\
             SELECT table_uuid FROM {catalog}.{schema}.relation_oid\n\
             GROUP BY table_uuid HAVING count(*) <> 1)\n\
           AND NOT EXISTS (\n\
             SELECT table_uuid, column_id FROM {catalog}.{schema}.attribute_number\n\
             GROUP BY table_uuid, column_id HAVING count(*) <> 1)\n\
           AND NOT EXISTS (\n\
             SELECT table_uuid, attnum FROM {catalog}.{schema}.attribute_number\n\
             GROUP BY table_uuid, attnum HAVING count(*) <> 1)\n\
           AND NOT EXISTS (\n\
             SELECT table_uuid, column_id\n\
             FROM {catalog}.{schema}.not_null_constraint_oid WHERE active\n\
             GROUP BY table_uuid, column_id HAVING count(*) <> 1)\n\
           AND NOT EXISTS (\n\
             SELECT oid FROM (\n\
               SELECT oid FROM {catalog}.{schema}.namespace_oid\n\
               UNION ALL\n\
               SELECT oid FROM {catalog}.{schema}.relation_oid\n\
               UNION ALL\n\
               SELECT row_type_oid FROM {catalog}.{schema}.relation_oid\n\
               UNION ALL\n\
               SELECT oid FROM {catalog}.{schema}.not_null_constraint_oid\n\
             ) allocated GROUP BY oid HAVING count(*) <> 1)\n\
           AND NOT EXISTS (\n\
             SELECT 1 FROM {catalog}.{schema}.namespace_oid\n\
             WHERE oid <> {PUBLIC_NAMESPACE_OID} AND oid < {DYNAMIC_OBJECT_OID_START})\n\
           AND NOT EXISTS (\n\
             SELECT 1 FROM {catalog}.{schema}.relation_oid\n\
             WHERE oid < {DYNAMIC_OBJECT_OID_START}\n\
                OR row_type_oid < {DYNAMIC_OBJECT_OID_START})\n\
           AND NOT EXISTS (\n\
             SELECT 1 FROM {catalog}.{schema}.not_null_constraint_oid\n\
             WHERE oid < {DYNAMIC_OBJECT_OID_START})\n\
           AND NOT EXISTS (\n\
             SELECT 1 FROM {catalog}.{schema}.relation_oid r\n\
             WHERE NOT EXISTS (\n\
               SELECT 1 FROM {catalog}.{schema}.namespace_oid n\n\
               WHERE n.oid = r.namespace_oid))\n\
           AND NOT EXISTS (\n\
             SELECT 1 FROM {catalog}.{schema}.attribute_number a\n\
             WHERE a.attnum <= 0 OR NOT EXISTS (\n\
               SELECT 1 FROM {catalog}.{schema}.relation_oid r\n\
               WHERE r.table_uuid = a.table_uuid))\n\
           AND NOT EXISTS (\n\
             SELECT 1 FROM {catalog}.{schema}.not_null_constraint_oid constraints\n\
             WHERE NOT EXISTS (\n\
               SELECT 1 FROM {catalog}.{schema}.relation_oid relations\n\
               WHERE relations.table_uuid = constraints.table_uuid)\n\
                OR NOT EXISTS (\n\
               SELECT 1 FROM {catalog}.{schema}.attribute_number attributes\n\
               WHERE attributes.table_uuid = constraints.table_uuid\n\
                 AND attributes.column_id = constraints.column_id))\n\
           AND (SELECT next_oid FROM {catalog}.{schema}.catalog_state) > coalesce((\n\
             SELECT max(oid) FROM (\n\
               SELECT oid FROM {catalog}.{schema}.namespace_oid\n\
               WHERE oid >= {DYNAMIC_OBJECT_OID_START}\n\
               UNION ALL\n\
               SELECT oid FROM {catalog}.{schema}.relation_oid\n\
               UNION ALL\n\
               SELECT row_type_oid FROM {catalog}.{schema}.relation_oid\n\
               UNION ALL\n\
               SELECT oid FROM {catalog}.{schema}.not_null_constraint_oid\n\
             ) allocated), {dynamic_predecessor})\n\
         THEN true ELSE error('QuackGIS catalog identity registry is inconsistent') END\n\
         AS registry_valid",
        dynamic_predecessor = DYNAMIC_OBJECT_OID_START - 1,
    )
}

/// Validate that every identity in the current committed DuckLake snapshot has
/// a complete, namespace-consistent registry mapping.
pub fn ducklake_identity_registry_coverage_sql(catalog: &str) -> String {
    let catalog_identifier = quote_identifier(catalog);
    let catalog_literal = quote_literal(catalog);
    let schema = quote_identifier(INTERNAL_SCHEMA);
    format!(
        "SELECT CASE WHEN NOT EXISTS (\n\
           SELECT 1\n\
           FROM ducklake_column_info({catalog_literal}) i\n\
           LEFT JOIN {catalog_identifier}.{schema}.namespace_oid n USING (schema_uuid)\n\
           LEFT JOIN {catalog_identifier}.{schema}.relation_oid r\n\
             ON r.table_uuid = i.table_uuid\n\
           LEFT JOIN {catalog_identifier}.{schema}.attribute_number a\n\
             ON a.table_uuid = i.table_uuid AND a.column_id = i.column_id\n\
           WHERE i.schema_name <> {internal_schema_literal}\n\
             AND (i.schema_name IN ('public', 'pg_catalog', 'information_schema',\n\
                                    'quackgis_pg_catalog')\n\
                  OR (i.schema_name = 'main' AND lower(i.table_name) IN\n\
                      ('geometry', 'geography', '_geometry', '_geography',\n\
                       'geometry_columns', 'spatial_ref_sys'))\n\
                  OR n.oid IS NULL OR r.oid IS NULL OR a.attnum IS NULL\n\
                  OR r.namespace_oid <> n.oid\n\
                  OR (i.schema_name = 'main' AND n.oid <> {PUBLIC_NAMESPACE_OID})\n\
                  OR (i.schema_name <> 'main' AND n.oid = {PUBLIC_NAMESPACE_OID}))\n\
         ) THEN true\n\
         ELSE error('QuackGIS catalog identity registry does not cover the committed snapshot')\n\
         END AS registry_coverage_valid",
        internal_schema_literal = quote_literal(INTERNAL_SCHEMA),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::role::RoleCatalog;
    use std::collections::HashSet;

    #[test]
    fn compatibility_version_matches_the_tracked_profile() {
        let profile: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/postgresql18_compatibility_profile.json"
        ))
        .expect("PostgreSQL compatibility profile");
        assert_eq!(
            profile["target"]["postgresql_version"].as_str(),
            Some(POSTGRESQL_COMPATIBILITY_VERSION)
        );
        assert_eq!(POSTGRESQL_COMPATIBILITY_VERSION_NUM, "180004");
    }

    #[test]
    fn bootstrap_contains_consistent_profile_and_qgis_type_identity() {
        let sql = duckdb_catalog_bootstrap_sql();
        for value in [
            GEOMETRY_OID,
            GEOGRAPHY_OID,
            PG_CATALOG_NAMESPACE_OID,
            PUBLIC_NAMESPACE_OID,
            BOOTSTRAP_OWNER_OID,
            QUACKGIS_DATABASE_OID,
        ] {
            assert!(sql.contains(&value.to_string()), "missing OID {value}");
        }
        assert_eq!(TYPE_ROWS.len(), 28);
        let oids = TYPE_ROWS.iter().map(|row| row.oid).collect::<HashSet<_>>();
        assert_eq!(oids.len(), TYPE_ROWS.len());
        let reserved_compatibility_oids = [
            GEOMETRY_OID,
            GEOGRAPHY_OID,
            GEOMETRY_ARRAY_OID,
            GEOGRAPHY_ARRAY_OID,
            POSTGIS_LIB_VERSION_PROC_OID,
            POSTGIS_VERSION_PROC_OID,
            POSTGIS_GEOS_VERSION_PROC_OID,
            POSTGIS_PROJ_VERSION_PROC_OID,
        ];
        assert_eq!(
            reserved_compatibility_oids
                .into_iter()
                .collect::<HashSet<_>>()
                .len(),
            reserved_compatibility_oids.len()
        );
        assert!(
            reserved_compatibility_oids
                .iter()
                .all(|oid| *oid < DYNAMIC_OBJECT_OID_START)
        );
        assert_eq!(PG_PROC_ROWS.len(), 4);
        for (oid, name) in PG_PROC_ROWS {
            assert!(oid < DYNAMIC_OBJECT_OID_START);
            assert!(sql.contains(name));
            assert!(
                crate::spatial_compat::rewrite_postgis_sql(&format!("SELECT {name}()"))
                    .contains(&format!("quackgis_{name}()"))
            );
        }
        for row in TYPE_ROWS {
            for reference in [row.element, row.array] {
                assert!(reference == 0 || oids.contains(&reference));
            }
            assert!(matches!(row.collation, 0 | 100 | 950));
        }
        let reference: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/postgresql18_column_core_reference.json"
        ))
        .expect("PostgreSQL 18 reference fixture");
        let actual_builtin_rows = TYPE_ROWS
            .iter()
            .filter(|row| row.namespace == PG_CATALOG_NAMESPACE_OID)
            .map(|row| {
                serde_json::json!([
                    row.oid,
                    row.name,
                    row.namespace,
                    row.len,
                    row.by_value,
                    "b",
                    row.category.to_string(),
                    row.preferred,
                    true,
                    row.delimiter.to_string(),
                    0,
                    row.element,
                    row.array,
                    false,
                    0,
                    -1,
                    0,
                    row.collation
                ])
            })
            .collect::<Vec<_>>();
        assert_eq!(
            serde_json::Value::Array(actual_builtin_rows),
            reference["builtin_type_rows"]
        );
        assert!(sql.contains("quackgis_pg_catalog.pg_namespace"));
        assert!(sql.contains("quackgis_pg_catalog.pg_database"));
        assert!(sql.contains("quackgis_pg_catalog.pg_proc"));
        assert!(sql.contains("quackgis_pg_catalog.pg_type"));
        assert!(sql.contains("quackgis_pg_catalog.pg_range"));
        assert!(sql.contains("quackgis_pg_catalog.pg_collation"));
        assert!(sql.contains("quackgis_pg_catalog.pg_roles"));
        assert!(sql.contains("quackgis_owner"));
        assert!(sql.contains("quackgis_current_database"));
        assert!(sql.contains("quackgis_current_schema"));
        assert!(sql.contains("quackgis_current_schemas"));
        assert!(!sql.contains("CREATE TABLE"));
    }

    #[test]
    fn configured_role_catalog_contains_no_credentials_and_stable_edge_identity() {
        let catalog = RoleCatalog::from_json(
            r#"{
              "roles":[
                {"oid":100001,"name":"authenticator","login":true},
                {"oid":100002,"name":"api_reader"}
              ],
              "memberships":[
                {"oid":200001,"role":"api_reader","member":"authenticator",
                 "inherit_option":false,"set_option":true}
              ],
              "table_owners":[{"table":"places","role":"authenticator"}]
            }"#,
        )
        .expect("role catalog");
        let auth = crate::auth::AuthConfig::password(
            "authenticator",
            "unique-secret-marker",
            None::<(&str, &str)>,
        )
        .expect("password auth")
        .with_role_catalog(catalog.clone())
        .expect("role auth");
        let sql = duckdb_role_catalog_sql(&catalog, &auth, true);
        assert!(sql.contains("100001::UINTEGER"));
        assert!(sql.contains("200001::UINTEGER"));
        assert!(sql.contains("pg_auth_members"));
        assert!(sql.contains("pg_table_owners"));
        assert!(sql.contains("'places'::VARCHAR, 100001::UINTEGER"));
        assert!(sql.contains("information_schema_table_visibility"));
        assert!(sql.contains("quackgis_information_schema_schemata"));
        assert!(sql.contains("quackgis_information_schema_tables"));
        assert!(sql.contains("quackgis_information_schema_columns"));
        assert!(sql.contains("quackgis_information_schema_table_privileges"));
        assert!(sql.contains("quackgis_information_schema_column_privileges"));
        assert!(sql.contains("quackgis_pg_attrdef_visible"));
        assert!(sql.contains("quackgis_pg_description_visible"));
        assert!(sql.contains("quackgis_pg_constraint_visible"));
        assert!(sql.contains("quackgis_pg_index_visible"));
        assert!(sql.contains("quackgis_pg_geometry_columns_visible"));
        assert!(sql.contains("'geometry_columns'::VARCHAR"));
        assert!(sql.contains("'spatial_ref_sys'::VARCHAR"));
        assert!(sql.contains("NULL::VARCHAR"));
        assert!(!sql.contains("unique-secret-marker"));
    }

    #[test]
    fn identity_registry_sql_is_reserved_and_explicitly_validated() {
        let bootstrap = ducklake_identity_registry_bootstrap_sql("quackgis");
        assert!(bootstrap.contains("\"quackgis\".\"_quackgis\".catalog_state"));
        assert!(bootstrap.contains("100000::UINTEGER"));
        assert!(bootstrap.contains("format_version USMALLINT"));
        assert!(bootstrap.contains("security_epoch UBIGINT"));
        assert!(bootstrap.contains("format_version = 2"));
        assert!(!bootstrap.contains("PRIMARY KEY"));
        assert!(!bootstrap.contains("UNIQUE"));

        let reconcile = ducklake_identity_registry_reconcile_sql("quackgis");
        assert!(reconcile.contains("ducklake_column_info('quackgis')"));
        assert!(reconcile.contains("schema_name <> '_quackgis'"));
        assert!(reconcile.contains("2200::UINTEGER"));
        assert!(reconcile.contains("identity_fingerprint"));

        let security = ducklake_security_epoch_reconcile_sql("quackgis", "fingerprint");
        assert!(security.contains("security_epoch = security_epoch + 1"));
        assert!(security.contains("security_fingerprint IS DISTINCT FROM 'fingerprint'"));
        let prepare = ducklake_security_epoch_prepare_sql("quackgis", "fingerprint");
        assert!(prepare.contains("security_fingerprint = NULL"));
        assert!(prepare.contains("security_fingerprint IS DISTINCT FROM 'fingerprint'"));

        let validation = ducklake_identity_registry_validation_sql("quackgis");
        assert!(validation.contains("registry is inconsistent"));
        assert!(validation.contains("GROUP BY table_uuid, attnum"));
        assert!(validation.contains("oid < 100000"));

        let coverage = ducklake_identity_registry_coverage_sql("quackgis");
        assert!(coverage.contains("ducklake_column_info('quackgis')"));
        assert!(coverage.contains("does not cover the committed snapshot"));
        assert!(coverage.contains("r.namespace_oid <> n.oid"));
        assert!(coverage.contains("'public', 'pg_catalog', 'information_schema'"));
        assert!(coverage.contains("'geometry_columns', 'spatial_ref_sys'"));

        let catalogs = duckdb_identity_catalog_bootstrap_sql("quackgis");
        for relation in [
            "_current_columns",
            "pg_namespace",
            "pg_type",
            "pg_class",
            "pg_attribute",
            "pg_attrdef",
            "pg_description",
            "pg_constraint",
            "pg_index",
            "geometry_columns",
            "spatial_ref_sys",
        ] {
            assert!(
                catalogs.contains(&format!("quackgis_pg_catalog.{relation}")),
                "missing identity catalog {relation}"
            );
        }
        assert!(catalogs.contains("PostgreSQL catalog identity snapshot is not reconciled"));
        assert!(catalogs.contains("quackgis_pg_schema_epoch"));
        assert!(catalogs.contains("quackgis_pg_security_epoch"));
        assert!(catalogs.contains("unsupported DuckLake column type"));
        assert!(catalogs.contains("row_type_oid"));
        assert!(catalogs.contains("CAST(max(columns.attnum) AS SMALLINT) AS relnatts"));
        assert!(catalogs.contains("LEFT JOIN quackgis_pg_catalog.pg_table_owners owner"));
        assert!(catalogs.contains("coalesce(max(owner.role_oid), 10::UINTEGER) AS relowner"));
        assert!(catalogs.contains("is_nullable = 'NO' AS attnotnull"));
        assert!(catalogs.contains("false AS attisdropped"));
        assert_eq!(IDENTITY_TYPE_ROWS.len(), 31);
        assert!(catalogs.contains("quackgis_pg_to_regclass"));
        assert!(catalogs.contains("quackgis_pg_to_regtype"));
        assert!(catalogs.contains("quackgis_pg_format_type"));
        assert!(catalogs.contains("quackgis_pg_get_expr"));
        assert!(catalogs.contains("quackgis_pg_col_description"));
        assert!(catalogs.contains("quackgis_pg_obj_description"));
        assert!(catalogs.contains("quackgis_pg_get_constraintdef"));
        assert!(catalogs.contains("quackgis_pg_get_indexdef"));
        assert!(catalogs.contains("column_default := column_default"));
        assert!(catalogs.contains("data_type := data_type"));
        assert!(catalogs.contains("WHEN data_type = 'GEOMETRY' THEN 90001"));
        assert!(catalogs.contains("not_null_constraint_oid"));
    }
}
