// SPDX-License-Identifier: Apache-2.0
//! QuackGIS pgwire authentication and coarse role configuration.

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

#[derive(Clone)]
pub struct AuthUser {
    pub role: AccessRole,
    pub(crate) scram_salt: Vec<u8>,
    pub(crate) scram_salted_password: Vec<u8>,
}

impl fmt::Debug for AuthUser {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthUser")
            .field("role", &self.role)
            .field("scram_salt_len", &self.scram_salt.len())
            .field("scram_salted_password", &"<redacted>")
            .finish()
    }
}

impl AuthUser {
    fn scram(password: &str, role: AccessRole) -> Self {
        let scram_salt = random_nonce().into_bytes();
        let scram_salted_password = gen_salted_password(password, &scram_salt, SCRAM_ITERATIONS);
        Self {
            role,
            scram_salt,
            scram_salted_password,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AuthConfig {
    mode: AuthMode,
    users: HashMap<String, AuthUser>,
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
        })
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
