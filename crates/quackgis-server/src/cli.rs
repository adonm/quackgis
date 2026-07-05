// SPDX-License-Identifier: Apache-2.0
//! CLI argument parsing for `quackgis-server`.

use clap::Parser;

/// QuackGIS server — PostGIS-compatible SQL over pgwire, backed by SedonaDB.
#[derive(Debug, Clone, Parser)]
#[command(
    name = "quackgis-server",
    about = "PostGIS-compatible spatial database server (datafusion-postgres + SedonaDB + DuckLake)",
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
    /// M1: SQLite is the only spec-compliant write backend; PostgreSQL
    /// catalogs are upstream-experimental and not yet wired in.
    #[arg(long, env = "QUACKGIS_CATALOG_PATH", default_value = "quackgis.db")]
    pub catalog_path: String,

    /// Directory under which Parquet data files are stored. Created if
    /// missing.
    #[arg(long, env = "QUACKGIS_DATA_PATH", default_value = "./data")]
    pub data_path: String,

    /// Optional TLS certificate path (PEM). If set, `--tls-key` is required.
    #[arg(long, env = "QUACKGIS_TLS_CERT")]
    pub tls_cert: Option<String>,

    /// Optional TLS private key path (PKCS#8 PEM).
    #[arg(long, env = "QUACKGIS_TLS_KEY")]
    pub tls_key: Option<String>,

    /// Log filter (`env_logger` syntax). Falls back to the `RUST_LOG` env var.
    #[arg(long, env = "QUACKGIS_LOG", default_value = "info")]
    pub log: String,
}
