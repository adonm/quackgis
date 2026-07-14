// SPDX-License-Identifier: Apache-2.0
//! Engine-neutral structural authorization for pgwire statements.

use std::collections::HashSet;
use std::ops::ControlFlow;
use std::sync::atomic::{AtomicU64, Ordering};

use sqlparser::ast::{
    Delete, Expr, FromTable, ObjectName, ObjectNamePart, Query, SetExpr, Statement, TableFactor,
    TableObject, TableWithJoins, visit_expressions,
};

use crate::auth::{AccessRole, AuthConfig};
use crate::engine_api::{EngineError, EngineErrorKind, EngineResult};
use crate::role::TablePrivilege;

static WRITE_DENIED_COUNTER: AtomicU64 = AtomicU64::new(0);
static READ_DENIED_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct TableKey {
    schema: String,
    table: String,
}

pub fn writes_denied_total() -> u64 {
    WRITE_DENIED_COUNTER.load(Ordering::Relaxed)
}

pub fn reads_denied_total() -> u64 {
    READ_DENIED_COUNTER.load(Ordering::Relaxed)
}

pub fn authorize_statement(
    auth: &AuthConfig,
    session_user: Option<&str>,
    effective_role: Option<&str>,
    statement: &Statement,
) -> EngineResult<()> {
    let effective_role = effective_role.or(session_user);
    if !statement_allowed_for_readonly(statement) {
        let target = write_target(statement);
        let target_ref = target
            .as_ref()
            .map(|target| (target.schema.as_str(), target.table.as_str()));
        if !auth.allows_write(session_user, target_ref) {
            let target_label = target
                .map(|target| format!("{}.{}", target.schema, target.table))
                .unwrap_or_else(|| "<indeterminate>".to_string());
            record_denial(session_user, statement_kind(statement), &target_label, true);
            let message = match auth.role_for_user(session_user) {
                AccessRole::ReadOnly => {
                    "read-only QuackGIS role cannot execute write statements".to_string()
                }
                AccessRole::ReadWrite => {
                    format!("QuackGIS write allowlist does not permit writes to {target_label}")
                }
            };
            return Err(EngineError::new(EngineErrorKind::Unauthorized, message));
        }
        if let Some(catalog) = auth.role_catalog() {
            let allowed =
                effective_role
                    .zip(target.as_ref())
                    .is_some_and(|(role, target)| match required_write_privilege(statement) {
                        Some(TableWritePrivilege::Create) => {
                            catalog.can_create_configured_table(role, &target.schema, &target.table)
                        }
                        Some(TableWritePrivilege::Table(privilege)) => catalog
                            .allows_table_operation(role, &target.schema, &target.table, privilege),
                        None => false,
                    });
            if !allowed {
                let target_label = target
                    .map(|target| format!("{}.{}", target.schema, target.table))
                    .unwrap_or_else(|| "<indeterminate>".to_owned());
                record_denial(
                    effective_role,
                    statement_kind(statement),
                    &target_label,
                    true,
                );
                return Err(EngineError::new(
                    EngineErrorKind::Unauthorized,
                    format!(
                        "PostgreSQL role lacks {} privilege on {target_label}",
                        required_write_privilege(statement)
                            .map(TableWritePrivilege::label)
                            .unwrap_or("required")
                    ),
                ));
            }
        }
    }

    if auth.read_policy_restricted() || auth.role_catalog().is_some() {
        let targets = read_targets(statement);
        if targets.sensitive_metadata
            || (targets.maintained_information_schema && auth.role_catalog().is_none())
        {
            record_denial(
                effective_role,
                statement_kind(statement),
                "<metadata>",
                false,
            );
            return Err(EngineError::new(
                EngineErrorKind::Unauthorized,
                "restricted read allowlist denies unfiltered catalog metadata surfaces",
            ));
        }
        for target in targets.tables {
            if auth.read_policy_restricted()
                && !auth.allows_read(session_user, (&target.schema, &target.table))
            {
                let label = format!("{}.{}", target.schema, target.table);
                record_denial(effective_role, statement_kind(statement), &label, false);
                return Err(EngineError::new(
                    EngineErrorKind::Unauthorized,
                    format!("QuackGIS read allowlist does not permit reads from {label}"),
                ));
            }
            if let Some(catalog) = auth.role_catalog()
                && !effective_role.is_some_and(|role| {
                    catalog.allows_table_operation(
                        role,
                        &target.schema,
                        &target.table,
                        TablePrivilege::Select,
                    )
                })
            {
                let label = format!("{}.{}", target.schema, target.table);
                record_denial(effective_role, statement_kind(statement), &label, false);
                return Err(EngineError::new(
                    EngineErrorKind::Unauthorized,
                    format!("PostgreSQL role lacks SELECT privilege on {label}"),
                ));
            }
        }
    }
    Ok(())
}

pub fn authorize_copy_target(
    auth: &AuthConfig,
    session_user: Option<&str>,
    effective_role: Option<&str>,
    schema: &str,
    table: &str,
) -> EngineResult<()> {
    let outer_allowed = auth.allows_write(session_user, Some((schema, table)));
    let role_allowed = auth.role_catalog().is_none_or(|catalog| {
        effective_role.or(session_user).is_some_and(|role| {
            catalog.allows_table_operation(role, schema, table, TablePrivilege::Insert)
        })
    });
    if outer_allowed && role_allowed {
        return Ok(());
    }
    let target = format!("{schema}.{table}");
    record_denial(effective_role.or(session_user), "copy", &target, true);
    let message = if outer_allowed {
        format!("PostgreSQL role lacks INSERT privilege on {target}")
    } else {
        match auth.role_for_user(session_user) {
            AccessRole::ReadOnly => "read-only QuackGIS role cannot execute COPY FROM".to_string(),
            AccessRole::ReadWrite => {
                format!("QuackGIS write allowlist does not permit COPY to {target}")
            }
        }
    };
    Err(EngineError::new(EngineErrorKind::Unauthorized, message))
}

fn record_denial(user: Option<&str>, kind: &str, target: &str, write: bool) {
    let counter = if write {
        &WRITE_DENIED_COUNTER
    } else {
        &READ_DENIED_COUNTER
    };
    let total = counter.fetch_add(1, Ordering::Relaxed) + 1;
    let user = user.unwrap_or("unknown");
    log::warn!(
        "quackgis_authorization_denied user={user} statement_kind={kind} target={target} denied_total={total}"
    );
    crate::audit::log_authorization_denied(user, kind, target, "table_policy");
}

fn statement_kind(statement: &Statement) -> &'static str {
    match statement {
        Statement::CreateTable(_) => "create_table",
        Statement::Delete(_) => "delete",
        Statement::Insert(_) => "insert",
        Statement::Query(_) => "query",
        Statement::Update { .. } => "update",
        Statement::Copy { .. } => "copy",
        _ => "other",
    }
}

fn statement_allowed_for_readonly(statement: &Statement) -> bool {
    match statement {
        Statement::Query(_)
        | Statement::Set(_)
        | Statement::ShowVariable { .. }
        | Statement::ShowStatus { .. }
        | Statement::Deallocate { .. }
        | Statement::Declare { .. }
        | Statement::Close { .. }
        | Statement::Discard { .. }
        | Statement::Reset(_)
        | Statement::ExplainTable { .. }
        | Statement::Commit { .. }
        | Statement::Rollback { .. } => true,
        Statement::StartTransaction { statements, .. } => {
            statements.iter().all(statement_allowed_for_readonly)
        }
        Statement::Explain { statement, .. } | Statement::Prepare { statement, .. } => {
            statement_allowed_for_readonly(statement)
        }
        Statement::Fetch { into, .. } => into.is_none(),
        Statement::Copy { to, .. } => *to,
        _ => false,
    }
}

fn write_target(statement: &Statement) -> Option<TableKey> {
    let parts = match statement {
        Statement::CreateTable(create) => table_name_parts(&create.name),
        Statement::Insert(insert) => insert_target_parts(&insert.table),
        Statement::Delete(delete) => delete_target_parts(delete),
        Statement::Update(update) => update_target_parts(&update.table),
        _ => None,
    }?;
    Some(TableKey {
        schema: parts.0,
        table: parts.1,
    })
}

#[derive(Clone, Copy)]
enum TableWritePrivilege {
    Create,
    Table(TablePrivilege),
}

impl TableWritePrivilege {
    const fn label(self) -> &'static str {
        match self {
            Self::Create => "ownership",
            Self::Table(TablePrivilege::Insert) => "INSERT",
            Self::Table(TablePrivilege::Update) => "UPDATE",
            Self::Table(TablePrivilege::Delete) => "DELETE",
            Self::Table(TablePrivilege::Select) => "SELECT",
            Self::Table(TablePrivilege::Maintain) => "MAINTAIN",
        }
    }
}

fn required_write_privilege(statement: &Statement) -> Option<TableWritePrivilege> {
    match statement {
        Statement::CreateTable(_) => Some(TableWritePrivilege::Create),
        Statement::Insert(_) => Some(TableWritePrivilege::Table(TablePrivilege::Insert)),
        Statement::Update { .. } => Some(TableWritePrivilege::Table(TablePrivilege::Update)),
        Statement::Delete(_) => Some(TableWritePrivilege::Table(TablePrivilege::Delete)),
        _ => None,
    }
}

#[derive(Default)]
struct ReadTargets {
    tables: Vec<TableKey>,
    sensitive_metadata: bool,
    maintained_information_schema: bool,
}

fn read_targets(statement: &Statement) -> ReadTargets {
    let mut targets = ReadTargets::default();
    if let Statement::Query(query) = statement {
        collect_query_targets(query, &mut targets);
    }
    targets.tables.sort();
    targets.tables.dedup();
    targets
}

fn collect_query_targets(query: &Query, targets: &mut ReadTargets) {
    let mut ctes = HashSet::new();
    if let Some(with) = &query.with {
        for cte in &with.cte_tables {
            ctes.insert(cte.alias.name.value.to_ascii_lowercase());
            collect_query_targets(&cte.query, targets);
        }
    }
    let _: ControlFlow<()> = visit_expressions(query, |expression| {
        let subquery = match expression {
            Expr::InSubquery { subquery, .. }
            | Expr::Exists { subquery, .. }
            | Expr::Subquery(subquery) => Some(subquery),
            _ => None,
        };
        if let Some(subquery) = subquery {
            collect_query_targets(subquery, targets);
        }
        ControlFlow::Continue(())
    });
    collect_set_targets(query.body.as_ref(), &ctes, targets);
}

fn collect_set_targets(expr: &SetExpr, ctes: &HashSet<String>, targets: &mut ReadTargets) {
    match expr {
        SetExpr::Select(select) => {
            for table in &select.from {
                collect_table_targets(&table.relation, ctes, targets);
                for join in &table.joins {
                    collect_table_targets(&join.relation, ctes, targets);
                }
            }
        }
        SetExpr::Query(query) => collect_query_targets(query, targets),
        SetExpr::SetOperation { left, right, .. } => {
            collect_set_targets(left, ctes, targets);
            collect_set_targets(right, ctes, targets);
        }
        _ => {}
    }
}

fn collect_table_targets(factor: &TableFactor, ctes: &HashSet<String>, targets: &mut ReadTargets) {
    match factor {
        TableFactor::Table { name, .. } => collect_name_target(name, ctes, targets),
        TableFactor::Function { name, .. } if sensitive_metadata_name(name) => {
            targets.sensitive_metadata = true;
        }
        TableFactor::Derived { subquery, .. } => collect_query_targets(subquery, targets),
        TableFactor::NestedJoin {
            table_with_joins, ..
        } => {
            collect_table_targets(&table_with_joins.relation, ctes, targets);
            for join in &table_with_joins.joins {
                collect_table_targets(&join.relation, ctes, targets);
            }
        }
        TableFactor::Pivot { table, .. }
        | TableFactor::Unpivot { table, .. }
        | TableFactor::MatchRecognize { table, .. } => {
            collect_table_targets(table, ctes, targets);
        }
        _ => {}
    }
}

fn collect_name_target(name: &ObjectName, ctes: &HashSet<String>, targets: &mut ReadTargets) {
    if matches!(name.0.as_slice(), [ObjectNamePart::Identifier(ident)] if ctes.contains(&ident.value.to_ascii_lowercase()))
    {
        return;
    }
    let schema = name
        .0
        .len()
        .checked_sub(2)
        .and_then(|index| name.0.get(index))
        .map(ToString::to_string)
        .unwrap_or_default()
        .trim_matches('"')
        .to_ascii_lowercase();
    if schema == "information_schema" && maintained_information_schema_name(name) {
        targets.maintained_information_schema = true;
    } else if schema == "information_schema" || sensitive_metadata_name(name) {
        targets.sensitive_metadata = true;
    } else if !matches!(schema.as_str(), "pg_catalog" | "quackgis_pg_catalog")
        && !unqualified_pg_catalog_name(name)
        && let Some((schema, table)) = table_name_parts(name)
    {
        targets.tables.push(TableKey { schema, table });
    }
}

fn maintained_information_schema_name(name: &ObjectName) -> bool {
    name.0.len() == 2
        && object_name_last(name).is_some_and(|name| {
            matches!(
                name.to_ascii_lowercase().as_str(),
                "schemata"
                    | "tables"
                    | "columns"
                    | "table_privileges"
                    | "role_table_grants"
                    | "column_privileges"
                    | "role_column_grants"
            )
        })
}

fn unqualified_pg_catalog_name(name: &ObjectName) -> bool {
    if name.0.len() != 1 {
        return false;
    }
    object_name_last(name).is_some_and(|name| {
        matches!(
            name.to_ascii_lowercase().as_str(),
            "pg_namespace"
                | "pg_database"
                | "pg_type"
                | "pg_class"
                | "pg_attribute"
                | "pg_attrdef"
                | "pg_description"
                | "pg_constraint"
                | "pg_index"
                | "pg_range"
                | "pg_collation"
                | "pg_roles"
                | "pg_auth_members"
        )
    })
}

fn sensitive_metadata_name(name: &ObjectName) -> bool {
    object_name_last(name).is_some_and(|name| {
        matches!(
            name.to_ascii_lowercase().as_str(),
            "ducklake_snapshots"
                | "ducklake_table_info"
                | "ducklake_list_files"
                | "ducklake_table_changes"
                | "ducklake_table_deletions"
        )
    })
}

fn table_name_parts(name: &ObjectName) -> Option<(String, String)> {
    let parts = name
        .0
        .iter()
        .map(|part| part.to_string().trim_matches('"').to_string())
        .collect::<Vec<_>>();
    match parts.as_slice() {
        [catalog, schema, table]
            if catalog.eq_ignore_ascii_case("quackgis") && is_public_schema(schema) =>
        {
            Some(("main".to_string(), table.clone()))
        }
        [schema, table] if is_public_schema(schema) => Some(("main".to_string(), table.clone())),
        [table] => Some(("main".to_string(), table.clone())),
        _ => None,
    }
}

fn is_public_schema(schema: &str) -> bool {
    schema.eq_ignore_ascii_case("main") || schema.eq_ignore_ascii_case("public")
}

fn insert_target_parts(table: &TableObject) -> Option<(String, String)> {
    match table {
        TableObject::TableName(name) => table_name_parts(name),
        _ => None,
    }
}

fn delete_target_parts(delete: &Delete) -> Option<(String, String)> {
    let from = match &delete.from {
        FromTable::WithFromKeyword(tables) | FromTable::WithoutKeyword(tables) => tables,
    };
    if from.len() != 1 || delete.using.is_some() || !delete.tables.is_empty() {
        return None;
    }
    table_factor_parts(&from[0].relation)
}

fn update_target_parts(table: &TableWithJoins) -> Option<(String, String)> {
    table
        .joins
        .is_empty()
        .then(|| table_factor_parts(&table.relation))?
}

fn table_factor_parts(factor: &TableFactor) -> Option<(String, String)> {
    match factor {
        TableFactor::Table { name, .. } => table_name_parts(name),
        _ => None,
    }
}

fn object_name_last(name: &ObjectName) -> Option<String> {
    name.0
        .last()
        .map(|part| part.to_string().trim_matches('"').to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::parse_read_allowlist;
    use sqlparser::dialect::PostgreSqlDialect;
    use sqlparser::parser::Parser;

    #[test]
    fn restricted_policy_allows_maintained_catalogs_but_not_user_or_raw_metadata() {
        let auth =
            AuthConfig::password("writer", "writer-secret", Some(("reader", "reader-secret")))
                .unwrap()
                .with_read_allowlist(parse_read_allowlist("allowed").unwrap());

        for sql in [
            "SELECT typname FROM pg_catalog.pg_type",
            "SELECT typname FROM pg_type",
            "SELECT EXISTS (SELECT 1 FROM pg_type)",
        ] {
            let statement = Parser::parse_sql(&PostgreSqlDialect {}, sql)
                .unwrap()
                .remove(0);
            authorize_statement(&auth, Some("reader"), Some("reader"), &statement)
                .unwrap_or_else(|error| panic!("maintained catalog denied for {sql}: {error}"));
        }

        for sql in [
            "SELECT table_name FROM memory.information_schema.tables",
            "SELECT (SELECT name FROM denied LIMIT 1)",
            "SELECT 1 LIMIT (SELECT count(*) FROM denied)",
            "SELECT * FROM denied PIVOT (count(*) FOR id IN (1))",
        ] {
            let statement = Parser::parse_sql(&PostgreSqlDialect {}, sql)
                .unwrap()
                .remove(0);
            assert!(
                authorize_statement(&auth, Some("reader"), Some("reader"), &statement).is_err()
            );
        }
    }
}
