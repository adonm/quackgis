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
    #[arg(long, env = "QUACKGIS_HOST", default_value = "127.0.0.1")]
    pub host: String,
    #[arg(long, env = "QUACKGIS_PORT", default_value_t = 5434)]
    pub port: u16,
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
    #[arg(long, env = "QUACKGIS_LOG", default_value = "info")]
    pub log: String,
    #[arg(long, env = "QUACKGIS_METRICS_HOST", default_value = "127.0.0.1")]
    pub metrics_host: String,
    #[arg(long, env = "QUACKGIS_METRICS_PORT")]
    pub metrics_port: Option<u16>,
}
