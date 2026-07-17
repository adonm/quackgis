// SPDX-License-Identifier: Apache-2.0

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceIdentity {
    pub server_version_num: u32,
    pub server_version: String,
    pub database_name: String,
    pub database_oid: u32,
    pub postgis_version: String,
    pub snapshot_started_at: String,
    pub snapshot_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceInventory {
    pub identity: SourceIdentity,
    pub tables: Vec<SourceTable>,
    pub objects: Vec<SourceObject>,
    pub roles: Vec<SourceRole>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceTable {
    pub schema: String,
    pub name: String,
    pub row_count: u64,
    pub estimated_bytes: u64,
    pub comment: Option<String>,
    pub row_security: bool,
    pub force_row_security: bool,
    pub replica_identity: String,
    pub partitioned: bool,
    pub trigger_count: u32,
    pub columns: Vec<SourceColumn>,
    pub constraints: Vec<SourceConstraint>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceColumn {
    pub position: i16,
    pub name: String,
    pub type_name: String,
    pub formatted_type: String,
    pub type_modifier: i32,
    pub nullable: bool,
    pub default_expression: Option<String>,
    pub comment: Option<String>,
    pub identity: bool,
    pub generated: bool,
    pub geometry_type: Option<String>,
    pub geometry_srid: Option<i32>,
    pub geometry_dimensions: Option<i32>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceConstraint {
    pub name: String,
    pub kind: ConstraintKind,
    pub definition: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConstraintKind {
    Check,
    PrimaryKey,
    Unique,
    ForeignKey,
    Exclusion,
    NotNull,
    Other,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceObject {
    pub schema: String,
    pub name: String,
    pub kind: SourceObjectKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceObjectKind {
    View,
    MaterializedView,
    Sequence,
    ForeignTable,
    Index,
    Function,
    Trigger,
    Other,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceRole {
    pub name: String,
    pub login: bool,
}
