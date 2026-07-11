// SPDX-License-Identifier: Apache-2.0
//! QuackGIS server library. Exposes the session-construction integration so
//! the binary, integration tests, and (later) the DuckLake/PostGIS-compat
//! crates can share it.

pub mod cli;
pub mod context;

pub mod audit;
pub mod auth;
pub mod catalog_compat;
mod catalog_metrics;
#[cfg(feature = "duckdb-adbc")]
pub mod duckdb_adbc_storage;
pub mod ducklake_sql;
pub mod engine;
pub mod engine_api;
pub mod geometry_columns;
pub mod metrics;
pub mod pgwire_server;
pub mod postgis_compat;
pub mod public_schema;
pub mod storage_authority;

pub mod mvt;
pub mod spatial_udfs;
