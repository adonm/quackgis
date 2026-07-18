// SPDX-License-Identifier: Apache-2.0
//! Fail-closed planning and execution for offline PostGIS snapshots.

pub mod config;
pub mod connect;
pub mod inventory;
pub mod migration;
pub mod plan;
pub mod report;
pub mod runtime;
pub mod source;

pub use config::{MigrationConfig, SourceRequirements, TableMapping};
pub use inventory::{
    ConstraintKind, SourceColumn, SourceConstraint, SourceGrant, SourceIdentity, SourceInventory,
    SourceObject, SourceObjectKind, SourceRole, SourceTable,
};
pub use migration::{
    CleanupReport, ColumnVerification, MigrationReport, MigrationState, PromotionReport,
    PromotionState, StagingCleanupReport, StagingPlan, StagingTarget, TableTransfer,
    TargetIdentity, VerificationReport, VerificationState, build_staging_config,
    cleanup_configured_targets, cleanup_staging, promote_migration_report, run_migration,
    verify_migration_report,
};
pub use plan::{
    Action, ColumnPlan, Disposition, ObjectPlan, PreflightReport, PreflightStatus, TablePlan,
    build_preflight,
};
pub use runtime::{RuntimeIdentity, RuntimeIdentityOptions, collect_runtime_identity};
pub use source::begin_source_snapshot;
