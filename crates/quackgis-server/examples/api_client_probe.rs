// SPDX-License-Identifier: Apache-2.0
//! Local API/client surface probe for Python/API/BI-style compatibility work.
//!
//! This intentionally uses `tokio-postgres` rather than heavy client runtimes. It
//! captures the PostgreSQL wire/catalog/SQL surfaces that psycopg, SQLAlchemy,
//! GeoPandas, pg_featureserv-style readers, BI tools, and MVT consumers need
//! before a containerized client probe is worth maintaining.

use anyhow::{Context, Result, ensure};
use tokio_postgres::{NoTls, types::Type};

#[derive(Debug)]
struct Config {
    host: String,
    port: u16,
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    let config = Config::from_env_and_args()?;
    let conn_str = format!(
        "host={} port={} user=postgres dbname=quackgis",
        config.host, config.port
    );
    let (client, connection) = tokio_postgres::connect(&conn_str, NoTls)
        .await
        .with_context(|| format!("connect to QuackGIS at {}:{}", config.host, config.port))?;
    tokio::spawn(async move {
        if let Err(err) = connection.await {
            eprintln!("postgres connection task failed: {err}");
        }
    });

    seed_probe_table(&client).await?;
    assert_psycopg_surface(&client).await?;
    assert_sqlalchemy_surface(&client).await?;
    assert_geopandas_surface(&client).await?;
    assert_pgfeatureserv_surface(&client).await?;
    assert_bi_surface(&client).await?;
    assert_mvt_surface(&client).await?;

    println!("api_client_probe_ok True");
    Ok(())
}

impl Config {
    fn from_env_and_args() -> Result<Self> {
        let mut host = std::env::var("QUACKGIS_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
        let mut port = std::env::var("QUACKGIS_PORT")
            .ok()
            .and_then(|value| value.parse::<u16>().ok())
            .unwrap_or(5434);
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--host" => host = args.next().context("--host requires a value")?,
                "--port" => {
                    port = args
                        .next()
                        .context("--port requires a value")?
                        .parse()
                        .context("--port must be a u16")?;
                }
                "--help" | "-h" => {
                    println!(
                        "Usage: cargo run -p quackgis-server --example api_client_probe -- [--host HOST] [--port PORT]"
                    );
                    std::process::exit(0);
                }
                other => anyhow::bail!("unknown argument: {other}"),
            }
        }
        Ok(Self { host, port })
    }
}

async fn seed_probe_table(client: &tokio_postgres::Client) -> Result<()> {
    client
        .batch_execute(
            "CREATE TABLE public.api_probe_points (
                 id INT,
                 geom BINARY,
                 name TEXT,
                 category TEXT
             );
             INSERT INTO public.api_probe_points (id, geom, name, category) VALUES
                 (1, X'010100000000000000000000000000000000000000', 'origin', 'alpha'),
                 (2, X'0101000000000000000000F03F000000000000F03F', 'one', 'alpha'),
                 (3, X'010100000000000000000014400000000000001440', 'far', 'beta');",
        )
        .await
        .context("seed api_probe_points")?;
    Ok(())
}

async fn assert_psycopg_surface(client: &tokio_postgres::Client) -> Result<()> {
    let text_param: String = client
        .query_one("SELECT ST_AsText(ST_GeomFromText($1))", &[&"POINT(3 4)"])
        .await
        .context("text parameter WKT roundtrip")?
        .get(0);
    ensure!(text_param == "POINT(3 4)", "text param got {text_param}");

    let binary_stmt = client
        .prepare_typed("SELECT ST_AsText(ST_GeomFromWKB($1))", &[Type::BYTEA])
        .await
        .context("prepare typed bytea parameter")?;
    let binary_param: String = client
        .query_one(&binary_stmt, &[&point_wkb(3.0, 4.0)])
        .await
        .context("binary WKB parameter roundtrip")?
        .get(0);
    ensure!(
        binary_param == "POINT(3 4)",
        "binary param got {binary_param}"
    );

    let ewkb: Vec<u8> = client
        .query_one(
            "SELECT ST_AsEWKB(geom) FROM public.api_probe_points WHERE id = 1",
            &[],
        )
        .await
        .context("ST_AsEWKB readback")?
        .get(0);
    ensure!(ewkb == point_wkb(0.0, 0.0), "EWKB/WKB bytes changed");
    println!("api_client_psycopg_surface text_param=True binary_wkb=True ewkb=True");
    Ok(())
}

async fn assert_sqlalchemy_surface(client: &tokio_postgres::Client) -> Result<()> {
    let table_count: i64 = client
        .query_one(
            "SELECT COUNT(*) FROM information_schema.tables
             WHERE table_schema = 'public' AND table_name = 'api_probe_points'",
            &[],
        )
        .await
        .context("information_schema table reflection")?
        .get(0);
    ensure!(table_count == 1, "table reflection count={table_count}");

    let columns = client
        .query(
            "SELECT column_name FROM information_schema.columns
             WHERE table_schema = 'public' AND table_name = 'api_probe_points'
             ORDER BY ordinal_position",
            &[],
        )
        .await
        .context("information_schema column reflection")?
        .into_iter()
        .map(|row| row.get::<_, String>(0))
        .collect::<Vec<_>>();
    for expected in ["id", "geom", "name", "category"] {
        ensure!(
            columns.iter().any(|column| column == expected),
            "reflected columns {columns:?} missing {expected}"
        );
    }
    ensure!(
        columns.iter().all(|column| !column.starts_with("_qg_")),
        "reflected public columns leaked hidden layout columns: {columns:?}"
    );
    println!("api_client_sqlalchemy_surface table=True columns={columns:?}");
    Ok(())
}

async fn assert_geopandas_surface(client: &tokio_postgres::Client) -> Result<()> {
    let rows = client
        .query(
            "SELECT id, name, ST_AsEWKB(geom) AS geom
             FROM public.api_probe_points
             ORDER BY id",
            &[],
        )
        .await
        .context("GeoPandas-style WKB row query")?;
    ensure!(rows.len() == 3, "GeoPandas row count={}", rows.len());
    let first_geom: Vec<u8> = rows[0].get(2);
    ensure!(first_geom == point_wkb(0.0, 0.0), "first WKB changed");
    println!("api_client_geopandas_surface feature_count=3 wkb=True crs_documented=srid0");
    Ok(())
}

async fn assert_pgfeatureserv_surface(client: &tokio_postgres::Client) -> Result<()> {
    let rows = client
        .query(
            "SELECT id, name
             FROM public.api_probe_points
             WHERE ST_Intersects(
                 ST_GeomFromWKB(geom),
                 ST_GeomFromWKB(ST_MakeEnvelope(-0.5, -0.5, 0.5, 0.5, 3857))
             )
             ORDER BY id",
            &[],
        )
        .await
        .context("pg_featureserv-style bbox query")?;
    let ids = rows
        .into_iter()
        .map(|row| row.get::<_, i32>(0))
        .collect::<Vec<_>>();
    ensure!(ids == vec![1], "bbox ids={ids:?}");
    println!("api_client_pgfeatureserv_surface bbox_count=1 properties=True");
    Ok(())
}

async fn assert_bi_surface(client: &tokio_postgres::Client) -> Result<()> {
    let grouped = client
        .query(
            "SELECT category, COUNT(*) AS n
             FROM public.api_probe_points
             GROUP BY category
             ORDER BY category",
            &[],
        )
        .await
        .context("BI grouped aggregate query")?
        .into_iter()
        .map(|row| (row.get::<_, String>(0), row.get::<_, i64>(1)))
        .collect::<Vec<_>>();
    ensure!(
        grouped == vec![("alpha".to_string(), 2), ("beta".to_string(), 1)],
        "grouped aggregate got {grouped:?}"
    );
    println!("api_client_bi_surface grouped={grouped:?}");
    Ok(())
}

async fn assert_mvt_surface(client: &tokio_postgres::Client) -> Result<()> {
    let tile: Vec<u8> = client
        .query_one(
            "SELECT ST_AsMVT(
                  ST_AsMVTGeom(
                   geom,
                   ST_MakeEnvelope(-1.0, -1.0, 2.0, 2.0, 3857),
                   4096,
                   64,
                    true
                  ),
                  'api_probe_points',
                  4096,
                  name,
                  category
               )
               FROM public.api_probe_points
               WHERE id <= 2",
            &[],
        )
        .await
        .context("MVT tile query")?
        .get(0);
    ensure!(!tile.is_empty(), "MVT tile was empty");
    for expected in ["api_probe_points", "name", "origin", "category", "alpha"] {
        ensure!(
            contains_bytes(&tile, expected.as_bytes()),
            "MVT tile missing expected attribute/layer token {expected:?}"
        );
    }
    println!(
        "api_client_mvt_surface tile_bytes={} attributes=True",
        tile.len()
    );
    Ok(())
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn point_wkb(x: f64, y: f64) -> Vec<u8> {
    let mut out = Vec::with_capacity(21);
    out.push(1);
    out.extend_from_slice(&1_u32.to_le_bytes());
    out.extend_from_slice(&x.to_le_bytes());
    out.extend_from_slice(&y.to_le_bytes());
    out
}
