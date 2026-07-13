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
         SELECT {BOOTSTRAP_OWNER_OID}::UINTEGER AS oid, 'quackgis_owner'::VARCHAR AS rolname;\n\
         CREATE OR REPLACE MACRO quackgis_current_database() AS 'quackgis';\n\
         CREATE OR REPLACE MACRO quackgis_current_schema() AS 'public';\n\
         CREATE OR REPLACE MACRO quackgis_current_schemas(include_implicit) AS\n\
           CASE WHEN CAST(include_implicit AS BOOLEAN)\n\
                THEN ['pg_catalog', 'public'] ELSE ['public'] END;"
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
             AND (n.oid IS NULL OR r.oid IS NULL OR a.attnum IS NULL\n\
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
    }
}
