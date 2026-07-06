// SPDX-License-Identifier: Apache-2.0
//! QuackGIS server library. Exposes the session-construction integration so
//! the binary, integration tests, and (later) the DuckLake/PostGIS-compat
//! crates can share it.

pub mod cli;
pub mod context;

pub mod catalog_compat;
pub mod ducklake_sql;
pub mod geometry_columns;
pub mod postgis_compat;
pub mod public_schema;

pub mod mvt;
pub mod spatial_udfs;
