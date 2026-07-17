// SPDX-License-Identifier: Apache-2.0
use std::sync::Arc;

use adbc_core::options::IngestMode;
use arrow_array::{Array, Int32Array, Int64Array, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema};
use quackgis_server::auth::{AuthConfig, parse_read_allowlist};
use quackgis_server::duckdb_adbc_storage::{DuckDbAdbcConfig, DuckDbAdbcStorage, ExtensionPolicy};
use quackgis_server::engine_api::{
    EngineStorageKernel, EngineTableRef, EngineTransactionState, IngestDisposition,
};
use quackgis_server::pgwire_server::ServerOptions;
use quackgis_server::role::RoleCatalog;

#[derive(Clone, Debug, Eq, PartialEq)]
struct IdentityRow {
    schema_name: String,
    schema_id: i64,
    schema_uuid: String,
    table_name: String,
    table_id: i64,
    table_uuid: String,
    column_name: String,
    column_id: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RegisteredOid(u32);

#[derive(Debug, Eq, PartialEq)]
struct PgNodeTree(String);

impl<'a> tokio_postgres::types::FromSql<'a> for PgNodeTree {
    fn from_sql(
        _ty: &tokio_postgres::types::Type,
        raw: &'a [u8],
    ) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        Ok(Self(std::str::from_utf8(raw)?.to_owned()))
    }

    fn accepts(ty: &tokio_postgres::types::Type) -> bool {
        ty.oid() == 194
    }
}

impl<'a> tokio_postgres::types::FromSql<'a> for RegisteredOid {
    fn from_sql(
        _ty: &tokio_postgres::types::Type,
        raw: &'a [u8],
    ) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        let bytes: [u8; 4] = raw.try_into()?;
        Ok(Self(u32::from_be_bytes(bytes)))
    }

    fn accepts(ty: &tokio_postgres::types::Type) -> bool {
        matches!(
            *ty,
            tokio_postgres::types::Type::REGCLASS
                | tokio_postgres::types::Type::REGTYPE
                | tokio_postgres::types::Type::REGNAMESPACE
                | tokio_postgres::types::Type::REGROLE
        )
    }
}

fn captured_trace_sql(raw: &str, query_id: &str) -> String {
    let trace: serde_json::Value = serde_json::from_str(raw).expect("captured client trace");
    trace["queries"]
        .as_array()
        .expect("captured trace queries")
        .iter()
        .find(|query| query["id"] == query_id)
        .and_then(|query| query["sql"].as_str())
        .unwrap_or_else(|| panic!("captured trace query {query_id}"))
        .to_owned()
}

fn identity_rows(batches: &[RecordBatch]) -> Vec<IdentityRow> {
    let mut rows = Vec::new();
    for batch in batches {
        let strings = |column: usize| {
            batch
                .column(column)
                .as_any()
                .downcast_ref::<StringArray>()
                .expect("identity string column")
        };
        let integers = |column: usize| {
            batch
                .column(column)
                .as_any()
                .downcast_ref::<Int64Array>()
                .expect("identity BIGINT column")
        };
        for row in 0..batch.num_rows() {
            rows.push(IdentityRow {
                schema_name: strings(0).value(row).to_owned(),
                schema_id: integers(1).value(row),
                schema_uuid: strings(2).value(row).to_owned(),
                table_name: strings(3).value(row).to_owned(),
                table_id: integers(4).value(row),
                table_uuid: strings(5).value(row).to_owned(),
                column_name: strings(6).value(row).to_owned(),
                column_id: integers(7).value(row),
            });
        }
    }
    rows
}

const COLUMN_IDENTITY_SQL: &str = "SELECT schema_name, schema_id, CAST(schema_uuid AS VARCHAR), table_name, \
            table_id, CAST(table_uuid AS VARCHAR), column_name, column_id \
     FROM ducklake_column_info('quackgis') \
     WHERE schema_name <> '_quackgis' ORDER BY table_id, column_id";

fn first_i64(batches: &[RecordBatch], column: usize) -> i64 {
    batches[0]
        .column(column)
        .as_any()
        .downcast_ref::<Int64Array>()
        .expect("registry BIGINT result")
        .value(0)
}

fn registry_state(storage: &DuckDbAdbcStorage) -> (i64, i64) {
    let rows = storage
        .query(
            "SELECT CAST(next_oid AS BIGINT), CAST(schema_epoch AS BIGINT) \
             FROM quackgis._quackgis.catalog_state WHERE singleton",
        )
        .expect("catalog registry state");
    (first_i64(&rows, 0), first_i64(&rows, 1))
}

fn registered_columns(
    storage: &DuckDbAdbcStorage,
    schema: &str,
    table: &str,
) -> Vec<(i64, i64, i64, i64)> {
    let sql = format!(
        "SELECT CAST(r.namespace_oid AS BIGINT), CAST(r.oid AS BIGINT), \
                i.column_id, CAST(a.attnum AS BIGINT) \
         FROM ducklake_column_info('quackgis') i \
         JOIN quackgis._quackgis.relation_oid r USING (table_uuid) \
         JOIN quackgis._quackgis.attribute_number a USING (table_uuid, column_id) \
         WHERE i.schema_name = '{}' AND i.table_name = '{}' \
         ORDER BY a.attnum",
        schema.replace('\'', "''"),
        table.replace('\'', "''")
    );
    storage
        .query(&sql)
        .expect("registered catalog columns")
        .into_iter()
        .flat_map(|batch| {
            let columns = (0..4)
                .map(|column| {
                    batch
                        .column(column)
                        .as_any()
                        .downcast_ref::<Int64Array>()
                        .expect("registered BIGINT column")
                })
                .collect::<Vec<_>>();
            (0..batch.num_rows())
                .map(|row| {
                    (
                        columns[0].value(row),
                        columns[1].value(row),
                        columns[2].value(row),
                        columns[3].value(row),
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

async fn prove_registry_catalog_pgwire(storage: Arc<DuckDbAdbcStorage>) {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("pinned catalog listener");
    let port = listener
        .local_addr()
        .expect("pinned catalog address")
        .port();
    let role_catalog = RoleCatalog::from_json(
        r#"{
          "roles": [
            {"oid": 110001, "name": "writer", "login": true},
            {"oid": 110002, "name": "reader", "login": true}
          ],
          "table_owners": [
            {"table": "catalog_projection", "role": "writer"},
            {"table": "catalog_projection_renamed", "role": "writer"},
            {"table": "private_metadata", "role": "writer"}
          ],
          "schema_grants": [
            {"schema": "public", "role": "PUBLIC", "privileges": ["USAGE"]}
          ],
          "table_grants": [
            {"table": "catalog_projection", "role": "reader", "privileges": ["SELECT"]},
            {"table": "private_metadata", "role": "reader", "privileges": ["SELECT"]}
          ]
        }"#,
    )
    .expect("pinned metadata role catalog");
    let auth = AuthConfig::password("writer", "writer-secret", Some(("reader", "reader-secret")))
        .expect("pinned metadata password auth")
        .with_read_allowlist(
            parse_read_allowlist("catalog_projection,catalog_projection_renamed")
                .expect("read allowlist"),
        )
        .with_role_catalog(role_catalog.clone())
        .expect("pinned metadata role auth");
    let epoch_auth = auth.clone();
    let server_storage = Arc::clone(&storage);
    let server = tokio::spawn(async move {
        quackgis_server::pgwire_server::serve_duckdb_on_listener(
            server_storage,
            listener,
            &ServerOptions::new().with_max_connections(4),
            auth,
        )
        .await
    });
    let mut writer_config = tokio_postgres::Config::new();
    writer_config
        .host("127.0.0.1")
        .port(port)
        .user("writer")
        .password("writer-secret")
        .dbname("quackgis");
    let (client, connection) = writer_config
        .connect(tokio_postgres::NoTls)
        .await
        .expect("pinned catalog pgwire connection");
    let connection = tokio::spawn(connection);

    let shared_epochs = client
        .query_one(
            "SELECT quackgis_schema_epoch(), pg_catalog.quackgis_security_epoch()",
            &[],
        )
        .await
        .expect("shared catalog epochs");
    assert_eq!(
        shared_epochs.columns()[0].type_(),
        &tokio_postgres::types::Type::INT8
    );
    assert_eq!(
        shared_epochs.columns()[1].type_(),
        &tokio_postgres::types::Type::INT8
    );
    let initial_epochs = storage
        .catalog_epochs()
        .expect("storage catalog epochs")
        .expect("pinned catalog epochs");
    assert_eq!(shared_epochs.get::<_, i64>(0), initial_epochs.schema as i64);
    assert_eq!(
        shared_epochs.get::<_, i64>(1),
        initial_epochs.security as i64
    );
    storage
        .install_role_catalog(&role_catalog, &epoch_auth)
        .expect("reinstall unchanged role catalog");
    assert_eq!(
        storage
            .catalog_epochs()
            .expect("epochs after unchanged role install")
            .expect("pinned catalog epochs")
            .security,
        initial_epochs.security
    );

    client
        .batch_execute(
            "CREATE TABLE quackgis.main.catalog_projection(\
             id BIGINT NOT NULL DEFAULT 7, label VARCHAR, geom_wkb BLOB, \
             native_geom GEOMETRY, score DOUBLE, active BOOLEAN)",
        )
        .await
        .expect("create projected catalog table");
    client
        .batch_execute(
            "INSERT INTO quackgis.main.catalog_projection(\
             id, label, geom_wkb, native_geom) VALUES \
             (8, 'first', ST_AsWKB(ST_GeomFromText('POINT Z (0 0 5)')), \
                           ST_GeomFromText('POINT Z (0 0 5)')), \
             (9, 'second', ST_AsWKB(ST_GeomFromText('POINT Z (2 3 9)')), \
                            ST_GeomFromText('POINT Z (2 3 9)'))",
        )
        .await
        .expect("insert projected spatial rows");
    let qgis_privileges = client
        .query_one(
            "SELECT has_table_privilege('\"public\".\"catalog_projection\"','SELECT'), \
                    pg_is_in_recovery(), current_schema(), \
                    has_any_column_privilege('\"public\".\"catalog_projection\"','INSERT'), \
                    has_table_privilege('\"public\".\"catalog_projection\"','DELETE'), \
                    has_any_column_privilege('\"public\".\"catalog_projection\"','UPDATE'), \
                    has_column_privilege('\"public\".\"catalog_projection\"','geom_wkb','UPDATE')",
            &[],
        )
        .await
        .expect("QGIS layer privilege and recovery inquiry");
    assert!(qgis_privileges.get::<_, bool>(0));
    assert!(!qgis_privileges.get::<_, bool>(1));
    assert_eq!(qgis_privileges.get::<_, String>(2), "public");
    for column in 3..7 {
        assert!(qgis_privileges.get::<_, bool>(column));
    }
    assert_eq!(
        qgis_privileges.columns()[1].type_(),
        &tokio_postgres::types::Type::BOOL
    );
    assert_eq!(
        qgis_privileges.columns()[2].type_(),
        &tokio_postgres::types::Type::NAME
    );
    let epoch_before_comments = storage
        .catalog_schema_epoch()
        .expect("catalog epoch before comments")
        .expect("pinned catalog epoch");
    let security_before_comments = storage
        .catalog_epochs()
        .expect("catalog epochs before comments")
        .expect("pinned catalog epochs")
        .security;
    execute_storage_update(
        &storage,
        "COMMENT ON TABLE quackgis.main.catalog_projection IS 'projected table'",
    )
    .await;
    execute_storage_update(
        &storage,
        "COMMENT ON COLUMN quackgis.main.catalog_projection.id IS 'stable identifier'",
    )
    .await;
    assert_eq!(
        storage
            .catalog_schema_epoch()
            .expect("catalog epoch after comments")
            .expect("pinned catalog epoch"),
        epoch_before_comments + 2
    );
    assert_eq!(
        storage
            .catalog_epochs()
            .expect("catalog epochs after comments")
            .expect("pinned catalog epochs")
            .security,
        security_before_comments
    );
    execute_storage_update(
        &storage,
        "CREATE TABLE quackgis.main.private_metadata(\
         id BIGINT NOT NULL DEFAULT 9, geom_wkb BLOB)",
    )
    .await;
    execute_storage_update(
        &storage,
        "COMMENT ON TABLE quackgis.main.private_metadata IS 'legacy-hidden table'",
    )
    .await;
    let projected_metadata_counts = storage
        .query(
            "SELECT (SELECT count(*)::BIGINT FROM quackgis_pg_catalog.pg_attrdef \
                     WHERE adrelid = quackgis_pg_regclass('catalog_projection')), \
                    (SELECT count(*)::BIGINT \
                     FROM quackgis_pg_attrdef_visible('writer', 'writer') \
                     WHERE adrelid = quackgis_pg_regclass('catalog_projection')), \
                    (SELECT count(*)::BIGINT FROM quackgis_pg_catalog.pg_description \
                     WHERE objoid = quackgis_pg_regclass('catalog_projection'))",
        )
        .expect("projected metadata counts");
    assert_eq!(first_i64(&projected_metadata_counts, 0), 1);
    assert_eq!(first_i64(&projected_metadata_counts, 1), 1);
    assert_eq!(first_i64(&projected_metadata_counts, 2), 2);

    let profile: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/postgresql18_compatibility_profile.json"
    ))
    .expect("client-neutral PostgreSQL catalog profile");
    for (relation, sql) in [
        (
            "pg_catalog.pg_namespace",
            "SELECT oid, nspname, nspowner FROM pg_namespace WHERE nspname = 'public'",
        ),
        (
            "pg_catalog.pg_proc",
            "SELECT oid, proname, pronamespace FROM pg_proc ORDER BY oid",
        ),
        (
            "pg_catalog.pg_class",
            "SELECT oid, relname, relnamespace, reltype, relowner, relkind, relnatts, \
                    relchecks, relhasindex, relhasrules, relhastriggers, relrowsecurity, \
                    relforcerowsecurity, reltoastrelid, relispartition, reloptions, \
                    reltablespace, reloftype, relpersistence, relreplident, relam \
             FROM pg_class WHERE relname = 'catalog_projection'",
        ),
        (
            "pg_catalog.pg_am",
            "SELECT oid, amname FROM pg_am WHERE amname = 'ducklake'",
        ),
        (
            "pg_catalog.pg_attribute",
            "SELECT a.attrelid, a.attname, a.atttypid, a.attlen, a.attnum, a.atttypmod, \
                    a.attnotnull, a.attidentity, a.attgenerated, a.attisdropped \
             FROM pg_attribute a JOIN pg_class c ON c.oid = a.attrelid \
             WHERE c.relname = 'catalog_projection' ORDER BY a.attnum",
        ),
        (
            "pg_catalog.pg_attrdef",
            "SELECT d.adrelid, d.adnum, d.adbin FROM pg_attrdef d \
             JOIN pg_class c ON c.oid = d.adrelid \
             WHERE c.relname = 'catalog_projection' ORDER BY d.adnum",
        ),
        (
            "pg_catalog.pg_description",
            "SELECT objoid, classoid, objsubid, description FROM pg_description \
             WHERE objoid = 'catalog_projection'::regclass ORDER BY objsubid",
        ),
        (
            "pg_catalog.pg_constraint",
            "SELECT c.oid, c.conname, c.connamespace, c.contype, c.condeferrable, \
                    c.condeferred, c.convalidated, c.conrelid, c.conindid, \
                    c.conparentid, c.confrelid, c.conislocal, c.coninhcount, \
                    c.connoinherit, c.conperiod, c.conkey \
             FROM pg_constraint c JOIN pg_class r ON r.oid = c.conrelid \
             WHERE r.relname = 'catalog_projection' ORDER BY c.conname",
        ),
        (
            "pg_catalog.pg_index",
            "SELECT indexrelid, indrelid, indisunique, indisprimary, indisclustered, \
                    indisvalid, indisreplident, indkey FROM pg_index \
             WHERE indrelid = 'catalog_projection'::regclass",
        ),
        (
            "geometry_columns",
            "SELECT f_table_catalog, f_table_schema, f_table_name, f_geometry_column, \
                    coord_dimension, srid, type FROM geometry_columns \
             WHERE f_table_name = 'catalog_projection' AND f_geometry_column = 'geom_wkb'",
        ),
        (
            "spatial_ref_sys",
            "SELECT srid, auth_name, auth_srid, srtext, proj4text \
             FROM spatial_ref_sys WHERE srid = 4326",
        ),
        (
            "pg_catalog.pg_type",
            "SELECT t.oid, t.typname, t.typnamespace, t.typlen, t.typbyval, t.typtype, \
                    t.typcategory, t.typispreferred, t.typisdefined, t.typdelim, \
                    t.typrelid, t.typelem, t.typarray, t.typnotnull, t.typbasetype, \
                    t.typtypmod, t.typndims, t.typcollation \
             FROM pg_type t WHERE t.typname = 'catalog_projection'",
        ),
        (
            "pg_catalog.pg_database",
            "SELECT oid, datname, datdba FROM pg_database WHERE datname = current_database()",
        ),
        (
            "pg_catalog.pg_roles",
            "SELECT oid, rolname FROM pg_roles WHERE rolname = 'quackgis_owner'",
        ),
    ] {
        let expected = profile["catalog_relations"]
            .as_array()
            .and_then(|relations| {
                relations
                    .iter()
                    .find(|candidate| candidate["name"] == relation)
            })
            .and_then(|catalog| catalog["required_columns"].as_array())
            .unwrap_or_else(|| panic!("profile relation {relation}"));
        let statement = client
            .prepare(sql)
            .await
            .unwrap_or_else(|error| panic!("prepare profile relation {relation}: {error}"));
        assert_eq!(statement.columns().len(), expected.len(), "{relation}");
        for (actual, expected) in statement.columns().iter().zip(expected) {
            assert_eq!(
                actual.name(),
                expected["name"].as_str().unwrap(),
                "{relation}"
            );
            assert_eq!(
                actual.type_().oid(),
                u32::try_from(expected["type_oid"].as_u64().unwrap()).unwrap(),
                "{relation}.{}",
                actual.name()
            );
        }
    }

    let geometry_metadata = client
        .query_one(
            "SELECT f_table_catalog, f_table_schema, f_table_name, f_geometry_column, \
                    coord_dimension, srid, type FROM geometry_columns \
             WHERE f_table_name = 'catalog_projection' AND f_geometry_column = 'geom_wkb'",
            &[],
        )
        .await
        .expect("generic geometry metadata");
    assert_eq!(geometry_metadata.get::<_, String>(0), "quackgis");
    assert_eq!(geometry_metadata.get::<_, String>(1), "public");
    assert_eq!(geometry_metadata.get::<_, String>(2), "catalog_projection");
    assert_eq!(geometry_metadata.get::<_, String>(3), "geom_wkb");
    assert_eq!(geometry_metadata.get::<_, i32>(4), 2);
    assert_eq!(geometry_metadata.get::<_, i32>(5), 0);
    assert_eq!(geometry_metadata.get::<_, String>(6), "GEOMETRY");

    let simple_type_kind = client
        .simple_query("SELECT typtype FROM pg_type WHERE oid = 90001")
        .await
        .expect("simple-protocol PostgreSQL char output")
        .into_iter()
        .find_map(|message| match message {
            tokio_postgres::SimpleQueryMessage::Row(row) => row.get(0).map(str::to_owned),
            _ => None,
        })
        .expect("simple-protocol PostgreSQL char row");
    assert_eq!(simple_type_kind, "b");
    for column_name in ["geom_wkb", "native_geom"] {
        let information_schema_geometry = client
            .query_one(
                &format!(
                    "SELECT data_type, udt_schema, udt_name \
                 FROM information_schema.columns \
                 WHERE table_schema = 'public' AND table_name = 'catalog_projection' \
                   AND column_name = '{column_name}'"
                ),
                &[],
            )
            .await
            .unwrap_or_else(|error| panic!("information schema geometry {column_name}: {error:?}"));
        assert_eq!(
            information_schema_geometry.get::<_, String>(0),
            "USER-DEFINED"
        );
        assert_eq!(information_schema_geometry.get::<_, String>(1), "public");
        assert_eq!(information_schema_geometry.get::<_, String>(2), "geometry");
    }
    assert_eq!(
        client
            .query_one(
                "SELECT count(*)::BIGINT FROM information_schema.tables \
                 WHERE table_name IN ('geometry_columns', 'spatial_ref_sys')",
                &[],
            )
            .await
            .expect("spatial metadata relations are discoverable")
            .get::<_, i64>(0),
        2
    );
    assert!(
        client
            .query(
                "SELECT auth_name, auth_srid, srtext, proj4text \
                 FROM spatial_ref_sys WHERE srid = 4326",
                &[],
            )
            .await
            .expect("typed empty reference-system catalog")
            .is_empty()
    );
    let spatial_functions = client
        .query_one(
            "SELECT ST_SRID('POINT EMPTY'::GEOMETRY), \
                    postgis_geos_version(), postgis_proj_version()",
            &[],
        )
        .await
        .expect("bounded PostGIS metadata functions");
    assert_eq!(spatial_functions.get::<_, i32>(0), 0);
    assert_eq!(spatial_functions.get::<_, String>(1), "QUACKGIS-DUCKDB");
    assert!(!spatial_functions.get::<_, String>(2).is_empty());
    let extents = client
        .query_one(
            "SELECT ST_Extent(geom_wkb), ST_3DExtent(geom_wkb), \
                    ST_Extent(native_geom), ST_3DExtent(native_geom) \
             FROM public.catalog_projection",
            &[],
        )
        .await
        .expect("PostGIS extent compatibility");
    assert_eq!(extents.get::<_, String>(0), "BOX(0 0,2 3)");
    assert_eq!(extents.get::<_, String>(1), "BOX3D(0 0 5,2 3 9)");
    assert_eq!(extents.get::<_, String>(2), "BOX(0 0,2 3)");
    assert_eq!(extents.get::<_, String>(3), "BOX3D(0 0 5,2 3 9)");
    let row_srids = client
        .query_one(
            "SELECT ST_SRID(geom_wkb), ST_SRID(native_geom), \
                    ST_Zmflag(geom_wkb), GeometryType(geom_wkb) \
             FROM public.catalog_projection WHERE id = 8",
            &[],
        )
        .await
        .expect("maintained WKB and native geometry SRID compatibility");
    assert_eq!(row_srids.get::<_, i32>(0), 0);
    assert_eq!(row_srids.get::<_, i32>(1), 0);
    assert_eq!(row_srids.get::<_, i16>(2), 2);
    assert_eq!(row_srids.get::<_, String>(3), "POINT");

    let catalog_sql = "SELECT c.oid, c.relname, c.relnamespace, c.reltype, c.relowner, \
                c.relkind, c.relnatts, c.relrowsecurity, a.attrelid, a.attname, \
                a.atttypid, a.attlen, a.attnum, a.atttypmod, a.attnotnull, \
                a.attidentity, a.attgenerated, a.attisdropped, t.typname, \
                rt.typrelid, n.nspname \
         FROM pg_catalog.pg_namespace n \
         JOIN pg_catalog.pg_class c ON c.relnamespace = n.oid \
         JOIN pg_catalog.pg_attribute a ON a.attrelid = c.oid \
         JOIN pg_catalog.pg_type t ON t.oid = a.atttypid \
         JOIN pg_catalog.pg_type rt ON rt.oid = c.reltype \
         WHERE n.nspname = 'public' AND c.relname = 'catalog_projection' \
         ORDER BY a.attnum";
    let catalog = client
        .prepare(catalog_sql)
        .await
        .expect("prepare registry-backed catalog join");
    let catalog_types = [
        tokio_postgres::types::Type::OID,
        tokio_postgres::types::Type::NAME,
        tokio_postgres::types::Type::OID,
        tokio_postgres::types::Type::OID,
        tokio_postgres::types::Type::OID,
        tokio_postgres::types::Type::CHAR,
        tokio_postgres::types::Type::INT2,
        tokio_postgres::types::Type::BOOL,
        tokio_postgres::types::Type::OID,
        tokio_postgres::types::Type::NAME,
        tokio_postgres::types::Type::OID,
        tokio_postgres::types::Type::INT2,
        tokio_postgres::types::Type::INT2,
        tokio_postgres::types::Type::INT4,
        tokio_postgres::types::Type::BOOL,
        tokio_postgres::types::Type::CHAR,
        tokio_postgres::types::Type::CHAR,
        tokio_postgres::types::Type::BOOL,
        tokio_postgres::types::Type::NAME,
        tokio_postgres::types::Type::OID,
        tokio_postgres::types::Type::NAME,
    ];
    assert_eq!(catalog.columns().len(), catalog_types.len());
    for (column, expected) in catalog.columns().iter().zip(catalog_types) {
        assert_eq!(
            column.type_(),
            &expected,
            "catalog column {}",
            column.name()
        );
    }
    let rows = client
        .query(&catalog, &[])
        .await
        .expect("registry-backed catalog rows");
    assert_eq!(rows.len(), 6);
    let relation_oid = rows[0].get::<_, u32>(0);
    let row_type_oid = rows[0].get::<_, u32>(3);
    assert!(relation_oid >= 100_000);
    assert!(row_type_oid >= 100_000);
    assert_ne!(relation_oid, row_type_oid);

    let psql_resolve_sql = captured_trace_sql(
        include_str!("../../../tests/fixtures/psql_18_3_postgresql18_describe_trace.json"),
        "resolve_relation",
    )
    .replace(":table_name", "catalog_projection")
    .replace(":schema_name", "public");
    let psql_resolve = client
        .prepare(&psql_resolve_sql)
        .await
        .expect("prepare captured psql relation resolution");
    let psql_resolve_types = [
        tokio_postgres::types::Type::OID,
        tokio_postgres::types::Type::NAME,
        tokio_postgres::types::Type::NAME,
    ];
    for (column, expected) in psql_resolve.columns().iter().zip(psql_resolve_types) {
        assert_eq!(
            column.type_(),
            &expected,
            "psql relation field {}",
            column.name()
        );
    }
    let psql_relation = client
        .query_one(&psql_resolve, &[])
        .await
        .expect("captured psql relation resolution");
    assert_eq!(psql_relation.get::<_, u32>(0), relation_oid);
    assert_eq!(psql_relation.get::<_, String>(1), "public");
    assert_eq!(psql_relation.get::<_, String>(2), "catalog_projection");

    let psql_properties_sql = format!(
        "SELECT c.relchecks, c.relkind, c.relhasindex, c.relhasrules, \
         c.relhastriggers, c.relrowsecurity, c.relforcerowsecurity, \
         false AS relhasoids, c.relispartition, \
         pg_catalog.array_to_string(c.reloptions || array(SELECT 'toast.' || x \
         FROM pg_catalog.unnest(tc.reloptions) x), ', '), c.reltablespace, \
         CASE WHEN c.reloftype = 0 THEN '' ELSE \
         c.reloftype::pg_catalog.regtype::pg_catalog.text END, \
         c.relpersistence, c.relreplident, am.amname \
         FROM pg_catalog.pg_class c \
         LEFT JOIN pg_catalog.pg_class tc ON (c.reltoastrelid = tc.oid) \
         LEFT JOIN pg_catalog.pg_am am ON (c.relam = am.oid) \
         WHERE c.oid = '{relation_oid}'"
    );
    let psql_properties = client
        .prepare(&psql_properties_sql)
        .await
        .expect("prepare captured psql relation properties");
    let psql_property_types = [
        tokio_postgres::types::Type::INT2,
        tokio_postgres::types::Type::CHAR,
        tokio_postgres::types::Type::BOOL,
        tokio_postgres::types::Type::BOOL,
        tokio_postgres::types::Type::BOOL,
        tokio_postgres::types::Type::BOOL,
        tokio_postgres::types::Type::BOOL,
        tokio_postgres::types::Type::BOOL,
        tokio_postgres::types::Type::BOOL,
        tokio_postgres::types::Type::TEXT,
        tokio_postgres::types::Type::OID,
        tokio_postgres::types::Type::TEXT,
        tokio_postgres::types::Type::CHAR,
        tokio_postgres::types::Type::CHAR,
        tokio_postgres::types::Type::NAME,
    ];
    for (column, expected) in psql_properties.columns().iter().zip(psql_property_types) {
        assert_eq!(
            column.type_(),
            &expected,
            "psql relation property field {}",
            column.name()
        );
    }
    let properties = client
        .query_one(&psql_properties, &[])
        .await
        .expect("captured psql relation properties");
    assert_eq!(properties.get::<_, i16>(0), 0);
    assert_eq!(properties.get::<_, i8>(1), b'r' as i8);
    for index in 2..=8 {
        assert!(!properties.get::<_, bool>(index));
    }
    assert_eq!(properties.get::<_, Option<String>>(9), None);
    assert_eq!(properties.get::<_, u32>(10), 0);
    assert_eq!(properties.get::<_, String>(11), "");
    assert_eq!(properties.get::<_, i8>(12), b'p' as i8);
    assert_eq!(properties.get::<_, i8>(13), b'd' as i8);
    assert_eq!(properties.get::<_, String>(14), "ducklake");

    let psql_columns_sql = format!(
        "SELECT a.attname, pg_catalog.format_type(a.atttypid, a.atttypmod), \
         (SELECT pg_catalog.pg_get_expr(d.adbin, d.adrelid, true) \
         FROM pg_catalog.pg_attrdef d WHERE d.adrelid = a.attrelid \
         AND d.adnum = a.attnum AND a.atthasdef), a.attnotnull, \
         (SELECT c.collname FROM pg_catalog.pg_collation c, pg_catalog.pg_type t \
         WHERE c.oid = a.attcollation AND t.oid = a.atttypid \
         AND a.attcollation <> t.typcollation) AS attcollation, a.attidentity, \
         a.attgenerated, a.attstorage, a.attcompression AS attcompression, \
         CASE WHEN a.attstattarget = -1 THEN NULL ELSE a.attstattarget END \
         AS attstattarget, pg_catalog.col_description(a.attrelid, a.attnum) \
         FROM pg_catalog.pg_attribute a WHERE a.attrelid = '{relation_oid}' \
         AND a.attnum > 0 AND NOT a.attisdropped ORDER BY a.attnum"
    );
    let psql_columns = client
        .prepare(&psql_columns_sql)
        .await
        .expect("prepare captured psql column properties");
    let psql_column_types = [
        tokio_postgres::types::Type::NAME,
        tokio_postgres::types::Type::TEXT,
        tokio_postgres::types::Type::TEXT,
        tokio_postgres::types::Type::BOOL,
        tokio_postgres::types::Type::NAME,
        tokio_postgres::types::Type::CHAR,
        tokio_postgres::types::Type::CHAR,
        tokio_postgres::types::Type::CHAR,
        tokio_postgres::types::Type::CHAR,
        tokio_postgres::types::Type::INT2,
        tokio_postgres::types::Type::TEXT,
    ];
    for (column, expected) in psql_columns.columns().iter().zip(psql_column_types) {
        assert_eq!(
            column.type_(),
            &expected,
            "psql column property field {}",
            column.name()
        );
    }
    let column_properties = client
        .query(&psql_columns, &[])
        .await
        .expect("captured psql column properties");
    assert_eq!(column_properties.len(), 6);
    let id_properties = &column_properties[0];
    assert_eq!(id_properties.get::<_, String>(0), "id");
    assert_eq!(id_properties.get::<_, String>(1), "bigint");
    assert_eq!(
        id_properties.get::<_, Option<String>>(2).as_deref(),
        Some("'7'")
    );
    assert!(id_properties.get::<_, bool>(3));
    assert_eq!(id_properties.get::<_, Option<String>>(4), None);
    assert_eq!(id_properties.get::<_, i8>(5), 0);
    assert_eq!(id_properties.get::<_, i8>(6), 0);
    assert_eq!(id_properties.get::<_, i8>(7), b'p' as i8);
    assert_eq!(id_properties.get::<_, i8>(8), 0);
    assert_eq!(id_properties.get::<_, Option<i16>>(9), None);
    assert_eq!(
        id_properties.get::<_, Option<String>>(10).as_deref(),
        Some("stable identifier")
    );

    let expected = [
        ("id", 20_u32, 8_i16, true),
        ("label", 25, -1, false),
        ("geom_wkb", 90_001, -1, false),
        ("native_geom", 90_001, -1, false),
        ("score", 701, 8, false),
        ("active", 16, 1, false),
    ];
    for (index, (row, expected)) in rows.iter().zip(expected).enumerate() {
        assert_eq!(row.get::<_, u32>(0), relation_oid);
        assert_eq!(row.get::<_, String>(1), "catalog_projection");
        assert_eq!(row.get::<_, u32>(2), 2_200);
        assert_eq!(row.get::<_, u32>(3), row_type_oid);
        assert_eq!(row.get::<_, u32>(4), 110_001);
        assert_eq!(row.get::<_, i8>(5), b'r' as i8);
        assert_eq!(row.get::<_, i16>(6), 6);
        assert!(!row.get::<_, bool>(7));
        assert_eq!(row.get::<_, u32>(8), relation_oid);
        assert_eq!(row.get::<_, String>(9), expected.0);
        assert_eq!(row.get::<_, u32>(10), expected.1);
        assert_eq!(row.get::<_, i16>(11), expected.2);
        assert_eq!(row.get::<_, i16>(12), i16::try_from(index + 1).unwrap());
        assert_eq!(row.get::<_, i32>(13), -1);
        assert_eq!(row.get::<_, bool>(14), expected.3);
        assert!(!row.get::<_, bool>(17));
        assert_eq!(row.get::<_, u32>(19), relation_oid);
        assert_eq!(row.get::<_, String>(20), "public");
    }
    let not_null = client
        .prepare(
            "SELECT c.oid, c.conname, c.connamespace, c.contype, c.conrelid, \
                    c.conkey, pg_get_constraintdef(c.oid, true) \
             FROM pg_constraint c \
             WHERE c.conrelid = 'catalog_projection'::regclass",
        )
        .await
        .expect("prepare registry-backed not-null constraint");
    let not_null_types = [
        tokio_postgres::types::Type::OID,
        tokio_postgres::types::Type::NAME,
        tokio_postgres::types::Type::OID,
        tokio_postgres::types::Type::CHAR,
        tokio_postgres::types::Type::OID,
        tokio_postgres::types::Type::INT2_ARRAY,
        tokio_postgres::types::Type::TEXT,
    ];
    for (column, expected) in not_null.columns().iter().zip(not_null_types) {
        assert_eq!(column.type_(), &expected, "{}", column.name());
    }
    let not_null = client
        .query_one(&not_null, &[])
        .await
        .expect("registry-backed not-null constraint");
    let not_null_oid = not_null.get::<_, u32>(0);
    assert!(not_null_oid >= 100_000);
    assert_eq!(
        not_null.get::<_, String>(1),
        "catalog_projection_id_not_null"
    );
    assert_eq!(not_null.get::<_, u32>(2), 2_200);
    assert_eq!(not_null.get::<_, i8>(3), b'n' as i8);
    assert_eq!(not_null.get::<_, u32>(4), relation_oid);
    assert_eq!(not_null.get::<_, Vec<i16>>(5), vec![1]);
    assert_eq!(not_null.get::<_, String>(6), "NOT NULL id");
    let empty_indexes = client
        .prepare(
            "SELECT indexrelid, indrelid, indisprimary, indisunique, indkey, \
                    pg_get_indexdef(indexrelid, 0, true) AS definition \
             FROM pg_index WHERE indrelid = 'catalog_projection'::regclass",
        )
        .await
        .expect("prepare truthfully empty index catalog");
    let empty_index_types = [
        tokio_postgres::types::Type::OID,
        tokio_postgres::types::Type::OID,
        tokio_postgres::types::Type::BOOL,
        tokio_postgres::types::Type::BOOL,
        tokio_postgres::types::Type::INT2_VECTOR,
        tokio_postgres::types::Type::TEXT,
    ];
    for (column, expected) in empty_indexes.columns().iter().zip(empty_index_types) {
        assert_eq!(column.type_(), &expected, "{}", column.name());
    }
    assert!(
        client
            .query(&empty_indexes, &[])
            .await
            .expect("truthfully empty index catalog")
            .is_empty()
    );
    let structural_metadata = client
        .prepare(
            "SELECT d.adrelid, d.adnum, d.adbin, \
                    pg_get_expr(d.adbin, d.adrelid) AS default_expression, \
                    col_description(d.adrelid, d.adnum) AS column_description, \
                    obj_description(d.adrelid, 'pg_class') AS table_description \
             FROM pg_attrdef d WHERE d.adrelid = $1 ORDER BY d.adnum",
        )
        .await
        .expect("prepare default/comment projection");
    let structural_types = [
        tokio_postgres::types::Type::OID,
        tokio_postgres::types::Type::INT2,
        tokio_postgres::types::Type::PG_NODE_TREE,
        tokio_postgres::types::Type::TEXT,
        tokio_postgres::types::Type::TEXT,
        tokio_postgres::types::Type::TEXT,
    ];
    for (column, expected) in structural_metadata.columns().iter().zip(structural_types) {
        assert_eq!(
            column.type_(),
            &expected,
            "structural metadata field {}",
            column.name()
        );
    }
    let structural_metadata = client
        .query_one(&structural_metadata, &[&relation_oid])
        .await
        .expect("default/comment projection");
    assert_eq!(structural_metadata.get::<_, u32>(0), relation_oid);
    assert_eq!(structural_metadata.get::<_, i16>(1), 1);
    assert_eq!(structural_metadata.get::<_, PgNodeTree>(2).0, "'7'");
    assert_eq!(structural_metadata.get::<_, String>(3), "'7'");
    assert_eq!(structural_metadata.get::<_, String>(4), "stable identifier");
    assert_eq!(structural_metadata.get::<_, String>(5), "projected table");

    let qgis_attribute_sql = captured_trace_sql(
        include_str!("../../../tests/fixtures/qgis_3_44_postgresql18_trace.json"),
        "attribute_structure",
    )
    .replace(":relation_oid", &relation_oid.to_string());
    let qgis_attribute = client
        .prepare(&qgis_attribute_sql)
        .await
        .expect("prepare captured QGIS attribute structure query");
    let qgis_attribute_types = [
        tokio_postgres::types::Type::OID,
        tokio_postgres::types::Type::INT2,
        tokio_postgres::types::Type::TEXT,
        tokio_postgres::types::Type::TEXT,
        tokio_postgres::types::Type::TEXT,
        tokio_postgres::types::Type::OID,
        tokio_postgres::types::Type::INT4,
        tokio_postgres::types::Type::INT4,
        tokio_postgres::types::Type::CHAR,
        tokio_postgres::types::Type::CHAR,
    ];
    for (column, expected) in qgis_attribute.columns().iter().zip(qgis_attribute_types) {
        assert_eq!(column.type_(), &expected, "QGIS field {}", column.name());
    }
    let qgis_attributes = client
        .query(&qgis_attribute, &[])
        .await
        .expect("captured QGIS attribute structure query");
    assert_eq!(qgis_attributes.len(), 6);
    let qgis_id = qgis_attributes
        .iter()
        .find(|row| row.get::<_, i16>(1) == 1)
        .expect("QGIS identifier attribute");
    assert_eq!(qgis_id.get::<_, u32>(0), relation_oid);
    assert_eq!(qgis_id.get::<_, String>(2), "bigint");
    assert_eq!(qgis_id.get::<_, String>(3), "stable identifier");
    assert_eq!(qgis_id.get::<_, String>(4), "'7'");
    assert_eq!(qgis_id.get::<_, u32>(5), 20);
    assert_eq!(qgis_id.get::<_, i32>(6), 1);
    assert_eq!(qgis_id.get::<_, Option<i32>>(7), None);

    let ogr_column_sql = captured_trace_sql(
        include_str!("../../../tests/fixtures/ogr_3_11_5_postgresql18_trace.json"),
        "column_structure",
    )
    .replace(":relation_oid", &relation_oid.to_string());
    let ogr_column = client
        .prepare(&ogr_column_sql)
        .await
        .expect("prepare captured OGR column structure query");
    let ogr_column_types = [
        tokio_postgres::types::Type::NAME,
        tokio_postgres::types::Type::NAME,
        tokio_postgres::types::Type::INT2,
        tokio_postgres::types::Type::TEXT,
        tokio_postgres::types::Type::BOOL,
        tokio_postgres::types::Type::TEXT,
        tokio_postgres::types::Type::BOOL,
        tokio_postgres::types::Type::TEXT,
        tokio_postgres::types::Type::CHAR,
    ];
    for (column, expected) in ogr_column.columns().iter().zip(ogr_column_types) {
        assert_eq!(column.type_(), &expected, "OGR field {}", column.name());
    }
    let ogr_columns = client
        .query(&ogr_column, &[])
        .await
        .expect("captured OGR column structure query");
    assert_eq!(ogr_columns.len(), 6);
    let ogr_id = &ogr_columns[0];
    assert_eq!(ogr_id.get::<_, String>(0), "id");
    assert_eq!(ogr_id.get::<_, String>(1), "int8");
    assert_eq!(ogr_id.get::<_, i16>(2), 8);
    assert_eq!(ogr_id.get::<_, String>(3), "bigint");
    assert!(ogr_id.get::<_, bool>(4));
    assert_eq!(ogr_id.get::<_, String>(5), "'7'");
    assert_eq!(ogr_id.get::<_, Option<bool>>(6), None);
    assert_eq!(ogr_id.get::<_, String>(7), "stable identifier");

    let ogr_primary_key_sql = captured_trace_sql(
        include_str!("../../../tests/fixtures/ogr_3_11_5_postgresql18_trace.json"),
        "primary_key_columns",
    )
    .replace(":relation_oid", &relation_oid.to_string());
    let ogr_primary_key = client
        .prepare(&ogr_primary_key_sql)
        .await
        .expect("prepare captured OGR primary-key probe");
    let ogr_primary_key_types = [
        tokio_postgres::types::Type::NAME,
        tokio_postgres::types::Type::INT2,
        tokio_postgres::types::Type::NAME,
        tokio_postgres::types::Type::BOOL,
    ];
    for (column, expected) in ogr_primary_key.columns().iter().zip(ogr_primary_key_types) {
        assert_eq!(
            column.type_(),
            &expected,
            "OGR primary-key field {}",
            column.name()
        );
    }
    assert!(
        client
            .query(&ogr_primary_key, &[])
            .await
            .expect("truthfully empty captured OGR primary-key probe")
            .is_empty()
    );

    let descriptions = client
        .query(
            "SELECT objsubid, description FROM pg_description \
             WHERE objoid = 'catalog_projection'::regclass \
               AND classoid = 'pg_class'::regclass ORDER BY objsubid",
            &[],
        )
        .await
        .expect("direct description catalog");
    assert_eq!(descriptions.len(), 2);
    assert_eq!(descriptions[0].get::<_, i32>(0), 0);
    assert_eq!(descriptions[0].get::<_, String>(1), "projected table");
    assert_eq!(descriptions[1].get::<_, i32>(0), 1);
    assert_eq!(descriptions[1].get::<_, String>(1), "stable identifier");
    let private_relation_oid = client
        .query_one("SELECT to_regclass('private_metadata')::oid", &[])
        .await
        .expect("private metadata relation OID")
        .get::<_, u32>(0);

    let mut reader_config = tokio_postgres::Config::new();
    reader_config
        .host("127.0.0.1")
        .port(port)
        .user("reader")
        .password("reader-secret")
        .dbname("quackgis");
    let (reader, reader_connection) = reader_config
        .connect(tokio_postgres::NoTls)
        .await
        .expect("pinned metadata reader connection");
    let reader_connection = tokio::spawn(reader_connection);
    let reader_descriptions = reader
        .query_one(
            "SELECT obj_description(to_regclass('catalog_projection'), 'pg_class'), \
                    obj_description(to_regclass('private_metadata'), 'pg_class')",
            &[],
        )
        .await
        .expect("role and legacy-filtered structural metadata");
    assert_eq!(reader_descriptions.get::<_, String>(0), "projected table");
    assert_eq!(reader_descriptions.get::<_, Option<String>>(1), None);
    let private_qgis_attribute_sql = captured_trace_sql(
        include_str!("../../../tests/fixtures/qgis_3_44_postgresql18_trace.json"),
        "attribute_structure",
    )
    .replace(":relation_oid", &private_relation_oid.to_string());
    let private_qgis_attributes = reader
        .query(&private_qgis_attribute_sql, &[])
        .await
        .expect("filtered captured QGIS attribute query");
    assert_eq!(private_qgis_attributes.len(), 2);
    assert!(private_qgis_attributes.iter().all(|row| {
        row.get::<_, Option<String>>(3).is_none()
            && row.get::<_, Option<String>>(4).is_none()
            && row.get::<_, Option<i32>>(7).is_none()
    }));
    assert_eq!(
        reader
            .query_one(
                "SELECT count(*)::BIGINT FROM pg_attrdef \
                 WHERE adrelid = to_regclass('catalog_projection')",
                &[],
            )
            .await
            .expect("reader-visible defaults")
            .get::<_, i64>(0),
        1
    );
    assert_eq!(
        reader
            .query_one(
                "SELECT count(*)::BIGINT FROM pg_attrdef \
                 WHERE adrelid = to_regclass('private_metadata')",
                &[],
            )
            .await
            .expect("legacy-hidden defaults")
            .get::<_, i64>(0),
        0
    );
    assert_eq!(
        reader
            .query_one(
                "SELECT count(*)::BIGINT FROM geometry_columns \
                 WHERE f_table_name = 'catalog_projection'",
                &[],
            )
            .await
            .expect("reader-visible geometry metadata")
            .get::<_, i64>(0),
        2
    );
    assert_eq!(
        reader
            .query_one(
                "SELECT count(*)::BIGINT FROM geometry_columns \
                 WHERE f_table_name = 'private_metadata'",
                &[],
            )
            .await
            .expect("legacy-hidden geometry metadata")
            .get::<_, i64>(0),
        0
    );
    assert_eq!(
        reader
            .query_one(
                "SELECT count(*)::BIGINT FROM pg_constraint \
                 WHERE conrelid = to_regclass('catalog_projection')",
                &[],
            )
            .await
            .expect("reader-visible constraints")
            .get::<_, i64>(0),
        1
    );
    assert_eq!(
        reader
            .query_one(
                "SELECT count(*)::BIGINT FROM pg_constraint \
                 WHERE conrelid = to_regclass('private_metadata')",
                &[],
            )
            .await
            .expect("legacy-hidden constraints")
            .get::<_, i64>(0),
        0
    );
    for sql in [
        "SELECT count(*)::BIGINT FROM pg_type t \
         LEFT JOIN pg_type e ON e.oid = t.typelem \
         LEFT JOIN pg_type a ON a.oid = t.typarray \
         LEFT JOIN pg_namespace n ON n.oid = t.typnamespace \
         WHERE n.oid IS NULL OR (t.typelem <> 0 AND e.oid IS NULL) \
            OR (t.typarray <> 0 AND (a.oid IS NULL OR a.typelem <> t.oid))",
        "SELECT count(*)::BIGINT FROM pg_class c \
         LEFT JOIN pg_namespace n ON n.oid = c.relnamespace \
         LEFT JOIN pg_type t ON t.oid = c.reltype \
         LEFT JOIN pg_roles r ON r.oid = c.relowner \
         WHERE n.oid IS NULL OR t.oid IS NULL OR t.typrelid <> c.oid OR r.oid IS NULL",
        "SELECT count(*)::BIGINT FROM pg_attribute a \
         LEFT JOIN pg_class c ON c.oid = a.attrelid \
         LEFT JOIN pg_type t ON t.oid = a.atttypid \
         WHERE c.oid IS NULL OR t.oid IS NULL OR a.attnum <= 0 OR a.attnum > c.relnatts",
    ] {
        let unresolved = client
            .query_one(sql, &[])
            .await
            .unwrap_or_else(|error| panic!("catalog reference integrity {sql}: {error}"));
        assert_eq!(unresolved.get::<_, i64>(0), 0, "{sql}");
    }
    let analytics = client
        .query_one(
            "SELECT n.oid, c.oid, c.reltype, a.attnum FROM pg_namespace n \
             JOIN pg_class c ON c.relnamespace = n.oid \
             JOIN pg_attribute a ON a.attrelid = c.oid \
             WHERE n.nspname = 'analytics' AND c.relname = 'measurements'",
            &[],
        )
        .await
        .expect("non-public registry-backed catalog row");
    assert!(analytics.get::<_, u32>(0) >= 100_000);
    let analytics_relation_oid = analytics.get::<_, u32>(1);
    assert!(analytics_relation_oid >= 100_000);
    assert!(analytics.get::<_, u32>(2) >= 100_000);
    assert_eq!(analytics.get::<_, i16>(3), 1);

    let registered = client
        .prepare(
            "SELECT to_regclass('catalog_projection') AS relation_by_path, \
                    pg_catalog.to_regclass('\"public\".\"catalog_projection\"') AS relation_qualified, \
                    to_regclass('missing_relation') AS missing_relation, \
                    'catalog_projection'::regclass AS relation_cast, \
                    c.oid::pg_catalog.regclass AS relation_oid_cast, \
                    'integer'::regtype AS integer_type, \
                    to_regtype('geometry') AS geometry_type, \
                    to_regtype('integer[]') AS integer_array_type, \
                    to_regtype('catalog_projection') AS row_type, \
                    to_regtype('analytics.measurements') AS analytics_row_type, \
                    'public'::regnamespace AS namespace, \
                    'quackgis_owner'::regrole AS role, \
                    format_type(20, -1) AS bigint_name, \
                    pg_catalog.format_type(1700, 655366) AS numeric_name, \
                    format_type(90001, -1) AS geometry_name, \
                    format_type(c.reltype, -1) AS row_type_name, \
                    format_type(99999, -1) AS unknown_type_name \
             FROM pg_class c WHERE c.relname = 'catalog_projection'",
        )
        .await
        .expect("prepare registered-object resolution");
    let registered_types = [
        tokio_postgres::types::Type::REGCLASS,
        tokio_postgres::types::Type::REGCLASS,
        tokio_postgres::types::Type::REGCLASS,
        tokio_postgres::types::Type::REGCLASS,
        tokio_postgres::types::Type::REGCLASS,
        tokio_postgres::types::Type::REGTYPE,
        tokio_postgres::types::Type::REGTYPE,
        tokio_postgres::types::Type::REGTYPE,
        tokio_postgres::types::Type::REGTYPE,
        tokio_postgres::types::Type::REGTYPE,
        tokio_postgres::types::Type::REGNAMESPACE,
        tokio_postgres::types::Type::REGROLE,
        tokio_postgres::types::Type::TEXT,
        tokio_postgres::types::Type::TEXT,
        tokio_postgres::types::Type::TEXT,
        tokio_postgres::types::Type::TEXT,
        tokio_postgres::types::Type::TEXT,
    ];
    for (column, expected) in registered.columns().iter().zip(registered_types) {
        assert_eq!(
            column.type_(),
            &expected,
            "registered field {}",
            column.name()
        );
    }
    let registered = client
        .query_one(&registered, &[])
        .await
        .expect("registered-object resolution row");
    for column in [0, 1, 3, 4] {
        assert_eq!(
            registered.get::<_, RegisteredOid>(column),
            RegisteredOid(relation_oid),
            "registered relation column {column}"
        );
    }
    assert_eq!(registered.get::<_, Option<RegisteredOid>>(2), None);
    assert_eq!(registered.get::<_, RegisteredOid>(5), RegisteredOid(23));
    assert_eq!(registered.get::<_, RegisteredOid>(6), RegisteredOid(90_001));
    assert_eq!(registered.get::<_, RegisteredOid>(7), RegisteredOid(1007));
    assert_eq!(
        registered.get::<_, RegisteredOid>(8),
        RegisteredOid(row_type_oid)
    );
    assert_eq!(
        registered.get::<_, RegisteredOid>(9),
        RegisteredOid(analytics.get::<_, u32>(2))
    );
    assert_eq!(registered.get::<_, RegisteredOid>(10), RegisteredOid(2_200));
    assert_eq!(registered.get::<_, RegisteredOid>(11), RegisteredOid(10));
    assert_eq!(registered.get::<_, String>(12), "bigint");
    assert_eq!(registered.get::<_, String>(13), "numeric(10,2)");
    assert_eq!(registered.get::<_, String>(14), "geometry");
    assert_eq!(registered.get::<_, String>(15), "catalog_projection");
    assert_eq!(registered.get::<_, String>(16), "???");
    let function_cast = client
        .query_one(
            "SELECT regclass('\"public\".\"catalog_projection\"')::oid",
            &[],
        )
        .await
        .expect("function-form regclass OID cast");
    assert_eq!(function_cast.get::<_, u32>(0), relation_oid);
    let registered_text = client
        .query_one(
            "SELECT 'catalog_projection'::regclass::text, \
                    'integer'::regtype::text, \
                    'public'::regnamespace::pg_catalog.text, \
                    'quackgis_owner'::regrole::text",
            &[],
        )
        .await
        .expect("registered-object text output");
    assert_eq!(registered_text.get::<_, String>(0), "catalog_projection");
    assert_eq!(registered_text.get::<_, String>(1), "integer");
    assert_eq!(registered_text.get::<_, String>(2), "public");
    assert_eq!(registered_text.get::<_, String>(3), "quackgis_owner");
    let simple_registered_text = client
        .simple_query(
            "SELECT to_regclass('catalog_projection')::text, \
                    to_regtype('integer')::text",
        )
        .await
        .expect("simple-protocol registered-object text output")
        .into_iter()
        .find_map(|message| match message {
            tokio_postgres::SimpleQueryMessage::Row(row) => {
                Some((row.get(0).map(str::to_owned), row.get(1).map(str::to_owned)))
            }
            _ => None,
        })
        .expect("simple-protocol registered-object row");
    assert_eq!(
        simple_registered_text,
        (
            Some("catalog_projection".to_owned()),
            Some("integer".to_owned())
        )
    );
    let bound_registered = client
        .prepare_typed(
            "SELECT to_regclass($1), to_regtype($2)",
            &[
                tokio_postgres::types::Type::TEXT,
                tokio_postgres::types::Type::TEXT,
            ],
        )
        .await
        .expect("prepare bound registered-object lookup");
    let bound_registered = client
        .query_one(&bound_registered, &[&"catalog_projection", &"integer"])
        .await
        .expect("bound registered-object lookup");
    assert_eq!(
        bound_registered.get::<_, RegisteredOid>(0),
        RegisteredOid(relation_oid)
    );
    assert_eq!(
        bound_registered.get::<_, RegisteredOid>(1),
        RegisteredOid(23)
    );
    for (sql, expected_sqlstate) in [
        ("SELECT 'missing_relation'::regclass", "42P01"),
        ("SELECT 'missing_type'::regtype", "42704"),
        ("SELECT 'missing_schema'::regnamespace", "3F000"),
        ("SELECT 'missing_role'::regrole", "42704"),
    ] {
        let error = match client.query(sql, &[]).await {
            Ok(_) => panic!("strict registered-object lookup must fail: {sql}"),
            Err(error) => error,
        };
        assert_eq!(
            error.code().map(tokio_postgres::error::SqlState::code),
            Some(expected_sqlstate),
            "{sql}"
        );
    }
    let missing = client
        .query_one(
            "SELECT to_regclass('a.b.c'), to_regtype('missing_type'), \
                    to_regnamespace('missing_schema'), to_regrole('missing_role')",
            &[],
        )
        .await
        .expect("nullable registered-object lookups");
    for column in 0..4 {
        assert_eq!(missing.get::<_, Option<RegisteredOid>>(column), None);
    }

    let direct = client
        .prepare(
            "SELECT p.id, p.label AS renamed, p.id + 1 AS expression \
             FROM public.catalog_projection p",
        )
        .await
        .expect("prepare direct-column origins");
    assert_eq!(direct.columns()[0].table_oid(), Some(relation_oid));
    assert_eq!(direct.columns()[0].column_id(), Some(1));
    assert_eq!(direct.columns()[1].table_oid(), Some(relation_oid));
    assert_eq!(direct.columns()[1].column_id(), Some(2));
    assert_eq!(direct.columns()[2].table_oid(), None);
    assert_eq!(direct.columns()[2].column_id(), None);

    let joined = client
        .prepare(
            "SELECT p.id, m.value, p.id + m.value AS expression \
             FROM public.catalog_projection p \
             JOIN quackgis.analytics.measurements m ON true",
        )
        .await
        .expect("prepare joined direct-column origins");
    assert_eq!(joined.columns()[0].table_oid(), Some(relation_oid));
    assert_eq!(joined.columns()[0].column_id(), Some(1));
    assert_eq!(
        joined.columns()[1].table_oid(),
        Some(analytics_relation_oid)
    );
    assert_eq!(joined.columns()[1].column_id(), Some(1));
    assert_eq!(joined.columns()[2].table_oid(), None);
    assert_eq!(joined.columns()[2].column_id(), None);

    let wildcard = client
        .prepare("SELECT p.* FROM public.catalog_projection p")
        .await
        .expect("prepare wildcard origins");
    assert_eq!(wildcard.columns().len(), 6);
    for (index, column) in wildcard.columns().iter().enumerate() {
        assert_eq!(column.table_oid(), Some(relation_oid));
        assert_eq!(column.column_id(), Some(i16::try_from(index + 1).unwrap()));
    }

    execute_storage_update(
        &storage,
        "ALTER TABLE quackgis.main.catalog_projection RENAME COLUMN label TO title",
    )
    .await;
    execute_storage_update(
        &storage,
        "ALTER TABLE quackgis.main.catalog_projection RENAME TO catalog_projection_renamed",
    )
    .await;
    let stale = client
        .query(&direct, &[])
        .await
        .expect_err("schema epoch must invalidate the prepared statement");
    assert_eq!(
        stale.code(),
        Some(&tokio_postgres::error::SqlState::FEATURE_NOT_SUPPORTED)
    );
    let renamed = client
        .prepare("SELECT p.id, p.title FROM public.catalog_projection_renamed p")
        .await
        .expect("prepare renamed direct-column origins");
    assert_eq!(renamed.columns()[0].table_oid(), Some(relation_oid));
    assert_eq!(renamed.columns()[0].column_id(), Some(1));
    assert_eq!(renamed.columns()[1].table_oid(), Some(relation_oid));
    assert_eq!(renamed.columns()[1].column_id(), Some(2));

    let renamed_catalog = client
        .query(
            "SELECT c.oid, a.attname, a.attnum FROM pg_class c \
             JOIN pg_attribute a ON a.attrelid = c.oid \
             WHERE c.relname = 'catalog_projection_renamed' ORDER BY a.attnum",
            &[],
        )
        .await
        .expect("catalog rows after rename");
    assert_eq!(renamed_catalog.len(), 6);
    assert!(
        renamed_catalog
            .iter()
            .all(|row| row.get::<_, u32>(0) == relation_oid)
    );
    assert_eq!(renamed_catalog[1].get::<_, String>(1), "title");
    assert_eq!(renamed_catalog[1].get::<_, i16>(2), 2);
    assert_eq!(
        client
            .query_one(
                "SELECT oid FROM pg_constraint \
                 WHERE conrelid = 'catalog_projection_renamed'::regclass \
                   AND contype = 'n'",
                &[],
            )
            .await
            .expect("not-null identity after table rename")
            .get::<_, u32>(0),
        not_null_oid
    );
    let renamed_resolution = client
        .query_one(
            &format!(
                "SELECT to_regclass('catalog_projection'), \
                        to_regclass('catalog_projection_renamed'), \
                        to_regtype('catalog_projection'), \
                        to_regtype('catalog_projection_renamed'), \
                        format_type({row_type_oid}, -1)"
            ),
            &[],
        )
        .await
        .expect("registered-object resolution after rename");
    assert_eq!(renamed_resolution.get::<_, Option<RegisteredOid>>(0), None);
    assert_eq!(
        renamed_resolution.get::<_, RegisteredOid>(1),
        RegisteredOid(relation_oid)
    );
    assert_eq!(renamed_resolution.get::<_, Option<RegisteredOid>>(2), None);
    assert_eq!(
        renamed_resolution.get::<_, RegisteredOid>(3),
        RegisteredOid(row_type_oid)
    );
    assert_eq!(
        renamed_resolution.get::<_, String>(4),
        "catalog_projection_renamed"
    );

    execute_storage_update(
        &storage,
        "ALTER TABLE quackgis.main.catalog_projection_renamed DROP COLUMN active",
    )
    .await;
    execute_storage_update(
        &storage,
        "ALTER TABLE quackgis.main.catalog_projection_renamed ADD COLUMN active BOOLEAN",
    )
    .await;
    let readded = client
        .query_one(
            "SELECT c.relnatts, a.attnum FROM pg_class c \
             JOIN pg_attribute a ON a.attrelid = c.oid \
             WHERE c.relname = 'catalog_projection_renamed' AND a.attname = 'active'",
            &[],
        )
        .await
        .expect("catalog tombstone attribute numbering");
    assert_eq!(readded.get::<_, i16>(0), 7);
    assert_eq!(readded.get::<_, i16>(1), 7);

    execute_storage_update(
        &storage,
        "CREATE TABLE quackgis.main.unsupported_catalog_type(payload STRUCT(x INTEGER))",
    )
    .await;
    client
        .query(
            "SELECT a.atttypid FROM pg_attribute a JOIN pg_class c ON c.oid = a.attrelid \
             WHERE c.relname = 'unsupported_catalog_type'",
            &[],
        )
        .await
        .expect_err("unsupported user-column type must fail closed");
    execute_storage_update(
        &storage,
        "DROP TABLE quackgis.main.unsupported_catalog_type",
    )
    .await;

    execute_storage_update(
        &storage,
        "DROP TABLE quackgis.main.catalog_projection_renamed",
    )
    .await;
    execute_storage_update(
        &storage,
        "CREATE TABLE quackgis.main.catalog_projection_renamed(id BIGINT)",
    )
    .await;
    let recreated = client
        .query_one(
            "SELECT c.oid, c.reltype, a.attnum FROM pg_class c \
             JOIN pg_attribute a ON a.attrelid = c.oid \
             WHERE c.relname = 'catalog_projection_renamed'",
            &[],
        )
        .await
        .expect("recreated catalog identity");
    assert_ne!(recreated.get::<_, u32>(0), relation_oid);
    assert_ne!(recreated.get::<_, u32>(1), row_type_oid);
    assert_eq!(recreated.get::<_, i16>(2), 1);
    let recreated_resolution = client
        .query_one(
            "SELECT to_regclass('catalog_projection_renamed'), \
                    to_regtype('catalog_projection_renamed')",
            &[],
        )
        .await
        .expect("registered-object resolution after recreate");
    assert_eq!(
        recreated_resolution.get::<_, RegisteredOid>(0),
        RegisteredOid(recreated.get::<_, u32>(0))
    );
    assert_eq!(
        recreated_resolution.get::<_, RegisteredOid>(1),
        RegisteredOid(recreated.get::<_, u32>(1))
    );

    let before_security_change = storage
        .catalog_epochs()
        .expect("epochs before security change")
        .expect("pinned catalog epochs");
    let changed_role_catalog = RoleCatalog::from_json(
        r#"{
          "roles": [
            {"oid": 110001, "name": "writer", "login": true},
            {"oid": 110002, "name": "reader", "login": true}
          ],
          "table_owners": [
            {"table": "catalog_projection", "role": "writer"},
            {"table": "catalog_projection_renamed", "role": "writer"},
            {"table": "private_metadata", "role": "writer"}
          ],
          "schema_grants": [
            {"schema": "public", "role": "PUBLIC", "privileges": ["USAGE"]}
          ],
          "table_grants": [
            {"table": "catalog_projection", "role": "reader", "privileges": ["SELECT"]},
            {"table": "private_metadata", "role": "reader", "privileges": ["SELECT", "DELETE"]}
          ]
        }"#,
    )
    .expect("changed pinned metadata role catalog");
    storage
        .install_role_catalog(&changed_role_catalog, &epoch_auth)
        .expect("publish changed security catalogs");
    let after_security_change = storage
        .catalog_epochs()
        .expect("epochs after security change")
        .expect("pinned catalog epochs");
    assert_eq!(
        after_security_change,
        quackgis_server::duckdb_adbc_storage::CatalogEpochs {
            schema: before_security_change.schema,
            security: before_security_change.security + 1,
        }
    );
    let published_security = client
        .query_one(
            "SELECT quackgis_schema_epoch(), quackgis_security_epoch()",
            &[],
        )
        .await
        .expect("published security epochs");
    assert_eq!(
        published_security.get::<_, i64>(0),
        after_security_change.schema as i64
    );
    assert_eq!(
        published_security.get::<_, i64>(1),
        after_security_change.security as i64
    );

    reader_connection.abort();
    connection.abort();
    server.abort();
}

async fn execute_storage_update(storage: &Arc<DuckDbAdbcStorage>, sql: &'static str) {
    let storage = Arc::clone(storage);
    tokio::task::spawn_blocking(move || storage.execute_update(sql))
        .await
        .expect("pinned catalog DDL worker")
        .unwrap_or_else(|error| panic!("pinned catalog DDL {sql}: {error}"));
}

#[test]
#[ignore = "requires an explicitly selected supported pinned extension"]
fn pinned_ducklake_column_identity_contract() {
    let driver_path =
        std::env::var_os("QUACKGIS_DUCKDB_ADBC_DRIVER").expect("set QUACKGIS_DUCKDB_ADBC_DRIVER");
    let extension_path =
        std::env::var_os("QUACKGIS_DUCKLAKE_EXTENSION").expect("set QUACKGIS_DUCKLAKE_EXTENSION");
    let extension_sha256 = std::env::var("QUACKGIS_DUCKLAKE_EXTENSION_SHA256")
        .expect("set QUACKGIS_DUCKLAKE_EXTENSION_SHA256");
    let temp = tempfile::tempdir().expect("temporary DuckLake root");
    let data_path = temp.path().join("data");
    std::fs::create_dir(&data_path).expect("DuckLake data directory");
    let config = DuckDbAdbcConfig {
        driver_path: driver_path.into(),
        database_uri: ":memory:".to_owned(),
        ducklake_uri: format!(
            "ducklake:{}",
            temp.path().join("catalog.ducklake").display()
        ),
        catalog_name: "quackgis".to_owned(),
        data_path: data_path.display().to_string(),
        extension_policy: ExtensionPolicy::PinnedDuckLake {
            path: extension_path.into(),
            sha256: extension_sha256,
        },
    };
    let storage = Arc::new(DuckDbAdbcStorage::open(config.clone()).expect("open pinned DuckLake"));

    let description = storage
        .describe("SELECT * FROM ducklake_column_info('quackgis')")
        .expect("describe column identity function");
    assert_eq!(
        description
            .result_schema
            .fields()
            .iter()
            .map(|field| field.name().as_str())
            .collect::<Vec<_>>(),
        [
            "schema_name",
            "schema_id",
            "schema_uuid",
            "table_name",
            "table_id",
            "table_uuid",
            "column_name",
            "column_id",
        ]
    );

    storage
        .start_update_operation()
        .expect("start catalog DDL operation")
        .execute(
            "CREATE TABLE quackgis.main.identity_probe(\
             id BIGINT, label VARCHAR, payload STRUCT(x INTEGER))",
            None,
        )
        .expect("create empty identity table");
    storage
        .execute_update("CREATE VIEW quackgis.main.identity_view AS SELECT 1 AS id")
        .expect("create excluded view");
    let initial = identity_rows(
        &storage
            .query(COLUMN_IDENTITY_SQL)
            .expect("initial column identities"),
    );
    assert_eq!(initial.len(), 3);
    assert!(initial.iter().all(|row| row.schema_name == "main"));
    assert!(initial.iter().all(|row| row.table_name == "identity_probe"));
    assert_eq!(
        initial
            .iter()
            .map(|row| (row.column_name.as_str(), row.column_id))
            .collect::<Vec<_>>(),
        [("id", 1), ("label", 2), ("payload", 3)]
    );
    assert!(
        initial
            .iter()
            .all(|row| row.table_id == initial[0].table_id)
    );
    assert!(
        initial
            .iter()
            .all(|row| row.table_uuid == initial[0].table_uuid)
    );
    let initial_registry = registered_columns(&storage, "main", "identity_probe");
    assert_eq!(
        initial_registry
            .iter()
            .map(|row| (row.2, row.3))
            .collect::<Vec<_>>(),
        [(1, 1), (2, 2), (3, 3)]
    );
    assert!(initial_registry.iter().all(|row| row.0 == 2_200));
    assert!(initial_registry.iter().all(|row| row.1 >= 100_000));
    assert!(
        initial_registry
            .iter()
            .all(|row| row.1 == initial_registry[0].1)
    );
    let (initial_next_oid, initial_epoch) = registry_state(&storage);
    let row_type = storage
        .query(&format!(
            "SELECT CAST(row_type_oid AS BIGINT) \
             FROM quackgis._quackgis.relation_oid WHERE oid = {}",
            initial_registry[0].1
        ))
        .expect("reserved relation row type OID");
    assert_eq!(first_i64(&row_type, 0), initial_registry[0].1 + 1);
    assert_eq!(initial_next_oid, initial_registry[0].1 + 2);
    assert_eq!(initial_epoch, 1);

    storage
        .begin_transaction()
        .expect("begin rename transaction");
    storage
        .start_update_operation()
        .expect("start table rename")
        .execute(
            "ALTER TABLE quackgis.main.identity_probe RENAME TO identity_renamed",
            None,
        )
        .expect("rename table in transaction");
    storage
        .start_update_operation()
        .expect("start column rename")
        .execute(
            "ALTER TABLE quackgis.main.identity_renamed RENAME COLUMN label TO title",
            None,
        )
        .expect("rename column in transaction");
    storage
        .commit_transaction()
        .expect("commit supported renames");
    let renamed = identity_rows(
        &storage
            .query(COLUMN_IDENTITY_SQL)
            .expect("renamed identities"),
    );
    assert_eq!(
        renamed
            .iter()
            .map(|row| (row.column_name.as_str(), row.column_id))
            .collect::<Vec<_>>(),
        [("id", 1), ("title", 2), ("payload", 3)]
    );
    for (before, after) in initial.iter().zip(&renamed) {
        assert_eq!(before.schema_id, after.schema_id);
        assert_eq!(before.schema_uuid, after.schema_uuid);
        assert_eq!(before.table_id, after.table_id);
        assert_eq!(before.table_uuid, after.table_uuid);
        assert_eq!(before.column_id, after.column_id);
    }
    assert_eq!(
        registered_columns(&storage, "main", "identity_renamed"),
        initial_registry
    );
    let renamed_state = registry_state(&storage);
    assert_eq!(renamed_state, (initial_next_oid, initial_epoch + 1));

    let rollback = storage.transaction::<()>(|transaction| {
        transaction
            .execute_update("ALTER TABLE quackgis.main.identity_renamed RENAME TO must_rollback")?;
        let pinned = identity_rows(&transaction.query(COLUMN_IDENTITY_SQL)?);
        assert!(
            pinned
                .iter()
                .all(|row| row.table_name == "identity_renamed")
        );
        anyhow::bail!("intentional identity rollback")
    });
    assert!(rollback.is_err());
    assert_eq!(
        identity_rows(
            &storage
                .query(COLUMN_IDENTITY_SQL)
                .expect("identities after rollback")
        ),
        renamed
    );
    assert_eq!(registry_state(&storage), renamed_state);
    assert_eq!(
        registered_columns(&storage, "main", "identity_renamed"),
        initial_registry
    );

    storage
        .execute_update("ALTER TABLE quackgis.main.identity_renamed ADD COLUMN added BOOLEAN")
        .expect("add committed column");
    let with_added = identity_rows(
        &storage
            .query(COLUMN_IDENTITY_SQL)
            .expect("identities with new column"),
    );
    assert_eq!(
        with_added.last().expect("added column").column_name,
        "added"
    );
    let added = with_added.last().expect("added column");
    assert!(added.column_id > renamed.iter().map(|row| row.column_id).max().unwrap());
    assert!(
        renamed
            .iter()
            .all(|existing| existing.column_id != added.column_id)
    );
    let added_registry = registered_columns(&storage, "main", "identity_renamed");
    assert_eq!(&added_registry[..3], initial_registry.as_slice());
    assert_eq!(added_registry[3].2, added.column_id);
    assert_eq!(added_registry[3].3, 4);
    let added_state = registry_state(&storage);
    assert_eq!(added_state, (initial_next_oid, renamed_state.1 + 1));

    storage
        .execute_update("ALTER TABLE quackgis.main.identity_renamed DROP COLUMN added")
        .expect("drop added column");
    storage
        .execute_update("ALTER TABLE quackgis.main.identity_renamed ADD COLUMN added BOOLEAN")
        .expect("recreate added column");
    let with_readded = identity_rows(
        &storage
            .query(COLUMN_IDENTITY_SQL)
            .expect("identities with recreated column"),
    );
    let readded = with_readded.last().expect("recreated column");
    assert_eq!(readded.column_name, "added");
    assert_ne!(readded.column_id, added.column_id);
    let readded_registry = registered_columns(&storage, "main", "identity_renamed");
    assert_eq!(&readded_registry[..3], initial_registry.as_slice());
    assert_eq!(readded_registry[3].2, readded.column_id);
    assert_eq!(readded_registry[3].3, 5);
    let readded_state = registry_state(&storage);
    assert_eq!(readded_state, (initial_next_oid, added_state.1 + 2));
    drop(storage);

    let reopened =
        Arc::new(DuckDbAdbcStorage::open(config.clone()).expect("reopen pinned DuckLake"));
    assert_eq!(
        identity_rows(
            &reopened
                .query(COLUMN_IDENTITY_SQL)
                .expect("reopened identities")
        ),
        with_readded
    );
    assert_eq!(registry_state(&reopened), readded_state);
    assert_eq!(
        registered_columns(&reopened, "main", "identity_renamed"),
        readded_registry
    );
    let old_table_id = with_readded[0].table_id;
    let old_table_uuid = with_readded[0].table_uuid.clone();
    let old_relation_oid = readded_registry[0].1;
    reopened
        .execute_update("DROP TABLE quackgis.main.identity_renamed")
        .expect("drop identity table");
    reopened
        .execute_update("CREATE TABLE quackgis.main.identity_renamed(id BIGINT)")
        .expect("recreate identity table");
    let recreated = identity_rows(
        &reopened
            .query(COLUMN_IDENTITY_SQL)
            .expect("recreated identities"),
    );
    assert_eq!(recreated.len(), 1);
    assert_ne!(recreated[0].table_id, old_table_id);
    assert_ne!(recreated[0].table_uuid, old_table_uuid);
    let recreated_registry = registered_columns(&reopened, "main", "identity_renamed");
    assert_eq!(recreated_registry.len(), 1);
    assert_eq!(recreated_registry[0].0, 2_200);
    assert_ne!(recreated_registry[0].1, old_relation_oid);
    assert_eq!(recreated_registry[0].3, 1);
    assert_eq!(registry_state(&reopened).1, readded_state.1 + 2);
    let mappings = reopened
        .query(
            "SELECT CAST(count(*) AS BIGINT) \
             FROM quackgis._quackgis.relation_oid",
        )
        .expect("retained relation mappings");
    assert_eq!(first_i64(&mappings, 0), 2);
    let attribute_mappings = reopened
        .query(&format!(
            "SELECT CAST(count(*) AS BIGINT) \
             FROM quackgis._quackgis.attribute_number \
             WHERE table_uuid = CAST('{}' AS UUID)",
            old_table_uuid.replace('\'', "''")
        ))
        .expect("retained attribute mappings");
    assert_eq!(first_i64(&attribute_mappings, 0), 5);

    EngineStorageKernel::execute_update_contract(
        reopened.as_ref(),
        "CREATE TABLE quackgis.main.contract_identity(flag BOOLEAN)",
    )
    .expect("contract DDL reconciliation");
    assert_eq!(
        registered_columns(&reopened, "main", "contract_identity").len(),
        1
    );

    let ingest_batch = || {
        RecordBatch::try_new(
            Arc::new(Schema::new(vec![Field::new(
                "value",
                DataType::Int32,
                false,
            )])),
            vec![Arc::new(Int32Array::from(vec![1]))],
        )
        .expect("ingest identity batch")
    };
    reopened
        .ingest(
            "main",
            "inherent_ingest_identity",
            vec![ingest_batch()],
            IngestMode::Create,
        )
        .expect("inherent ingest reconciliation");
    assert_eq!(
        registered_columns(&reopened, "main", "inherent_ingest_identity").len(),
        1
    );
    EngineStorageKernel::ingest_contract(
        reopened.as_ref(),
        &EngineTableRef {
            catalog: "quackgis".to_owned(),
            schema: "main".to_owned(),
            table: "contract_ingest_identity".to_owned(),
        },
        vec![ingest_batch()],
        IngestDisposition::Create,
    )
    .expect("contract ingest reconciliation");
    assert_eq!(
        registered_columns(&reopened, "main", "contract_ingest_identity").len(),
        1
    );

    let before_schema = registry_state(&reopened);
    reopened
        .transaction(|transaction| {
            transaction.execute_update("CREATE SCHEMA quackgis.analytics")?;
            transaction
                .execute_update("CREATE TABLE quackgis.analytics.measurements(value DOUBLE)")?;
            Ok(())
        })
        .expect("commit schema and table together");
    let analytics = registered_columns(&reopened, "analytics", "measurements");
    assert_eq!(analytics.len(), 1);
    assert!(analytics[0].0 >= 100_000);
    assert!(analytics[0].1 >= 100_000);
    assert_ne!(analytics[0].0, analytics[0].1);
    assert_eq!((analytics[0].2, analytics[0].3), (1, 1));
    let after_schema = registry_state(&reopened);
    assert_eq!(after_schema.0, before_schema.0 + 3);
    assert_eq!(after_schema.1, before_schema.1 + 1);

    let session_a = Arc::new(reopened.open_session().expect("first concurrent session"));
    let session_b = Arc::new(reopened.open_session().expect("second concurrent session"));
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let create =
        |storage: Arc<DuckDbAdbcStorage>, barrier: Arc<std::sync::Barrier>, table: &'static str| {
            std::thread::spawn(move || {
                barrier.wait();
                storage.execute_update(&format!("CREATE TABLE quackgis.main.{table}(id BIGINT)"))
            })
        };
    let writer_a = create(
        Arc::clone(&session_a),
        Arc::clone(&barrier),
        "concurrent_identity_a",
    );
    let writer_b = create(session_b, barrier, "concurrent_identity_b");
    writer_a
        .join()
        .expect("first writer thread")
        .expect("first serialized catalog commit");
    writer_b
        .join()
        .expect("second writer thread")
        .expect("second serialized catalog commit");
    assert_eq!(
        registered_columns(&reopened, "main", "concurrent_identity_a").len(),
        1
    );
    assert_eq!(
        registered_columns(&reopened, "main", "concurrent_identity_b").len(),
        1
    );
    let missing = reopened
        .query(
            "SELECT CAST(count(*) AS BIGINT) FROM (\
               SELECT DISTINCT i.table_uuid \
               FROM ducklake_column_info('quackgis') i \
               LEFT JOIN quackgis._quackgis.relation_oid r USING (table_uuid) \
               WHERE i.schema_name <> '_quackgis' AND r.oid IS NULL\
             ) missing",
        )
        .expect("complete concurrent registry coverage");
    assert_eq!(first_i64(&missing, 0), 0);

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("pinned catalog Tokio runtime")
        .block_on(prove_registry_catalog_pgwire(Arc::clone(&reopened)));

    let corruption = reopened.transaction(|transaction| {
        transaction.execute_update(
            "INSERT INTO quackgis._quackgis.relation_oid \
             SELECT * FROM quackgis._quackgis.relation_oid LIMIT 1",
        )?;
        Ok(())
    });
    let corruption = corruption.expect_err("duplicate registry key must fail closed");
    assert!(corruption.to_string().contains("commit succeeded"));
    assert_eq!(
        reopened.transaction_state(),
        EngineTransactionState::Quarantined
    );
    drop(reopened);
    match DuckDbAdbcStorage::open(config) {
        Ok(_) => panic!("startup must reject a corrupt identity registry"),
        Err(error) => assert!(error.to_string().contains("catalog identity")),
    }
}
