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
    metadata_queries: Vec<MetadataQuery>,
    ordinary_catalog_query: CountQuery,
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
struct CountQuery {
    sql: String,
    expected_count: i64,
}

#[derive(Debug, Eq, PartialEq)]
struct GeometryBytes(Vec<u8>);

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

#[tokio::test]
#[ignore = "requires the pinned DuckDB ADBC runtime"]
async fn client_neutral_catalog_contract() {
    let contract: CatalogContract = serde_json::from_str(include_str!(
        "../../../tests/fixtures/duckdb_catalog_contract.json"
    ))
    .expect("catalog contract fixture");
    assert_eq!(contract.schema_version, 1);

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

    for case in &contract.spatial_type_lookup.cases {
        let rows = client
            .query(&contract.spatial_type_lookup.sql, &[&case.oid])
            .await
            .unwrap_or_else(|error| panic!("spatial type lookup {}: {error}", case.oid));
        match &case.expected_type_name {
            Some(expected) => {
                assert_eq!(rows.len(), 1, "spatial type OID {}", case.oid);
                assert_eq!(rows[0].get::<_, String>(0), *expected);
                assert_eq!(rows[0].get::<_, i8>(1), b'b' as i8);
                assert_eq!(rows[0].get::<_, u32>(2), 0);
                assert_eq!(rows[0].get::<_, Option<u32>>(3), None);
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

    let ordinary_count = client
        .query_one(&contract.ordinary_catalog_query.sql, &[])
        .await
        .expect("ordinary native catalog query")
        .get::<_, i64>(0);
    assert_eq!(
        ordinary_count,
        contract.ordinary_catalog_query.expected_count
    );
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
    assert_eq!(contract.schema_version, 1);
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
}
