// SPDX-License-Identifier: Apache-2.0
//! QuackGIS server library. Exposes the session-construction integration so
//! the binary, integration tests, and (later) the DuckLake/PostGIS-compat
//! crates can share it.

pub mod cli;
pub mod context;
