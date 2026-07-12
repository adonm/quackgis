// SPDX-License-Identifier: Apache-2.0

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use axum::Router;
use axum::extract::{Path as AxumPath, RawQuery, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use clap::Parser;
use pg_query_engine::{
    ApiRequest, CountOption, FilterNode, ReadRequest, SelectItem, build_sql, parse_filter,
    parse_logic_filter, parse_order, parse_select,
};
use pg_schema_cache_types::{Column, QualifiedName, SchemaCache, Table};
use rustls::RootCertStore;
use subtle::ConstantTimeEq;
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tokio_postgres::types::{ToSql, Type};
use tokio_postgres::{Client, Config, NoTls};
use tokio_postgres_rustls::MakeRustlsConnect;

const UPSTREAM_REVISION: &str = "b7915d3c3361f0fee45de6e292e62f6f6186375f";
const RESERVED_PARAMS: &[&str] = &["select", "order", "limit", "offset"];

#[derive(Debug, Parser)]
#[command(name = "quackgis-rest")]
struct Cli {
    #[arg(long, env = "QUACKGIS_REST_HOST", default_value = "127.0.0.1")]
    host: String,
    #[arg(long, env = "QUACKGIS_REST_PORT", default_value_t = 3000)]
    port: u16,
    #[arg(long, env = "QUACKGIS_REST_DATABASE_URL")]
    database_url: String,
    #[arg(long, env = "QUACKGIS_REST_DATABASE_CA")]
    database_ca: Option<PathBuf>,
    #[arg(long, env = "QUACKGIS_REST_BEARER_TOKEN_FILE")]
    bearer_token_file: PathBuf,
    #[arg(long, env = "QUACKGIS_REST_TABLES")]
    tables: String,
    #[arg(
        long,
        env = "QUACKGIS_REST_STATEMENT_TIMEOUT_MS",
        default_value_t = 30_000
    )]
    statement_timeout_ms: u64,
}

struct AppState {
    client: Client,
    cache: RwLock<Arc<SchemaCache>>,
    bearer_token: Vec<u8>,
    exposed_tables: HashSet<String>,
    statement_timeout: Duration,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    if cli.statement_timeout_ms == 0 {
        bail!("--statement-timeout-ms must be positive");
    }
    let token = std::fs::read(&cli.bearer_token_file)
        .with_context(|| format!("read bearer token file {}", cli.bearer_token_file.display()))?;
    let token = trim_ascii(&token);
    if token.len() < 32 || token.iter().any(u8::is_ascii_whitespace) {
        bail!("REST bearer token must contain at least 32 non-whitespace bytes");
    }

    let exposed_tables = parse_tables(&cli.tables)?;
    let client = connect_database(&cli.database_url, cli.database_ca.as_deref()).await?;
    let cache = discover_schema(&client, &exposed_tables).await?;
    let state = Arc::new(AppState {
        client,
        cache: RwLock::new(Arc::new(cache)),
        bearer_token: token.to_vec(),
        exposed_tables,
        statement_timeout: Duration::from_millis(cli.statement_timeout_ms),
    });
    let app = build_router(state);
    let address = format!("{}:{}", cli.host, cli.port);
    let listener = TcpListener::bind(&address).await?;
    println!("quackgis_rest_ready address={address} mode=read_only upstream={UPSTREAM_REVISION}");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

fn build_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(openapi))
        .route("/live", get(live))
        .route("/ready", get(ready))
        .route("/reload", axum::routing::post(reload))
        .route("/{table}", get(read_table).head(head_table))
        .fallback(method_or_route_not_found)
        .with_state(state)
}

async fn connect_database(database_url: &str, ca: Option<&Path>) -> Result<Client> {
    let config: Config = database_url.parse().context("parse database URL")?;
    if let Some(ca) = ca {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let mut reader = std::io::BufReader::new(
            std::fs::File::open(ca)
                .with_context(|| format!("open database CA {}", ca.display()))?,
        );
        let certificates = rustls_pemfile::certs(&mut reader).collect::<Result<Vec<_>, _>>()?;
        if certificates.is_empty() {
            bail!("database CA contains no certificates");
        }
        let mut roots = RootCertStore::empty();
        for certificate in certificates {
            roots.add(certificate)?;
        }
        let tls = rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        let (client, connection) = config.connect(MakeRustlsConnect::new(tls)).await?;
        tokio::spawn(async move {
            if let Err(error) = connection.await {
                eprintln!("quackgis_rest_database_connection_error error={error}");
            }
        });
        Ok(client)
    } else {
        let (client, connection) = config.connect(NoTls).await?;
        tokio::spawn(async move {
            if let Err(error) = connection.await {
                eprintln!("quackgis_rest_database_connection_error error={error}");
            }
        });
        Ok(client)
    }
}

async fn discover_schema(client: &Client, exposed_tables: &HashSet<String>) -> Result<SchemaCache> {
    const SQL: &str = "SELECT table_name::VARCHAR, column_name::VARCHAR, \
        data_type::VARCHAR, is_nullable::VARCHAR, column_default::VARCHAR \
        FROM information_schema.columns WHERE table_schema = 'main' \
        ORDER BY table_name, ordinal_position";
    let rows = client.query(SQL, &[]).await?;
    let mut tables: HashMap<QualifiedName, Table> = HashMap::new();
    for row in rows {
        let table_name: String = row.get(0);
        if !exposed_tables.contains(&table_name) {
            continue;
        }
        let column_name: String = row.get(1);
        let data_type: String = row.get(2);
        let nullable: String = row.get(3);
        let default_expr: Option<String> = row.get(4);
        let key = QualifiedName::new("public", table_name);
        let table = tables.entry(key.clone()).or_insert_with(|| Table {
            name: key,
            columns: Vec::new(),
            column_index: HashMap::new(),
            primary_key: Vec::new(),
            is_view: false,
            insertable: false,
            updatable: false,
            deletable: false,
            comment: None,
        });
        table.columns.push(Column {
            name: column_name,
            pg_type: postgres_type_name(&data_type).to_owned(),
            nullable: nullable.eq_ignore_ascii_case("YES"),
            has_default: default_expr.is_some(),
            default_expr,
            max_length: None,
            is_pk: false,
            is_generated: false,
            comment: None,
            enum_values: None,
        });
    }
    for table in tables.values_mut() {
        table.rebuild_column_index();
    }
    let missing = exposed_tables
        .iter()
        .filter(|table| !tables.keys().any(|key| &key.name == *table))
        .cloned()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        bail!(
            "configured REST tables were not found: {}",
            missing.join(",")
        );
    }
    Ok(SchemaCache {
        tables,
        relationships: Vec::new(),
        functions: HashMap::new(),
    })
}

fn postgres_type_name(data_type: &str) -> &'static str {
    let data_type = data_type.to_ascii_uppercase();
    if data_type.starts_with("DECIMAL(") || data_type.starts_with("NUMERIC(") {
        return "numeric";
    }
    match data_type.as_str() {
        "TINYINT" | "SMALLINT" => "int2",
        "INTEGER" => "int4",
        "BIGINT" => "int8",
        "UTINYINT" | "USMALLINT" | "UINTEGER" | "UBIGINT" => "numeric",
        "REAL" | "FLOAT" => "float4",
        "DOUBLE" => "float8",
        "DECIMAL" => "numeric",
        "BOOLEAN" => "bool",
        "DATE" => "date",
        "TIMESTAMP" | "TIMESTAMP WITHOUT TIME ZONE" => "timestamp",
        "TIMESTAMP WITH TIME ZONE" => "timestamptz",
        "BLOB" | "BYTEA" => "bytea",
        "JSON" => "json",
        _ => "text",
    }
}

async fn live() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

async fn ready(State(state): State<Arc<AppState>>) -> Response {
    match tokio::time::timeout(
        state.statement_timeout,
        state.client.simple_query("SELECT 1"),
    )
    .await
    {
        Ok(Ok(_)) => (StatusCode::OK, "ready").into_response(),
        _ => api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "QGRST503",
            "database unavailable",
        ),
    }
}

async fn openapi(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if !authorized(&headers, &state.bearer_token) {
        return unauthorized();
    }
    let cache = state.cache.read().await;
    let paths = cache
        .tables
        .keys()
        .map(|table| (format!("/{}", table.name), serde_json::json!({"get": {}})))
        .collect::<serde_json::Map<_, _>>();
    axum::Json(serde_json::json!({
        "openapi": "3.0.0",
        "info": {"title": "QuackGIS REST", "version": env!("CARGO_PKG_VERSION")},
        "paths": paths,
        "x-quackgis-mode": "read-only",
        "x-pg-rest-server-upstream": UPSTREAM_REVISION,
    }))
    .into_response()
}

async fn reload(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if !authorized(&headers, &state.bearer_token) {
        return unauthorized();
    }
    match discover_schema(&state.client, &state.exposed_tables).await {
        Ok(cache) => {
            *state.cache.write().await = Arc::new(cache);
            StatusCode::NO_CONTENT.into_response()
        }
        Err(_) => api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "QGRST503",
            "schema reload failed",
        ),
    }
}

async fn read_table(
    State(state): State<Arc<AppState>>,
    AxumPath(table): AxumPath<String>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
) -> Response {
    read_table_response(state, table, query, headers, false).await
}

async fn head_table(
    State(state): State<Arc<AppState>>,
    AxumPath(table): AxumPath<String>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
) -> Response {
    read_table_response(state, table, query, headers, true).await
}

async fn read_table_response(
    state: Arc<AppState>,
    table: String,
    query: Option<String>,
    headers: HeaderMap,
    head: bool,
) -> Response {
    if !authorized(&headers, &state.bearer_token) {
        return unauthorized();
    }
    let request = match parse_read_request(&table, query.as_deref().unwrap_or("")) {
        Ok(request) => request,
        Err(message) => return api_error(StatusCode::BAD_REQUEST, "PGRST100", &message),
    };
    let cache = state.cache.read().await.clone();
    let output = match build_sql(&cache, &ApiRequest::Read(request), &["public".to_owned()]) {
        Ok(output) => output,
        Err(error) => return api_error(StatusCode::NOT_FOUND, "PGRST205", &error.to_string()),
    };
    let sql = adapt_quackgis_sql(&output.sql);
    let parameters = output
        .params
        .iter()
        .map(|value| value as &(dyn ToSql + Sync))
        .collect::<Vec<_>>();
    let parameter_types = vec![Type::TEXT; parameters.len()];
    let result = tokio::time::timeout(state.statement_timeout, async {
        let statement = state.client.prepare_typed(&sql, &parameter_types).await?;
        state.client.query_one(&statement, &parameters).await
    })
    .await;
    let row = match result {
        Ok(Ok(row)) => row,
        Ok(Err(error)) => {
            return api_error(StatusCode::BAD_REQUEST, "QGRST400", &bounded_error(&error));
        }
        Err(_) => return api_error(StatusCode::GATEWAY_TIMEOUT, "QGRST504", "query timed out"),
    };
    let body: String = row.get(0);
    let mut response = if head {
        StatusCode::OK.into_response()
    } else {
        (StatusCode::OK, body).into_response()
    };
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("application/json; charset=utf-8"),
    );
    response
}

fn parse_read_request(table: &str, query: &str) -> Result<ReadRequest, String> {
    let pairs = url::form_urlencoded::parse(query.as_bytes())
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    let mut select = vec![SelectItem::Star];
    let mut order = Vec::new();
    let mut limit = None;
    let mut offset = None;
    let mut filters = Vec::new();
    for (key, value) in pairs {
        match key.as_str() {
            "select" => select = parse_select(&value).map_err(|error| error.to_string())?,
            "order" => order = parse_order(&value).map_err(|error| error.to_string())?,
            "limit" => limit = Some(parse_nonnegative(&value, "limit")?),
            "offset" => offset = Some(parse_nonnegative(&value, "offset")?),
            "or" | "and" => {
                filters.push(parse_logic_filter(&key, &value).map_err(|error| error.to_string())?)
            }
            _ if RESERVED_PARAMS.contains(&key.as_str()) => {}
            column => filters.push(FilterNode::Condition(
                parse_filter(column, &value).map_err(|error| error.to_string())?,
            )),
        }
    }
    Ok(ReadRequest {
        table: QualifiedName::new("public", table),
        select,
        filters: FilterNode::And(filters),
        order,
        limit,
        offset,
        count: CountOption::None,
    })
}

fn parse_nonnegative(value: &str, label: &str) -> Result<i64, String> {
    value
        .parse::<i64>()
        .ok()
        .filter(|value| *value >= 0)
        .ok_or_else(|| format!("{label} must be a non-negative integer"))
}

fn parse_tables(value: &str) -> Result<HashSet<String>> {
    let tables = value
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect::<HashSet<_>>();
    if tables.is_empty() {
        bail!("--tables must contain at least one table");
    }
    if let Some(invalid) = tables.iter().find(|table| {
        let mut characters = table.chars();
        !characters
            .next()
            .is_some_and(|character| character == '_' || character.is_ascii_alphabetic())
            || characters.any(|character| character != '_' && !character.is_ascii_alphanumeric())
    }) {
        bail!("invalid REST table name: {invalid}");
    }
    Ok(tables)
}

fn adapt_quackgis_sql(sql: &str) -> String {
    sql.replace("json_agg(", "json_group_array(")
        .replace("to_jsonb(", "to_json(")
}

fn authorized(headers: &HeaderMap, expected: &[u8]) -> bool {
    let Some(value) = headers.get(header::AUTHORIZATION) else {
        return false;
    };
    let Ok(value) = value.to_str() else {
        return false;
    };
    let Some(token) = value.strip_prefix("Bearer ") else {
        return false;
    };
    token.as_bytes().ct_eq(expected).into()
}

fn trim_ascii(value: &[u8]) -> &[u8] {
    let start = value
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(value.len());
    let end = value
        .iter()
        .rposition(|byte| !byte.is_ascii_whitespace())
        .map_or(start, |index| index + 1);
    &value[start..end]
}

fn bounded_error(error: &tokio_postgres::Error) -> String {
    error
        .as_db_error()
        .map_or_else(
            || error.to_string(),
            |database| database.message().to_owned(),
        )
        .chars()
        .take(512)
        .collect()
}

fn unauthorized() -> Response {
    api_error(StatusCode::UNAUTHORIZED, "PGRST301", "invalid bearer token")
}

fn api_error(status: StatusCode, code: &str, message: &str) -> Response {
    (
        status,
        axum::Json(serde_json::json!({
            "code": code,
            "details": null,
            "hint": null,
            "message": message,
        })),
    )
        .into_response()
}

async fn method_or_route_not_found() -> Response {
    api_error(StatusCode::NOT_FOUND, "PGRST205", "route not found")
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use quackgis_server::auth::AuthConfig;
    use quackgis_server::duckdb_adbc_storage::{
        DuckDbAdbcConfig, DuckDbAdbcStorage, ExtensionPolicy,
    };
    use quackgis_server::pgwire_server::{ServerOptions, serve_duckdb_on_listener};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn read_query_uses_upstream_parser_and_bounded_pagination() {
        let request = parse_read_request(
            "points",
            "select=id,name&id=gte.2&order=id.desc&limit=10&offset=1",
        )
        .unwrap();
        assert_eq!(request.table.name, "points");
        assert_eq!(request.limit, Some(10));
        assert_eq!(request.offset, Some(1));
        assert_eq!(request.select.len(), 2);
        assert!(parse_read_request("points", "limit=-1").is_err());
    }

    #[test]
    fn dialect_adapter_changes_only_required_generated_functions() {
        assert_eq!(
            adapt_quackgis_sql("SELECT coalesce(json_agg(t), '[]'), to_jsonb(1)"),
            "SELECT coalesce(json_group_array(t), '[]'), to_json(1)"
        );
    }

    #[test]
    fn tokens_are_trimmed_and_compared_exactly() {
        assert_eq!(trim_ascii(b"  secret\n"), b"secret");
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, "Bearer secret".parse().unwrap());
        assert!(authorized(&headers, b"secret"));
        assert!(!authorized(&headers, b"Secret"));
        assert_eq!(parse_tables("points, roads").unwrap().len(), 2);
        assert!(parse_tables("").is_err());
        assert!(parse_tables("points;drop").is_err());
        assert_eq!(postgres_type_name("DECIMAL(10,2)"), "numeric");
        assert_eq!(postgres_type_name("INTEGER"), "int4");
    }

    #[tokio::test]
    #[ignore = "requires the pinned DuckDB ADBC runtime"]
    async fn actual_postgrest_compat_and_quackgis_extensions() {
        let driver_path = std::env::var_os("QUACKGIS_DUCKDB_ADBC_DRIVER")
            .expect("set QUACKGIS_DUCKDB_ADBC_DRIVER");
        let temp = tempfile::tempdir().expect("tempdir");
        let data_path = temp.path().join("data");
        std::fs::create_dir(&data_path).expect("data path");
        let storage = Arc::new(
            DuckDbAdbcStorage::open(DuckDbAdbcConfig {
                driver_path: driver_path.into(),
                database_uri: ":memory:".to_owned(),
                ducklake_uri: format!(
                    "ducklake:{}",
                    temp.path().join("catalog.ducklake").display()
                ),
                catalog_name: "quackgis".to_owned(),
                data_path: data_path.display().to_string(),
                extension_policy: ExtensionPolicy::LoadOnly,
            })
            .expect("DuckDB storage"),
        );
        storage
            .execute_update(
                "CREATE TABLE quackgis.main.rest_points( \
                     id INTEGER, name VARCHAR, geom_wkb BLOB); \
                 INSERT INTO quackgis.main.rest_points VALUES \
                     (1, 'one', ST_AsWKB(ST_Point(1, 2))), \
                     (2, 'two', ST_AsWKB(ST_Point(2, 3))), \
                     (3, 'three', ST_AsWKB(ST_Point(3, 4)))",
            )
            .expect("REST fixture");

        let pg_listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let pg_port = pg_listener.local_addr().unwrap().port();
        let pg_options = ServerOptions::new()
            .with_host("127.0.0.1".to_owned())
            .with_port(pg_port);
        let server_storage = Arc::clone(&storage);
        let pg_task = tokio::spawn(async move {
            let _ = serve_duckdb_on_listener(
                server_storage,
                pg_listener,
                &pg_options,
                AuthConfig::trust(),
            )
            .await;
        });

        let client = connect_database(
            &format!("postgres://postgres@127.0.0.1:{pg_port}/quackgis"),
            None,
        )
        .await
        .expect("REST pgwire connection");
        let exposed_tables = parse_tables("rest_points").unwrap();
        let cache = discover_schema(&client, &exposed_tables)
            .await
            .expect("REST schema");
        assert!(
            cache
                .find_table("rest_points", &["public".to_owned()])
                .is_some()
        );
        let state = Arc::new(AppState {
            client,
            cache: RwLock::new(Arc::new(cache)),
            bearer_token: b"0123456789abcdef0123456789abcdef".to_vec(),
            exposed_tables,
            statement_timeout: Duration::from_secs(5),
        });
        let rest_listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let rest_port = rest_listener.local_addr().unwrap().port();
        let rest_task = tokio::spawn(async move {
            axum::serve(rest_listener, build_router(state))
                .await
                .unwrap();
        });

        let response = http_request(
            rest_port,
            "GET",
            "/rest_points?select=id,name&id=gte.2&order=id.desc",
            Some("0123456789abcdef0123456789abcdef"),
        )
        .await;
        assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
        assert!(response.contains(r#"[{"id":3,"name":"three"},{"id":2,"name":"two"}]"#));
        let paged = http_request(
            rest_port,
            "GET",
            "/rest_points?select=id&order=id.asc&limit=1&offset=1",
            Some("0123456789abcdef0123456789abcdef"),
        )
        .await;
        assert!(paged.contains(r#"[{"id":2}]"#), "{paged}");
        let spatial = http_request(
            rest_port,
            "GET",
            "/rest_points?select=id,geom_wkb&id=eq.1",
            Some("0123456789abcdef0123456789abcdef"),
        )
        .await;
        assert!(spatial.contains(r#""geom_wkb":"\\x01\\x01"#), "{spatial}");
        let openapi = http_request(
            rest_port,
            "GET",
            "/",
            Some("0123456789abcdef0123456789abcdef"),
        )
        .await;
        assert!(openapi.contains(r#""/rest_points":{"get":{}}"#));
        let missing = http_request(
            rest_port,
            "GET",
            "/missing_table",
            Some("0123456789abcdef0123456789abcdef"),
        )
        .await;
        assert!(missing.starts_with("HTTP/1.1 404 Not Found"), "{missing}");
        let mutation = http_request(
            rest_port,
            "POST",
            "/rest_points",
            Some("0123456789abcdef0123456789abcdef"),
        )
        .await;
        assert!(
            mutation.starts_with("HTTP/1.1 405 Method Not Allowed"),
            "{mutation}"
        );
        let unauthorized = http_request(rest_port, "GET", "/rest_points", None).await;
        assert!(
            unauthorized.starts_with("HTTP/1.1 401 Unauthorized"),
            "{unauthorized}"
        );

        rest_task.abort();
        pg_task.abort();
    }

    async fn http_request(port: u16, method: &str, path: &str, token: Option<&str>) -> String {
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("HTTP connect");
        let authorization = token
            .map(|token| format!("Authorization: Bearer {token}\r\n"))
            .unwrap_or_default();
        stream
            .write_all(
                format!(
                    "{method} {path} HTTP/1.1\r\nHost: localhost\r\n{authorization}Content-Length: 0\r\nConnection: close\r\n\r\n"
                )
                .as_bytes(),
            )
            .await
            .expect("HTTP write");
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await.expect("HTTP read");
        String::from_utf8(response).expect("HTTP UTF-8")
    }
}
