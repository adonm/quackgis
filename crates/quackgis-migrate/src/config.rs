// SPDX-License-Identifier: Apache-2.0

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

const MAX_CONFIG_BYTES: u64 = 1024 * 1024;
const MAX_SCHEMAS: usize = 64;
const MAX_TABLES: usize = 1024;
const MAX_ROLE_MAPPINGS: usize = 4096;
const MAX_GRANT_MAPPINGS: usize = 65_536;
const MAX_IDENTIFIER_BYTES: usize = 63;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MigrationConfig {
    pub format_version: u32,
    pub source: SourceRequirements,
    pub source_schemas: Vec<String>,
    pub tables: Vec<TableMapping>,
    #[serde(default)]
    pub role_mappings: BTreeMap<String, String>,
    #[serde(default)]
    pub grant_mappings: Vec<GrantMapping>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SourceRequirements {
    pub postgres_version_num: u32,
    pub postgis_version: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TableMapping {
    pub source_schema: String,
    pub source_table: String,
    pub target_schema: String,
    pub target_table: String,
    #[serde(default)]
    pub column_mappings: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GrantMapping {
    pub source_role: String,
    pub source_schema: String,
    pub source_table: String,
    #[serde(default)]
    pub source_column: Option<String>,
    pub privilege: String,
    pub target_role: String,
}

impl MigrationConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let metadata = std::fs::symlink_metadata(path)
            .with_context(|| format!("cannot inspect migration config {}", path.display()))?;
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || metadata.len() > MAX_CONFIG_BYTES
        {
            bail!(
                "migration config must be a non-symlink regular file no larger than {MAX_CONFIG_BYTES} bytes"
            );
        }
        let raw = std::fs::read(path)
            .with_context(|| format!("cannot read migration config {}", path.display()))?;
        let config: Self = serde_json::from_slice(&raw)
            .with_context(|| format!("invalid migration config JSON in {}", path.display()))?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        if self.format_version != 1 {
            bail!("unsupported migration config format_version");
        }
        if self.source.postgres_version_num < 100_000 {
            bail!("source postgres_version_num must be an exact six-digit server version number");
        }
        if self.source.postgis_version.is_empty()
            || self.source.postgis_version.len() > 64
            || !self
                .source
                .postgis_version
                .bytes()
                .all(|byte| byte.is_ascii_digit() || byte == b'.')
        {
            bail!("source postgis_version must be an exact dotted numeric version");
        }
        if self.source_schemas.is_empty() || self.source_schemas.len() > MAX_SCHEMAS {
            bail!("source_schemas must contain between 1 and {MAX_SCHEMAS} entries");
        }
        if self.tables.is_empty() || self.tables.len() > MAX_TABLES {
            bail!("tables must contain between 1 and {MAX_TABLES} entries");
        }
        if self.role_mappings.len() > MAX_ROLE_MAPPINGS {
            bail!("role_mappings cannot contain more than {MAX_ROLE_MAPPINGS} entries");
        }
        if self.grant_mappings.len() > MAX_GRANT_MAPPINGS {
            bail!("grant_mappings cannot contain more than {MAX_GRANT_MAPPINGS} entries");
        }

        let mut schemas = HashSet::new();
        for schema in &self.source_schemas {
            validate_identifier(schema, "source schema")?;
            if !schemas.insert(schema.as_str()) {
                bail!("source_schemas contains duplicate {schema:?}");
            }
        }

        let mut sources = HashSet::new();
        let mut targets = HashSet::new();
        for table in &self.tables {
            validate_identifier(&table.source_schema, "source schema")?;
            validate_identifier(&table.source_table, "source table")?;
            validate_identifier(&table.target_schema, "target schema")?;
            validate_identifier(&table.target_table, "target table")?;
            if !schemas.contains(table.source_schema.as_str()) {
                bail!(
                    "table {}.{} is outside source_schemas",
                    table.source_schema,
                    table.source_table
                );
            }
            if !sources.insert((&table.source_schema, &table.source_table)) {
                bail!(
                    "duplicate source table mapping for {}.{}",
                    table.source_schema,
                    table.source_table
                );
            }
            if !targets.insert((&table.target_schema, &table.target_table)) {
                bail!(
                    "duplicate target table mapping for {}.{}",
                    table.target_schema,
                    table.target_table
                );
            }
            let mut target_columns = HashSet::new();
            for (source, target) in &table.column_mappings {
                validate_identifier(source, "source column")?;
                validate_identifier(target, "target column")?;
                if !target_columns.insert(target) {
                    bail!(
                        "{}.{} maps multiple columns to target column {target:?}",
                        table.source_schema,
                        table.source_table
                    );
                }
            }
        }
        for (source, target) in &self.role_mappings {
            validate_identifier(source, "source role")?;
            validate_identifier(target, "target role")?;
        }
        let mut grants = HashSet::new();
        for grant in &self.grant_mappings {
            validate_identifier(&grant.source_role, "grant source role")?;
            validate_identifier(&grant.source_schema, "grant source schema")?;
            validate_identifier(&grant.source_table, "grant source table")?;
            if let Some(column) = &grant.source_column {
                validate_identifier(column, "grant source column")?;
            }
            validate_identifier(&grant.target_role, "grant target role")?;
            let privilege = grant.privilege.to_ascii_uppercase();
            if !matches!(
                privilege.as_str(),
                "SELECT" | "INSERT" | "UPDATE" | "DELETE"
            ) {
                bail!(
                    "grant privilege {:?} is outside the maintained SELECT/INSERT/UPDATE/DELETE set",
                    grant.privilege
                );
            }
            let Some(target_role) = self.role_mappings.get(&grant.source_role) else {
                bail!(
                    "grant source role {:?} has no explicit role mapping",
                    grant.source_role
                );
            };
            if target_role != &grant.target_role {
                bail!(
                    "grant target role {:?} differs from role_mappings target {:?}",
                    grant.target_role,
                    target_role
                );
            }
            if self
                .table_mapping(&grant.source_schema, &grant.source_table)
                .is_none()
            {
                bail!(
                    "grant mapping references unselected source table {}.{}",
                    grant.source_schema,
                    grant.source_table
                );
            }
            if !grants.insert(grant.clone()) {
                bail!("grant_mappings contains a duplicate entry");
            }
        }
        Ok(())
    }

    pub fn table_mapping(&self, schema: &str, table: &str) -> Option<&TableMapping> {
        self.tables
            .iter()
            .find(|mapping| mapping.source_schema == schema && mapping.source_table == table)
    }
}

fn validate_identifier(value: &str, label: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > MAX_IDENTIFIER_BYTES
        || value.as_bytes().contains(&0)
        || value.chars().any(char::is_control)
    {
        bail!("{label} must contain 1 to {MAX_IDENTIFIER_BYTES} non-control UTF-8 bytes");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> MigrationConfig {
        MigrationConfig {
            format_version: 1,
            source: SourceRequirements {
                postgres_version_num: 180_004,
                postgis_version: "3.6.1".to_owned(),
            },
            source_schemas: vec!["public".to_owned()],
            tables: vec![TableMapping {
                source_schema: "public".to_owned(),
                source_table: "places".to_owned(),
                target_schema: "main".to_owned(),
                target_table: "places".to_owned(),
                column_mappings: BTreeMap::from([("location".to_owned(), "geom_wkb".to_owned())]),
            }],
            role_mappings: BTreeMap::new(),
            grant_mappings: vec![],
        }
    }

    #[test]
    fn accepts_explicit_unique_mapping() {
        config().validate().expect("valid migration config");
    }

    #[test]
    fn rejects_implicit_scope_and_duplicate_targets() {
        let mut invalid = config();
        invalid.tables[0].source_schema = "private".to_owned();
        assert!(invalid.validate().is_err());

        let mut invalid = config();
        invalid.tables.push(TableMapping {
            source_table: "other".to_owned(),
            column_mappings: BTreeMap::new(),
            ..invalid.tables[0].clone()
        });
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn rejects_ambiguous_column_targets() {
        let mut invalid = config();
        invalid.tables[0]
            .column_mappings
            .insert("label".to_owned(), "geom_wkb".to_owned());
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn validates_explicit_role_and_grant_mappings() {
        let mut mapped = config();
        mapped
            .role_mappings
            .insert("source_reader".to_owned(), "reader".to_owned());
        mapped.grant_mappings.push(GrantMapping {
            source_role: "source_reader".to_owned(),
            source_schema: "public".to_owned(),
            source_table: "places".to_owned(),
            source_column: Some("label".to_owned()),
            privilege: "select".to_owned(),
            target_role: "reader".to_owned(),
        });
        mapped.validate().expect("explicit role and grant mapping");

        let mut invalid = mapped.clone();
        invalid.grant_mappings[0].target_role = "editor".to_owned();
        assert!(invalid.validate().is_err());

        let mut invalid = mapped;
        invalid.grant_mappings[0].privilege = "TRIGGER".to_owned();
        assert!(invalid.validate().is_err());
    }
}
