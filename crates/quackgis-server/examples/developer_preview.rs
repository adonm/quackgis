// SPDX-License-Identifier: Apache-2.0
//! Developer-preview smoke against a running QuackGIS server.
//!
//! Exercises the coherent preview path over pgwire: CREATE TABLE, COPY FROM
//! STDIN, spatial query, explicit compaction, and read-back after compaction.

use std::io::Cursor;
use std::pin::pin;

use anyhow::{Context, Result, bail};
use futures::SinkExt;
use tokio_postgres::NoTls;

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

    client
        .batch_execute(
            "DROP TABLE IF EXISTS public.preview_points;
             CREATE TABLE public.preview_points (id INT, name TEXT, geom BINARY);",
        )
        .await
        .context("create preview table")?;

    let copy_data = format!(
        "1\torigin\t{}\n2\tone\t{}\n3\ttwo\t{}\n",
        copy_bytea_hex(&point_wkb(0.0, 0.0)),
        copy_bytea_hex(&point_wkb(1.0, 1.0)),
        copy_bytea_hex(&point_wkb(2.0, 2.0)),
    );
    let sink = client
        .copy_in::<_, Cursor<Vec<u8>>>("COPY public.preview_points (id, name, geom) FROM STDIN")
        .await
        .context("enter COPY FROM STDIN")?;
    let mut sink = pin!(sink);
    sink.as_mut()
        .send(Cursor::new(copy_data.into_bytes()))
        .await
        .context("send preview COPY data")?;
    let copied = sink.as_mut().finish().await.context("finish COPY")?;
    if copied != 3 {
        bail!("expected COPY 3, got COPY {copied}");
    }

    let before = preview_rows(&client).await?;
    client
        .batch_execute("CALL quackgis_compact_table('public.preview_points')")
        .await
        .context("compact preview table")?;
    let after = preview_rows(&client).await?;

    let ok = before == after
        && after
            == vec![
                (
                    "1".to_string(),
                    "origin".to_string(),
                    "POINT(0 0)".to_string(),
                ),
                ("2".to_string(), "one".to_string(), "POINT(1 1)".to_string()),
                ("3".to_string(), "two".to_string(), "POINT(2 2)".to_string()),
            ];

    println!("preview_table public.preview_points");
    println!("preview_copy_rows {copied}");
    println!("preview_rows {after:?}");
    println!(
        "preview_connection host={} port={} dbname=quackgis user=postgres table=public.preview_points",
        config.host, config.port
    );
    println!("developer_preview_ok {}", if ok { "True" } else { "False" });
    if !ok {
        bail!("developer preview smoke failed");
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
                        "Usage: cargo run -p quackgis-server --example developer_preview -- [--host HOST] [--port PORT]"
                    );
                    std::process::exit(0);
                }
                other => bail!("unknown argument: {other}"),
            }
        }
        Ok(Self { host, port })
    }
}

async fn preview_rows(client: &tokio_postgres::Client) -> Result<Vec<(String, String, String)>> {
    let rows = client
        .query(
            "SELECT id::TEXT AS preview_id, name AS preview_name, ST_AsText(ST_GeomFromWKB(geom)) AS wkt \
             FROM public.preview_points ORDER BY id",
            &[],
        )
        .await
        .context("query preview rows")?;
    Ok(rows
        .into_iter()
        .map(|row| (row.get(0), row.get(1), row.get(2)))
        .collect())
}

fn point_wkb(x: f64, y: f64) -> Vec<u8> {
    let mut out = Vec::with_capacity(1 + 4 + 8 + 8);
    out.push(1);
    out.extend_from_slice(&1_u32.to_le_bytes());
    out.extend_from_slice(&x.to_le_bytes());
    out.extend_from_slice(&y.to_le_bytes());
    out
}

fn copy_bytea_hex(bytes: &[u8]) -> String {
    format!("\\\\x{}", hex_encode(bytes))
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
