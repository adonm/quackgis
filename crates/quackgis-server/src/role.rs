// SPDX-License-Identifier: Apache-2.0
//! Immutable PostgreSQL role and future grant configuration.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow, bail};
use serde::Deserialize;

const MAX_ROLE_CONFIG_BYTES: usize = 1_048_576;
const MAX_ROLE_NAME_BYTES: usize = 63;
const BOOTSTRAP_OWNER_OID: u32 = 10;
pub const REQUEST_JWT_CLAIMS: &str = "request.jwt.claims";
const MAX_REQUEST_SETTING_BYTES: usize = 16_384;
const MAX_REQUEST_CONTEXT_BYTES: usize = 32_768;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Role {
    pub oid: u32,
    pub name: String,
    pub login: bool,
    pub inherit: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoleMembership {
    pub oid: u32,
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

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RolePrivilege {
    Member,
    Usage,
    Set,
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

    pub fn role_by_oid(&self, oid: u32) -> Option<&Role> {
        self.roles.iter().find(|role| role.oid == oid)
    }

    pub fn can_set_role(&self, session_user: &str, target: &str) -> bool {
        self.has_role_privilege(session_user, target, RolePrivilege::Set)
    }

    pub fn has_role_privilege(&self, member: &str, target: &str, privilege: RolePrivilege) -> bool {
        if self.role(member).is_none() || self.role(target).is_none() {
            return false;
        }
        if member == target {
            return true;
        }
        let mut visited = HashSet::from([member]);
        let mut pending = VecDeque::from([member]);
        while let Some(current) = pending.pop_front() {
            for edge in self.memberships.iter().filter(|edge| {
                edge.member == current
                    && match privilege {
                        RolePrivilege::Member => true,
                        RolePrivilege::Usage => edge.inherit_option,
                        RolePrivilege::Set => edge.set_option,
                    }
            }) {
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

    pub fn has_schema_privilege(
        &self,
        role: &str,
        schema: &str,
        privilege: SchemaPrivilege,
    ) -> bool {
        let public = role.eq_ignore_ascii_case("PUBLIC");
        if !public && self.role(role).is_none() {
            return false;
        }
        let inherited = self.inherited_roles(role);
        self.schema_grants.iter().any(|grant| {
            grant.schema.eq_ignore_ascii_case(schema)
                && grant.privileges.contains(&privilege)
                && grant
                    .role
                    .as_ref()
                    .is_none_or(|grantee| !public && inherited.contains(grantee.as_str()))
        })
    }

    pub fn has_table_privilege(
        &self,
        role: &str,
        schema: &str,
        table: &str,
        privilege: TablePrivilege,
    ) -> bool {
        let public = role.eq_ignore_ascii_case("PUBLIC");
        if !public && self.role(role).is_none() {
            return false;
        }
        let inherited = self.inherited_roles(role);
        if !public
            && self.table_owners.iter().any(|owner| {
                owner.schema.eq_ignore_ascii_case(schema)
                    && owner.table.eq_ignore_ascii_case(table)
                    && inherited.contains(owner.role.as_str())
            })
        {
            return true;
        }
        self.table_grants.iter().any(|grant| {
            grant.schema.eq_ignore_ascii_case(schema)
                && grant.table.eq_ignore_ascii_case(table)
                && grant.privileges.contains(&privilege)
                && grant
                    .role
                    .as_ref()
                    .is_none_or(|grantee| !public && inherited.contains(grantee.as_str()))
        })
    }

    pub fn can_create_configured_table(&self, role: &str, schema: &str, table: &str) -> bool {
        let inherited = self.inherited_roles(role);
        self.has_schema_privilege(role, schema, SchemaPrivilege::Usage)
            && self.table_owners.iter().any(|owner| {
                owner.schema.eq_ignore_ascii_case(schema)
                    && owner.table.eq_ignore_ascii_case(table)
                    && inherited.contains(owner.role.as_str())
            })
    }

    pub fn allows_table_operation(
        &self,
        role: &str,
        schema: &str,
        table: &str,
        privilege: TablePrivilege,
    ) -> bool {
        self.has_schema_privilege(role, schema, SchemaPrivilege::Usage)
            && self.has_table_privilege(role, schema, table, privilege)
    }

    fn inherited_roles<'a>(&'a self, role: &'a str) -> HashSet<&'a str> {
        if self.role(role).is_none() {
            return HashSet::new();
        }
        let mut inherited = HashSet::from([role]);
        let mut pending = VecDeque::from([role]);
        while let Some(member) = pending.pop_front() {
            for edge in self
                .memberships
                .iter()
                .filter(|edge| edge.inherit_option && edge.member == member)
            {
                if inherited.insert(edge.role.as_str()) {
                    pending.push_back(edge.role.as_str());
                }
            }
        }
        inherited
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionIdentity {
    pub session_user: String,
    pub current_user: String,
    pub epoch: u64,
    pub request_context: HashMap<String, String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum RoleSelection {
    SessionUser,
    Named(String),
}

#[derive(Debug)]
struct RoleSession {
    session_user: String,
    session_role: RoleSelection,
    local_role: Option<RoleSelection>,
    epoch: u64,
    request_context: HashMap<String, String>,
}

#[derive(Debug)]
pub struct RoleSessionState {
    catalog: Option<Arc<RoleCatalog>>,
    session: Mutex<RoleSession>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RoleSessionErrorKind {
    UnknownRole,
    PermissionDenied,
    NoTransaction,
    InvalidInput,
    Internal,
}

#[derive(Debug)]
pub struct RoleSessionError {
    pub kind: RoleSessionErrorKind,
    message: String,
}

impl RoleSessionError {
    fn new(kind: RoleSessionErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl fmt::Display for RoleSessionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for RoleSessionError {}

impl RoleSessionState {
    pub fn new(session_user: String, catalog: Option<Arc<RoleCatalog>>) -> Result<Self> {
        if let Some(catalog) = &catalog {
            let role = catalog
                .role(&session_user)
                .ok_or_else(|| anyhow!("authenticated user has no configured role"))?;
            if !role.login {
                bail!("authenticated user names a NOLOGIN role");
            }
        }
        Ok(Self {
            catalog,
            session: Mutex::new(RoleSession {
                session_user,
                session_role: RoleSelection::SessionUser,
                local_role: None,
                epoch: 0,
                request_context: HashMap::new(),
            }),
        })
    }

    pub fn identity(&self) -> Result<SessionIdentity, RoleSessionError> {
        let session = self.session.lock().map_err(|_| {
            RoleSessionError::new(
                RoleSessionErrorKind::Internal,
                "PostgreSQL role session state is unavailable",
            )
        })?;
        Ok(SessionIdentity {
            session_user: session.session_user.clone(),
            current_user: effective_role(&session).to_owned(),
            epoch: session.epoch,
            request_context: session.request_context.clone(),
        })
    }

    pub fn set_role(
        &self,
        target: Option<&str>,
        local: bool,
        in_transaction: bool,
    ) -> Result<(), RoleSessionError> {
        if local && !in_transaction {
            return Err(RoleSessionError::new(
                RoleSessionErrorKind::NoTransaction,
                "SET LOCAL ROLE requires an explicit transaction",
            ));
        }
        let mut session = self.session.lock().map_err(|_| {
            RoleSessionError::new(
                RoleSessionErrorKind::Internal,
                "PostgreSQL role session state is unavailable",
            )
        })?;
        let selection = match target {
            None => RoleSelection::SessionUser,
            Some(target) => {
                let Some(catalog) = &self.catalog else {
                    return Err(RoleSessionError::new(
                        RoleSessionErrorKind::UnknownRole,
                        format!("role {target:?} does not exist"),
                    ));
                };
                if catalog.role(target).is_none() {
                    return Err(RoleSessionError::new(
                        RoleSessionErrorKind::UnknownRole,
                        format!("role {target:?} does not exist"),
                    ));
                }
                if !catalog.can_set_role(&session.session_user, target) {
                    return Err(RoleSessionError::new(
                        RoleSessionErrorKind::PermissionDenied,
                        format!("permission denied to set role {target:?}"),
                    ));
                }
                RoleSelection::Named(target.to_owned())
            }
        };
        let changed = if local {
            session.local_role.as_ref() != Some(&selection)
        } else {
            session.session_role != selection || session.local_role.is_some()
        };
        if local {
            session.local_role = Some(selection);
        } else {
            session.session_role = selection;
            session.local_role = None;
        }
        if changed {
            session.epoch = session.epoch.wrapping_add(1);
        }
        Ok(())
    }

    pub fn reset_role(&self) -> Result<(), RoleSessionError> {
        let mut session = self.session.lock().map_err(|_| {
            RoleSessionError::new(
                RoleSessionErrorKind::Internal,
                "PostgreSQL role session state is unavailable",
            )
        })?;
        if session.session_role != RoleSelection::SessionUser || session.local_role.is_some() {
            session.session_role = RoleSelection::SessionUser;
            session.local_role = None;
            session.epoch = session.epoch.wrapping_add(1);
        }
        Ok(())
    }

    pub fn end_transaction(&self) -> Result<(), RoleSessionError> {
        let mut session = self.session.lock().map_err(|_| {
            RoleSessionError::new(
                RoleSessionErrorKind::Internal,
                "PostgreSQL role session state is unavailable",
            )
        })?;
        if session.local_role.take().is_some() || !session.request_context.is_empty() {
            session.request_context.clear();
            session.epoch = session.epoch.wrapping_add(1);
        }
        Ok(())
    }

    pub fn set_request_setting(
        &self,
        name: &str,
        value: &str,
        in_transaction: bool,
    ) -> Result<(), RoleSessionError> {
        if !in_transaction {
            return Err(RoleSessionError::new(
                RoleSessionErrorKind::NoTransaction,
                "transaction-local request context requires an explicit transaction",
            ));
        }
        if name != REQUEST_JWT_CLAIMS {
            return Err(RoleSessionError::new(
                RoleSessionErrorKind::InvalidInput,
                format!("request setting {name:?} is not allowlisted"),
            ));
        }
        if value.len() > MAX_REQUEST_SETTING_BYTES || value.contains('\0') {
            return Err(RoleSessionError::new(
                RoleSessionErrorKind::InvalidInput,
                "request setting exceeds 16384 bytes or contains NUL",
            ));
        }
        let mut session = self.session.lock().map_err(|_| {
            RoleSessionError::new(
                RoleSessionErrorKind::Internal,
                "PostgreSQL role session state is unavailable",
            )
        })?;
        let replaced_bytes = session
            .request_context
            .get(name)
            .map_or(0, |current| name.len() + current.len());
        let current_bytes = session
            .request_context
            .iter()
            .map(|(name, value)| name.len() + value.len())
            .sum::<usize>();
        let next_bytes = current_bytes - replaced_bytes + name.len() + value.len();
        if next_bytes > MAX_REQUEST_CONTEXT_BYTES {
            return Err(RoleSessionError::new(
                RoleSessionErrorKind::InvalidInput,
                "request context exceeds the 32768-byte session limit",
            ));
        }
        if session.request_context.get(name).map(String::as_str) != Some(value) {
            session
                .request_context
                .insert(name.to_owned(), value.to_owned());
            session.epoch = session.epoch.wrapping_add(1);
        }
        Ok(())
    }
}

fn effective_role(session: &RoleSession) -> &str {
    match session.local_role.as_ref().unwrap_or(&session.session_role) {
        RoleSelection::SessionUser => &session.session_user,
        RoleSelection::Named(role) => role,
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
        let mut membership_oids = HashSet::new();
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
            if configured.oid == 0 || !membership_oids.insert(configured.oid) {
                bail!(
                    "membership from {:?} to {:?} has a zero or duplicate OID {}",
                    member,
                    role,
                    configured.oid
                );
            }
            if !membership_keys.insert((member.to_owned(), role.to_owned())) {
                bail!("duplicate membership from {:?} to {:?}", member, role);
            }
            let inherit_option = configured
                .inherit_option
                .unwrap_or_else(|| roles[role_indexes[member]].inherit);
            memberships.push(RoleMembership {
                oid: configured.oid,
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
    oid: u32,
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
        {"oid": 200001, "role": "analyst", "member": "authenticator", "set_option": true},
        {"oid": 200002, "role": "api_reader", "member": "analyst", "inherit_option": false}
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
            r#"{"roles":[{"oid":11,"name":"a"},{"oid":12,"name":"b"}],"memberships":[{"oid":21,"role":"b","member":"a"},{"oid":22,"role":"a","member":"b"}]}"#,
            r#"{"roles":[{"oid":10,"name":"reserved"}]}"#,
            r#"{"roles":[{"oid":11,"name":"UPPER"}]}"#,
            r#"{"roles":[{"oid":11,"name":"a"},{"oid":11,"name":"b"}]}"#,
            r#"{"roles":[{"oid":11,"name":"a"}],"memberships":[{"oid":21,"role":"missing","member":"a"}]}"#,
            r#"{"roles":[{"oid":11,"name":"a"}],"memberships":[{"oid":21,"role":"a","member":"a"}]}"#,
            r#"{"roles":[{"oid":11,"name":"a"}],"memberships":[{"oid":21,"role":"a","member":"missing"}]}"#,
            r#"{"roles":[{"oid":11,"name":"a"},{"oid":12,"name":"b"},{"oid":13,"name":"c"}],"memberships":[{"oid":21,"role":"a","member":"b"},{"oid":21,"role":"a","member":"c"}]}"#,
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

    #[test]
    fn role_session_uses_original_login_reachability_and_cleans_local_state() {
        let catalog = Arc::new(RoleCatalog::from_json(CONFIG).expect("role configuration"));
        let state =
            RoleSessionState::new("authenticator".to_owned(), Some(catalog)).expect("role session");
        assert_eq!(
            state.identity().expect("initial identity"),
            SessionIdentity {
                session_user: "authenticator".to_owned(),
                current_user: "authenticator".to_owned(),
                epoch: 0,
                request_context: HashMap::new(),
            }
        );

        state
            .set_role(Some("analyst"), false, false)
            .expect("set reachable role");
        assert_eq!(
            state.identity().expect("assumed identity").current_user,
            "analyst"
        );
        let denied = state
            .set_role(Some("authenticator"), true, false)
            .expect_err("SET LOCAL outside transaction");
        assert_eq!(denied.kind, RoleSessionErrorKind::NoTransaction);
        state
            .set_role(Some("api_reader"), true, true)
            .expect("set transitive local role from original login");
        assert_eq!(
            state.identity().expect("local identity").current_user,
            "api_reader"
        );
        state.end_transaction().expect("transaction cleanup");
        assert_eq!(
            state.identity().expect("restored identity").current_user,
            "analyst"
        );
        state.reset_role().expect("reset role");
        assert_eq!(
            state.identity().expect("reset identity").current_user,
            "authenticator"
        );
    }

    #[test]
    fn assumed_role_does_not_expand_original_login_reachability() {
        let catalog = Arc::new(
            RoleCatalog::from_json(
                r#"{
                  "roles":[
                    {"oid":11,"name":"login","login":true},
                    {"oid":12,"name":"first"},
                    {"oid":13,"name":"unreachable"},
                    {"oid":14,"name":"target"}
                  ],
                  "memberships":[
                    {"oid":21,"role":"first","member":"login"},
                    {"oid":22,"role":"target","member":"unreachable"}
                  ]
                }"#,
            )
            .expect("role configuration"),
        );
        let state = RoleSessionState::new("login".to_owned(), Some(catalog)).expect("role session");
        state
            .set_role(Some("first"), false, false)
            .expect("first assumption");
        let denied = state
            .set_role(Some("target"), false, false)
            .expect_err("current role cannot expand assumption graph");
        assert_eq!(denied.kind, RoleSessionErrorKind::PermissionDenied);
    }

    #[test]
    fn request_context_is_transaction_local_allowlisted_and_bounded() {
        let state = RoleSessionState::new("postgres".to_owned(), None).expect("role session");
        let outside = state
            .set_request_setting(REQUEST_JWT_CLAIMS, "{}", false)
            .expect_err("context outside transaction");
        assert_eq!(outside.kind, RoleSessionErrorKind::NoTransaction);
        let unknown = state
            .set_request_setting("arbitrary.setting", "value", true)
            .expect_err("unknown request setting");
        assert_eq!(unknown.kind, RoleSessionErrorKind::InvalidInput);
        let oversized = "x".repeat(MAX_REQUEST_SETTING_BYTES + 1);
        assert!(
            state
                .set_request_setting(REQUEST_JWT_CLAIMS, &oversized, true)
                .is_err()
        );

        state
            .set_request_setting(REQUEST_JWT_CLAIMS, r#"{"sub":"reader"}"#, true)
            .expect("bounded request context");
        let identity = state.identity().expect("context identity snapshot");
        assert_eq!(
            identity
                .request_context
                .get(REQUEST_JWT_CLAIMS)
                .map(String::as_str),
            Some(r#"{"sub":"reader"}"#)
        );
        state.end_transaction().expect("context cleanup");
        assert!(
            state
                .identity()
                .expect("clean identity")
                .request_context
                .is_empty()
        );
    }

    #[test]
    fn one_privilege_engine_combines_ownership_grants_public_and_inheritance() {
        let catalog = RoleCatalog::from_json(
            r#"{
              "roles": [
                {"oid": 11, "name": "owner"},
                {"oid": 12, "name": "reader", "login": true},
                {"oid": 13, "name": "editor", "login": true},
                {"oid": 14, "name": "noinherit", "login": true, "inherit": false}
              ],
              "memberships": [
                {"oid": 21, "role": "owner", "member": "editor", "inherit_option": true},
                {"oid": 22, "role": "owner", "member": "noinherit"}
              ],
              "table_owners": [{"table": "places", "role": "owner"}],
              "schema_grants": [
                {"schema": "public", "role": "PUBLIC", "privileges": ["USAGE"]}
              ],
              "table_grants": [
                {"table": "places", "role": "reader", "privileges": ["SELECT"]},
                {"table": "places", "role": "PUBLIC", "privileges": ["MAINTAIN"]}
              ]
            }"#,
        )
        .expect("privilege catalog");

        assert!(catalog.allows_table_operation("reader", "main", "places", TablePrivilege::Select));
        assert!(!catalog.allows_table_operation(
            "reader",
            "main",
            "places",
            TablePrivilege::Update
        ));
        assert!(catalog.allows_table_operation("editor", "main", "places", TablePrivilege::Delete));
        assert!(catalog.can_create_configured_table("editor", "main", "places"));
        assert!(catalog.has_role_privilege("editor", "owner", RolePrivilege::Member));
        assert!(catalog.has_role_privilege("editor", "owner", RolePrivilege::Usage));
        assert!(catalog.has_role_privilege("editor", "owner", RolePrivilege::Set));
        assert!(catalog.has_role_privilege("noinherit", "owner", RolePrivilege::Member));
        assert!(!catalog.has_role_privilege("noinherit", "owner", RolePrivilege::Usage));
        assert!(catalog.has_role_privilege("noinherit", "owner", RolePrivilege::Set));
        assert!(!catalog.allows_table_operation(
            "noinherit",
            "main",
            "places",
            TablePrivilege::Select
        ));
        assert!(catalog.allows_table_operation(
            "noinherit",
            "main",
            "places",
            TablePrivilege::Maintain
        ));
        assert!(catalog.has_schema_privilege("PUBLIC", "main", SchemaPrivilege::Usage));
        assert!(catalog.has_table_privilege("PUBLIC", "main", "places", TablePrivilege::Maintain));
        assert!(!catalog.has_table_privilege("PUBLIC", "main", "places", TablePrivilege::Select));
        assert!(!catalog.has_table_privilege(
            "missing",
            "main",
            "places",
            TablePrivilege::Maintain
        ));
    }
}
