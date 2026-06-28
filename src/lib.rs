// SPDX-License-Identifier: Apache-2.0
//
//! # duckdb_sedona
//!
//! A DuckDB loadable extension exposing a catalog of vector spatial (`ST_*`)
//! functions over WKB-encoded geometries.
//!
//! ## Architecture in one paragraph
//!
//! Rather than writing one FFI callback per SQL function, this extension uses
//! a *Unified Vectorized Dispatch Pipeline*: a handful of generic executors
//! ([`dispatch`]) read WKB blobs out of DuckDB's columnar vectors, apply a
//! geometry operation, and write the result back. The [`registry`] module
//! then maps every SQL name to one of those executors through a declarative
//! macro matrix, so adding a function is a single line. The geometry work is
//! done by the [`geo`] crate — the same library Apache SedonaDB wraps —
//! reading and writing WKB via the [`wkb`] crate.
//!
//! See `README.md` for the full design, the function catalog, and build /
//! load instructions.
//!
//! ## Loading
//!
//! ```sql
//! INSTALL sedonadb;
//! LOAD sedonadb;
//! SELECT st_geometrytype(geom) FROM my_table;
//! ```
//!
//! The extension symbol that DuckDB looks up is `sedonadb_init_c_api`, emitted
//! by the [`quack_rs::entry_point!`] macro below.

#![deny(unsafe_op_in_unsafe_fn)]

// The dispatch executors take opaque DuckDB pointer handles
// (`duckdb_data_chunk`, `duckdb_vector`) provided by DuckDB callbacks; their
// safety contract is documented at each call site, so we silence clippy's
// `not_unsafe_ptr_arg_deref` for that module.
#[allow(clippy::not_unsafe_ptr_arg_deref, clippy::missing_safety_doc)]
pub mod dispatch;
#[allow(clippy::needless_borrow)]
pub mod functions;
pub mod geometry;
pub mod raster;
pub mod registry;
pub mod spatial_join;

use quack_rs::entry_point;

// The DuckDB entry point. The macro emits a `#[no_mangle] pub unsafe extern
// "C" fn sedonadb_init_c_api(...) -> bool` that opens a connection and hands
// it to `registry::register_all`. The symbol name MUST follow DuckDB's
// `<extension_name>_init_c_api` convention for an extension named `sedonadb`.
entry_point!(sedonadb_init_c_api, registry::register_all);
