// SPDX-License-Identifier: Apache-2.0
//! QuackGIS pgwire authentication and coarse role/table-policy configuration.

use std::{
    collections::{HashMap, HashSet},
    fmt,
    sync::Arc,
};

use anyhow::{Result, anyhow};
use pgwire::api::ClientInfo;
use pgwire::api::auth::sasl::scram::{SCRAM_ITERATIONS, gen_salted_password, random_nonce};

use crate::role::{RoleCatalog, RoleSessionState};

const METADATA_USER: &str = "user";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AuthMode {
    Trust,
    Password,
    EdgePreauthenticated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AccessRole {
    ReadWrite,
    ReadOnly,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct WriteTarget {
    pub schema: String,
    pub table: String,
}

#[derive(Clone, Default, PartialEq, Eq)]
enum TablePolicy {
    #[default]
    All,
    Only(Vec<WriteTarget>),
}

impl fmt::Debug for TablePolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::All => f.write_str("All"),
            Self::Only(targets) => f.debug_tuple("Only").field(targets).finish(),
        }
    }
}

impl TablePolicy {
    fn only(mut targets: Vec<WriteTarget>) -> Self {
        targets.sort();
        targets.dedup();
        Self::Only(targets)
    }

    fn allows(&self, target: Option<(&str, &str)>) -> bool {
        match self {
            Self::All => true,
            Self::Only(targets) => target.is_some_and(|(schema, table)| {
                targets.iter().any(|allowed| {
                    allowed.schema.eq_ignore_ascii_case(schema)
                        && allowed.table.eq_ignore_ascii_case(table)
                })
            }),
        }
    }

    fn allowed_targets(&self) -> Option<&[WriteTarget]> {
        match self {
            Self::All => None,
            Self::Only(targets) => Some(targets),
        }
    }

    fn is_restricted(&self) -> bool {
        matches!(self, Self::Only(_))
    }
}

#[derive(Clone)]
pub struct AuthUser {
    pub role: AccessRole,
    write_policy: TablePolicy,
    pub(crate) scram_salt: Vec<u8>,
    pub(crate) scram_salted_password: Vec<u8>,
}

impl fmt::Debug for AuthUser {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthUser")
            .field("role", &self.role)
            .field("write_policy", &self.write_policy)
            .field("scram_salt_len", &self.scram_salt.len())
            .field("scram_salted_password", &"<redacted>")
            .finish()
    }
}

impl AuthUser {
    fn scram(password: &str, role: AccessRole) -> Self {
        Self::scram_with_policy(password, role, TablePolicy::All)
    }

    fn scram_with_policy(password: &str, role: AccessRole, write_policy: TablePolicy) -> Self {
        let scram_salt = random_nonce().into_bytes();
        let scram_salted_password = gen_salted_password(password, &scram_salt, SCRAM_ITERATIONS);
        Self {
            role,
            write_policy,
            scram_salt,
            scram_salted_password,
        }
    }

    pub fn write_targets(&self) -> Option<&[WriteTarget]> {
        self.write_policy.allowed_targets()
    }

    pub fn allows_write_target(&self, schema: &str, table: &str) -> bool {
        matches!(self.role, AccessRole::ReadWrite)
            && self.write_policy.allows(Some((schema, table)))
    }
}

#[derive(Debug, Clone)]
pub struct AuthConfig {
    mode: AuthMode,
    users: HashMap<String, AuthUser>,
    trust_write_policy: TablePolicy,
    read_policy: TablePolicy,
    maintenance_user: Option<String>,
    role_catalog: Option<Arc<RoleCatalog>>,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self::trust()
    }
}

impl AuthConfig {
    pub fn trust() -> Self {
        Self {
            mode: AuthMode::Trust,
            users: HashMap::new(),
            trust_write_policy: TablePolicy::All,
            read_policy: TablePolicy::All,
            maintenance_user: None,
            role_catalog: None,
        }
    }

    pub fn password(
        readwrite_user: impl Into<String>,
        readwrite_password: impl Into<String>,
        readonly: Option<(impl Into<String>, impl Into<String>)>,
    ) -> Result<Self> {
        let readwrite_user = readwrite_user.into();
        let readwrite_password = readwrite_password.into();
        validate_user_password("readwrite", &readwrite_user, &readwrite_password)?;

        let mut users = HashMap::new();
        users.insert(
            readwrite_user.clone(),
            AuthUser::scram(&readwrite_password, AccessRole::ReadWrite),
        );

        if let Some((readonly_user, readonly_password)) = readonly {
            let readonly_user = readonly_user.into();
            let readonly_password = readonly_password.into();
            validate_user_password("readonly", &readonly_user, &readonly_password)?;
            if readonly_user == readwrite_user {
                return Err(anyhow!(
                    "readonly user must differ from readwrite user in password auth mode"
                ));
            }
            users.insert(
                readonly_user,
                AuthUser::scram(&readonly_password, AccessRole::ReadOnly),
            );
        }

        Ok(Self {
            mode: AuthMode::Password,
            users,
            trust_write_policy: TablePolicy::All,
            read_policy: TablePolicy::All,
            maintenance_user: None,
            role_catalog: None,
        })
    }

    pub fn edge_preauthenticated(role_catalog: RoleCatalog) -> Result<Self> {
        if !role_catalog.roles().iter().any(|role| role.login) {
            return Err(anyhow!(
                "edge-preauthenticated role configuration requires at least one LOGIN role"
            ));
        }
        Ok(Self {
            mode: AuthMode::EdgePreauthenticated,
            users: HashMap::new(),
            trust_write_policy: TablePolicy::All,
            read_policy: TablePolicy::All,
            maintenance_user: None,
            role_catalog: Some(Arc::new(role_catalog)),
        })
    }

    pub fn with_role_catalog(mut self, role_catalog: RoleCatalog) -> Result<Self> {
        if self.mode != AuthMode::Password {
            return Err(anyhow!(
                "PostgreSQL role configuration requires password authentication"
            ));
        }
        let auth_users = self
            .users
            .keys()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        let login_roles = role_catalog
            .roles()
            .iter()
            .filter(|role| role.login)
            .map(|role| role.name.as_str())
            .collect::<HashSet<_>>();
        if auth_users != login_roles {
            return Err(anyhow!(
                "configured LOGIN roles must exactly match password-authenticated users"
            ));
        }
        self.role_catalog = Some(Arc::new(role_catalog));
        Ok(self)
    }

    pub fn with_maintenance_user(mut self, user: impl Into<String>) -> Result<Self> {
        let user = user.into();
        if user.trim() != user || user.is_empty() || user.chars().any(char::is_control) {
            return Err(anyhow!(
                "maintenance user cannot be empty or contain surrounding whitespace/control characters"
            ));
        }
        let valid_maintenance_user = match self.mode {
            AuthMode::Trust => true,
            AuthMode::Password => self
                .users
                .get(&user)
                .is_some_and(|configured| configured.role == AccessRole::ReadWrite),
            AuthMode::EdgePreauthenticated => self.allows_preauthenticated_login(&user),
        };
        if !valid_maintenance_user {
            return Err(anyhow!(
                "maintenance user must name the configured readwrite identity"
            ));
        }
        self.maintenance_user = Some(user);
        Ok(self)
    }

    pub fn with_readwrite_allowlist(mut self, targets: Vec<WriteTarget>) -> Self {
        let policy = TablePolicy::only(targets);
        match self.mode {
            AuthMode::Trust => {
                self.trust_write_policy = policy;
            }
            AuthMode::Password => {
                for user in self.users.values_mut() {
                    if user.role == AccessRole::ReadWrite {
                        user.write_policy = policy.clone();
                    }
                }
            }
            AuthMode::EdgePreauthenticated => self.trust_write_policy = policy,
        }
        self
    }

    pub fn with_read_allowlist(mut self, targets: Vec<WriteTarget>) -> Self {
        self.read_policy = TablePolicy::only(targets);
        self
    }

    pub fn mode(&self) -> AuthMode {
        self.mode
    }

    pub fn user(&self, name: &str) -> Option<&AuthUser> {
        self.users.get(name)
    }

    pub fn users(&self) -> impl Iterator<Item = (&str, &AuthUser)> {
        self.users.iter().map(|(name, user)| (name.as_str(), user))
    }

    pub fn role_catalog(&self) -> Option<&Arc<RoleCatalog>> {
        self.role_catalog.as_ref()
    }

    pub fn start_role_session(&self, name: Option<&str>) -> Result<RoleSessionState> {
        let session_user = match self.mode {
            AuthMode::Trust => name.unwrap_or("postgres"),
            AuthMode::Password => name
                .filter(|name| self.users.contains_key(*name))
                .ok_or_else(|| {
                    anyhow!("authenticated user is missing from password configuration")
                })?,
            AuthMode::EdgePreauthenticated => name
                .filter(|name| self.allows_preauthenticated_login(name))
                .ok_or_else(|| anyhow!("preauthenticated user is not a configured LOGIN role"))?,
        };
        RoleSessionState::new(session_user.to_owned(), self.role_catalog.clone())
    }

    pub fn read_targets(&self) -> Option<&[WriteTarget]> {
        self.read_policy.allowed_targets()
    }

    pub fn read_policy_restricted(&self) -> bool {
        self.read_policy.is_restricted()
    }

    pub fn role_for_user(&self, name: Option<&str>) -> AccessRole {
        match self.mode {
            AuthMode::Trust => AccessRole::ReadWrite,
            AuthMode::Password => name
                .and_then(|name| self.users.get(name))
                .map(|user| user.role)
                .unwrap_or(AccessRole::ReadOnly),
            AuthMode::EdgePreauthenticated => {
                if name.is_some_and(|name| self.allows_preauthenticated_login(name)) {
                    AccessRole::ReadWrite
                } else {
                    AccessRole::ReadOnly
                }
            }
        }
    }

    pub fn role_for_client<C>(&self, client: &C) -> AccessRole
    where
        C: ClientInfo + ?Sized,
    {
        self.role_for_user(client.metadata().get(METADATA_USER).map(String::as_str))
    }

    pub fn allows_write(&self, name: Option<&str>, target: Option<(&str, &str)>) -> bool {
        match self.mode {
            AuthMode::Trust => self.trust_write_policy.allows(target),
            AuthMode::Password => name
                .and_then(|name| self.users.get(name))
                .is_some_and(|user| {
                    user.role == AccessRole::ReadWrite && user.write_policy.allows(target)
                }),
            AuthMode::EdgePreauthenticated => {
                name.is_some_and(|name| self.allows_preauthenticated_login(name))
                    && self.trust_write_policy.allows(target)
            }
        }
    }

    pub fn allows_read(&self, name: Option<&str>, target: (&str, &str)) -> bool {
        let known_identity = match self.mode {
            AuthMode::Trust => true,
            AuthMode::Password => name.is_some_and(|name| self.users.contains_key(name)),
            AuthMode::EdgePreauthenticated => {
                name.is_some_and(|name| self.allows_preauthenticated_login(name))
            }
        };
        known_identity && self.read_policy.allows(Some(target))
    }

    pub fn allows_preauthenticated_login(&self, name: &str) -> bool {
        self.mode == AuthMode::EdgePreauthenticated
            && self
                .role_catalog
                .as_ref()
                .and_then(|catalog| catalog.role(name))
                .is_some_and(|role| role.login)
    }

    pub fn allows_maintenance(&self, name: Option<&str>, target: (&str, &str)) -> bool {
        self.maintenance_user.as_deref() == name && self.allows_write(name, Some(target))
    }
}

pub fn parse_write_allowlist(raw: &str) -> Result<Vec<WriteTarget>> {
    parse_table_allowlist(raw, "write")
}

pub fn parse_read_allowlist(raw: &str) -> Result<Vec<WriteTarget>> {
    parse_table_allowlist(raw, "read")
}

fn parse_table_allowlist(raw: &str, label: &str) -> Result<Vec<WriteTarget>> {
    let mut targets = Vec::new();
    for entry in raw.split(',') {
        let entry = entry.trim();
        if entry.is_empty() {
            return Err(anyhow!(
                "{label} allowlist entries cannot be empty; use comma-separated table names"
            ));
        }
        targets.push(parse_table_target(entry, label)?);
    }
    if targets.is_empty() {
        return Err(anyhow!("{label} allowlist cannot be empty"));
    }
    targets.sort();
    targets.dedup();
    Ok(targets)
}

pub fn parse_write_target(raw: &str) -> Result<WriteTarget> {
    parse_table_target(raw, "write")
}

fn parse_table_target(raw: &str, label: &str) -> Result<WriteTarget> {
    let parts = raw
        .split('.')
        .map(|part| normalize_identifier_part(part, label))
        .collect::<Result<Vec<_>>>()?;
    let table = match parts.as_slice() {
        [table] => table.clone(),
        [schema, table] if is_public_schema(schema) => table.clone(),
        [catalog, schema, table]
            if catalog.eq_ignore_ascii_case("quackgis") && is_public_schema(schema) =>
        {
            table.clone()
        }
        _ => {
            return Err(anyhow!(
                "{label} allowlist entry {raw:?} must be a DuckLake table name: table, public.table, main.table, or quackgis.main.table"
            ));
        }
    };
    Ok(WriteTarget {
        schema: "main".to_string(),
        table,
    })
}

fn normalize_identifier_part(raw: &str, label: &str) -> Result<String> {
    let part = raw.trim().trim_matches('"');
    if part.is_empty() {
        return Err(anyhow!("{label} allowlist identifiers cannot be empty"));
    }
    if part.chars().any(char::is_control) {
        return Err(anyhow!(
            "{label} allowlist identifiers cannot contain control characters"
        ));
    }
    Ok(part.to_string())
}

fn is_public_schema(schema: &str) -> bool {
    schema.eq_ignore_ascii_case("public") || schema.eq_ignore_ascii_case("main")
}

fn validate_user_password(label: &str, user: &str, password: &str) -> Result<()> {
    if user.trim().is_empty() {
        return Err(anyhow!(
            "{label} user cannot be empty in password auth mode"
        ));
    }
    if user != user.trim() || user.chars().any(char::is_control) {
        return Err(anyhow!(
            "{label} user cannot contain leading/trailing whitespace or control characters in password auth mode"
        ));
    }
    if password.is_empty() {
        return Err(anyhow!(
            "{label} password cannot be empty in password auth mode"
        ));
    }
    if password.contains('\0') {
        return Err(anyhow!(
            "{label} password cannot contain NUL bytes in password auth mode"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_allowlist_normalizes_ducklake_public_names() {
        let targets =
            parse_write_allowlist("allowed, public.allowed, main.other, quackgis.main.third")
                .expect("allowlist parses");
        assert_eq!(
            targets,
            vec![
                WriteTarget {
                    schema: "main".to_string(),
                    table: "allowed".to_string(),
                },
                WriteTarget {
                    schema: "main".to_string(),
                    table: "other".to_string(),
                },
                WriteTarget {
                    schema: "main".to_string(),
                    table: "third".to_string(),
                },
            ]
        );
    }

    #[test]
    fn write_allowlist_rejects_ambiguous_or_empty_entries() {
        assert!(parse_write_allowlist("public.ok,").is_err());
        assert!(parse_write_allowlist("other_schema.table").is_err());
        assert!(parse_write_allowlist("catalog.other.table").is_err());
    }

    #[test]
    fn read_allowlist_uses_same_normalized_ducklake_targets() {
        let auth = AuthConfig::password(
            "postgres",
            "readwrite-secret",
            Some(("reader", "reader-secret")),
        )
        .expect("auth config")
        .with_read_allowlist(
            parse_read_allowlist("allowed, public.other").expect("read allowlist parses"),
        );

        assert!(auth.read_policy_restricted());
        assert!(auth.allows_read(Some("postgres"), ("main", "allowed")));
        assert!(auth.allows_read(Some("reader"), ("main", "other")));
        assert!(!auth.allows_read(Some("reader"), ("main", "denied")));
        assert!(!auth.allows_read(Some("missing"), ("main", "allowed")));
    }

    #[test]
    fn maintenance_requires_explicit_readwrite_identity_and_table_policy() {
        let auth = AuthConfig::password(
            "writer",
            "readwrite-secret",
            Some(("reader", "reader-secret")),
        )
        .expect("auth config")
        .with_readwrite_allowlist(parse_write_allowlist("allowed").expect("allowlist"));
        assert!(auth.clone().with_maintenance_user("reader").is_err());
        let auth = auth
            .with_maintenance_user("writer")
            .expect("maintenance identity");
        assert!(auth.allows_maintenance(Some("writer"), ("main", "allowed")));
        assert!(!auth.allows_maintenance(Some("writer"), ("main", "denied")));
        assert!(!auth.allows_maintenance(Some("reader"), ("main", "allowed")));
    }

    #[test]
    fn role_catalog_login_roles_must_exactly_match_password_users() {
        let auth = AuthConfig::password(
            "writer",
            "readwrite-secret",
            Some(("reader", "reader-secret")),
        )
        .expect("auth config");
        let matching = RoleCatalog::from_json(
            r#"{"roles":[{"oid":100001,"name":"reader","login":true},{"oid":100002,"name":"writer","login":true},{"oid":100003,"name":"group"}]}"#,
        )
        .expect("matching role catalog");
        assert!(auth.clone().with_role_catalog(matching).is_ok());

        for mismatched in [
            r#"{"roles":[{"oid":100001,"name":"writer","login":true}]}"#,
            r#"{"roles":[{"oid":100001,"name":"writer","login":true},{"oid":100002,"name":"reader"}]}"#,
            r#"{"roles":[{"oid":100001,"name":"writer","login":true},{"oid":100002,"name":"reader","login":true},{"oid":100003,"name":"extra","login":true}]}"#,
        ] {
            let roles = RoleCatalog::from_json(mismatched).expect("valid role graph");
            assert!(auth.clone().with_role_catalog(roles).is_err());
        }
        let roles =
            RoleCatalog::from_json(r#"{"roles":[{"oid":100001,"name":"postgres","login":true}]}"#)
                .expect("trust role graph");
        assert!(AuthConfig::trust().with_role_catalog(roles).is_err());
    }
}
