// SPDX-License-Identifier: Apache-2.0
//! QuackGIS pgwire handler assembly.
//!
//! datafusion-postgres exposes query hooks, but COPY FROM STDIN also needs a
//! pgwire COPY sub-protocol handler. This module keeps the binary and tests on
//! the same handler stack.

use std::sync::Arc;

use async_trait::async_trait;
use datafusion::prelude::SessionContext;
use datafusion_postgres::hooks::QueryHook;
use datafusion_postgres::hooks::cursor::CursorStatementHook;
use datafusion_postgres::hooks::set_show::SetShowHook;
use datafusion_postgres::hooks::transactions::TransactionStatementHook;
use datafusion_postgres::pgwire::api::auth::cleartext::CleartextPasswordAuthStartupHandler;
use datafusion_postgres::pgwire::api::auth::noop::NoopStartupHandler;
use datafusion_postgres::pgwire::api::auth::{
    AuthSource, DefaultServerParameterProvider, LoginInfo, Password, StartupHandler,
};
use datafusion_postgres::pgwire::api::cancel::{CancelHandler, DefaultCancelHandler};
use datafusion_postgres::pgwire::api::copy::CopyHandler;
use datafusion_postgres::pgwire::api::query::{ExtendedQueryHandler, SimpleQueryHandler};
use datafusion_postgres::pgwire::api::{
    ClientInfo, ConnectionManager, ErrorHandler, PgWireServerHandlers,
};
use datafusion_postgres::{DfSessionService, ServerOptions, serve_with_handlers};

use crate::auth::{AuthConfig, AuthMode};
use crate::catalog_compat::CatalogCompatHook;
use crate::context::StoragePaths;
use crate::ducklake_sql::{DuckLakeCopyHandler, DuckLakeSqlHook};

pub async fn serve(
    session_context: Arc<SessionContext>,
    opts: &ServerOptions,
    storage_paths: StoragePaths,
) -> Result<(), std::io::Error> {
    serve_with_auth(session_context, opts, storage_paths, AuthConfig::trust()).await
}

pub async fn serve_with_auth(
    session_context: Arc<SessionContext>,
    opts: &ServerOptions,
    storage_paths: StoragePaths,
    auth: AuthConfig,
) -> Result<(), std::io::Error> {
    let factory = Arc::new(QuackGisHandlerFactory::new(
        session_context,
        storage_paths,
        auth,
    ));
    serve_with_handlers(factory, opts).await
}

struct QuackGisHandlerFactory {
    session_service: Arc<DfSessionService>,
    cancel_handler: Arc<DefaultCancelHandler>,
    startup_handler: Arc<QuackGisStartupHandler>,
    copy_handler: Arc<DuckLakeCopyHandler>,
}

impl QuackGisHandlerFactory {
    fn new(
        session_context: Arc<SessionContext>,
        storage_paths: StoragePaths,
        auth: AuthConfig,
    ) -> Self {
        let ducklake_hook = Arc::new(DuckLakeSqlHook::new_with_auth(
            storage_paths.clone(),
            auth.clone(),
        ));
        let hooks: Vec<Arc<dyn QueryHook>> = vec![
            ducklake_hook,
            Arc::new(CatalogCompatHook),
            Arc::new(CursorStatementHook),
            Arc::new(SetShowHook),
            Arc::new(TransactionStatementHook),
        ];
        let session_service = Arc::new(DfSessionService::new_with_hooks(
            Arc::clone(&session_context),
            hooks,
        ));
        let connection_manager = Arc::new(ConnectionManager::new());
        let startup_handler = match auth.mode() {
            AuthMode::Trust => QuackGisStartupHandler::Trust(SimpleStartupHandler {
                connection_manager: Arc::clone(&connection_manager),
            }),
            AuthMode::Password => {
                let handler = CleartextPasswordAuthStartupHandler::new(
                    StaticPasswordAuthSource { auth: auth.clone() },
                    DefaultServerParameterProvider::default(),
                )
                .with_connection_manager(Arc::clone(&connection_manager));
                QuackGisStartupHandler::Password(Box::new(handler))
            }
        };
        Self {
            session_service,
            cancel_handler: Arc::new(DefaultCancelHandler::new(Arc::clone(&connection_manager))),
            startup_handler: Arc::new(startup_handler),
            copy_handler: Arc::new(DuckLakeCopyHandler::new_with_auth(
                storage_paths,
                session_context,
                auth,
            )),
        }
    }
}

#[derive(Debug, Clone)]
struct StaticPasswordAuthSource {
    auth: AuthConfig,
}

#[async_trait]
impl AuthSource for StaticPasswordAuthSource {
    async fn get_password(
        &self,
        login: &LoginInfo,
    ) -> datafusion_postgres::pgwire::error::PgWireResult<Password> {
        let Some(username) = login.user() else {
            return Err(
                datafusion_postgres::pgwire::error::PgWireError::InvalidPassword(String::new()),
            );
        };
        let Some(user) = self.auth.user(username) else {
            return Err(
                datafusion_postgres::pgwire::error::PgWireError::InvalidPassword(
                    username.to_string(),
                ),
            );
        };
        Ok(Password::new(None, user.password.as_bytes().to_vec()))
    }
}

enum QuackGisStartupHandler {
    Trust(SimpleStartupHandler),
    Password(
        Box<
            CleartextPasswordAuthStartupHandler<
                StaticPasswordAuthSource,
                DefaultServerParameterProvider,
            >,
        >,
    ),
}

#[async_trait]
impl StartupHandler for QuackGisStartupHandler {
    async fn on_startup<C>(
        &self,
        client: &mut C,
        message: datafusion_postgres::pgwire::messages::PgWireFrontendMessage,
    ) -> datafusion_postgres::pgwire::error::PgWireResult<()>
    where
        C: ClientInfo
            + futures::Sink<datafusion_postgres::pgwire::messages::PgWireBackendMessage>
            + Unpin
            + Send
            + Sync,
        C::Error: std::fmt::Debug,
        datafusion_postgres::pgwire::error::PgWireError:
            From<<C as futures::Sink<datafusion_postgres::pgwire::messages::PgWireBackendMessage>>::Error>,
    {
        match self {
            Self::Trust(handler) => handler.on_startup(client, message).await,
            Self::Password(handler) => handler.on_startup(client, message).await,
        }
    }
}

impl PgWireServerHandlers for QuackGisHandlerFactory {
    fn simple_query_handler(&self) -> Arc<impl SimpleQueryHandler> {
        Arc::clone(&self.session_service)
    }

    fn extended_query_handler(&self) -> Arc<impl ExtendedQueryHandler> {
        Arc::clone(&self.session_service)
    }

    fn startup_handler(&self) -> Arc<impl StartupHandler> {
        Arc::clone(&self.startup_handler)
    }

    fn copy_handler(&self) -> Arc<impl CopyHandler> {
        Arc::clone(&self.copy_handler)
    }

    fn error_handler(&self) -> Arc<impl ErrorHandler> {
        Arc::new(LoggingErrorHandler)
    }

    fn cancel_handler(&self) -> Arc<impl CancelHandler> {
        Arc::clone(&self.cancel_handler)
    }
}

struct SimpleStartupHandler {
    connection_manager: Arc<ConnectionManager>,
}

#[async_trait]
impl NoopStartupHandler for SimpleStartupHandler {
    fn connection_manager(&self) -> Option<Arc<ConnectionManager>> {
        Some(Arc::clone(&self.connection_manager))
    }
}

struct LoggingErrorHandler;

impl ErrorHandler for LoggingErrorHandler {
    fn on_error<C>(&self, _client: &C, error: &mut datafusion_postgres::pgwire::error::PgWireError)
    where
        C: ClientInfo,
    {
        log::info!("Sending error: {error}");
    }
}
