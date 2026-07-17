// SPDX-License-Identifier: Apache-2.0
//! Fail-closed planning and execution for offline PostGIS snapshots.

pub mod config;
pub mod inventory;
pub mod plan;

pub use config::{MigrationConfig, TableMapping};
pub use inventory::{
    ConstraintKind, SourceColumn, SourceConstraint, SourceIdentity, SourceInventory, SourceObject,
    SourceObjectKind, SourceRole, SourceTable,
};
pub use plan::{
    Action, ColumnPlan, Disposition, ObjectPlan, PreflightReport, PreflightStatus, TablePlan,
    build_preflight,
};
