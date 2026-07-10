// SPDX-License-Identifier: Apache-2.0
//! CLI argument parsing for `quackgis-server`.

use clap::{Parser, ValueEnum};

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum CliAuthMode {
    /// Trust startup packets and do not request a password. Development only.
    Trust,
    /// Require PostgreSQL SCRAM-SHA-256 password authentication for configured users.
    Password,
}

/// QuackGIS server — PostGIS-compatible SQL over pgwire, backed by SedonaDB.
#[derive(Debug, Clone, Parser)]
#[command(
    name = "quackgis-server",
    about = "PostGIS-compatible SedonaDB + DuckLake spatial lakehouse server",
    long_about = None,
    version
)]
pub struct Cli {
    /// Bind host.
    #[arg(long, env = "QUACKGIS_HOST", default_value = "127.0.0.1")]
    pub host: String,

    /// Bind port. Default 5434 to coexist with a system PostgreSQL on 5432.
    #[arg(long, env = "QUACKGIS_PORT", default_value_t = 5434)]
    pub port: u16,

    /// Path to the DuckLake SQLite catalog file. Created if missing.
    /// Ignored when --catalog-url is set.
    #[arg(long, env = "QUACKGIS_CATALOG_PATH", default_value = "quackgis.db")]
    pub catalog_path: String,

    /// PostgreSQL connection URL for the DuckLake catalog metadata store.
    /// When set, QuackGIS uses the DuckLake multicatalog PostgreSQL writer.
    #[arg(long, env = "QUACKGIS_CATALOG_URL")]
    pub catalog_url: Option<String>,

    /// DuckLake catalog name inside the PostgreSQL metadata store.
    #[arg(
        long,
        env = "QUACKGIS_DUCKLAKE_CATALOG_NAME",
        default_value = "quackgis"
    )]
    pub ducklake_catalog_name: String,

    /// Directory under which Parquet data files are stored. Created if
    /// missing for local paths. May also be an S3 URL such as
    /// s3://bucket/prefix for S3-compatible object storage.
    #[arg(long, env = "QUACKGIS_DATA_PATH", default_value = "./data")]
    pub data_path: String,

    /// S3-compatible endpoint URL for object storage, e.g. http://s3s-fs:8014.
    #[arg(long, env = "QUACKGIS_S3_ENDPOINT")]
    pub s3_endpoint: Option<String>,

    /// S3 access key for object storage.
    #[arg(long, env = "QUACKGIS_S3_ACCESS_KEY_ID")]
    pub s3_access_key_id: Option<String>,

    /// S3 secret key for object storage.
    #[arg(long, env = "QUACKGIS_S3_SECRET_ACCESS_KEY")]
    pub s3_secret_access_key: Option<String>,

    /// S3 region used by the object-store client.
    #[arg(long, env = "QUACKGIS_S3_REGION", default_value = "us-east-1")]
    pub s3_region: String,

    /// Allow plain HTTP for S3-compatible development endpoints such as s3s-fs.
    #[arg(long, env = "QUACKGIS_S3_ALLOW_HTTP", default_value_t = false)]
    pub s3_allow_http: bool,

    /// Optional TLS certificate path (PEM). If set, `--tls-key` is required.
    #[arg(long, env = "QUACKGIS_TLS_CERT")]
    pub tls_cert: Option<String>,

    /// Optional TLS private key path (PKCS#8 PEM).
    #[arg(long, env = "QUACKGIS_TLS_KEY")]
    pub tls_key: Option<String>,

    /// Pgwire authentication mode. `trust` is for local/dev only.
    #[arg(long, env = "QUACKGIS_AUTH_MODE", value_enum, default_value_t = CliAuthMode::Trust)]
    pub auth_mode: CliAuthMode,

    /// Read/write login user for password auth mode.
    #[arg(long, env = "QUACKGIS_READWRITE_USER", default_value = "postgres")]
    pub readwrite_user: String,

    /// Read/write login password; required when auth mode is `password`.
    #[arg(long, env = "QUACKGIS_READWRITE_PASSWORD")]
    pub readwrite_password: Option<String>,

    /// Optional read-only login user for password auth mode.
    #[arg(
        long,
        env = "QUACKGIS_READONLY_USER",
        default_value = "quackgis_readonly"
    )]
    pub readonly_user: String,

    /// Optional read-only login password. If unset, the read-only role is disabled.
    #[arg(long, env = "QUACKGIS_READONLY_PASSWORD")]
    pub readonly_password: Option<String>,

    /// Optional comma-separated DuckLake table allowlist for write-capable
    /// identities. Entries may be table, public.table, main.table, or
    /// quackgis.main.table. When set, indeterminate write/maintenance statements
    /// are denied before planning.
    #[arg(long, env = "QUACKGIS_WRITE_ALLOWLIST")]
    pub write_allowlist: Option<String>,

    /// Log filter (`env_logger` syntax). Falls back to the `RUST_LOG` env var.
    #[arg(long, env = "QUACKGIS_LOG", default_value = "info")]
    pub log: String,

    /// Bind host for the optional Prometheus metrics endpoint.
    #[arg(long, env = "QUACKGIS_METRICS_HOST", default_value = "127.0.0.1")]
    pub metrics_host: String,

    /// Optional Prometheus metrics endpoint port. Disabled when unset.
    #[arg(long, env = "QUACKGIS_METRICS_PORT")]
    pub metrics_port: Option<u16>,

    /// Inventory unreferenced Parquet candidates and exit. This is always a dry
    /// run; no catalog rows or objects are deleted.
    #[arg(long, default_value_t = false)]
    pub orphan_inventory: bool,

    /// Ignore unreferenced files newer than this age. Must be greater than zero.
    #[arg(long, default_value_t = 3600, requires = "orphan_inventory")]
    pub orphan_min_age_seconds: u64,

    /// Print candidate paths. By default only the redaction-safe count is shown.
    #[arg(long, default_value_t = false, requires = "orphan_inventory")]
    pub orphan_show_paths: bool,
}
