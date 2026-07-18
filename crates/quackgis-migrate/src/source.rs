// SPDX-License-Identifier: Apache-2.0

use anyhow::{Context, Result, bail};
use tokio_postgres::{Client, IsolationLevel, Row, Transaction};

use crate::config::MigrationConfig;
use crate::inventory::{
    ConstraintKind, SourceColumn, SourceConstraint, SourceGrant, SourceIdentity, SourceInventory,
    SourceObject, SourceObjectKind, SourceRole, SourceTable,
};

const MAX_SOURCE_TABLES: usize = 1024;
const MAX_COLUMNS_PER_TABLE: usize = 1024;
const MAX_CONSTRAINTS_PER_TABLE: usize = 1024;
const MAX_SOURCE_OBJECTS: usize = 16_384;
const MAX_SOURCE_ROLES: usize = 4096;
const MAX_SOURCE_GRANTS: usize = 65_536;

pub async fn begin_source_snapshot<'a>(
    client: &'a mut Client,
    config: &MigrationConfig,
) -> Result<(Transaction<'a>, SourceInventory)> {
    let transaction = client
        .build_transaction()
        .isolation_level(IsolationLevel::RepeatableRead)
        .read_only(true)
        .start()
        .await
        .context("begin read-only repeatable-read source snapshot")?;
    let identity = inspect_identity(&transaction, config).await?;
    let tables = inspect_tables(&transaction, config).await?;
    let objects = inspect_objects(&transaction, config).await?;
    let roles = inspect_roles(&transaction, config).await?;
    let grants = inspect_grants(&transaction, config).await?;
    Ok((
        transaction,
        SourceInventory {
            identity,
            tables,
            objects,
            roles,
            grants,
        },
    ))
}

async fn inspect_identity(
    transaction: &Transaction<'_>,
    config: &MigrationConfig,
) -> Result<SourceIdentity> {
    let row = transaction
        .query_one(
            "SELECT current_setting('server_version_num')::INTEGER, \
                    current_setting('server_version'), current_database(), \
                    (SELECT oid::BIGINT FROM pg_catalog.pg_database \
                     WHERE datname = current_database()), \
                    public.postgis_lib_version(), \
                    transaction_timestamp()::TEXT, pg_current_snapshot()::TEXT",
            &[],
        )
        .await
        .context("read PostgreSQL/PostGIS source identity")?;
    let server_version_num = u32::try_from(row.get::<_, i32>(0))?;
    let postgis_version = row.get::<_, String>(4);
    if server_version_num != config.source.postgres_version_num {
        bail!(
            "source PostgreSQL version {server_version_num} does not match pinned {}",
            config.source.postgres_version_num
        );
    }
    if postgis_version != config.source.postgis_version {
        bail!(
            "source PostGIS version {postgis_version:?} does not match pinned {:?}",
            config.source.postgis_version
        );
    }
    Ok(SourceIdentity {
        server_version_num,
        server_version: row.get(1),
        database_name: row.get(2),
        database_oid: u32::try_from(row.get::<_, i64>(3))?,
        postgis_version,
        snapshot_started_at: row.get(5),
        snapshot_id: row.get(6),
    })
}

async fn inspect_tables(
    transaction: &Transaction<'_>,
    config: &MigrationConfig,
) -> Result<Vec<SourceTable>> {
    let rows = transaction
        .query(
            "SELECT c.oid::BIGINT, n.nspname, c.relname, c.relkind::TEXT, \
                    COALESCE(pg_catalog.pg_total_relation_size(c.oid), 0)::BIGINT, \
                    pg_catalog.obj_description(c.oid, 'pg_class'), c.relrowsecurity, \
                    c.relforcerowsecurity, \
                    CASE c.relreplident WHEN 'd' THEN 'default' WHEN 'n' THEN 'nothing' \
                         WHEN 'f' THEN 'full' WHEN 'i' THEN 'index' ELSE 'unknown' END, \
                    (SELECT count(*)::INTEGER FROM pg_catalog.pg_trigger t \
                     WHERE t.tgrelid = c.oid AND NOT t.tgisinternal) \
             FROM pg_catalog.pg_class c \
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
             WHERE n.nspname = ANY($1::TEXT[]) AND c.relkind IN ('r', 'p') \
             ORDER BY n.nspname, c.relname LIMIT 1025",
            &[&config.source_schemas],
        )
        .await
        .context("inventory source tables")?;
    enforce_limit("source tables", rows.len(), MAX_SOURCE_TABLES)?;
    let mut tables = Vec::with_capacity(rows.len());
    for row in rows {
        let oid = u32::try_from(row.get::<_, i64>(0))?;
        let schema = row.get::<_, String>(1);
        let name = row.get::<_, String>(2);
        let count_sql = format!(
            "SELECT count(*)::BIGINT FROM {}.{}",
            quote_identifier(&schema),
            quote_identifier(&name)
        );
        let row_count = u64::try_from(
            transaction
                .query_one(&count_sql, &[])
                .await
                .with_context(|| format!("count source table {schema}.{name}"))?
                .get::<_, i64>(0),
        )?;
        tables.push(SourceTable {
            schema,
            name,
            row_count,
            estimated_bytes: u64::try_from(row.get::<_, i64>(4))?,
            comment: row.get(5),
            row_security: row.get(6),
            force_row_security: row.get(7),
            replica_identity: row.get(8),
            partitioned: row.get::<_, String>(3) == "p",
            trigger_count: u32::try_from(row.get::<_, i32>(9))?,
            columns: inspect_columns(transaction, oid).await?,
            constraints: inspect_constraints(transaction, oid).await?,
        });
    }
    Ok(tables)
}

async fn inspect_columns(
    transaction: &Transaction<'_>,
    relation_oid: u32,
) -> Result<Vec<SourceColumn>> {
    let rows = transaction
        .query(
            "SELECT a.attnum::SMALLINT, a.attname, t.typname, \
                    pg_catalog.format_type(a.atttypid, a.atttypmod), a.atttypmod, \
                    NOT a.attnotnull, pg_catalog.pg_get_expr(d.adbin, d.adrelid), \
                    pg_catalog.col_description(a.attrelid, a.attnum), \
                    a.attidentity <> '', a.attgenerated <> '', \
                    CASE WHEN t.typname IN ('geometry', 'geography') \
                         THEN public.postgis_typmod_type(a.atttypmod) END, \
                    CASE WHEN t.typname IN ('geometry', 'geography') \
                         THEN public.postgis_typmod_srid(a.atttypmod) END, \
                    CASE WHEN t.typname IN ('geometry', 'geography') \
                         THEN public.postgis_typmod_dims(a.atttypmod) END \
             FROM pg_catalog.pg_attribute a \
             JOIN pg_catalog.pg_type t ON t.oid = a.atttypid \
             LEFT JOIN pg_catalog.pg_attrdef d \
                    ON d.adrelid = a.attrelid AND d.adnum = a.attnum \
             WHERE a.attrelid = $1::OID AND a.attnum > 0 AND NOT a.attisdropped \
             ORDER BY a.attnum LIMIT 1025",
            &[&relation_oid],
        )
        .await
        .context("inventory source columns")?;
    enforce_limit(
        "columns on one source table",
        rows.len(),
        MAX_COLUMNS_PER_TABLE,
    )?;
    rows.into_iter().map(source_column).collect()
}

fn source_column(row: Row) -> Result<SourceColumn> {
    Ok(SourceColumn {
        position: row.get(0),
        name: row.get(1),
        type_name: row.get(2),
        formatted_type: row.get(3),
        type_modifier: row.get(4),
        nullable: row.get(5),
        default_expression: row.get(6),
        comment: row.get(7),
        identity: row.get(8),
        generated: row.get(9),
        geometry_type: row.get(10),
        geometry_srid: row.get(11),
        geometry_dimensions: row.get(12),
    })
}

async fn inspect_constraints(
    transaction: &Transaction<'_>,
    relation_oid: u32,
) -> Result<Vec<SourceConstraint>> {
    let rows = transaction
        .query(
            "SELECT conname, contype::TEXT, pg_catalog.pg_get_constraintdef(oid, true) \
             FROM pg_catalog.pg_constraint WHERE conrelid = $1::OID \
             ORDER BY conname LIMIT 1025",
            &[&relation_oid],
        )
        .await
        .context("inventory source constraints")?;
    enforce_limit(
        "constraints on one source table",
        rows.len(),
        MAX_CONSTRAINTS_PER_TABLE,
    )?;
    rows.into_iter()
        .map(|row| {
            let kind = match row.get::<_, String>(1).as_str() {
                "c" => ConstraintKind::Check,
                "p" => ConstraintKind::PrimaryKey,
                "u" => ConstraintKind::Unique,
                "f" => ConstraintKind::ForeignKey,
                "x" => ConstraintKind::Exclusion,
                "n" => ConstraintKind::NotNull,
                _ => ConstraintKind::Other,
            };
            Ok(SourceConstraint {
                name: row.get(0),
                kind,
                definition: row.get(2),
            })
        })
        .collect()
}

async fn inspect_objects(
    transaction: &Transaction<'_>,
    config: &MigrationConfig,
) -> Result<Vec<SourceObject>> {
    let mut objects = transaction
        .query(
            "SELECT n.nspname, c.relname, c.relkind::TEXT \
             FROM pg_catalog.pg_class c \
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
             WHERE n.nspname = ANY($1::TEXT[]) AND c.relkind IN ('v','m','S','f','i','I') \
             ORDER BY n.nspname, c.relname LIMIT 16385",
            &[&config.source_schemas],
        )
        .await
        .context("inventory unsupported source relations")?
        .into_iter()
        .map(|row| SourceObject {
            schema: row.get(0),
            name: row.get(1),
            kind: match row.get::<_, String>(2).as_str() {
                "v" => SourceObjectKind::View,
                "m" => SourceObjectKind::MaterializedView,
                "S" => SourceObjectKind::Sequence,
                "f" => SourceObjectKind::ForeignTable,
                "i" | "I" => SourceObjectKind::Index,
                _ => SourceObjectKind::Other,
            },
        })
        .collect::<Vec<_>>();
    enforce_limit("source objects", objects.len(), MAX_SOURCE_OBJECTS)?;
    for row in transaction
        .query(
            "SELECT n.nspname, e.extname FROM pg_catalog.pg_extension e \
             JOIN pg_catalog.pg_namespace n ON n.oid = e.extnamespace \
             WHERE n.nspname = ANY($1::TEXT[]) \
             ORDER BY n.nspname, e.extname LIMIT 16385",
            &[&config.source_schemas],
        )
        .await
        .context("inventory source extensions")?
    {
        objects.push(SourceObject {
            schema: row.get(0),
            name: row.get(1),
            kind: SourceObjectKind::Extension,
        });
        enforce_limit("source objects", objects.len(), MAX_SOURCE_OBJECTS)?;
    }
    for row in transaction
        .query(
            "SELECT n.nspname, p.proname || '(' || pg_catalog.pg_get_function_identity_arguments(p.oid) || ')' \
             FROM pg_catalog.pg_proc p \
             JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace \
             WHERE n.nspname = ANY($1::TEXT[]) \
               AND NOT EXISTS (SELECT 1 FROM pg_catalog.pg_depend d \
                               WHERE d.classid = 'pg_proc'::regclass \
                                 AND d.objid = p.oid AND d.deptype = 'e') \
             ORDER BY n.nspname, p.proname, p.oid LIMIT 16385",
            &[&config.source_schemas],
        )
        .await
        .context("inventory source functions")?
    {
        objects.push(SourceObject {
            schema: row.get(0),
            name: row.get(1),
            kind: SourceObjectKind::Function,
        });
        enforce_limit("source objects", objects.len(), MAX_SOURCE_OBJECTS)?;
    }
    for row in transaction
        .query(
            "SELECT n.nspname, c.relname || '.' || t.tgname \
             FROM pg_catalog.pg_trigger t \
             JOIN pg_catalog.pg_class c ON c.oid = t.tgrelid \
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
             WHERE n.nspname = ANY($1::TEXT[]) AND NOT t.tgisinternal \
             ORDER BY n.nspname, c.relname, t.tgname LIMIT 16385",
            &[&config.source_schemas],
        )
        .await
        .context("inventory source triggers")?
    {
        objects.push(SourceObject {
            schema: row.get(0),
            name: row.get(1),
            kind: SourceObjectKind::Trigger,
        });
        enforce_limit("source objects", objects.len(), MAX_SOURCE_OBJECTS)?;
    }
    objects.sort_by(|left, right| {
        (&left.schema, &left.name, format!("{:?}", left.kind)).cmp(&(
            &right.schema,
            &right.name,
            format!("{:?}", right.kind),
        ))
    });
    Ok(objects)
}

async fn inspect_roles(
    transaction: &Transaction<'_>,
    config: &MigrationConfig,
) -> Result<Vec<SourceRole>> {
    let rows = transaction
        .query(
            "SELECT DISTINCT r.rolname, r.rolcanlogin \
             FROM pg_catalog.pg_roles r \
             WHERE r.oid IN ( \
                 SELECT c.relowner FROM pg_catalog.pg_class c \
                 JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
                 WHERE n.nspname = ANY($1::TEXT[]) \
             ) OR r.rolname IN ( \
                 SELECT grantee FROM information_schema.role_table_grants \
                 WHERE table_schema = ANY($1::TEXT[]) \
                 UNION \
                 SELECT grantee FROM information_schema.role_column_grants \
                 WHERE table_schema = ANY($1::TEXT[]) \
             ) ORDER BY r.rolname LIMIT 4097",
            &[&config.source_schemas],
        )
        .await
        .context("inventory source owner roles")?;
    enforce_limit("source owner roles", rows.len(), MAX_SOURCE_ROLES)?;
    Ok(rows
        .into_iter()
        .map(|row| SourceRole {
            name: row.get(0),
            login: row.get(1),
        })
        .collect())
}

async fn inspect_grants(
    transaction: &Transaction<'_>,
    config: &MigrationConfig,
) -> Result<Vec<SourceGrant>> {
    let mut grants = transaction
        .query(
            "SELECT grantee, table_schema, table_name, privilege_type \
             FROM information_schema.role_table_grants \
             WHERE table_schema = ANY($1::TEXT[]) \
             ORDER BY table_schema, table_name, grantee, privilege_type LIMIT 65537",
            &[&config.source_schemas],
        )
        .await
        .context("inventory source table grants")?
        .into_iter()
        .map(|row| SourceGrant {
            grantee: row.get(0),
            schema: row.get(1),
            table: row.get(2),
            column: None,
            privilege: row.get(3),
        })
        .collect::<Vec<_>>();
    enforce_limit("source grants", grants.len(), MAX_SOURCE_GRANTS)?;
    for row in transaction
        .query(
            "SELECT grantee, table_schema, table_name, column_name, privilege_type \
             FROM information_schema.role_column_grants \
             WHERE table_schema = ANY($1::TEXT[]) \
             ORDER BY table_schema, table_name, column_name, grantee, privilege_type \
             LIMIT 65537",
            &[&config.source_schemas],
        )
        .await
        .context("inventory source column grants")?
    {
        grants.push(SourceGrant {
            grantee: row.get(0),
            schema: row.get(1),
            table: row.get(2),
            column: Some(row.get(3)),
            privilege: row.get(4),
        });
        enforce_limit("source grants", grants.len(), MAX_SOURCE_GRANTS)?;
    }
    Ok(grants)
}

pub fn quote_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn enforce_limit(label: &str, actual: usize, maximum: usize) -> Result<()> {
    if actual > maximum {
        bail!("{label} exceeds the {maximum}-item migration inventory limit");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_identifiers_without_treating_them_as_sql() {
        assert_eq!(quote_identifier("normal"), "\"normal\"");
        assert_eq!(
            quote_identifier("odd\"; DROP TABLE x; --"),
            "\"odd\"\"; DROP TABLE x; --\""
        );
    }
}
