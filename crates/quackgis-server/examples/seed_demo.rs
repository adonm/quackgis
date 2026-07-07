// SPDX-License-Identifier: Apache-2.0
//! Seed stable demo layers in a running local QuackGIS server.

use anyhow::{Context, Result, bail};
use tokio_postgres::{NoTls, SimpleQueryMessage};

const ORIGIN_WKB: &str = "010100000000000000000000000000000000000000";
const ONE_WKB: &str = "0101000000000000000000F03F000000000000F03F";

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

    seed(&client).await?;
    let point_rows = query_rows(
        &client,
        "SELECT id, name, ST_AsText(ST_GeomFromWKB(geom)) AS wkt \
         FROM public.demo_points ORDER BY id",
    )
    .await?;
    let polygon_rows = query_rows(
        &client,
        "SELECT id, name, ST_AsText(ST_GeomFromWKB(geom)) AS wkt \
         FROM public.demo_polygons ORDER BY id",
    )
    .await?;

    let ok = point_rows
        == vec![
            (
                "1".to_string(),
                "origin".to_string(),
                "POINT(0 0)".to_string(),
            ),
            ("2".to_string(), "one".to_string(), "POINT(1 1)".to_string()),
        ]
        && polygon_rows.len() == 2;

    println!("demo_tables ['public.demo_points', 'public.demo_polygons']");
    println!("demo_points {point_rows:?}");
    println!("demo_polygons {polygon_rows:?}");
    println!(
        "qgis_connection host={} port={} dbname=quackgis user=postgres tables=public.demo_points,public.demo_polygons",
        config.host, config.port
    );
    println!(
        "ogr_points ogrinfo 'PG:host={} port={} user=postgres dbname=quackgis' demo_points -so",
        config.host, config.port
    );
    println!(
        "sample_sql SELECT name, ST_AsText(ST_GeomFromWKB(geom)) FROM public.demo_points ORDER BY id;"
    );
    println!("demo_ok {}", if ok { "True" } else { "False" });
    if !ok {
        bail!("demo seed verification failed");
    }
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
                        "Usage: cargo run -p quackgis-server --example seed_demo -- [--host HOST] [--port PORT]"
                    );
                    std::process::exit(0);
                }
                other => bail!("unknown argument: {other}"),
            }
        }
        Ok(Self { host, port })
    }
}

async fn seed(client: &tokio_postgres::Client) -> Result<()> {
    reset_table(client, "demo_points").await?;
    client
        .simple_query(&format!(
            "INSERT INTO public.{} VALUES \
             (1, X'{ORIGIN_WKB}', 'origin'), \
             (2, X'{ONE_WKB}', 'one')",
            quote_ident("demo_points")
        ))
        .await
        .context("insert demo points")?;

    reset_table(client, "demo_polygons").await?;
    let square = polygon_wkb_hex(&[(0.0, 0.0), (2.0, 0.0), (2.0, 2.0), (0.0, 2.0)]);
    let triangle = polygon_wkb_hex(&[(3.0, 0.0), (5.0, 0.0), (4.0, 2.0)]);
    client
        .simple_query(&format!(
            "INSERT INTO public.{} VALUES \
             (1, X'{square}', 'square'), \
             (2, X'{triangle}', 'triangle')",
            quote_ident("demo_polygons")
        ))
        .await
        .context("insert demo polygons")?;
    Ok(())
}

async fn reset_table(client: &tokio_postgres::Client, table: &str) -> Result<()> {
    let table_ref = format!("public.{}", quote_ident(table));
    if client
        .simple_query(&format!("DELETE FROM {table_ref}"))
        .await
        .is_err()
    {
        client
            .simple_query(&format!(
                "CREATE TABLE {table_ref} (id INT, geom BINARY, name TEXT)"
            ))
            .await
            .with_context(|| format!("create {table_ref}"))?;
    }
    Ok(())
}

async fn query_rows(
    client: &tokio_postgres::Client,
    sql: &str,
) -> Result<Vec<(String, String, String)>> {
    let messages = client.simple_query(sql).await.context("query demo rows")?;
    let rows = messages
        .into_iter()
        .filter_map(|message| match message {
            SimpleQueryMessage::Row(row) => Some((
                row.get("id").unwrap_or_default().to_string(),
                row.get("name").unwrap_or_default().to_string(),
                row.get("wkt").unwrap_or_default().to_string(),
            )),
            _ => None,
        })
        .collect();
    Ok(rows)
}

fn polygon_wkb_hex(coords: &[(f64, f64)]) -> String {
    let mut ring = coords.to_vec();
    if ring.first() != ring.last() {
        ring.push(ring[0]);
    }
    let mut bytes = Vec::with_capacity(1 + 4 + 4 + 4 + ring.len() * 16);
    bytes.push(1); // little endian
    bytes.extend_from_slice(&3_u32.to_le_bytes()); // Polygon
    bytes.extend_from_slice(&1_u32.to_le_bytes()); // one ring
    bytes.extend_from_slice(&(ring.len() as u32).to_le_bytes());
    for (x, y) in ring {
        bytes.extend_from_slice(&x.to_le_bytes());
        bytes.extend_from_slice(&y.to_le_bytes());
    }
    hex_encode(&bytes)
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn quote_ident(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}
