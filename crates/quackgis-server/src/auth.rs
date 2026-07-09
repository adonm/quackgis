// SPDX-License-Identifier: Apache-2.0
//! QuackGIS pgwire authentication and coarse role configuration.

use std::collections::HashMap;

use anyhow::{Result, anyhow};
use datafusion_postgres::pgwire::api::ClientInfo;

const METADATA_USER: &str = "user";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMode {
    Trust,
    Password,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessRole {
    ReadWrite,
    ReadOnly,
}

#[derive(Debug, Clone)]
pub struct AuthUser {
    pub password: String,
    pub role: AccessRole,
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
            AuthUser {
                password: readwrite_password,
                role: AccessRole::ReadWrite,
            },
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
                AuthUser {
                    password: readonly_password,
                    role: AccessRole::ReadOnly,
                },
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
    if password.is_empty() {
        return Err(anyhow!(
            "{label} password cannot be empty in password auth mode"
        ));
    }
    Ok(())
}
