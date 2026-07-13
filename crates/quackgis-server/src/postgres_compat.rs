// SPDX-License-Identifier: Apache-2.0
//! PostgreSQL-facing catalog constants projected into the private DuckDB control catalog.

use arrow_pg::datatypes::{GEOGRAPHY_OID, GEOMETRY_OID};

pub const PG_CATALOG_NAMESPACE_OID: u32 = 11;
pub const PUBLIC_NAMESPACE_OID: u32 = 2_200;
pub const BOOTSTRAP_OWNER_OID: u32 = 10;
pub const GEOMETRY_ARRAY_OID: u32 = 90_003;
pub const GEOGRAPHY_ARRAY_OID: u32 = 90_004;

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

fn type_rows_sql() -> String {
    TYPE_ROWS
        .iter()
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
         SELECT {BOOTSTRAP_OWNER_OID}::UINTEGER AS oid, 'quackgis_owner'::VARCHAR AS rolname;"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
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
        assert!(sql.contains("quackgis_pg_catalog.pg_type"));
        assert!(sql.contains("quackgis_pg_catalog.pg_range"));
        assert!(sql.contains("quackgis_pg_catalog.pg_collation"));
        assert!(sql.contains("quackgis_pg_catalog.pg_roles"));
        assert!(sql.contains("quackgis_owner"));
        assert!(!sql.contains("CREATE TABLE"));
    }
}
