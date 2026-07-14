// SPDX-License-Identifier: Apache-2.0
//! PostgreSQL-facing catalog constants projected into the private DuckDB control catalog.

use arrow_pg::datatypes::{GEOGRAPHY_OID, GEOMETRY_OID};

pub const PG_CATALOG_NAMESPACE_OID: u32 = 11;
pub const PUBLIC_NAMESPACE_OID: u32 = 2_200;
pub const BOOTSTRAP_OWNER_OID: u32 = 10;
pub const QUACKGIS_DATABASE_OID: u32 = 16_384;
pub const GEOMETRY_ARRAY_OID: u32 = 90_003;
pub const GEOGRAPHY_ARRAY_OID: u32 = 90_004;
pub const DYNAMIC_OBJECT_OID_START: u32 = 100_000;
pub const INTERNAL_SCHEMA: &str = "_quackgis";

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
    builtin_array(1001, "_bytea", 17, 0),
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

/// Create the first relational PostgreSQL compatibility catalog.
///
/// These views live only in DuckDB's process-local control database. Client SQL
/// reaches them through a structural `pg_catalog` rewrite; they are not user data
/// and are rebuilt at startup from constants. Wider user-object catalogs will be
/// derived from DuckDB/DuckLake metadata after the durable identity registry lands.
pub fn duckdb_catalog_bootstrap_sql() -> String {
    let type_rows = type_rows_sql();
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
                THEN ['pg_catalog', 'public'] ELSE ['public'] END;"
    )
}

/// Replace bootstrap role catalogs with one immutable configured role graph.
pub fn duckdb_role_catalog_sql(catalog: &crate::role::RoleCatalog) -> String {
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
    format!(
        "CREATE OR REPLACE VIEW quackgis_pg_catalog.pg_roles AS\n\
         SELECT * FROM (VALUES\n{}\n\
         ) AS r(oid, rolname, rolsuper, rolinherit, rolcreaterole, rolcreatedb,\n\
                rolcanlogin, rolreplication, rolconnlimit, rolpassword, rolvaliduntil,\n\
                rolbypassrls, rolconfig);\n\
         CREATE OR REPLACE VIEW quackgis_pg_catalog.pg_auth_members AS\n\
         {membership_sql};\n\
         CREATE OR REPLACE VIEW quackgis_pg_catalog.pg_table_owners AS\n\
         {owner_sql};",
        role_rows.join(",\n"),
    )
}

fn duckdb_column_type_oid_sql() -> &'static str {
    "CASE
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
           (SELECT __quackgis_class.oid FROM quackgis_pg_catalog.pg_class __quackgis_class
            JOIN quackgis_pg_catalog.pg_namespace __quackgis_namespace
              ON __quackgis_namespace.oid = __quackgis_class.relnamespace
            WHERE (try_cast(value AS UINTEGER) IS NOT NULL
                   AND __quackgis_class.oid = try_cast(value AS UINTEGER))
               OR (try_cast(value AS UINTEGER) IS NULL
                   AND __quackgis_class.relname = quackgis_pg_name_object(value)
                   AND __quackgis_namespace.nspname =
                       coalesce(quackgis_pg_name_schema(value), 'public'))
            LIMIT 1);
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
           coalesce((SELECT CASE WHEN __quackgis_namespace.nspname = 'public'
                    THEN quackgis_pg_quote_identifier(__quackgis_class.relname)
                    ELSE quackgis_pg_quote_identifier(__quackgis_namespace.nspname) || '.' ||
                         quackgis_pg_quote_identifier(__quackgis_class.relname) END
             FROM quackgis_pg_catalog.pg_class __quackgis_class
             JOIN quackgis_pg_catalog.pg_namespace __quackgis_namespace
               ON __quackgis_namespace.oid = __quackgis_class.relnamespace
             WHERE __quackgis_class.oid = try_cast(value AS UINTEGER)),
             CAST(value AS VARCHAR));
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
           quackgis_pg_format_type(value, -1);"#
}

/// Replace the baseline catalog views with registry-backed user-object views.
///
/// This SQL is executed only after the checksum-pinned development DuckLake
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
           SELECT * FROM ducklake_column_info({catalog_literal})\n\
           WHERE schema_name <> {internal_schema_literal}\n\
         ), identity_fingerprint AS (\n\
           SELECT sha256(coalesce(CAST(to_json(list(struct_pack(\n\
             schema_uuid := schema_uuid, schema_name := schema_name,\n\
             table_uuid := table_uuid, table_name := table_name,\n\
             column_id := column_id, column_name := column_name)\n\
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
               LEFT JOIN information_schema.columns c\n\
                 ON c.table_catalog = {catalog_literal}\n\
                AND c.table_schema = i.schema_name\n\
                AND c.table_name = i.table_name\n\
                AND c.column_name = i.column_name\n\
               WHERE i.schema_name IN ('public', 'pg_catalog', 'information_schema',\n\
                                       'quackgis_pg_catalog')\n\
                  OR (i.schema_name = 'main' AND lower(i.table_name) IN\n\
                      ('geometry', 'geography', '_geometry', '_geography'))\n\
                  OR n.oid IS NULL OR r.oid IS NULL OR a.attnum IS NULL\n\
                  OR c.column_name IS NULL OR r.namespace_oid <> n.oid)\n\
             THEN true ELSE error('PostgreSQL catalog identity snapshot is not reconciled') END AS ok\n\
           FROM identity_fingerprint f, {catalog_identifier}.{schema}.catalog_state s\n\
           WHERE s.singleton\n\
         )\n\
         SELECT i.schema_name, i.schema_uuid, i.table_name, i.table_uuid,\n\
                i.column_name, i.column_id, n.oid AS namespace_oid,\n\
                r.oid AS relation_oid, r.row_type_oid, a.attnum,\n\
                c.ordinal_position, c.data_type, c.is_nullable,\n\
                c.numeric_precision, c.numeric_scale\n\
         FROM identity i\n\
         JOIN {catalog_identifier}.{schema}.namespace_oid n USING (schema_uuid)\n\
         JOIN {catalog_identifier}.{schema}.relation_oid r USING (table_uuid)\n\
         JOIN {catalog_identifier}.{schema}.attribute_number a USING (table_uuid, column_id)\n\
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
                ''::VARCHAR AS attidentity, ''::VARCHAR AS attgenerated,\n\
                false AS attisdropped\n\
         FROM typed JOIN quackgis_pg_catalog.pg_type t ON t.oid = typed.atttypid;\n\
         {macros}",
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
           identity_fingerprint VARCHAR);\n\
         INSERT INTO {catalog}.{schema}.catalog_state\n\
           SELECT true, 1::USMALLINT, {DYNAMIC_OBJECT_OID_START}::UINTEGER,\n\
                  0::UBIGINT, NULL::VARCHAR\n\
           WHERE NOT EXISTS (SELECT 1 FROM {catalog}.{schema}.catalog_state);\n\
         CREATE TABLE IF NOT EXISTS {catalog}.{schema}.namespace_oid(\n\
           schema_uuid UUID NOT NULL, schema_id BIGINT NOT NULL, oid UINTEGER NOT NULL);\n\
         CREATE TABLE IF NOT EXISTS {catalog}.{schema}.relation_oid(\n\
           table_uuid UUID NOT NULL, table_id BIGINT NOT NULL,\n\
           namespace_oid UINTEGER NOT NULL, oid UINTEGER NOT NULL,\n\
           row_type_oid UINTEGER NOT NULL);\n\
         CREATE TABLE IF NOT EXISTS {catalog}.{schema}.attribute_number(\n\
           table_uuid UUID NOT NULL, column_id BIGINT NOT NULL, attnum SMALLINT NOT NULL);"
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
           SELECT * FROM ducklake_column_info({catalog_literal})\n\
           WHERE schema_name <> {internal_schema_literal};\n\
         CREATE OR REPLACE TEMP TABLE __quackgis_identity_fingerprint AS\n\
           SELECT sha256(coalesce(CAST(to_json(list(struct_pack(\n\
             schema_uuid := schema_uuid, schema_name := schema_name,\n\
             table_uuid := table_uuid, table_name := table_name,\n\
             column_id := column_id, column_name := column_name)\n\
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
                        (SELECT count(*) FROM __quackgis_new_attributes)",
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
                WHERE singleton AND format_version = 1\n\
                  AND next_oid >= {DYNAMIC_OBJECT_OID_START}) = 1\n\
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
             SELECT oid FROM (\n\
               SELECT oid FROM {catalog}.{schema}.namespace_oid\n\
               UNION ALL\n\
               SELECT oid FROM {catalog}.{schema}.relation_oid\n\
               UNION ALL\n\
               SELECT row_type_oid FROM {catalog}.{schema}.relation_oid\n\
             ) allocated GROUP BY oid HAVING count(*) <> 1)\n\
           AND NOT EXISTS (\n\
             SELECT 1 FROM {catalog}.{schema}.namespace_oid\n\
             WHERE oid <> {PUBLIC_NAMESPACE_OID} AND oid < {DYNAMIC_OBJECT_OID_START})\n\
           AND NOT EXISTS (\n\
             SELECT 1 FROM {catalog}.{schema}.relation_oid\n\
             WHERE oid < {DYNAMIC_OBJECT_OID_START}\n\
                OR row_type_oid < {DYNAMIC_OBJECT_OID_START})\n\
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
           AND (SELECT next_oid FROM {catalog}.{schema}.catalog_state) > coalesce((\n\
             SELECT max(oid) FROM (\n\
               SELECT oid FROM {catalog}.{schema}.namespace_oid\n\
               WHERE oid >= {DYNAMIC_OBJECT_OID_START}\n\
               UNION ALL\n\
               SELECT oid FROM {catalog}.{schema}.relation_oid\n\
               UNION ALL\n\
               SELECT row_type_oid FROM {catalog}.{schema}.relation_oid\n\
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
                      ('geometry', 'geography', '_geometry', '_geography'))\n\
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
        let sql = duckdb_role_catalog_sql(&catalog);
        assert!(sql.contains("100001::UINTEGER"));
        assert!(sql.contains("200001::UINTEGER"));
        assert!(sql.contains("pg_auth_members"));
        assert!(sql.contains("pg_table_owners"));
        assert!(sql.contains("'places'::VARCHAR, 100001::UINTEGER"));
        assert!(sql.contains("NULL::VARCHAR"));
        assert!(!sql.contains("password-secret"));
    }

    #[test]
    fn identity_registry_sql_is_reserved_and_explicitly_validated() {
        let bootstrap = ducklake_identity_registry_bootstrap_sql("quackgis");
        assert!(bootstrap.contains("\"quackgis\".\"_quackgis\".catalog_state"));
        assert!(bootstrap.contains("100000::UINTEGER"));
        assert!(bootstrap.contains("format_version USMALLINT"));
        assert!(!bootstrap.contains("PRIMARY KEY"));
        assert!(!bootstrap.contains("UNIQUE"));

        let reconcile = ducklake_identity_registry_reconcile_sql("quackgis");
        assert!(reconcile.contains("ducklake_column_info('quackgis')"));
        assert!(reconcile.contains("schema_name <> '_quackgis'"));
        assert!(reconcile.contains("2200::UINTEGER"));
        assert!(reconcile.contains("identity_fingerprint"));

        let validation = ducklake_identity_registry_validation_sql("quackgis");
        assert!(validation.contains("registry is inconsistent"));
        assert!(validation.contains("GROUP BY table_uuid, attnum"));
        assert!(validation.contains("oid < 100000"));

        let coverage = ducklake_identity_registry_coverage_sql("quackgis");
        assert!(coverage.contains("ducklake_column_info('quackgis')"));
        assert!(coverage.contains("does not cover the committed snapshot"));
        assert!(coverage.contains("r.namespace_oid <> n.oid"));
        assert!(coverage.contains("'public', 'pg_catalog', 'information_schema'"));
        assert!(coverage.contains("'geometry', 'geography', '_geometry', '_geography'"));

        let catalogs = duckdb_identity_catalog_bootstrap_sql("quackgis");
        for relation in [
            "_current_columns",
            "pg_namespace",
            "pg_type",
            "pg_class",
            "pg_attribute",
        ] {
            assert!(
                catalogs.contains(&format!("quackgis_pg_catalog.{relation}")),
                "missing identity catalog {relation}"
            );
        }
        assert!(catalogs.contains("PostgreSQL catalog identity snapshot is not reconciled"));
        assert!(catalogs.contains("unsupported DuckLake column type"));
        assert!(catalogs.contains("row_type_oid"));
        assert!(catalogs.contains("CAST(max(columns.attnum) AS SMALLINT) AS relnatts"));
        assert!(catalogs.contains("LEFT JOIN quackgis_pg_catalog.pg_table_owners owner"));
        assert!(catalogs.contains("coalesce(max(owner.role_oid), 10::UINTEGER) AS relowner"));
        assert!(catalogs.contains("is_nullable = 'NO' AS attnotnull"));
        assert!(catalogs.contains("false AS attisdropped"));
        assert_eq!(IDENTITY_TYPE_ROWS.len(), 28);
        assert!(catalogs.contains("quackgis_pg_to_regclass"));
        assert!(catalogs.contains("quackgis_pg_to_regtype"));
        assert!(catalogs.contains("quackgis_pg_format_type"));
    }
}
