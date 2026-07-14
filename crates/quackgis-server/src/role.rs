// SPDX-License-Identifier: Apache-2.0
//! Immutable PostgreSQL role and future grant configuration.

use std::collections::{HashMap, HashSet, VecDeque};

use anyhow::{Result, anyhow, bail};
use serde::Deserialize;

const MAX_ROLE_CONFIG_BYTES: usize = 1_048_576;
const MAX_ROLE_NAME_BYTES: usize = 63;
const BOOTSTRAP_OWNER_OID: u32 = 10;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Role {
    pub oid: u32,
    pub name: String,
    pub login: bool,
    pub inherit: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoleMembership {
    pub role: String,
    pub member: String,
    pub inherit_option: bool,
    pub set_option: bool,
    pub admin_option: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TableOwner {
    pub schema: String,
    pub table: String,
    pub role: String,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SchemaPrivilege {
    Usage,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TablePrivilege {
    Select,
    Insert,
    Update,
    Delete,
    Maintain,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SchemaGrant {
    pub schema: String,
    pub role: Option<String>,
    pub privileges: Vec<SchemaPrivilege>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TableGrant {
    pub schema: String,
    pub table: String,
    pub role: Option<String>,
    pub privileges: Vec<TablePrivilege>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoleCatalog {
    roles: Vec<Role>,
    memberships: Vec<RoleMembership>,
    table_owners: Vec<TableOwner>,
    schema_grants: Vec<SchemaGrant>,
    table_grants: Vec<TableGrant>,
    role_indexes: HashMap<String, usize>,
}

impl RoleCatalog {
    pub fn from_json(raw: &str) -> Result<Self> {
        if raw.len() > MAX_ROLE_CONFIG_BYTES {
            bail!("role configuration exceeds the 1048576-byte limit");
        }
        let document: RoleConfigDocument = serde_json::from_str(raw)
            .map_err(|error| anyhow!("invalid role configuration: {error}"))?;
        Self::try_from(document)
    }

    pub fn roles(&self) -> &[Role] {
        &self.roles
    }

    pub fn memberships(&self) -> &[RoleMembership] {
        &self.memberships
    }

    pub fn table_owners(&self) -> &[TableOwner] {
        &self.table_owners
    }

    pub fn schema_grants(&self) -> &[SchemaGrant] {
        &self.schema_grants
    }

    pub fn table_grants(&self) -> &[TableGrant] {
        &self.table_grants
    }

    pub fn role(&self, name: &str) -> Option<&Role> {
        self.role_indexes
            .get(name)
            .and_then(|index| self.roles.get(*index))
    }

    pub fn can_set_role(&self, session_user: &str, target: &str) -> bool {
        if session_user == target {
            return self.role(session_user).is_some();
        }
        let mut visited = HashSet::from([session_user]);
        let mut pending = VecDeque::from([session_user]);
        while let Some(member) = pending.pop_front() {
            for edge in self
                .memberships
                .iter()
                .filter(|edge| edge.set_option && edge.member == member)
            {
                if edge.role == target {
                    return true;
                }
                if visited.insert(edge.role.as_str()) {
                    pending.push_back(edge.role.as_str());
                }
            }
        }
        false
    }
}

impl TryFrom<RoleConfigDocument> for RoleCatalog {
    type Error = anyhow::Error;

    fn try_from(document: RoleConfigDocument) -> Result<Self> {
        if document.roles.is_empty() {
            bail!("role configuration must declare at least one role");
        }
        let mut names = HashSet::new();
        let mut oids = HashSet::new();
        let mut roles = Vec::with_capacity(document.roles.len());
        for configured in document.roles {
            validate_role_name(&configured.name)?;
            if !names.insert(configured.name.clone()) {
                bail!("duplicate configured role {:?}", configured.name);
            }
            if configured.oid == 0 || configured.oid == BOOTSTRAP_OWNER_OID {
                bail!(
                    "configured role {:?} uses reserved PostgreSQL role OID {}",
                    configured.name,
                    configured.oid
                );
            }
            if !oids.insert(configured.oid) {
                bail!("duplicate configured role OID {}", configured.oid);
            }
            roles.push(Role {
                oid: configured.oid,
                name: configured.name,
                login: configured.login,
                inherit: configured.inherit,
            });
        }
        roles.sort_by_key(|role| role.oid);
        let role_indexes = roles
            .iter()
            .enumerate()
            .map(|(index, role)| (role.name.clone(), index))
            .collect::<HashMap<_, _>>();

        let mut membership_keys = HashSet::new();
        let mut memberships = Vec::with_capacity(document.memberships.len());
        for configured in document.memberships {
            let role = known_role(&role_indexes, &configured.role, "membership role")?;
            let member = known_role(&role_indexes, &configured.member, "membership member")?;
            if role == member {
                bail!("role {:?} cannot be a member of itself", role);
            }
            if configured.admin_option {
                bail!("membership admin_option is not supported by immutable role provisioning");
            }
            if !membership_keys.insert((member.to_owned(), role.to_owned())) {
                bail!("duplicate membership from {:?} to {:?}", member, role);
            }
            let inherit_option = configured
                .inherit_option
                .unwrap_or_else(|| roles[role_indexes[member]].inherit);
            memberships.push(RoleMembership {
                role: role.to_owned(),
                member: member.to_owned(),
                inherit_option,
                set_option: configured.set_option,
                admin_option: false,
            });
        }
        memberships
            .sort_by(|left, right| (&left.member, &left.role).cmp(&(&right.member, &right.role)));
        reject_membership_cycles(&roles, &memberships)?;

        let mut owner_keys = HashSet::new();
        let mut table_owners = Vec::with_capacity(document.table_owners.len());
        for configured in document.table_owners {
            let role = known_role(&role_indexes, &configured.role, "table owner")?;
            let (schema, table) = normalize_table(&configured.table, "table owner")?;
            if !owner_keys.insert((schema.clone(), table.clone())) {
                bail!("duplicate owner for table {schema}.{table}");
            }
            table_owners.push(TableOwner {
                schema,
                table,
                role: role.to_owned(),
            });
        }
        table_owners
            .sort_by(|left, right| (&left.schema, &left.table).cmp(&(&right.schema, &right.table)));

        let mut schema_grant_keys = HashSet::new();
        let mut schema_grants = Vec::with_capacity(document.schema_grants.len());
        for configured in document.schema_grants {
            let role = normalize_grantee(&role_indexes, &configured.role)?;
            let schema = normalize_schema(&configured.schema, "schema grant")?;
            let privileges = nonempty_unique(
                configured
                    .privileges
                    .into_iter()
                    .map(|privilege| match privilege {
                        SchemaPrivilegeConfig::Usage => SchemaPrivilege::Usage,
                    }),
                "schema grant",
            )?;
            if !schema_grant_keys.insert((schema.clone(), role.clone())) {
                bail!(
                    "duplicate schema grant for {:?} on {schema}",
                    configured.role
                );
            }
            schema_grants.push(SchemaGrant {
                schema,
                role,
                privileges,
            });
        }
        schema_grants
            .sort_by(|left, right| (&left.schema, &left.role).cmp(&(&right.schema, &right.role)));

        let mut table_grant_keys = HashSet::new();
        let mut table_grants = Vec::with_capacity(document.table_grants.len());
        for configured in document.table_grants {
            let role = normalize_grantee(&role_indexes, &configured.role)?;
            let (schema, table) = normalize_table(&configured.table, "table grant")?;
            let privileges = nonempty_unique(
                configured
                    .privileges
                    .into_iter()
                    .map(|privilege| match privilege {
                        TablePrivilegeConfig::Select => TablePrivilege::Select,
                        TablePrivilegeConfig::Insert => TablePrivilege::Insert,
                        TablePrivilegeConfig::Update => TablePrivilege::Update,
                        TablePrivilegeConfig::Delete => TablePrivilege::Delete,
                        TablePrivilegeConfig::Maintain => TablePrivilege::Maintain,
                    }),
                "table grant",
            )?;
            if !table_grant_keys.insert((schema.clone(), table.clone(), role.clone())) {
                bail!(
                    "duplicate table grant for {:?} on {schema}.{table}",
                    configured.role
                );
            }
            table_grants.push(TableGrant {
                schema,
                table,
                role,
                privileges,
            });
        }
        table_grants.sort_by(|left, right| {
            (&left.schema, &left.table, &left.role).cmp(&(&right.schema, &right.table, &right.role))
        });

        Ok(Self {
            roles,
            memberships,
            table_owners,
            schema_grants,
            table_grants,
            role_indexes,
        })
    }
}

fn validate_role_name(name: &str) -> Result<()> {
    let mut chars = name.chars();
    if name.len() > MAX_ROLE_NAME_BYTES
        || !chars
            .next()
            .is_some_and(|character| character == '_' || character.is_ascii_lowercase())
        || chars.any(|character| {
            character != '_'
                && character != '$'
                && !character.is_ascii_lowercase()
                && !character.is_ascii_digit()
        })
    {
        bail!(
            "configured role {name:?} must be a lowercase unquoted PostgreSQL identifier of at most 63 bytes"
        );
    }
    if matches!(name, "public" | "none" | "current_user" | "session_user") {
        bail!("configured role name {name:?} is reserved");
    }
    Ok(())
}

fn known_role<'a>(
    indexes: &'a HashMap<String, usize>,
    name: &'a str,
    label: &str,
) -> Result<&'a str> {
    if indexes.contains_key(name) {
        Ok(name)
    } else {
        bail!("{label} names unknown role {name:?}")
    }
}

fn normalize_grantee(indexes: &HashMap<String, usize>, grantee: &str) -> Result<Option<String>> {
    if grantee == "PUBLIC" {
        Ok(None)
    } else {
        known_role(indexes, grantee, "grant grantee").map(|role| Some(role.to_owned()))
    }
}

fn normalize_schema(raw: &str, label: &str) -> Result<String> {
    if raw.eq_ignore_ascii_case("public") || raw.eq_ignore_ascii_case("main") {
        Ok("main".to_owned())
    } else {
        bail!("{label} supports the public schema only, not {raw:?}")
    }
}

fn normalize_table(raw: &str, label: &str) -> Result<(String, String)> {
    let parts = raw.split('.').collect::<Vec<_>>();
    let (schema, table) = match parts.as_slice() {
        [table] => ("main", *table),
        [schema, table]
            if schema.eq_ignore_ascii_case("public") || schema.eq_ignore_ascii_case("main") =>
        {
            ("main", *table)
        }
        _ => bail!("{label} table {raw:?} must be table, public.table, or main.table"),
    };
    if table.is_empty()
        || table.len() > MAX_ROLE_NAME_BYTES
        || table.chars().any(|character| character.is_control())
    {
        bail!("{label} table name is empty, too long, or contains control characters");
    }
    Ok((schema.to_owned(), table.to_owned()))
}

fn nonempty_unique<T>(values: impl Iterator<Item = T>, label: &str) -> Result<Vec<T>>
where
    T: Copy + Eq + std::hash::Hash,
{
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for value in values {
        if !seen.insert(value) {
            bail!("{label} repeats a privilege");
        }
        result.push(value);
    }
    if result.is_empty() {
        bail!("{label} must declare at least one privilege");
    }
    Ok(result)
}

fn reject_membership_cycles(roles: &[Role], memberships: &[RoleMembership]) -> Result<()> {
    let outgoing = memberships.iter().fold(
        HashMap::<&str, Vec<&str>>::new(),
        |mut outgoing, membership| {
            outgoing
                .entry(&membership.member)
                .or_default()
                .push(&membership.role);
            outgoing
        },
    );
    for role in roles {
        let mut visited = HashSet::new();
        let mut pending = Vec::from([role.name.as_str()]);
        while let Some(member) = pending.pop() {
            for granted in outgoing.get(member).into_iter().flatten() {
                if *granted == role.name {
                    bail!(
                        "role membership graph contains a cycle through {:?}",
                        role.name
                    );
                }
                if visited.insert(*granted) {
                    pending.push(granted);
                }
            }
        }
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RoleConfigDocument {
    roles: Vec<RoleConfig>,
    #[serde(default)]
    memberships: Vec<MembershipConfig>,
    #[serde(default)]
    table_owners: Vec<TableOwnerConfig>,
    #[serde(default)]
    schema_grants: Vec<SchemaGrantConfig>,
    #[serde(default)]
    table_grants: Vec<TableGrantConfig>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RoleConfig {
    oid: u32,
    name: String,
    #[serde(default)]
    login: bool,
    #[serde(default = "default_true")]
    inherit: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MembershipConfig {
    role: String,
    member: String,
    inherit_option: Option<bool>,
    #[serde(default = "default_true")]
    set_option: bool,
    #[serde(default)]
    admin_option: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TableOwnerConfig {
    table: String,
    role: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SchemaGrantConfig {
    schema: String,
    role: String,
    privileges: Vec<SchemaPrivilegeConfig>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
enum SchemaPrivilegeConfig {
    Usage,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TableGrantConfig {
    table: String,
    role: String,
    privileges: Vec<TablePrivilegeConfig>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
enum TablePrivilegeConfig {
    Select,
    Insert,
    Update,
    Delete,
    Maintain,
}

const fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = r#"{
      "roles": [
        {"oid": 100003, "name": "api_reader"},
        {"oid": 100001, "name": "authenticator", "login": true},
        {"oid": 100002, "name": "analyst", "inherit": false}
      ],
      "memberships": [
        {"role": "analyst", "member": "authenticator", "set_option": true},
        {"role": "api_reader", "member": "analyst", "inherit_option": false}
      ],
      "table_owners": [{"table": "public.points", "role": "analyst"}],
      "schema_grants": [
        {"schema": "public", "role": "PUBLIC", "privileges": ["USAGE"]}
      ],
      "table_grants": [
        {"table": "points", "role": "api_reader", "privileges": ["SELECT"]}
      ]
    }"#;

    #[test]
    fn role_configuration_is_order_independent_and_reachable_only_over_set_edges() {
        let catalog = RoleCatalog::from_json(CONFIG).expect("role configuration");
        assert_eq!(
            catalog
                .roles()
                .iter()
                .map(|role| (role.oid, role.name.as_str()))
                .collect::<Vec<_>>(),
            vec![
                (100001, "authenticator"),
                (100002, "analyst"),
                (100003, "api_reader")
            ]
        );
        assert!(catalog.can_set_role("authenticator", "analyst"));
        assert!(catalog.can_set_role("authenticator", "api_reader"));
        assert!(!catalog.can_set_role("api_reader", "authenticator"));
        assert_eq!(catalog.schema_grants()[0].role, None);
        assert_eq!(catalog.table_owners()[0].schema, "main");
        assert_eq!(catalog.table_grants()[0].schema, "main");
        assert!(!catalog.memberships()[0].inherit_option);
    }

    #[test]
    fn role_configuration_rejects_cycles_and_privilege_or_identity_ambiguity() {
        for invalid in [
            r#"{"roles":[{"oid":11,"name":"a"},{"oid":12,"name":"b"}],"memberships":[{"role":"b","member":"a"},{"role":"a","member":"b"}]}"#,
            r#"{"roles":[{"oid":10,"name":"reserved"}]}"#,
            r#"{"roles":[{"oid":11,"name":"UPPER"}]}"#,
            r#"{"roles":[{"oid":11,"name":"a"},{"oid":11,"name":"b"}]}"#,
            r#"{"roles":[{"oid":11,"name":"a"}],"memberships":[{"role":"missing","member":"a"}]}"#,
            r#"{"roles":[{"oid":11,"name":"a"}],"memberships":[{"role":"a","member":"a"}]}"#,
            r#"{"roles":[{"oid":11,"name":"a"}],"memberships":[{"role":"a","member":"missing"}]}"#,
            r#"{"roles":[{"oid":11,"name":"a"}],"table_grants":[{"table":"points","role":"a","privileges":[]}]}"#,
            r#"{"roles":[{"oid":11,"name":"a"}],"table_grants":[{"table":"points","role":"a","privileges":["SELECT","SELECT"]}]}"#,
            r#"{"roles":[{"oid":11,"name":"a"}],"unknown":true}"#,
        ] {
            assert!(
                RoleCatalog::from_json(invalid).is_err(),
                "invalid config accepted: {invalid}"
            );
        }
    }

    #[test]
    fn role_configuration_is_size_bounded_before_json_parsing() {
        let oversized = " ".repeat(MAX_ROLE_CONFIG_BYTES + 1);
        let error = RoleCatalog::from_json(&oversized).expect_err("oversized config");
        assert!(error.to_string().contains("1048576-byte"));
    }
}
