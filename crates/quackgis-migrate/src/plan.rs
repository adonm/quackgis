// SPDX-License-Identifier: Apache-2.0

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use sqlparser::ast::{Expr, SelectItem, SetExpr, Statement, UnaryOperator, ValueWithSpan};
use sqlparser::dialect::PostgreSqlDialect;
use sqlparser::parser::Parser;

use crate::config::{MigrationConfig, TableMapping};
use crate::inventory::{ConstraintKind, SourceColumn, SourceInventory, SourceObjectKind};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreflightStatus {
    Ready,
    Rejected,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Migrate,
    Map,
    Reject,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Disposition {
    pub action: Action,
    pub reason: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PreflightReport {
    pub format_version: u32,
    pub status: PreflightStatus,
    pub source: crate::inventory::SourceIdentity,
    pub tables: Vec<TablePlan>,
    pub objects: Vec<ObjectPlan>,
    pub roles: Vec<ObjectPlan>,
    pub grants: Vec<ObjectPlan>,
    pub errors: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TablePlan {
    pub source_schema: String,
    pub source_table: String,
    pub target_schema: Option<String>,
    pub target_table: Option<String>,
    pub row_count: u64,
    pub estimated_bytes: u64,
    pub comment: Option<String>,
    pub disposition: Disposition,
    pub columns: Vec<ColumnPlan>,
    pub blockers: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ColumnPlan {
    pub source_name: String,
    pub source_type: String,
    pub target_name: Option<String>,
    pub target_type: Option<String>,
    pub nullable: bool,
    pub default_expression: Option<String>,
    pub comment: Option<String>,
    pub disposition: Disposition,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ObjectPlan {
    pub identity: String,
    pub kind: String,
    pub disposition: Disposition,
}

pub fn build_preflight(config: &MigrationConfig, inventory: SourceInventory) -> PreflightReport {
    let mut errors = Vec::new();
    let mut seen_mappings = HashSet::new();
    let mut tables = inventory
        .tables
        .iter()
        .map(|table| {
            let mapping = config.table_mapping(&table.schema, &table.name);
            if mapping.is_some() {
                seen_mappings.insert((table.schema.as_str(), table.name.as_str()));
            }
            build_table_plan(table, mapping)
        })
        .collect::<Vec<_>>();

    for mapping in &config.tables {
        if !seen_mappings.contains(&(
            mapping.source_schema.as_str(),
            mapping.source_table.as_str(),
        )) {
            let message = format!(
                "configured source table {}.{} does not exist",
                mapping.source_schema, mapping.source_table
            );
            errors.push(message.clone());
            tables.push(TablePlan {
                source_schema: mapping.source_schema.clone(),
                source_table: mapping.source_table.clone(),
                target_schema: Some(mapping.target_schema.clone()),
                target_table: Some(mapping.target_table.clone()),
                row_count: 0,
                estimated_bytes: 0,
                comment: None,
                disposition: reject(&message),
                columns: vec![],
                blockers: vec![message],
            });
        }
    }
    tables.sort_by(|left, right| {
        (&left.source_schema, &left.source_table).cmp(&(&right.source_schema, &right.source_table))
    });

    let objects = inventory
        .objects
        .iter()
        .map(|object| ObjectPlan {
            identity: format!("{}.{}", object.schema, object.name),
            kind: format!("{:?}", object.kind).to_lowercase(),
            disposition: reject(match object.kind {
                SourceObjectKind::Extension => "extensions are not copied implicitly",
                SourceObjectKind::View => "views are not copied implicitly",
                SourceObjectKind::MaterializedView => {
                    "materialized views are not copied implicitly"
                }
                SourceObjectKind::Sequence => "sequences are not copied implicitly",
                SourceObjectKind::ForeignTable => "foreign tables are not copied implicitly",
                SourceObjectKind::Index => "indexes are not copied without target key semantics",
                SourceObjectKind::Function => "functions are not copied implicitly",
                SourceObjectKind::Trigger => "triggers are not copied implicitly",
                SourceObjectKind::Other => "source object kind is unsupported",
            }),
        })
        .collect();
    let grants = inventory
        .grants
        .iter()
        .map(|grant| ObjectPlan {
            identity: format!("{}:{}", grant.object_identity, grant.grantee),
            kind: grant.privilege.to_ascii_lowercase(),
            disposition: reject("source grants require an explicit target role/grant mapping"),
        })
        .collect();
    let roles = inventory
        .roles
        .iter()
        .map(|role| ObjectPlan {
            identity: role.name.clone(),
            kind: if role.login { "login_role" } else { "role" }.to_owned(),
            disposition: reject("roles and passwords are not copied implicitly"),
        })
        .collect();
    let status = if errors.is_empty()
        && tables
            .iter()
            .filter(|table| table.target_table.is_some())
            .all(|table| table.disposition.action != Action::Reject && table.blockers.is_empty())
    {
        PreflightStatus::Ready
    } else {
        PreflightStatus::Rejected
    };
    PreflightReport {
        format_version: 1,
        status,
        source: inventory.identity,
        tables,
        objects,
        roles,
        grants,
        errors,
    }
}

fn build_table_plan(
    table: &crate::inventory::SourceTable,
    mapping: Option<&TableMapping>,
) -> TablePlan {
    let Some(mapping) = mapping else {
        return TablePlan {
            source_schema: table.schema.clone(),
            source_table: table.name.clone(),
            target_schema: None,
            target_table: None,
            row_count: table.row_count,
            estimated_bytes: table.estimated_bytes,
            comment: table.comment.clone(),
            disposition: reject("table is outside the explicit migration selection"),
            columns: table
                .columns
                .iter()
                .map(|column| rejected_column(column, "table is not selected"))
                .collect(),
            blockers: vec![],
        };
    };

    let mut blockers = Vec::new();
    if table.partitioned {
        blockers.push("partitioned tables are unsupported".to_owned());
    }
    if table.row_security || table.force_row_security {
        blockers.push("row-level security is not copied implicitly".to_owned());
    }
    if table.trigger_count != 0 {
        blockers.push(format!(
            "{} user trigger(s) require explicit rejection",
            table.trigger_count
        ));
    }
    if table.replica_identity != "default" && table.replica_identity != "nothing" {
        blockers.push(format!(
            "replica identity {:?} has no maintained target semantics",
            table.replica_identity
        ));
    }
    for constraint in &table.constraints {
        if constraint.kind != ConstraintKind::NotNull {
            blockers.push(format!(
                "constraint {:?} ({}) is unsupported",
                constraint.kind, constraint.name
            ));
        }
    }

    let mut target_names = HashSet::new();
    let mut columns = Vec::with_capacity(table.columns.len());
    for column in &table.columns {
        let target_name = mapping
            .column_mappings
            .get(&column.name)
            .cloned()
            .unwrap_or_else(|| column.name.clone());
        let mut column_plan = classify_column(column, target_name);
        if let Some(target) = &column_plan.target_name
            && !target_names.insert(target.clone())
        {
            column_plan.disposition = reject("multiple source columns map to this target name");
            column_plan.target_type = None;
        }
        if column_plan.disposition.action == Action::Reject {
            blockers.push(format!(
                "column {}: {}",
                column.name, column_plan.disposition.reason
            ));
        }
        columns.push(column_plan);
    }
    for source in mapping.column_mappings.keys() {
        if !table.columns.iter().any(|column| &column.name == source) {
            blockers.push(format!(
                "column mapping references missing source column {source:?}"
            ));
        }
    }
    blockers.sort();
    blockers.dedup();
    TablePlan {
        source_schema: table.schema.clone(),
        source_table: table.name.clone(),
        target_schema: Some(mapping.target_schema.clone()),
        target_table: Some(mapping.target_table.clone()),
        row_count: table.row_count,
        estimated_bytes: table.estimated_bytes,
        comment: table.comment.clone(),
        disposition: if blockers.is_empty() {
            migrate("selected base table has a complete supported column mapping")
        } else {
            reject("selected table has blocking unsupported semantics")
        },
        columns,
        blockers,
    }
}

fn classify_column(column: &SourceColumn, target_name: String) -> ColumnPlan {
    let rejected = |reason| ColumnPlan {
        source_name: column.name.clone(),
        source_type: column.formatted_type.clone(),
        target_name: Some(target_name.clone()),
        target_type: None,
        nullable: column.nullable,
        default_expression: None,
        comment: column.comment.clone(),
        disposition: reject(reason),
    };
    if column.identity {
        return rejected("identity generation is unsupported");
    }
    if column.generated {
        return rejected("generated columns are unsupported");
    }
    let default_expression = match column.default_expression.as_deref() {
        Some(raw) => match normalize_literal_default(raw) {
            Some(normalized) => Some(normalized),
            None => return rejected("default is not a literal or literal cast"),
        },
        None => None,
    };
    let (target_type, disposition) = match column.type_name.as_str() {
        "int2" => ("SMALLINT".to_owned(), migrate("exact release scalar")),
        "int4" => ("INTEGER".to_owned(), migrate("exact release scalar")),
        "int8" => ("BIGINT".to_owned(), migrate("exact release scalar")),
        "bool" => ("BOOLEAN".to_owned(), migrate("exact release scalar")),
        "float4" => ("REAL".to_owned(), migrate("exact release scalar")),
        "float8" => ("DOUBLE".to_owned(), migrate("exact release scalar")),
        "date" => ("DATE".to_owned(), migrate("exact release scalar")),
        "timestamp" => (
            timestamp_target_type(column.type_modifier),
            migrate("timestamp without time zone uses microsecond target precision"),
        ),
        "varchar" => (
            varchar_target_type(column.type_modifier),
            migrate("character varying length is retained"),
        ),
        "text" => (
            "VARCHAR".to_owned(),
            map("PostgreSQL text maps to unbounded DuckDB VARCHAR"),
        ),
        "bytea" => (
            "BLOB".to_owned(),
            map("PostgreSQL bytea maps to DuckDB BLOB"),
        ),
        "numeric" => match decimal_target_type(column.type_modifier) {
            Some(target) => (
                target,
                migrate("bounded decimal precision and scale are retained"),
            ),
            None => return rejected("unbounded or invalid numeric typmod is unsupported"),
        },
        "geometry" => {
            if column.geometry_type.as_deref() != Some("Point")
                || column.geometry_srid != Some(0)
                || column.geometry_dimensions != Some(2)
            {
                return rejected("only 2D SRID-0 Point geometry is supported");
            }
            if !is_geometry_target_name(&target_name) {
                return rejected("geometry target column needs a maintained WKB geometry name");
            }
            (
                "BLOB".to_owned(),
                map("2D SRID-0 Point is converted to canonical NDR WKB"),
            )
        }
        "geography" => return rejected("geography target semantics are not authoritative"),
        _ => return rejected("source type is outside the release migration scalar set"),
    };
    ColumnPlan {
        source_name: column.name.clone(),
        source_type: column.formatted_type.clone(),
        target_name: Some(target_name),
        target_type: Some(target_type),
        nullable: column.nullable,
        default_expression,
        comment: column.comment.clone(),
        disposition,
    }
}

fn rejected_column(column: &SourceColumn, reason: &str) -> ColumnPlan {
    ColumnPlan {
        source_name: column.name.clone(),
        source_type: column.formatted_type.clone(),
        target_name: None,
        target_type: None,
        nullable: column.nullable,
        default_expression: None,
        comment: column.comment.clone(),
        disposition: reject(reason),
    }
}

fn varchar_target_type(type_modifier: i32) -> String {
    let length = type_modifier.saturating_sub(4);
    if length > 0 {
        format!("VARCHAR({length})")
    } else {
        "VARCHAR".to_owned()
    }
}

fn timestamp_target_type(type_modifier: i32) -> String {
    if (0..=6).contains(&type_modifier) {
        format!("TIMESTAMP({type_modifier})")
    } else {
        "TIMESTAMP".to_owned()
    }
}

fn decimal_target_type(type_modifier: i32) -> Option<String> {
    if type_modifier < 4 {
        return None;
    }
    let modifier = type_modifier - 4;
    let precision = (modifier >> 16) & 0xffff;
    let scale = modifier & 0x7ff;
    (precision > 0 && precision <= 38 && scale <= precision)
        .then(|| format!("DECIMAL({precision},{scale})"))
}

fn is_geometry_target_name(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "geom"
            | "geometry"
            | "the_geom"
            | "wkb_geometry"
            | "wkb_geom"
            | "geom_wkb"
            | "shape"
            | "footprint"
            | "way"
    )
}

fn normalize_literal_default(raw: &str) -> Option<String> {
    let mut statements = Parser::parse_sql(&PostgreSqlDialect {}, &format!("SELECT {raw}")).ok()?;
    if statements.len() != 1 {
        return None;
    }
    let Statement::Query(query) = statements.remove(0) else {
        return None;
    };
    let SetExpr::Select(select) = *query.body else {
        return None;
    };
    let [SelectItem::UnnamedExpr(expression)] = select.projection.as_slice() else {
        return None;
    };
    literal_expression(expression).then(|| expression.to_string())
}

fn literal_expression(expression: &Expr) -> bool {
    match expression {
        Expr::Value(ValueWithSpan { .. }) | Expr::TypedString { .. } => true,
        Expr::Nested(expression)
        | Expr::Cast {
            expr: expression, ..
        } => literal_expression(expression),
        Expr::UnaryOp {
            op: UnaryOperator::Plus | UnaryOperator::Minus,
            expr,
        } => literal_expression(expr),
        _ => false,
    }
}

fn migrate(reason: &str) -> Disposition {
    Disposition {
        action: Action::Migrate,
        reason: reason.to_owned(),
    }
}

fn map(reason: &str) -> Disposition {
    Disposition {
        action: Action::Map,
        reason: reason.to_owned(),
    }
}

fn reject(reason: &str) -> Disposition {
    Disposition {
        action: Action::Reject,
        reason: reason.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::config::TableMapping;
    use crate::inventory::{
        SourceColumn, SourceConstraint, SourceIdentity, SourceInventory, SourceObject,
        SourceObjectKind, SourceRole, SourceTable,
    };

    fn identity() -> SourceIdentity {
        SourceIdentity {
            server_version_num: 180004,
            server_version: "18.4".to_owned(),
            database_name: "fixture".to_owned(),
            database_oid: 16_384,
            postgis_version: "3.6.1".to_owned(),
            snapshot_started_at: "2026-07-18T00:00:00Z".to_owned(),
            snapshot_id: "100:100:".to_owned(),
        }
    }

    fn column(position: i16, name: &str, type_name: &str, formatted_type: &str) -> SourceColumn {
        SourceColumn {
            position,
            name: name.to_owned(),
            type_name: type_name.to_owned(),
            formatted_type: formatted_type.to_owned(),
            type_modifier: -1,
            nullable: true,
            default_expression: None,
            comment: None,
            identity: false,
            generated: false,
            geometry_type: None,
            geometry_srid: None,
            geometry_dimensions: None,
        }
    }

    fn table() -> SourceTable {
        let mut amount = column(3, "amount", "numeric", "numeric(10,2)");
        amount.type_modifier = 4 + (10 << 16) + 2;
        amount.nullable = false;
        amount.default_expression = Some("12.34".to_owned());
        amount.comment = Some("declared amount".to_owned());
        let mut geometry = column(4, "location", "geometry", "geometry(Point)");
        geometry.geometry_type = Some("Point".to_owned());
        geometry.geometry_srid = Some(0);
        geometry.geometry_dimensions = Some(2);
        SourceTable {
            schema: "public".to_owned(),
            name: "places".to_owned(),
            row_count: 2,
            estimated_bytes: 8192,
            comment: Some("fixture places".to_owned()),
            row_security: false,
            force_row_security: false,
            replica_identity: "default".to_owned(),
            partitioned: false,
            trigger_count: 0,
            columns: vec![
                column(1, "id", "int8", "bigint"),
                column(2, "label", "text", "text"),
                amount,
                geometry,
            ],
            constraints: vec![],
        }
    }

    fn config() -> MigrationConfig {
        MigrationConfig {
            format_version: 1,
            source: crate::config::SourceRequirements {
                postgres_version_num: 180_004,
                postgis_version: "3.6.1".to_owned(),
            },
            source_schemas: vec!["public".to_owned()],
            tables: vec![TableMapping {
                source_schema: "public".to_owned(),
                source_table: "places".to_owned(),
                target_schema: "main".to_owned(),
                target_table: "places_copy".to_owned(),
                column_mappings: BTreeMap::from([("location".to_owned(), "geom_wkb".to_owned())]),
            }],
        }
    }

    #[test]
    fn plans_release_scalars_and_point_wkb_without_omitting_objects() {
        let report = build_preflight(
            &config(),
            SourceInventory {
                identity: identity(),
                tables: vec![table()],
                objects: vec![SourceObject {
                    schema: "public".to_owned(),
                    name: "place_view".to_owned(),
                    kind: SourceObjectKind::View,
                }],
                roles: vec![SourceRole {
                    name: "source_reader".to_owned(),
                    login: true,
                }],
                grants: vec![],
            },
        );
        assert_eq!(report.status, PreflightStatus::Ready);
        assert_eq!(report.tables[0].columns.len(), 4);
        assert_eq!(
            report.tables[0].columns[1].target_type.as_deref(),
            Some("VARCHAR")
        );
        assert_eq!(
            report.tables[0].columns[2].target_type.as_deref(),
            Some("DECIMAL(10,2)")
        );
        assert_eq!(
            report.tables[0].columns[3].target_name.as_deref(),
            Some("geom_wkb")
        );
        assert_eq!(
            report.tables[0].columns[3].target_type.as_deref(),
            Some("BLOB")
        );
        assert_eq!(report.objects[0].disposition.action, Action::Reject);
        assert_eq!(report.roles[0].disposition.action, Action::Reject);
    }

    #[test]
    fn rejects_keys_rls_generated_columns_and_nonzero_srid() {
        let mut source = table();
        source.row_security = true;
        source.constraints.push(SourceConstraint {
            name: "places_pkey".to_owned(),
            kind: ConstraintKind::PrimaryKey,
            definition: "PRIMARY KEY (id)".to_owned(),
        });
        source.columns[1].generated = true;
        source.columns[3].geometry_srid = Some(4326);
        let report = build_preflight(
            &config(),
            SourceInventory {
                identity: identity(),
                tables: vec![source],
                objects: vec![],
                roles: vec![],
                grants: vec![],
            },
        );
        assert_eq!(report.status, PreflightStatus::Rejected);
        assert!(
            report.tables[0]
                .blockers
                .iter()
                .any(|reason| reason.contains("row-level"))
        );
        assert!(
            report.tables[0]
                .blockers
                .iter()
                .any(|reason| reason.contains("PrimaryKey"))
        );
        assert!(
            report.tables[0]
                .blockers
                .iter()
                .any(|reason| reason.contains("generated"))
        );
        assert!(
            report.tables[0]
                .blockers
                .iter()
                .any(|reason| reason.contains("SRID-0"))
        );
    }

    #[test]
    fn accepts_only_literal_defaults() {
        for value in [
            "7",
            "-12.34",
            "'name'::text",
            "DATE '2026-07-18'",
            "NULL::integer",
        ] {
            assert!(normalize_literal_default(value).is_some(), "{value}");
        }
        for value in ["nextval('items_id_seq')", "now()", "current_user", "1 + 2"] {
            assert!(normalize_literal_default(value).is_none(), "{value}");
        }
    }

    #[test]
    fn preserves_postgresql_timestamp_precision_typmods() {
        assert_eq!(timestamp_target_type(-1), "TIMESTAMP");
        assert_eq!(timestamp_target_type(0), "TIMESTAMP(0)");
        assert_eq!(timestamp_target_type(6), "TIMESTAMP(6)");
    }

    #[test]
    fn missing_and_unselected_tables_are_explicit() {
        let mut config = config();
        config.tables.push(TableMapping {
            source_schema: "public".to_owned(),
            source_table: "missing".to_owned(),
            target_schema: "main".to_owned(),
            target_table: "missing".to_owned(),
            column_mappings: BTreeMap::new(),
        });
        let mut unselected = table();
        unselected.name = "other".to_owned();
        let report = build_preflight(
            &config,
            SourceInventory {
                identity: identity(),
                tables: vec![table(), unselected],
                objects: vec![],
                roles: vec![],
                grants: vec![],
            },
        );
        assert_eq!(report.status, PreflightStatus::Rejected);
        assert_eq!(report.tables.len(), 3);
        assert!(report.errors[0].contains("missing"));
        assert!(
            report
                .tables
                .iter()
                .any(|table| { table.source_table == "other" && table.target_table.is_none() })
        );
    }
}
