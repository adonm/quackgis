// M0 wire spike — validate psql <-> datafusion-postgres <-> SedonaDB.
//
// Goal: `psql -c "SELECT ST_AsText(ST_GeomFromText('POINT(1 2)'))"` returns
// `POINT(1 2)`. If this works, the redesign's wire assumption holds and M0 can
// proceed to retiring v0.1. See README.md for scope.
//
// Tracks upstream master for both pillars:
// - datafusion-postgres master (DataFusion 53)
// - sedona from adonm/sedona-db@quackgis/df53 (SedonaDB bumped to DF 53 to
//   align — fork commit f274c942, upstream PR candidate).

use std::sync::Arc;

use datafusion_postgres::{serve, ServerOptions};
use sedona::context::SedonaContext;

const HOST: &str = "0.0.0.0";

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .write_style(env_logger::WriteStyle::Auto)
        .init();

    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(5434);

    // SedonaContext wraps a DataFusion SessionContext and registers the full
    // SedonaDB function catalog (ST_*, CRS, spatial joins) plus information_schema
    // and the local filesystem object store.
    let sctx = SedonaContext::new_local_interactive().await?;
    let ctx = Arc::new(sctx.ctx);

    // Note: deliberately NOT calling setup_pg_catalog here — v0.15 needs an
    // AuthManager/ContextProvider and the spike only validates the wire path
    // + function execution, not QGIS introspection. That is M3.

    let opts = ServerOptions::new()
        .with_host(HOST.to_string())
        .with_port(port);

    eprintln!(
        "quackgis wire spike: listening on {HOST}:{port}  (no auth, no TLS)"
    );
    eprintln!(
        "try:  psql -h 127.0.0.1 -p {port} -U postgres -c \"SELECT ST_AsText(ST_GeomFromText('POINT(1 2)'))\""
    );

    serve(ctx, &opts).await?;
    Ok(())
}
