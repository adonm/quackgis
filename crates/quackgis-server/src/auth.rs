// SPDX-License-Identifier: Apache-2.0
//! QuackGIS pgwire authentication and coarse role/write-policy configuration.

use std::{collections::HashMap, fmt};

use anyhow::{Result, anyhow};
use datafusion_postgres::pgwire::api::ClientInfo;
use datafusion_postgres::pgwire::api::auth::sasl::scram::{
    SCRAM_ITERATIONS, gen_salted_password, random_nonce,
};

const METADATA_USER: &str = "user";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AuthMode {
    Trust,
    Password,
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
enum WritePolicy {
    #[default]
    All,
    Only(Vec<WriteTarget>),
}

impl fmt::Debug for WritePolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::All => f.write_str("All"),
            Self::Only(targets) => f.debug_tuple("Only").field(targets).finish(),
        }
    }
}

impl WritePolicy {
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
}

#[derive(Clone)]
pub struct AuthUser {
    pub role: AccessRole,
    write_policy: WritePolicy,
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
        Self::scram_with_policy(password, role, WritePolicy::All)
    }

    fn scram_with_policy(password: &str, role: AccessRole, write_policy: WritePolicy) -> Self {
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
    trust_write_policy: WritePolicy,
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
            trust_write_policy: WritePolicy::All,
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
            trust_write_policy: WritePolicy::All,
        })
    }

    pub fn with_readwrite_allowlist(mut self, targets: Vec<WriteTarget>) -> Self {
        let policy = WritePolicy::only(targets);
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
        }
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

    pub fn role_for_user(&self, name: Option<&str>) -> AccessRole {
        match self.mode {
            AuthMode::Trust => AccessRole::ReadWrite,
            AuthMode::Password => name
                .and_then(|name| self.users.get(name))
                .map(|user| user.role)
                .unwrap_or(AccessRole::ReadOnly),
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
        }
    }
}

pub fn parse_write_allowlist(raw: &str) -> Result<Vec<WriteTarget>> {
    let mut targets = Vec::new();
    for entry in raw.split(',') {
        let entry = entry.trim();
        if entry.is_empty() {
            return Err(anyhow!(
                "write allowlist entries cannot be empty; use comma-separated table names"
            ));
        }
        targets.push(parse_write_target(entry)?);
    }
    if targets.is_empty() {
        return Err(anyhow!("write allowlist cannot be empty"));
    }
    targets.sort();
    targets.dedup();
    Ok(targets)
}

pub fn parse_write_target(raw: &str) -> Result<WriteTarget> {
    let parts = raw
        .split('.')
        .map(normalize_identifier_part)
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
                "write allowlist entry {raw:?} must be a DuckLake table name: table, public.table, main.table, or quackgis.main.table"
            ));
        }
    };
    Ok(WriteTarget {
        schema: "main".to_string(),
        table,
    })
}

fn normalize_identifier_part(raw: &str) -> Result<String> {
    let part = raw.trim().trim_matches('"');
    if part.is_empty() {
        return Err(anyhow!("write allowlist identifiers cannot be empty"));
    }
    if part.chars().any(char::is_control) {
        return Err(anyhow!(
            "write allowlist identifiers cannot contain control characters"
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
}
