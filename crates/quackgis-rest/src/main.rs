// SPDX-License-Identifier: Apache-2.0

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use axum::Router;
use axum::extract::{Path as AxumPath, RawQuery, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use clap::Parser;
use pg_query_engine::{
    ApiRequest, CountOption, FilterNode, ReadRequest, SelectItem, build_sql, parse_filter,
    parse_logic_filter, parse_order, parse_select,
};
use pg_schema_cache_types::{Column, QualifiedName, SchemaCache, Table};
use ring::hmac;
use rustls::RootCertStore;
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;
use tokio::sync::{Mutex, RwLock};
use tokio_postgres::config::SslMode;
use tokio_postgres::error::SqlState;
use tokio_postgres::types::{ToSql, Type};
use tokio_postgres::{Client, Config, NoTls, Row};
use tokio_postgres_rustls::MakeRustlsConnect;

const UPSTREAM_REVISION: &str = "b7915d3c3361f0fee45de6e292e62f6f6186375f";
const RESERVED_PARAMS: &[&str] = &["select", "order", "limit", "offset"];
const MAX_JWT_BYTES: usize = 24_576;
const MAX_JWT_CLAIMS_BYTES: usize = 16_384;
const MAX_JWT_SECRET_BYTES: usize = 4096;
const MAX_JWT_SECRET_FILE_BYTES: u64 = 8192;
const MAX_DATABASE_PASSWORD_BYTES: usize = 4096;
const MAX_DATABASE_PASSWORD_FILE_BYTES: u64 = 8192;
const CATALOG_EPOCH_SQL: &str = "SELECT pg_catalog.quackgis_schema_epoch(), \
    pg_catalog.quackgis_security_epoch()";
const JWT_CLOCK_SKEW_SECONDS: u64 = 30;

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
    #[arg(long, env = "QUACKGIS_REST_DATABASE_PASSWORD_FILE")]
    database_password_file: PathBuf,
    #[arg(long, env = "QUACKGIS_REST_JWT_SECRET_FILE")]
    jwt_secret_file: PathBuf,
    #[arg(long, env = "QUACKGIS_REST_JWT_ISSUER")]
    jwt_issuer: String,
    #[arg(long, env = "QUACKGIS_REST_JWT_AUDIENCE")]
    jwt_audience: String,
    #[arg(long, env = "QUACKGIS_REST_JWT_ROLES")]
    jwt_roles: String,
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
    database: DatabaseConnection,
    caches: RwLock<HashMap<String, SchemaCacheEntry>>,
    jwt: JwtVerifier,
    exposed_tables: HashSet<String>,
    statement_timeout: Duration,
}

struct DatabaseConnection {
    connector: DatabaseConnector,
    active: Mutex<Option<ActiveDatabaseClient>>,
    next_generation: AtomicU64,
}

struct DatabaseConnector {
    database_url: String,
    ca: Option<PathBuf>,
    password_file: PathBuf,
}

struct ActiveDatabaseClient {
    client: Client,
    credential_revision: [u8; 32],
    generation: u64,
    epoch_capability: Option<bool>,
}

struct DatabaseCredential {
    password: Vec<u8>,
    revision: [u8; 32],
}

#[derive(Clone)]
struct SchemaCacheEntry {
    role: String,
    schema: Arc<SchemaCache>,
    validation: SchemaValidation,
}

struct DiscoveredSchema {
    schema: SchemaCache,
    validation: SchemaValidation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SchemaValidation {
    Epochs {
        schema: i64,
        security: i64,
        connection_generation: u64,
    },
    Revision([u8; 32]),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EpochValidation {
    Available(SchemaValidation),
    Unsupported,
}

#[derive(Clone)]
struct JwtVerifier {
    secret_file: PathBuf,
    issuer: String,
    audience: String,
    roles: HashSet<String>,
}

struct RequestIdentity {
    role: String,
    claims: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    if cli.statement_timeout_ms == 0 {
        bail!("--statement-timeout-ms must be positive");
    }
    let jwt = JwtVerifier::from_file(
        &cli.jwt_secret_file,
        cli.jwt_issuer,
        cli.jwt_audience,
        parse_roles(&cli.jwt_roles)?,
    )?;
    let exposed_tables = parse_tables(&cli.tables)?;
    let database = DatabaseConnection::new(
        cli.database_url,
        cli.database_ca,
        cli.database_password_file,
    )
    .await?;
    let mut caches = HashMap::new();
    let mut discovered_tables = HashSet::new();
    let mut roles = jwt.roles.iter().collect::<Vec<_>>();
    roles.sort_unstable();
    for role in roles {
        let discovered = database.discover_schema(&exposed_tables, role).await?;
        discovered_tables.extend(
            discovered
                .schema
                .tables
                .keys()
                .map(|table| table.name.clone()),
        );
        caches.insert(role.clone(), SchemaCacheEntry::new(role, discovered));
    }
    let mut missing = exposed_tables
        .difference(&discovered_tables)
        .cloned()
        .collect::<Vec<_>>();
    missing.sort_unstable();
    if !missing.is_empty() {
        bail!(
            "configured REST tables were not visible to any JWT role: {}",
            missing.join(",")
        );
    }
    let state = Arc::new(AppState {
        database,
        caches: RwLock::new(caches),
        jwt,
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

async fn connect_database(
    database_url: &str,
    ca: Option<&Path>,
    password: &[u8],
) -> Result<Client> {
    let mut config = database_config(database_url, password)?;
    if let Some(ca) = ca {
        config.ssl_mode(SslMode::Require);
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
        if config.get_ssl_mode() == SslMode::Require {
            bail!("database URL requires TLS but --database-ca is not configured");
        }
        config.ssl_mode(SslMode::Disable);
        let (client, connection) = config.connect(NoTls).await?;
        tokio::spawn(async move {
            if let Err(error) = connection.await {
                eprintln!("quackgis_rest_database_connection_error error={error}");
            }
        });
        Ok(client)
    }
}

fn database_config(database_url: &str, password: &[u8]) -> Result<Config> {
    let mut config: Config = database_url.parse().context("parse database URL")?;
    if config.get_password().is_some() {
        bail!("database URL must not contain a password; use --database-password-file");
    }
    config.password(password);
    Ok(config)
}

impl DatabaseConnection {
    async fn new(
        database_url: String,
        ca: Option<PathBuf>,
        password_file: PathBuf,
    ) -> Result<Self> {
        let connector = DatabaseConnector {
            database_url,
            ca,
            password_file,
        };
        let credential = connector.credential()?;
        let client = connector.connect(&credential).await?;
        Ok(Self {
            connector,
            active: Mutex::new(Some(ActiveDatabaseClient {
                client,
                credential_revision: credential.revision,
                generation: 1,
                epoch_capability: None,
            })),
            next_generation: AtomicU64::new(2),
        })
    }

    fn next_generation(&self) -> Result<u64> {
        self.next_generation
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |generation| {
                generation.checked_add(1)
            })
            .map_err(|_| anyhow::anyhow!("database connection generation exhausted"))
    }

    async fn active(&self) -> Result<tokio::sync::MutexGuard<'_, Option<ActiveDatabaseClient>>> {
        let mut active = self.active.lock().await;
        let credential = match self.connector.credential() {
            Ok(credential) => credential,
            Err(error) => {
                active.take();
                return Err(error);
            }
        };
        let replace = active.as_ref().is_none_or(|active| {
            active.client.is_closed() || active.credential_revision != credential.revision
        });
        if replace {
            active.take();
            let client = self.connector.connect(&credential).await?;
            *active = Some(ActiveDatabaseClient {
                client,
                credential_revision: credential.revision,
                generation: self.next_generation()?,
                epoch_capability: None,
            });
        }
        Ok(active)
    }

    async fn epoch_validation(&self) -> Result<EpochValidation> {
        let mut active = self.active().await?;
        let result = {
            let active = active.as_mut().expect("active database client");
            if active.epoch_capability == Some(false) {
                Ok(EpochValidation::Unsupported)
            } else {
                match query_catalog_epochs(&active.client).await {
                    Ok((schema, security)) => {
                        active.epoch_capability = Some(true);
                        Ok(EpochValidation::Available(SchemaValidation::Epochs {
                            schema,
                            security,
                            connection_generation: active.generation,
                        }))
                    }
                    Err(error) if epoch_capability_is_unsupported(&error) => {
                        active.epoch_capability = Some(false);
                        Ok(EpochValidation::Unsupported)
                    }
                    Err(error) => Err(error),
                }
            }
        };
        if result.as_ref().is_err_and(tokio_postgres::Error::is_closed) {
            active.take();
        }
        result.map_err(Into::into)
    }

    async fn discover_schema(
        &self,
        exposed_tables: &HashSet<String>,
        role: &str,
    ) -> Result<DiscoveredSchema> {
        let mut active = self.active().await?;
        let result = {
            let active = active.as_mut().expect("active database client");
            let use_epochs = match active.epoch_capability {
                Some(available) => available,
                None => match query_catalog_epochs(&active.client).await {
                    Ok(_) => {
                        active.epoch_capability = Some(true);
                        true
                    }
                    Err(error) if epoch_capability_is_unsupported(&error) => {
                        active.epoch_capability = Some(false);
                        false
                    }
                    Err(error) => return Err(error.into()),
                },
            };
            discover_schema(
                &mut active.client,
                exposed_tables,
                role,
                use_epochs.then_some(active.generation),
            )
            .await
        };
        if result.as_ref().is_err_and(database_error_is_closed) {
            active.take();
        }
        result
    }

    async fn ready(&self) -> Result<()> {
        let mut active = self.active().await?;
        let result = active
            .as_mut()
            .expect("active database client")
            .client
            .simple_query("SELECT 1")
            .await;
        if result.as_ref().is_err_and(tokio_postgres::Error::is_closed) {
            active.take();
        }
        result?;
        Ok(())
    }

    async fn execute_read(
        &self,
        identity: &RequestIdentity,
        sql: &str,
        parameter_types: &[Type],
        parameters: &[&(dyn ToSql + Sync)],
    ) -> Result<String> {
        let mut active = self.active().await?;
        let result = execute_read_with_client(
            &mut active.as_mut().expect("active database client").client,
            identity,
            sql,
            parameter_types,
            parameters,
        )
        .await;
        if result.as_ref().is_err_and(tokio_postgres::Error::is_closed) {
            active.take();
        }
        result.map_err(Into::into)
    }

    #[cfg(test)]
    async fn session_identity(&self) -> Result<(String, Option<String>)> {
        let mut active = self.active().await?;
        let row = active
            .as_mut()
            .expect("active database client")
            .client
            .query_one(
                "SELECT current_user, current_setting('request.jwt.claims', true)",
                &[],
            )
            .await?;
        Ok((row.get(0), row.get(1)))
    }

    #[cfg(test)]
    async fn disconnect(&self) {
        self.active.lock().await.take();
    }
}

impl DatabaseConnector {
    fn credential(&self) -> Result<DatabaseCredential> {
        read_database_password(&self.password_file)
    }

    async fn connect(&self, credential: &DatabaseCredential) -> Result<Client> {
        connect_database(&self.database_url, self.ca.as_deref(), &credential.password).await
    }
}

fn database_error_is_closed(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<tokio_postgres::Error>()
        .is_some_and(tokio_postgres::Error::is_closed)
}

async fn query_catalog_epochs(client: &Client) -> Result<(i64, i64), tokio_postgres::Error> {
    let row = client.query_one(CATALOG_EPOCH_SQL, &[]).await?;
    Ok(catalog_epochs_from_row(&row))
}

fn catalog_epochs_from_row(row: &Row) -> (i64, i64) {
    (row.get(0), row.get(1))
}

fn epoch_capability_is_unsupported(error: &tokio_postgres::Error) -> bool {
    error
        .as_db_error()
        .is_some_and(|database| database.code() == &SqlState::FEATURE_NOT_SUPPORTED)
}

async fn discover_schema(
    client: &mut Client,
    exposed_tables: &HashSet<String>,
    role: &str,
    connection_generation: Option<u64>,
) -> Result<DiscoveredSchema> {
    const SQL: &str = "SELECT table_name::VARCHAR, column_name::VARCHAR, \
        udt_name::VARCHAR, is_nullable::VARCHAR, column_default::VARCHAR \
        FROM information_schema.columns WHERE table_schema = 'public' \
        ORDER BY table_name, ordinal_position";
    let transaction = client.transaction().await?;
    let epochs_before = if connection_generation.is_some() {
        Some(catalog_epochs_from_row(
            &transaction.query_one(CATALOG_EPOCH_SQL, &[]).await?,
        ))
    } else {
        None
    };
    transaction
        .batch_execute(&format!("SET LOCAL ROLE {role}"))
        .await?;
    let rows = transaction.query(SQL, &[]).await?;
    let mut tables: HashMap<QualifiedName, Table> = HashMap::new();
    let mut revision = Sha256::new();
    for row in rows {
        let table_name: String = row.get(0);
        if !exposed_tables.contains(&table_name) {
            continue;
        }
        let column_name: String = row.get(1);
        let pg_type: String = row.get(2);
        let nullable: String = row.get(3);
        let default_expr: Option<String> = row.get(4);
        update_schema_revision(&mut revision, Some(&table_name));
        update_schema_revision(&mut revision, Some(&column_name));
        update_schema_revision(&mut revision, Some(&pg_type));
        update_schema_revision(&mut revision, Some(&nullable));
        update_schema_revision(&mut revision, default_expr.as_deref());
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
            pg_type,
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
    let validation = if let (Some(connection_generation), Some((schema, security))) =
        (connection_generation, epochs_before)
    {
        let epochs_after =
            catalog_epochs_from_row(&transaction.query_one(CATALOG_EPOCH_SQL, &[]).await?);
        if epochs_after != (schema, security) {
            bail!("catalog epochs changed during REST schema discovery");
        }
        SchemaValidation::Epochs {
            schema,
            security,
            connection_generation,
        }
    } else {
        SchemaValidation::Revision(revision.finalize().into())
    };
    transaction.commit().await?;
    Ok(DiscoveredSchema {
        schema: SchemaCache {
            tables,
            relationships: Vec::new(),
            functions: HashMap::new(),
        },
        validation,
    })
}

fn update_schema_revision(revision: &mut Sha256, value: Option<&str>) {
    match value {
        Some(value) => {
            revision.update((value.len() as u64).to_be_bytes());
            revision.update(value.as_bytes());
        }
        None => revision.update(u64::MAX.to_be_bytes()),
    }
}

impl SchemaCacheEntry {
    fn new(role: &str, discovered: DiscoveredSchema) -> Self {
        Self {
            role: role.to_owned(),
            schema: Arc::new(discovered.schema),
            validation: discovered.validation,
        }
    }

    fn matches(&self, role: &str, validation: SchemaValidation) -> bool {
        self.role == role && self.validation == validation
    }
}

async fn role_schema(
    state: &AppState,
    role: &str,
    force_reload: bool,
) -> Result<Arc<SchemaCache>, ()> {
    if !force_reload {
        let validation =
            tokio::time::timeout(state.statement_timeout, state.database.epoch_validation())
                .await
                .map_err(|_| ())?
                .map_err(|_| ())?;
        if let EpochValidation::Available(validation) = validation
            && let Some(cached) = state.caches.read().await.get(role)
            && cached.matches(role, validation)
        {
            return Ok(Arc::clone(&cached.schema));
        }
    }
    let discovered = tokio::time::timeout(state.statement_timeout, async {
        state
            .database
            .discover_schema(&state.exposed_tables, role)
            .await
    })
    .await
    .map_err(|_| ())?
    .map_err(|_| ())?;

    let mut caches = state.caches.write().await;
    if let Some(cached) = caches.get(role)
        && cached.matches(role, discovered.validation)
    {
        return Ok(Arc::clone(&cached.schema));
    }
    let entry = SchemaCacheEntry::new(role, discovered);
    let schema = Arc::clone(&entry.schema);
    caches.insert(role.to_owned(), entry);
    Ok(schema)
}

async fn live() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

async fn ready(State(state): State<Arc<AppState>>) -> Response {
    if state.jwt.secret().is_err() {
        return api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "QGRST503",
            "JWT signing key unavailable",
        );
    }
    match tokio::time::timeout(state.statement_timeout, async {
        state.database.ready().await
    })
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
    let identity = match state.jwt.identity(&headers) {
        Ok(identity) => identity,
        Err(()) => return unauthorized(),
    };
    let cache = match role_schema(&state, &identity.role, false).await {
        Ok(cache) => cache,
        Err(()) => {
            return api_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "QGRST503",
                "schema validation failed",
            );
        }
    };
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
        "x-quackgis-role": identity.role,
        "x-pg-rest-server-upstream": UPSTREAM_REVISION,
    }))
    .into_response()
}

async fn reload(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let identity = match state.jwt.identity(&headers) {
        Ok(identity) => identity,
        Err(()) => return unauthorized(),
    };
    match role_schema(&state, &identity.role, true).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(()) => api_error(
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
    let identity = match state.jwt.identity(&headers) {
        Ok(identity) => identity,
        Err(()) => return unauthorized(),
    };
    let request = match parse_read_request(&table, query.as_deref().unwrap_or("")) {
        Ok(request) => request,
        Err(message) => return api_error(StatusCode::BAD_REQUEST, "PGRST100", &message),
    };
    let cache = match role_schema(&state, &identity.role, false).await {
        Ok(cache) => cache,
        Err(()) => {
            return api_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "QGRST503",
                "schema validation failed",
            );
        }
    };
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
    let result = tokio::time::timeout(
        state.statement_timeout,
        state
            .database
            .execute_read(&identity, &sql, &parameter_types, &parameters),
    )
    .await;
    let body = match result {
        Ok(Ok(body)) => body,
        Ok(Err(error)) => {
            return api_error(StatusCode::BAD_REQUEST, "QGRST400", &bounded_error(&error));
        }
        Err(_) => return api_error(StatusCode::GATEWAY_TIMEOUT, "QGRST504", "query timed out"),
    };
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

fn parse_roles(value: &str) -> Result<HashSet<String>> {
    let roles = value
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect::<HashSet<_>>();
    if roles.is_empty() {
        bail!("--jwt-roles must contain at least one role");
    }
    if let Some(invalid) = roles.iter().find(|role| {
        !valid_identifier(role)
            || matches!(
                role.as_str(),
                "public" | "none" | "current_user" | "session_user"
            )
    }) {
        bail!("invalid JWT database role: {invalid}");
    }
    Ok(roles)
}

fn valid_identifier(value: &str) -> bool {
    value.len() <= 63
        && value
            .chars()
            .next()
            .is_some_and(|character| character == '_' || character.is_ascii_lowercase())
        && value.chars().all(|character| {
            character == '_'
                || character == '$'
                || character.is_ascii_lowercase()
                || character.is_ascii_digit()
        })
}

fn adapt_quackgis_sql(sql: &str) -> String {
    sql.replace("json_agg(", "json_group_array(")
        .replace("to_jsonb(", "to_json(")
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

impl JwtVerifier {
    fn from_file(
        path: &Path,
        issuer: String,
        audience: String,
        roles: HashSet<String>,
    ) -> Result<Self> {
        read_jwt_secret(path)?;
        if !valid_jwt_label(&issuer) || !valid_jwt_label(&audience) {
            bail!("REST JWT issuer and audience must be 1 to 256 printable bytes");
        }
        Ok(Self {
            secret_file: path.to_owned(),
            issuer,
            audience,
            roles,
        })
    }

    fn identity(&self, headers: &HeaderMap) -> Result<RequestIdentity, ()> {
        let value = headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .ok_or(())?;
        self.verify(value)
    }

    fn verify(&self, token: &str) -> Result<RequestIdentity, ()> {
        if token.is_empty() || token.len() > MAX_JWT_BYTES {
            return Err(());
        }
        let parts = token.split('.').collect::<Vec<_>>();
        let [encoded_header, encoded_claims, encoded_signature] = parts.as_slice() else {
            return Err(());
        };
        let header = decode_json_segment(encoded_header, 1024)?;
        let header = header.as_object().ok_or(())?;
        if header.get("alg").and_then(serde_json::Value::as_str) != Some("HS256")
            || header
                .get("typ")
                .is_some_and(|value| value.as_str() != Some("JWT"))
            || header.contains_key("crit")
            || header.contains_key("b64")
        {
            return Err(());
        }
        let signature = URL_SAFE_NO_PAD.decode(encoded_signature).map_err(|_| ())?;
        let secret = self.secret().map_err(|_| ())?;
        let key = hmac::Key::new(hmac::HMAC_SHA256, &secret);
        hmac::verify(
            &key,
            format!("{encoded_header}.{encoded_claims}").as_bytes(),
            &signature,
        )
        .map_err(|_| ())?;

        let claims = decode_json_segment(encoded_claims, MAX_JWT_CLAIMS_BYTES)?;
        let claims = claims.as_object().ok_or(())?;
        if claims.get("iss").and_then(serde_json::Value::as_str) != Some(&self.issuer)
            || !valid_audience(claims.get("aud"), &self.audience)
        {
            return Err(());
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| ())?
            .as_secs();
        let expires = claims
            .get("exp")
            .and_then(serde_json::Value::as_u64)
            .ok_or(())?;
        if now > expires.saturating_add(JWT_CLOCK_SKEW_SECONDS) {
            return Err(());
        }
        if let Some(not_before) = claims.get("nbf") {
            let not_before = not_before.as_u64().ok_or(())?;
            if now.saturating_add(JWT_CLOCK_SKEW_SECONDS) < not_before {
                return Err(());
            }
        }
        let role = claims
            .get("role")
            .and_then(serde_json::Value::as_str)
            .filter(|role| self.roles.contains(*role))
            .ok_or(())?
            .to_owned();
        let claims = serde_json::to_string(claims).map_err(|_| ())?;
        if claims.len() > MAX_JWT_CLAIMS_BYTES {
            return Err(());
        }
        Ok(RequestIdentity { role, claims })
    }

    fn secret(&self) -> Result<Vec<u8>> {
        read_jwt_secret(&self.secret_file)
    }
}

fn read_jwt_secret(path: &Path) -> Result<Vec<u8>> {
    let file = std::fs::File::open(path)
        .with_context(|| format!("open JWT secret file {}", path.display()))?;
    let metadata = file
        .metadata()
        .with_context(|| format!("read JWT secret metadata {}", path.display()))?;
    if !metadata.is_file() || metadata.len() > MAX_JWT_SECRET_FILE_BYTES {
        bail!("REST JWT secret must be a regular file of at most 8192 bytes");
    }
    let mut bytes =
        Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(MAX_JWT_SECRET_BYTES));
    file.take(MAX_JWT_SECRET_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("read JWT secret file {}", path.display()))?;
    let secret = trim_ascii(&bytes);
    if secret.len() < 32
        || secret.len() > MAX_JWT_SECRET_BYTES
        || secret.iter().any(u8::is_ascii_whitespace)
    {
        bail!("REST JWT secret must contain 32 to 4096 non-whitespace bytes");
    }
    Ok(secret.to_vec())
}

fn read_database_password(path: &Path) -> Result<DatabaseCredential> {
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("read database password metadata {}", path.display()))?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() > MAX_DATABASE_PASSWORD_FILE_BYTES
    {
        bail!("REST database password must be a regular non-symlink file of at most 8192 bytes");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            bail!("REST database password file must not grant group or other permissions");
        }
    }
    let file = std::fs::File::open(path)
        .with_context(|| format!("open database password file {}", path.display()))?;
    let mut bytes =
        Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(MAX_DATABASE_PASSWORD_BYTES));
    file.take(MAX_DATABASE_PASSWORD_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("read database password file {}", path.display()))?;
    let password = trim_ascii(&bytes);
    if password.is_empty()
        || password.len() > MAX_DATABASE_PASSWORD_BYTES
        || password.iter().any(|byte| byte.is_ascii_control())
    {
        bail!("REST database password must contain 1 to 4096 bytes without control characters");
    }
    Ok(DatabaseCredential {
        revision: Sha256::digest(password).into(),
        password: password.to_vec(),
    })
}

fn decode_json_segment(value: &str, max_bytes: usize) -> Result<serde_json::Value, ()> {
    if value.len() > max_bytes.saturating_mul(2) {
        return Err(());
    }
    let bytes = URL_SAFE_NO_PAD.decode(value).map_err(|_| ())?;
    if bytes.len() > max_bytes {
        return Err(());
    }
    serde_json::from_slice(&bytes).map_err(|_| ())
}

fn valid_audience(value: Option<&serde_json::Value>, expected: &str) -> bool {
    value.is_some_and(|value| {
        value.as_str() == Some(expected)
            || value
                .as_array()
                .is_some_and(|values| values.iter().any(|value| value.as_str() == Some(expected)))
    })
}

fn valid_jwt_label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value
            .bytes()
            .all(|byte| byte.is_ascii_graphic() && !byte.is_ascii_control())
}

async fn execute_read_with_client(
    client: &mut Client,
    identity: &RequestIdentity,
    sql: &str,
    parameter_types: &[Type],
    parameters: &[&(dyn ToSql + Sync)],
) -> Result<String, tokio_postgres::Error> {
    let transaction = client.transaction().await?;
    transaction
        .batch_execute(&format!("SET LOCAL ROLE {}", identity.role))
        .await?;
    transaction
        .query_one(
            "SELECT set_config('request.jwt.claims', $1, true)",
            &[&identity.claims],
        )
        .await?;
    let statement = transaction.prepare_typed(sql, parameter_types).await?;
    let row = transaction.query_one(&statement, parameters).await?;
    let body = row.get(0);
    transaction.commit().await?;
    Ok(body)
}

fn bounded_error(error: &anyhow::Error) -> String {
    error
        .downcast_ref::<tokio_postgres::Error>()
        .and_then(tokio_postgres::Error::as_db_error)
        .map_or_else(
            || error.to_string(),
            |database| database.message().to_owned(),
        )
        .chars()
        .take(512)
        .collect()
}

fn unauthorized() -> Response {
    api_error(StatusCode::UNAUTHORIZED, "PGRST301", "invalid JWT")
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
    use quackgis_server::pgwire_server::{ServerOptions, serve_duckdb_on_listener_until};
    use quackgis_server::role::RoleCatalog;
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
    fn jwt_validation_is_bounded_and_role_allowlisted() {
        assert_eq!(trim_ascii(b"  secret\n"), b"secret");
        let (_secret_dir, verifier) = test_verifier(&["rest_reader"]);
        let token = test_jwt("rest_reader", unix_time() + 300);
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {token}").parse().unwrap(),
        );
        let identity = verifier.identity(&headers).expect("valid JWT");
        assert_eq!(identity.role, "rest_reader");
        assert!(identity.claims.contains(r#""sub":"test-user""#));
        assert!(
            verifier
                .verify(&test_jwt("rest_reader", unix_time().saturating_sub(60)))
                .is_err()
        );
        let (_other_secret_dir, other_role) = test_verifier(&["other_role"]);
        assert!(other_role.verify(&token).is_err());
        let (_wrong_secret_dir, mut wrong_audience) = test_verifier(&["rest_reader"]);
        wrong_audience.audience = "other-audience".to_owned();
        assert!(wrong_audience.verify(&token).is_err());
        let mut future_claims = test_claims("rest_reader", unix_time() + 300);
        future_claims["nbf"] = serde_json::json!(unix_time() + 60);
        assert!(
            verifier
                .verify(&sign_test_jwt(
                    serde_json::json!({"alg": "HS256", "typ": "JWT"}),
                    future_claims,
                ))
                .is_err()
        );
        let mut malformed_claims = test_claims("rest_reader", unix_time() + 300);
        malformed_claims["nbf"] = serde_json::json!("later");
        assert!(
            verifier
                .verify(&sign_test_jwt(
                    serde_json::json!({"alg": "HS256", "typ": "JWT"}),
                    malformed_claims,
                ))
                .is_err()
        );
        assert!(
            verifier
                .verify(&sign_test_jwt(
                    serde_json::json!({"alg": "HS512", "typ": "JWT"}),
                    test_claims("rest_reader", unix_time() + 300),
                ))
                .is_err()
        );
        assert!(verifier.verify(&"a".repeat(MAX_JWT_BYTES + 1)).is_err());
        let mut tampered = token.into_bytes();
        let last = tampered.last_mut().expect("JWT signature");
        *last = if *last == b'a' { b'b' } else { b'a' };
        assert!(
            verifier
                .verify(std::str::from_utf8(&tampered).unwrap())
                .is_err()
        );
        assert_eq!(parse_tables("points, roads").unwrap().len(), 2);
        assert!(parse_tables("").is_err());
        assert!(parse_tables("points;drop").is_err());
        assert_eq!(parse_roles("rest_reader,other_role").unwrap().len(), 2);
        assert!(parse_roles("UpperCase").is_err());
    }

    #[test]
    fn database_password_is_owner_only_bounded_and_separate_from_url() {
        let temp = tempfile::tempdir().expect("database password tempdir");
        let password_file = temp.path().join("database-password");
        write_secret_file(&password_file, b"database-secret\n");
        let credential = read_database_password(&password_file).expect("database password");
        assert_eq!(credential.password, b"database-secret");
        assert!(
            database_config(
                "postgres://authenticator:embedded@127.0.0.1/quackgis",
                b"database-secret"
            )
            .is_err()
        );
        write_secret_file(&password_file, b"invalid\ncontrol\n");
        assert!(read_database_password(&password_file).is_err());

        #[cfg(unix)]
        {
            use std::os::unix::fs::{PermissionsExt, symlink};
            write_secret_file(&password_file, b"database-secret");
            std::fs::set_permissions(&password_file, std::fs::Permissions::from_mode(0o640))
                .expect("weaken database password permissions");
            assert!(read_database_password(&password_file).is_err());
            let target = temp.path().join("database-password-target");
            write_secret_file(&target, b"database-secret");
            let link = temp.path().join("database-password-link");
            symlink(&target, &link).expect("database password symlink");
            assert!(read_database_password(&link).is_err());
        }
    }

    #[tokio::test]
    #[ignore = "requires the pinned DuckDB ADBC runtime"]
    async fn actual_postgrest_compat_and_quackgis_extensions() {
        actual_postgrest_compat_with_policy(ExtensionPolicy::LoadOnly, false).await;
    }

    #[tokio::test]
    #[ignore = "requires the checksum-pinned development DuckLake extension"]
    async fn shared_catalog_epochs_invalidate_rest_caches() {
        let extension = std::env::var_os("QUACKGIS_DEV_DUCKLAKE_EXTENSION")
            .expect("set QUACKGIS_DEV_DUCKLAKE_EXTENSION");
        let sha256 = std::env::var("QUACKGIS_DEV_DUCKLAKE_EXTENSION_SHA256")
            .expect("set QUACKGIS_DEV_DUCKLAKE_EXTENSION_SHA256");
        actual_postgrest_compat_with_policy(
            ExtensionPolicy::DevelopmentDuckLake {
                path: extension.into(),
                sha256,
            },
            true,
        )
        .await;
    }

    async fn actual_postgrest_compat_with_policy(
        extension_policy: ExtensionPolicy,
        expect_shared_epochs: bool,
    ) {
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
                extension_policy,
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
        let roles = RoleCatalog::from_json(
            r#"{
              "roles": [
                {"oid": 100001, "name": "authenticator", "login": true},
                {"oid": 100002, "name": "rest_reader"},
                {"oid": 100003, "name": "denied_reader"}
              ],
              "memberships": [
                {"oid": 200001, "role": "rest_reader", "member": "authenticator", "set_option": true},
                {"oid": 200002, "role": "denied_reader", "member": "authenticator", "set_option": true}
              ],
              "schema_grants": [
                {"schema": "public", "role": "PUBLIC", "privileges": ["USAGE"]}
              ],
              "table_grants": [
                {"table": "rest_points", "role": "rest_reader", "privileges": ["SELECT"]}
              ]
            }"#,
        )
        .expect("REST role catalog");
        let auth = AuthConfig::password(
            "authenticator",
            "authenticator-secret",
            None::<(&str, &str)>,
        )
        .expect("REST password auth")
        .with_role_catalog(roles.clone())
        .expect("REST role auth");
        let server_storage = Arc::clone(&storage);
        let (pg_shutdown, pg_shutdown_rx) = tokio::sync::watch::channel(false);
        let pg_task = tokio::spawn(async move {
            let _ = serve_duckdb_on_listener_until(
                server_storage,
                pg_listener,
                &pg_options,
                auth,
                pg_shutdown_rx,
            )
            .await;
        });

        let database_password_file = temp.path().join("database-password");
        write_secret_file(&database_password_file, b"authenticator-secret");
        let database = DatabaseConnection::new(
            format!("postgres://authenticator@127.0.0.1:{pg_port}/quackgis"),
            None,
            database_password_file.clone(),
        )
        .await
        .expect("REST pgwire connection");
        let exposed_tables = parse_tables("rest_points").unwrap();
        let cache = database
            .discover_schema(&exposed_tables, "rest_reader")
            .await
            .expect("REST schema");
        assert!(
            cache
                .schema
                .find_table("rest_points", &["public".to_owned()])
                .is_some()
        );
        let rest_points = cache
            .schema
            .find_table("rest_points", &["public".to_owned()])
            .expect("REST table");
        assert_eq!(rest_points.columns[0].pg_type, "int4");
        assert_eq!(rest_points.columns[1].pg_type, "text");
        assert_eq!(rest_points.columns[2].pg_type, "geometry");
        let denied_cache = database
            .discover_schema(&exposed_tables, "denied_reader")
            .await
            .expect("denied REST schema");
        assert!(denied_cache.schema.tables.is_empty());
        let (_jwt_secret_dir, jwt) = test_verifier(&["rest_reader", "denied_reader"]);
        let jwt_secret_file = jwt.secret_file.clone();
        let reader_jwt = test_jwt("rest_reader", unix_time() + 300);
        let denied_jwt = test_jwt("denied_reader", unix_time() + 300);
        assert_eq!(
            matches!(cache.validation, SchemaValidation::Epochs { .. }),
            expect_shared_epochs
        );
        let reader_entry = SchemaCacheEntry::new("rest_reader", cache);
        let reader_cache = Arc::clone(&reader_entry.schema);
        let caches = HashMap::from([
            ("rest_reader".to_owned(), reader_entry.clone()),
            (
                "denied_reader".to_owned(),
                SchemaCacheEntry::new("denied_reader", denied_cache),
            ),
        ]);
        let state = Arc::new(AppState {
            database,
            caches: RwLock::new(caches),
            jwt,
            exposed_tables,
            statement_timeout: Duration::from_secs(5),
        });
        let rest_listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let rest_port = rest_listener.local_addr().unwrap().port();
        let router_state = Arc::clone(&state);
        let rest_task = tokio::spawn(async move {
            axum::serve(rest_listener, build_router(router_state))
                .await
                .unwrap();
        });

        let response = http_request(
            rest_port,
            "GET",
            "/rest_points?select=id,name&id=gte.2&order=id.desc",
            Some(&reader_jwt),
        )
        .await;
        assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
        assert!(response.contains(r#"[{"id":3,"name":"three"},{"id":2,"name":"two"}]"#));
        let paged = http_request(
            rest_port,
            "GET",
            "/rest_points?select=id&order=id.asc&limit=1&offset=1",
            Some(&reader_jwt),
        )
        .await;
        assert!(paged.contains(r#"[{"id":2}]"#), "{paged}");
        let spatial = http_request(
            rest_port,
            "GET",
            "/rest_points?select=id,geom_wkb&id=eq.1",
            Some(&reader_jwt),
        )
        .await;
        assert!(spatial.contains(r#""geom_wkb":"\\x01\\x01"#), "{spatial}");
        let openapi = http_request(rest_port, "GET", "/", Some(&reader_jwt)).await;
        assert!(openapi.contains(r#""/rest_points":{"get":{}}"#));
        assert!(openapi.contains(r#""x-quackgis-role":"rest_reader""#));
        let denied_openapi = http_request(rest_port, "GET", "/", Some(&denied_jwt)).await;
        assert!(!denied_openapi.contains("/rest_points"), "{denied_openapi}");
        let denied_read = http_request(rest_port, "GET", "/rest_points", Some(&denied_jwt)).await;
        assert!(
            denied_read.starts_with("HTTP/1.1 404 Not Found"),
            "{denied_read}"
        );

        let denied_request = parse_read_request("rest_points", "select=id").unwrap();
        let denied_sql = build_sql(
            &reader_cache,
            &ApiRequest::Read(denied_request),
            &["public".to_owned()],
        )
        .expect("stale schema SQL");
        let denied_sql = adapt_quackgis_sql(&denied_sql.sql);
        let denied_identity = RequestIdentity {
            role: "denied_reader".to_owned(),
            claims: "{}".to_owned(),
        };
        let database_denied = state
            .database
            .execute_read(&denied_identity, &denied_sql, &[], &[])
            .await
            .expect_err("database must deny stale wide cache");
        assert!(
            bounded_error(&database_denied).contains("lacks SELECT privilege"),
            "{database_denied}"
        );

        state
            .caches
            .write()
            .await
            .insert("denied_reader".to_owned(), reader_entry.clone());
        let invalidated = http_request(rest_port, "GET", "/rest_points", Some(&denied_jwt)).await;
        assert!(
            invalidated.starts_with("HTTP/1.1 404 Not Found"),
            "{invalidated}"
        );

        let initial_validation = state
            .caches
            .read()
            .await
            .get("rest_reader")
            .expect("reader cache")
            .validation;
        storage
            .execute_update("ALTER TABLE quackgis.main.rest_points ADD COLUMN category VARCHAR")
            .expect("add REST-visible column");
        storage
            .execute_update("UPDATE quackgis.main.rest_points SET category = 'new' WHERE id = 1")
            .expect("populate REST-visible column");
        let changed_schema = http_request(
            rest_port,
            "GET",
            "/rest_points?select=id,category&id=eq.1",
            Some(&reader_jwt),
        )
        .await;
        assert!(
            changed_schema.starts_with("HTTP/1.1 200 OK")
                && changed_schema.contains(r#"[{"id":1,"category":"new"}]"#),
            "{changed_schema}"
        );
        let changed_validation = state
            .caches
            .read()
            .await
            .get("rest_reader")
            .expect("updated reader cache")
            .validation;
        assert_ne!(initial_validation, changed_validation);

        let missing = http_request(rest_port, "GET", "/missing_table", Some(&reader_jwt)).await;
        assert!(missing.starts_with("HTTP/1.1 404 Not Found"), "{missing}");
        let mutation = http_request(rest_port, "POST", "/rest_points", Some(&reader_jwt)).await;
        assert!(
            mutation.starts_with("HTTP/1.1 405 Method Not Allowed"),
            "{mutation}"
        );
        let unauthorized = http_request(rest_port, "GET", "/rest_points", None).await;
        assert!(
            unauthorized.starts_with("HTTP/1.1 401 Unauthorized"),
            "{unauthorized}"
        );
        let replacement_secret = jwt_secret_file.with_extension("next");
        std::fs::write(&replacement_secret, b"invalid").expect("write invalid JWT secret");
        std::fs::rename(&replacement_secret, &jwt_secret_file).expect("install invalid JWT secret");
        let unavailable_key = http_request(rest_port, "GET", "/ready", None).await;
        assert!(
            unavailable_key.starts_with("HTTP/1.1 503 Service Unavailable"),
            "{unavailable_key}"
        );
        std::fs::write(&replacement_secret, ROTATED_TEST_JWT_SECRET)
            .expect("write replacement JWT secret");
        std::fs::rename(&replacement_secret, &jwt_secret_file).expect("rotate JWT secret");
        let old_key_denied = http_request(
            rest_port,
            "GET",
            "/rest_points?select=id&id=eq.1",
            Some(&reader_jwt),
        )
        .await;
        assert!(
            old_key_denied.starts_with("HTTP/1.1 401 Unauthorized"),
            "{old_key_denied}"
        );
        let rotated_jwt =
            test_jwt_with_secret("rest_reader", unix_time() + 300, ROTATED_TEST_JWT_SECRET);
        let new_key_accepted = http_request(
            rest_port,
            "GET",
            "/rest_points?select=id&id=eq.1",
            Some(&rotated_jwt),
        )
        .await;
        assert!(
            new_key_accepted.starts_with("HTTP/1.1 200 OK")
                && new_key_accepted.contains(r#"[{"id":1}]"#),
            "{new_key_accepted}"
        );
        let ready_after_rotation = http_request(rest_port, "GET", "/ready", None).await;
        assert!(
            ready_after_rotation.starts_with("HTTP/1.1 200 OK"),
            "{ready_after_rotation}"
        );

        let replacement_password = database_password_file.with_extension("next");
        write_secret_file(&replacement_password, ROTATED_TEST_DATABASE_PASSWORD);
        std::fs::rename(&replacement_password, &database_password_file)
            .expect("install replacement database password");
        let credential_mismatch = http_request(rest_port, "GET", "/ready", None).await;
        assert!(
            credential_mismatch.starts_with("HTTP/1.1 503 Service Unavailable"),
            "{credential_mismatch}"
        );

        pg_shutdown.send(true).expect("request database shutdown");
        tokio::time::timeout(Duration::from_secs(5), pg_task)
            .await
            .expect("database shutdown deadline")
            .expect("database task join");
        let rotated_pg_listener = TcpListener::bind(("127.0.0.1", pg_port))
            .await
            .expect("bind rotated database");
        let rotated_pg_options = ServerOptions::new()
            .with_host("127.0.0.1".to_owned())
            .with_port(pg_port);
        let rotated_auth = AuthConfig::password(
            "authenticator",
            std::str::from_utf8(ROTATED_TEST_DATABASE_PASSWORD).unwrap(),
            None::<(&str, &str)>,
        )
        .expect("rotated REST password auth")
        .with_role_catalog(roles)
        .expect("rotated REST role auth");
        let rotated_storage = Arc::clone(&storage);
        let (rotated_shutdown, rotated_shutdown_rx) = tokio::sync::watch::channel(false);
        let rotated_pg_task = tokio::spawn(async move {
            let _ = serve_duckdb_on_listener_until(
                rotated_storage,
                rotated_pg_listener,
                &rotated_pg_options,
                rotated_auth,
                rotated_shutdown_rx,
            )
            .await;
        });

        let mut ready_after_database_rotation = String::new();
        for _ in 0..50 {
            ready_after_database_rotation = http_request(rest_port, "GET", "/ready", None).await;
            if ready_after_database_rotation.starts_with("HTTP/1.1 200 OK") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(
            ready_after_database_rotation.starts_with("HTTP/1.1 200 OK"),
            "{ready_after_database_rotation}"
        );
        let old_database_password = connect_database(
            &format!("postgres://authenticator@127.0.0.1:{pg_port}/quackgis"),
            None,
            b"authenticator-secret",
        )
        .await;
        assert!(old_database_password.is_err());
        let preserved_after_database_rotation = http_request(
            rest_port,
            "GET",
            "/rest_points?select=id,name&order=id.asc",
            Some(&rotated_jwt),
        )
        .await;
        assert!(
            preserved_after_database_rotation.starts_with("HTTP/1.1 200 OK")
                && preserved_after_database_rotation.contains(
                    r#"[{"id":1,"name":"one"},{"id":2,"name":"two"},{"id":3,"name":"three"}]"#
                ),
            "{preserved_after_database_rotation}"
        );
        let identity = state
            .database
            .session_identity()
            .await
            .expect("REST transaction-local context cleanup");
        assert_eq!(identity.0, "authenticator");
        assert_eq!(identity.1, None);

        rest_task.abort();
        state.database.disconnect().await;
        rotated_shutdown
            .send(true)
            .expect("request rotated database shutdown");
        tokio::time::timeout(Duration::from_secs(5), rotated_pg_task)
            .await
            .expect("rotated database shutdown deadline")
            .expect("rotated database task join");
    }

    const TEST_JWT_SECRET: &[u8] = b"0123456789abcdef0123456789abcdef";
    const ROTATED_TEST_JWT_SECRET: &[u8] = b"fedcba9876543210fedcba9876543210";
    const ROTATED_TEST_DATABASE_PASSWORD: &[u8] = b"rotated-authenticator-secret";

    fn write_secret_file(path: &Path, value: &[u8]) {
        std::fs::write(path, value).expect("write secret file");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
                .expect("protect secret file");
        }
    }

    fn test_verifier(roles: &[&str]) -> (tempfile::TempDir, JwtVerifier) {
        let secret_dir = tempfile::tempdir().expect("JWT secret tempdir");
        let secret_file = secret_dir.path().join("jwt-secret");
        std::fs::write(&secret_file, TEST_JWT_SECRET).expect("write JWT secret");
        let verifier = JwtVerifier::from_file(
            &secret_file,
            "https://issuer.test".to_owned(),
            "quackgis-rest".to_owned(),
            roles.iter().map(|role| (*role).to_owned()).collect(),
        )
        .expect("test JWT verifier");
        (secret_dir, verifier)
    }

    fn test_jwt(role: &str, expires: u64) -> String {
        test_jwt_with_secret(role, expires, TEST_JWT_SECRET)
    }

    fn test_jwt_with_secret(role: &str, expires: u64, secret: &[u8]) -> String {
        sign_test_jwt_with_secret(
            serde_json::json!({"alg": "HS256", "typ": "JWT"}),
            test_claims(role, expires),
            secret,
        )
    }

    fn sign_test_jwt(header: serde_json::Value, claims: serde_json::Value) -> String {
        sign_test_jwt_with_secret(header, claims, TEST_JWT_SECRET)
    }

    fn sign_test_jwt_with_secret(
        header: serde_json::Value,
        claims: serde_json::Value,
        secret: &[u8],
    ) -> String {
        let header = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&header).unwrap());
        let claims = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap());
        let signing_input = format!("{header}.{claims}");
        let key = hmac::Key::new(hmac::HMAC_SHA256, secret);
        let signature = hmac::sign(&key, signing_input.as_bytes());
        format!(
            "{signing_input}.{}",
            URL_SAFE_NO_PAD.encode(signature.as_ref())
        )
    }

    fn test_claims(role: &str, expires: u64) -> serde_json::Value {
        serde_json::json!({
            "iss": "https://issuer.test",
            "aud": "quackgis-rest",
            "sub": "test-user",
            "exp": expires,
            "role": role,
        })
    }

    fn unix_time() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
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
