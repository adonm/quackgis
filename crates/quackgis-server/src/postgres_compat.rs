// SPDX-License-Identifier: Apache-2.0
//! PostgreSQL-facing catalog constants projected into the private DuckDB control catalog.

use arrow_pg::datatypes::{GEOGRAPHY_OID, GEOMETRY_OID};

pub const PG_CATALOG_NAMESPACE_OID: u32 = 11;
pub const PUBLIC_NAMESPACE_OID: u32 = 2_200;
pub const BOOTSTRAP_OWNER_OID: u32 = 10;

/// Create the first relational PostgreSQL compatibility catalog.
///
/// These views live only in DuckDB's process-local control database. Client SQL
/// reaches them through a structural `pg_catalog` rewrite; they are not user data
/// and are rebuilt at startup from constants. Wider user-object catalogs will be
/// derived from DuckDB/DuckLake metadata after the durable identity registry lands.
pub fn duckdb_catalog_bootstrap_sql() -> String {
    format!(
        "CREATE SCHEMA IF NOT EXISTS quackgis_pg_catalog;\n\
         CREATE OR REPLACE VIEW quackgis_pg_catalog.pg_namespace AS\n\
         SELECT * FROM (VALUES\n\
           ({PG_CATALOG_NAMESPACE_OID}::UINTEGER, 'pg_catalog'::VARCHAR, {BOOTSTRAP_OWNER_OID}::UINTEGER),\n\
           ({PUBLIC_NAMESPACE_OID}::UINTEGER, 'public'::VARCHAR, {BOOTSTRAP_OWNER_OID}::UINTEGER)\n\
         ) AS n(oid, nspname, nspowner);\n\
         CREATE OR REPLACE VIEW quackgis_pg_catalog.pg_type AS\n\
         SELECT * FROM (VALUES\n\
           ({GEOMETRY_OID}::UINTEGER, 'geometry'::VARCHAR, {PUBLIC_NAMESPACE_OID}::UINTEGER,\n\
            -1::SMALLINT, false, 'b'::VARCHAR, 'U'::VARCHAR, false, true, ','::VARCHAR,\n\
            0::UINTEGER, 0::UINTEGER, 0::UINTEGER, false, 0::UINTEGER, -1::INTEGER,\n\
            0::INTEGER, 0::UINTEGER),\n\
           ({GEOGRAPHY_OID}::UINTEGER, 'geography'::VARCHAR, {PUBLIC_NAMESPACE_OID}::UINTEGER,\n\
            -1::SMALLINT, false, 'b'::VARCHAR, 'U'::VARCHAR, false, true, ','::VARCHAR,\n\
            0::UINTEGER, 0::UINTEGER, 0::UINTEGER, false, 0::UINTEGER, -1::INTEGER,\n\
            0::INTEGER, 0::UINTEGER)\n\
         ) AS t(oid, typname, typnamespace, typlen, typbyval, typtype, typcategory,\n\
                typispreferred, typisdefined, typdelim, typrelid, typelem, typarray,\n\
                typnotnull, typbasetype, typtypmod, typndims, typcollation);\n\
         CREATE OR REPLACE VIEW quackgis_pg_catalog.pg_range AS\n\
         SELECT NULL::UINTEGER AS rngtypid, NULL::UINTEGER AS rngsubtype WHERE false;\n\
         CREATE OR REPLACE VIEW quackgis_pg_catalog.pg_roles AS\n\
         SELECT {BOOTSTRAP_OWNER_OID}::UINTEGER AS oid, 'quackgis_owner'::VARCHAR AS rolname;"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_contains_consistent_spatial_identity() {
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
        assert!(sql.contains("quackgis_pg_catalog.pg_namespace"));
        assert!(sql.contains("quackgis_pg_catalog.pg_type"));
        assert!(sql.contains("quackgis_pg_catalog.pg_range"));
        assert!(sql.contains("quackgis_pg_catalog.pg_roles"));
        assert!(sql.contains("quackgis_owner"));
        assert!(!sql.contains("CREATE TABLE"));
    }
}
