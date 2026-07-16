// SPDX-License-Identifier: Apache-2.0
use bytes::Bytes;
use futures::SinkExt;
use quackgis_server::pgwire_server::ServerOptions;
use serde::Deserialize;

#[path = "support/runtime.rs"]
mod runtime;
use runtime::TestRuntime;

#[derive(Debug, Deserialize)]
struct CatalogContract {
    schema_version: u32,
    setup_sql: String,
    copy_sql: String,
    copy_row: String,
    spatial_type_lookup: SpatialTypeLookup,
    geometry: GeometryContract,
    geography: GeometryContract,
    metadata_queries: Vec<MetadataQuery>,
    ordinary_catalog_query: OrdinaryCatalogQuery,
    builtin_type_query: BuiltinTypeQuery,
    type_catalog_integrity: TypeCatalogIntegrity,
    session_discovery: SessionDiscovery,
}

#[derive(Debug, Deserialize)]
struct SpatialTypeLookup {
    sql: String,
    cases: Vec<SpatialTypeCase>,
}

#[derive(Debug, Deserialize)]
struct SpatialTypeCase {
    oid: u32,
    expected_type_name: Option<String>,
    expected_type_kind: Option<i8>,
    expected_element_oid: Option<u32>,
    expected_range_subtype_oid: Option<u32>,
    expected_base_oid: Option<u32>,
    expected_namespace: Option<String>,
    expected_relation_oid: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct GeometryContract {
    query: String,
    null_query: String,
    text_query: String,
    expected_oid: u32,
    expected_type_name: String,
    expected_hex: String,
    expected_text: String,
}

#[derive(Debug, Deserialize)]
struct MetadataQuery {
    sql: String,
    expected_rows: Vec<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct OrdinaryCatalogQuery {
    sql: String,
    expected_rows: Vec<OrdinaryTypeRow>,
}

#[derive(Debug, Deserialize)]
struct OrdinaryTypeRow {
    oid: u32,
    type_name: String,
    namespace_oid: u32,
    type_kind: i8,
    delimiter: i8,
    array_oid: u32,
}

#[derive(Debug, Deserialize)]
struct BuiltinTypeQuery {
    sql: String,
    expected_rows: Vec<BuiltinTypeRow>,
}

#[derive(Debug, Deserialize)]
struct BuiltinTypeRow {
    oid: u32,
    type_name: String,
    type_kind: i8,
    element_oid: u32,
    type_length: i16,
}

#[derive(Debug, Deserialize)]
struct TypeCatalogIntegrity {
    count_sql: String,
    expected_count: i64,
    unresolved_queries: Vec<String>,
    expected_unresolved: i64,
}

#[derive(Debug, Deserialize)]
struct SessionDiscovery {
    identity_sql: String,
    implicit_schemas_sql: String,
    explicit_schemas_sql: String,
    database_sql: String,
    expected_database_oid: u32,
    expected_database: String,
    expected_schema: String,
    expected_implicit_schemas: Vec<String>,
    expected_explicit_schemas: Vec<String>,
    expected_owner: String,
}

#[derive(Debug, Eq, PartialEq)]
struct GeometryBytes(Vec<u8>);

#[derive(Debug, Eq, PartialEq)]
struct GeographyBytes(Vec<u8>);

impl<'a> tokio_postgres::types::FromSql<'a> for GeometryBytes {
    fn from_sql(
        _ty: &tokio_postgres::types::Type,
        raw: &'a [u8],
    ) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        Ok(Self(raw.to_vec()))
    }

    fn accepts(ty: &tokio_postgres::types::Type) -> bool {
        ty.oid() == 90_001
    }
}

impl<'a> tokio_postgres::types::FromSql<'a> for GeographyBytes {
    fn from_sql(
        _ty: &tokio_postgres::types::Type,
        raw: &'a [u8],
    ) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        Ok(Self(raw.to_vec()))
    }

    fn accepts(ty: &tokio_postgres::types::Type) -> bool {
        ty.oid() == 90_002
    }
}

#[tokio::test]
#[ignore = "requires the pinned DuckDB ADBC runtime"]
async fn client_neutral_catalog_contract() {
    let contract: CatalogContract = serde_json::from_str(include_str!(
        "../../../tests/fixtures/duckdb_catalog_contract.json"
    ))
    .expect("catalog contract fixture");
    assert_eq!(contract.schema_version, 6);

    let runtime = TestRuntime::start(ServerOptions::new().with_max_connections(4)).await;
    let _storage = runtime.storage();
    let (client, connection) = runtime.connect().await;
    client
        .batch_execute(&contract.setup_sql)
        .await
        .expect("catalog fixture setup");
    let sink: tokio_postgres::CopyInSink<Bytes> = client
        .copy_in(&contract.copy_sql)
        .await
        .expect("catalog fixture COPY");
    let mut sink = Box::pin(sink);
    sink.as_mut()
        .send(Bytes::from(contract.copy_row.clone()))
        .await
        .expect("catalog fixture row");
    assert_eq!(
        sink.as_mut().finish().await.expect("finish fixture COPY"),
        1
    );

    let type_lookup = client
        .prepare(&contract.spatial_type_lookup.sql)
        .await
        .expect("prepare relational spatial type lookup");
    assert_eq!(type_lookup.params(), &[tokio_postgres::types::Type::OID]);
    let typed_lookup = client
        .prepare_typed(
            &contract.spatial_type_lookup.sql,
            &[tokio_postgres::types::Type::OID],
        )
        .await
        .expect("prepare custom type lookup with explicit OID parameter");
    assert_eq!(typed_lookup.params(), &[tokio_postgres::types::Type::OID]);
    assert_eq!(
        client
            .query_one(&typed_lookup, &[&90_001_u32])
            .await
            .expect("execute explicit OID parameter")
            .get::<_, String>(0),
        "geometry"
    );
    let expected_lookup_columns = [
        ("typname", tokio_postgres::types::Type::NAME),
        ("typtype", tokio_postgres::types::Type::CHAR),
        ("typelem", tokio_postgres::types::Type::OID),
        ("rngsubtype", tokio_postgres::types::Type::OID),
        ("typbasetype", tokio_postgres::types::Type::OID),
        ("nspname", tokio_postgres::types::Type::NAME),
        ("typrelid", tokio_postgres::types::Type::OID),
    ];
    assert_eq!(type_lookup.columns().len(), expected_lookup_columns.len());
    for (column, (expected_name, expected_type)) in
        type_lookup.columns().iter().zip(expected_lookup_columns)
    {
        assert_eq!(column.name(), expected_name);
        assert_eq!(column.type_(), &expected_type, "{expected_name}");
    }

    for case in &contract.spatial_type_lookup.cases {
        let rows = client
            .query(&type_lookup, &[&case.oid])
            .await
            .unwrap_or_else(|error| panic!("spatial type lookup {}: {error}", case.oid));
        match &case.expected_type_name {
            Some(expected) => {
                assert_eq!(rows.len(), 1, "spatial type OID {}", case.oid);
                assert_eq!(rows[0].get::<_, String>(0), *expected);
                assert_eq!(rows[0].get::<_, i8>(1), case.expected_type_kind.unwrap());
                assert_eq!(rows[0].get::<_, u32>(2), case.expected_element_oid.unwrap());
                assert_eq!(
                    rows[0].get::<_, Option<u32>>(3),
                    case.expected_range_subtype_oid
                );
                assert_eq!(rows[0].get::<_, u32>(4), case.expected_base_oid.unwrap());
                assert_eq!(
                    rows[0].get::<_, String>(5),
                    *case.expected_namespace.as_ref().unwrap()
                );
                assert_eq!(
                    rows[0].get::<_, u32>(6),
                    case.expected_relation_oid.unwrap()
                );
            }
            None => assert!(rows.is_empty(), "unknown OID {}", case.oid),
        }
    }

    let statement = client
        .prepare(&contract.geometry.query)
        .await
        .expect("geometry RowDescription");
    assert_eq!(statement.columns().len(), 1);
    assert_eq!(
        statement.columns()[0].type_().oid(),
        contract.geometry.expected_oid
    );
    assert_eq!(
        statement.columns()[0].type_().name(),
        contract.geometry.expected_type_name
    );
    let row = client
        .query_one(&statement, &[])
        .await
        .expect("geometry binary row");
    assert_eq!(
        row.get::<_, GeometryBytes>(0).0,
        decode_hex(&contract.geometry.expected_hex)
    );
    let null = client
        .query_one(&contract.geometry.null_query, &[])
        .await
        .expect("geometry NULL row");
    assert_eq!(null.get::<_, Option<GeometryBytes>>(0), None);
    let text = client
        .simple_query(&contract.geometry.text_query)
        .await
        .expect("geometry text row")
        .into_iter()
        .find_map(|message| match message {
            tokio_postgres::SimpleQueryMessage::Row(row) => row.get(0).map(str::to_owned),
            _ => None,
        })
        .expect("geometry text value");
    assert_eq!(text, contract.geometry.expected_text);

    let statement = client
        .prepare(&contract.geography.query)
        .await
        .expect("geography RowDescription");
    assert_eq!(statement.columns().len(), 1);
    assert_eq!(
        statement.columns()[0].type_().oid(),
        contract.geography.expected_oid
    );
    assert_eq!(
        statement.columns()[0].type_().name(),
        contract.geography.expected_type_name
    );
    let row = client
        .query_one(&statement, &[])
        .await
        .expect("geography binary row");
    assert_eq!(
        row.get::<_, GeographyBytes>(0).0,
        decode_hex(&contract.geography.expected_hex)
    );
    let null = client
        .query_one(&contract.geography.null_query, &[])
        .await
        .expect("geography NULL row");
    assert_eq!(null.get::<_, Option<GeographyBytes>>(0), None);
    let text = client
        .simple_query(&contract.geography.text_query)
        .await
        .expect("geography text row")
        .into_iter()
        .find_map(|message| match message {
            tokio_postgres::SimpleQueryMessage::Row(row) => row.get(0).map(str::to_owned),
            _ => None,
        })
        .expect("geography text value");
    assert_eq!(text, contract.geography.expected_text);

    for query in &contract.metadata_queries {
        let rows = client
            .query(&query.sql, &[])
            .await
            .unwrap_or_else(|error| panic!("metadata query failed: {error}"));
        let actual = rows
            .iter()
            .map(|row| {
                (0..row.len())
                    .map(|index| row.get::<_, String>(index))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        assert_eq!(actual, query.expected_rows, "metadata query: {}", query.sql);
    }

    let ordinary = client
        .prepare(&contract.ordinary_catalog_query.sql)
        .await
        .expect("ordinary relational catalog query");
    let expected_columns = [
        ("oid", tokio_postgres::types::Type::OID),
        ("typname", tokio_postgres::types::Type::NAME),
        ("typnamespace", tokio_postgres::types::Type::OID),
        ("typtype", tokio_postgres::types::Type::CHAR),
        ("typdelim", tokio_postgres::types::Type::CHAR),
        ("typarray", tokio_postgres::types::Type::OID),
    ];
    for (column, (name, ty)) in ordinary.columns().iter().zip(expected_columns) {
        assert_eq!(column.name(), name);
        assert_eq!(column.type_(), &ty, "{name}");
    }
    let rows = client
        .query(&ordinary, &[])
        .await
        .expect("ordinary relational catalog rows");
    assert_eq!(
        rows.len(),
        contract.ordinary_catalog_query.expected_rows.len()
    );
    for (row, expected) in rows
        .iter()
        .zip(&contract.ordinary_catalog_query.expected_rows)
    {
        assert_eq!(row.get::<_, u32>(0), expected.oid);
        assert_eq!(row.get::<_, String>(1), expected.type_name);
        assert_eq!(row.get::<_, u32>(2), expected.namespace_oid);
        assert_eq!(row.get::<_, i8>(3), expected.type_kind);
        assert_eq!(row.get::<_, i8>(4), expected.delimiter);
        assert_eq!(row.get::<_, u32>(5), expected.array_oid);
    }

    let builtins = client
        .prepare(&contract.builtin_type_query.sql)
        .await
        .expect("prepare unqualified built-in type scan");
    let expected_columns = [
        ("oid", tokio_postgres::types::Type::OID),
        ("typname", tokio_postgres::types::Type::NAME),
        ("typtype", tokio_postgres::types::Type::CHAR),
        ("typelem", tokio_postgres::types::Type::OID),
        ("typlen", tokio_postgres::types::Type::INT2),
    ];
    for (column, (name, ty)) in builtins.columns().iter().zip(expected_columns) {
        assert_eq!(column.name(), name);
        assert_eq!(column.type_(), &ty, "{name}");
    }
    let rows = client
        .query(&builtins, &[])
        .await
        .expect("unqualified built-in type rows");
    assert_eq!(rows.len(), contract.builtin_type_query.expected_rows.len());
    for (row, expected) in rows.iter().zip(&contract.builtin_type_query.expected_rows) {
        assert_eq!(row.get::<_, u32>(0), expected.oid);
        assert_eq!(row.get::<_, String>(1), expected.type_name);
        assert_eq!(row.get::<_, i8>(2), expected.type_kind);
        assert_eq!(row.get::<_, u32>(3), expected.element_oid);
        assert_eq!(row.get::<_, i16>(4), expected.type_length);
    }

    let count = client
        .query_one(&contract.type_catalog_integrity.count_sql, &[])
        .await
        .expect("maintained type count")
        .get::<_, i64>(0);
    assert_eq!(count, contract.type_catalog_integrity.expected_count);
    for sql in &contract.type_catalog_integrity.unresolved_queries {
        let unresolved = client
            .query_one(sql, &[])
            .await
            .unwrap_or_else(|error| panic!("maintained reference query: {error}"))
            .get::<_, i64>(0);
        assert_eq!(
            unresolved, contract.type_catalog_integrity.expected_unresolved,
            "maintained reference query: {sql}"
        );
    }

    let identity = client
        .prepare(&contract.session_discovery.identity_sql)
        .await
        .expect("prepare database identity query");
    assert_eq!(
        identity.columns()[0].type_(),
        &tokio_postgres::types::Type::NAME
    );
    assert_eq!(
        identity.columns()[1].type_(),
        &tokio_postgres::types::Type::NAME
    );
    let identity = client
        .query_one(&identity, &[])
        .await
        .expect("database identity row");
    assert_eq!(
        identity.get::<_, String>(0),
        contract.session_discovery.expected_database
    );
    assert_eq!(
        identity.get::<_, String>(1),
        contract.session_discovery.expected_schema
    );

    for (sql, expected) in [
        (
            &contract.session_discovery.implicit_schemas_sql,
            &contract.session_discovery.expected_implicit_schemas,
        ),
        (
            &contract.session_discovery.explicit_schemas_sql,
            &contract.session_discovery.expected_explicit_schemas,
        ),
    ] {
        let statement = client
            .prepare(sql)
            .await
            .expect("prepare current_schemas query");
        assert_eq!(
            statement.columns()[0].type_(),
            &tokio_postgres::types::Type::NAME_ARRAY
        );
        let schemas = client
            .query_one(&statement, &[])
            .await
            .expect("current_schemas row")
            .get::<_, Vec<String>>(0);
        assert_eq!(&schemas, expected);
    }

    let database = client
        .prepare(&contract.session_discovery.database_sql)
        .await
        .expect("prepare pg_database query");
    let database_columns = [
        tokio_postgres::types::Type::OID,
        tokio_postgres::types::Type::NAME,
        tokio_postgres::types::Type::OID,
        tokio_postgres::types::Type::NAME,
    ];
    for (column, expected_type) in database.columns().iter().zip(database_columns) {
        assert_eq!(column.type_(), &expected_type);
    }
    let database = client
        .query_one(&database, &[])
        .await
        .expect("pg_database row");
    assert_eq!(
        database.get::<_, u32>(0),
        contract.session_discovery.expected_database_oid
    );
    assert_eq!(
        database.get::<_, String>(1),
        contract.session_discovery.expected_database
    );
    assert_eq!(database.get::<_, u32>(2), 10);
    assert_eq!(
        database.get::<_, String>(3),
        contract.session_discovery.expected_owner
    );

    let aliases = client
        .prepare("SELECT -1::INTEGER AS oid, 'base'::VARCHAR AS typtype")
        .await
        .expect("ordinary aliases remain ordinary PostgreSQL types");
    assert_eq!(
        aliases.columns()[0].type_(),
        &tokio_postgres::types::Type::INT4
    );
    assert_eq!(
        aliases.columns()[1].type_(),
        &tokio_postgres::types::Type::TEXT
    );
    let row = client
        .query_one(&aliases, &[])
        .await
        .expect("ordinary alias row");
    assert_eq!(row.get::<_, i32>(0), -1);
    assert_eq!(row.get::<_, String>(1), "base");

    for sql in [
        "SELECT relname FROM pg_catalog.pg_class",
        "SELECT attname FROM pg_catalog.pg_attribute",
        "SELECT to_regclass('catalog_fixture')",
        "SELECT 'catalog_fixture'::regclass",
        "SELECT to_regtype('integer')",
        "SELECT format_type(23, -1)",
    ] {
        let error = client
            .query(sql, &[])
            .await
            .expect_err("baseline catalog must reject unavailable user identity");
        assert_eq!(
            error.code(),
            Some(&tokio_postgres::error::SqlState::FEATURE_NOT_SUPPORTED),
            "{sql}"
        );
    }
    connection.abort();
}

fn decode_hex(value: &str) -> Vec<u8> {
    assert!(value.len().is_multiple_of(2), "hex length");
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let pair = std::str::from_utf8(pair).expect("hex UTF-8");
            u8::from_str_radix(pair, 16).expect("hex byte")
        })
        .collect()
}

#[test]
fn catalog_fixture_is_valid_and_client_neutral() {
    let raw = include_str!("../../../tests/fixtures/duckdb_catalog_contract.json");
    let contract: CatalogContract = serde_json::from_str(raw).expect("catalog contract fixture");
    assert_eq!(contract.schema_version, 6);
    let words = raw
        .split(|character: char| !character.is_ascii_alphanumeric())
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>();
    for client_name in ["psql", "psycopg", "qgis", "gdal", "ogr"] {
        assert!(
            !words.iter().any(|word| word == client_name),
            "fixture must stay client-neutral: {client_name}"
        );
    }
    assert_eq!(decode_hex(&contract.geometry.expected_hex).len(), 21);
    assert_eq!(decode_hex(&contract.geography.expected_hex).len(), 21);
}
