//! Arrow data encoding and type mapping for Postgres(pgwire).

pub mod datatypes;
pub mod encoder;
mod error;
pub mod list_encoder;
pub mod row_encoder;
pub mod struct_encoder;

pub use datatypes::encode_recordbatch;
