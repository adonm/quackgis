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

use datafusion::prelude::SessionContext;
use datafusion_postgres::{ServerOptions, serve};

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

    // Register only the pure-Rust SedonaDB function surface needed by the
    // spike: base geometry functions plus sedona-geo kernels.
    let ctx = SessionContext::new();
    register_sedona_function_catalog(&ctx)?;
    let ctx = Arc::new(ctx);

    // Note: deliberately NOT calling setup_pg_catalog here — v0.15 needs an
    // AuthManager/ContextProvider and the spike only validates the wire path
    // + function execution, not QGIS introspection. That is M3.

    let opts = ServerOptions::new()
        .with_host(HOST.to_string())
        .with_port(port);

    eprintln!("quackgis wire spike: listening on {HOST}:{port}  (no auth, no TLS)");
    eprintln!(
        "try:  psql -h 127.0.0.1 -p {port} -U postgres -c \"SELECT ST_AsText(ST_GeomFromText('POINT(1 2)'))\""
    );

    serve(ctx, &opts).await?;
    Ok(())
}

fn register_sedona_function_catalog(ctx: &SessionContext) -> datafusion::common::Result<()> {
    let mut functions = sedona_functions::register::default_function_set();

    for (name, kernels) in sedona_geo::register::scalar_kernels() {
        functions.add_scalar_udf_impl(name, kernels)?;
    }
    for (name, kernel) in sedona_geo::register::aggregate_kernels() {
        functions.add_aggregate_udf_kernel(name, kernel)?;
    }

    for udf in functions.scalar_udfs() {
        ctx.register_udf(udf.clone().into());
    }
    for udaf in functions.aggregate_udfs() {
        ctx.register_udaf(udaf.clone().into());
    }

    Ok(())
}
