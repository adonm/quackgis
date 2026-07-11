// SPDX-License-Identifier: Apache-2.0
//! Server engine selection and the D0 adapter boundary.
//!
//! The legacy adapter still returns a DataFusion `SessionContext`; the feature-
//! gated DuckDB adapter owns its ADBC lifecycle in the pgwire server.

use std::fmt;
use std::sync::Arc;

use anyhow::{Result, bail};
use datafusion::prelude::SessionContext;

use crate::auth::AuthConfig;
use crate::context::StoragePaths;
use crate::storage_authority::StorageAuthority;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum EngineBackend {
    #[default]
    LegacyDataFusion,
    DuckDb,
}

impl fmt::Display for EngineBackend {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::LegacyDataFusion => "legacy-datafusion",
            Self::DuckDb => "duckdb",
        })
    }
}

/// A backend that is currently safe to expose through the pgwire server.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServerEngine {
    LegacyDataFusion,
    #[cfg(feature = "duckdb-adbc")]
    DuckDb,
}

impl ServerEngine {
    pub fn select(requested: EngineBackend) -> Result<Self> {
        match requested {
            EngineBackend::LegacyDataFusion => Ok(Self::LegacyDataFusion),
            EngineBackend::DuckDb => {
                #[cfg(feature = "duckdb-adbc")]
                {
                    Ok(Self::DuckDb)
                }
                #[cfg(not(feature = "duckdb-adbc"))]
                {
                    bail!(
                        "engine backend duckdb requires a binary built with --features duckdb-adbc"
                    )
                }
            }
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::LegacyDataFusion => "legacy-datafusion",
            #[cfg(feature = "duckdb-adbc")]
            Self::DuckDb => "duckdb",
        }
    }

    pub const fn storage_authority(self) -> StorageAuthority {
        match self {
            Self::LegacyDataFusion => StorageAuthority::LegacyDataFusionDuckLake,
            #[cfg(feature = "duckdb-adbc")]
            Self::DuckDb => StorageAuthority::DuckDbOfficialDuckLake,
        }
    }

    pub async fn claim_storage_authority(self, storage: &StoragePaths) -> Result<()> {
        storage.claim_authority(self.storage_authority()).await
    }

    pub async fn build_session_context(
        self,
        storage: StoragePaths,
        auth: &AuthConfig,
    ) -> Result<Arc<SessionContext>> {
        match self {
            Self::LegacyDataFusion => {
                crate::context::build_session_context_with_storage_and_auth(storage, auth).await
            }
            #[cfg(feature = "duckdb-adbc")]
            Self::DuckDb => bail!("DuckDB does not build a DataFusion session context"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_selection_matches_compiled_capabilities() {
        assert_eq!(
            ServerEngine::select(EngineBackend::LegacyDataFusion)
                .expect("legacy backend")
                .name(),
            "legacy-datafusion"
        );
        #[cfg(feature = "duckdb-adbc")]
        assert_eq!(
            ServerEngine::select(EngineBackend::DuckDb)
                .expect("feature-gated DuckDB backend")
                .name(),
            "duckdb"
        );
        #[cfg(not(feature = "duckdb-adbc"))]
        assert!(ServerEngine::select(EngineBackend::DuckDb).is_err());
    }
}
