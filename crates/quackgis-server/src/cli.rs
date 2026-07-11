// SPDX-License-Identifier: Apache-2.0
//! CLI argument parsing for the DuckDB-only server.

use clap::{Parser, ValueEnum};

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum CliAuthMode {
    Trust,
    Password,
}

#[derive(Debug, Clone, Parser)]
#[command(
    name = "quackgis-server",
    about = "PostGIS-compatible DuckDB + official-DuckLake server",
    version
)]
pub struct Cli {
    #[arg(long, env = "QUACKGIS_DUCKDB_ADBC_DRIVER")]
    pub duckdb_driver: Option<std::path::PathBuf>,
    #[arg(long, env = "QUACKGIS_DUCKDB_DATABASE_URI", default_value = ":memory:")]
    pub duckdb_database_uri: String,
    #[arg(long, env = "QUACKGIS_DUCKDB_THREADS")]
    pub duckdb_threads: Option<usize>,
    #[arg(long, env = "QUACKGIS_DUCKDB_MEMORY_LIMIT_BYTES")]
    pub duckdb_memory_limit_bytes: Option<u64>,
    #[arg(long, env = "QUACKGIS_DUCKDB_TEMP_DIRECTORY")]
    pub duckdb_temp_directory: Option<std::path::PathBuf>,
    #[arg(
        long,
        env = "QUACKGIS_DUCKDB_MAX_TEMP_DIRECTORY_BYTES",
        value_name = "BYTES"
    )]
    pub duckdb_max_temp_directory_bytes: Option<u64>,
    #[arg(long, env = "QUACKGIS_HOST", default_value = "127.0.0.1")]
    pub host: String,
    #[arg(long, env = "QUACKGIS_PORT", default_value_t = 5434)]
    pub port: u16,
    #[arg(long, env = "QUACKGIS_MAX_CONNECTIONS", default_value_t = 64)]
    pub max_connections: usize,
    #[arg(long, env = "QUACKGIS_MAX_ACTIVE_QUERIES", default_value_t = 8)]
    pub max_active_queries: usize,
    #[arg(long, env = "QUACKGIS_MAX_READER_QUERIES", default_value_t = 8)]
    pub max_reader_queries: usize,
    #[arg(long, env = "QUACKGIS_MAX_WRITER_QUERIES", default_value_t = 2)]
    pub max_writer_queries: usize,
    #[arg(long, env = "QUACKGIS_MAX_MAINTENANCE_QUERIES", default_value_t = 1)]
    pub max_maintenance_queries: usize,
    #[arg(long, env = "QUACKGIS_MAX_QUEUED_QUERIES", default_value_t = 64)]
    pub max_queued_queries: usize,
    #[arg(long, env = "QUACKGIS_MAX_BLOCKING_WORKERS", default_value_t = 9)]
    pub max_blocking_workers: usize,
    #[arg(long, env = "QUACKGIS_QUEUE_TIMEOUT_MS", default_value_t = 30_000)]
    pub queue_timeout_ms: u64,
    #[arg(long, env = "QUACKGIS_STATEMENT_TIMEOUT_MS", default_value_t = 300_000)]
    pub statement_timeout_ms: u64,
    #[arg(long, env = "QUACKGIS_SHUTDOWN_TIMEOUT_MS", default_value_t = 30_000)]
    pub shutdown_timeout_ms: u64,
    #[arg(long, env = "QUACKGIS_RESULT_BATCH_BYTES", default_value_t = 8_388_608)]
    pub result_batch_bytes: usize,
    #[arg(long, env = "QUACKGIS_COPY_BATCH_ROWS", default_value_t = 65_536)]
    pub copy_batch_rows: usize,
    #[arg(long, env = "QUACKGIS_COPY_BATCH_BYTES", default_value_t = 8_388_608)]
    pub copy_batch_bytes: usize,
    #[arg(long, env = "QUACKGIS_COPY_MAX_ROW_BYTES", default_value_t = 1_048_576)]
    pub copy_max_row_bytes: usize,
    #[arg(
        long,
        env = "QUACKGIS_CATALOG_PATH",
        default_value = "quackgis.ducklake"
    )]
    pub catalog_path: String,
    /// Reserved for the future official shared profile; currently rejected.
    #[arg(long, env = "QUACKGIS_CATALOG_URL")]
    pub catalog_url: Option<String>,
    #[arg(
        long,
        env = "QUACKGIS_DUCKLAKE_CATALOG_NAME",
        default_value = "quackgis"
    )]
    pub ducklake_catalog_name: String,
    #[arg(long, env = "QUACKGIS_DATA_PATH", default_value = "./data")]
    pub data_path: String,
    #[arg(long, env = "QUACKGIS_TLS_CERT")]
    pub tls_cert: Option<String>,
    #[arg(long, env = "QUACKGIS_TLS_KEY")]
    pub tls_key: Option<String>,
    #[arg(long, env = "QUACKGIS_AUTH_MODE", value_enum, default_value_t = CliAuthMode::Trust)]
    pub auth_mode: CliAuthMode,
    #[arg(long, env = "QUACKGIS_READWRITE_USER", default_value = "postgres")]
    pub readwrite_user: String,
    #[arg(long, env = "QUACKGIS_READWRITE_PASSWORD")]
    pub readwrite_password: Option<String>,
    #[arg(
        long,
        env = "QUACKGIS_READONLY_USER",
        default_value = "quackgis_readonly"
    )]
    pub readonly_user: String,
    #[arg(long, env = "QUACKGIS_READONLY_PASSWORD")]
    pub readonly_password: Option<String>,
    #[arg(long, env = "QUACKGIS_WRITE_ALLOWLIST")]
    pub write_allowlist: Option<String>,
    #[arg(long, env = "QUACKGIS_READ_ALLOWLIST")]
    pub read_allowlist: Option<String>,
    #[arg(long, env = "QUACKGIS_MAINTENANCE_USER")]
    pub maintenance_user: Option<String>,
    #[arg(long, env = "QUACKGIS_LOG", default_value = "info")]
    pub log: String,
    #[arg(long, env = "QUACKGIS_METRICS_HOST", default_value = "127.0.0.1")]
    pub metrics_host: String,
    #[arg(long, env = "QUACKGIS_METRICS_PORT")]
    pub metrics_port: Option<u16>,
}
