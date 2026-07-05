// SPDX-License-Identifier: Apache-2.0
mod common;
use common::ServerHandle;
use std::process::Stdio;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio_postgres::NoTls;

fn port_from_conn_str(s: &str) -> u16 {
    s.split_whitespace()
        .find_map(|p| p.strip_prefix("port="))
        .unwrap()
        .parse()
        .unwrap()
}

async fn free_port() -> u16 {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    port
}

async fn http_get(port: u16, path: &str) -> anyhow::Result<(u16, Vec<u8>, String)> {
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port)).await?;
    let req = format!(
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAccept: */*\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(req.as_bytes()).await?;
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await?;
    let text = String::from_utf8_lossy(&buf);
    let status = text
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(0);
    let split = buf
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|i| i + 4)
        .unwrap_or(buf.len());
    let headers = String::from_utf8_lossy(&buf[..split]).to_string();
    Ok((status, buf[split..].to_vec(), headers))
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires MARTIN_BIN or .tmp/bin/martin"]
async fn real_martin_serves_tile_from_quackgis() {
    let martin_bin = std::env::var("MARTIN_BIN").unwrap_or_else(|_| ".tmp/bin/martin".to_string());
    if !std::path::Path::new(&martin_bin).exists() {
        eprintln!("skipping: {martin_bin} not found");
        return;
    }

    let server = ServerHandle::start().await;
    let conn_str = server.conn_str();
    let (client, connection) = tokio_postgres::connect(&conn_str, NoTls).await.unwrap();
    let _conn = tokio::spawn(connection);

    // Use the default in-memory catalog for the real Martin binary E2E because
    // Martin's explicit table config addresses sources as schema.table, while
    // persisted DuckLake tables currently live at quackgis.main.table. The SQL
    // functions exercised by Martin are the same; DuckLake persistence has its
    // own integration coverage.
    client
        .simple_query(
            "CREATE TABLE points AS SELECT \
             1 AS id, \
             X'010100000000000000000000000000000000000000' AS geom, \
             'origin' AS name",
        )
        .await
        .unwrap();

    let pg_port = port_from_conn_str(&conn_str);
    let martin_port = free_port().await;
    let pg_url = format!("postgres://postgres@127.0.0.1:{pg_port}/quackgis");
    let config_path = server.tmp_dir().join("martin.yaml");
    std::fs::write(
        &config_path,
        format!(
            r#"---
listen_addresses: '127.0.0.1:{martin_port}'
postgres:
  connection_string: {pg_url}
  default_srid: 3857
  auto_publish: false
  tables:
    points:
      schema: public
      table: points
      srid: 3857
      geometry_column: geom
      geometry_type: POINT
      bounds: [-180.0, -85.0511, 180.0, 85.0511]
      properties: {{}}
"#,
        ),
    )
    .unwrap();

    let mut child = Command::new(&martin_bin)
        .arg("--config")
        .arg(&config_path)
        .arg("--on-invalid")
        .arg("warn")
        .env("RUST_LOG", "debug")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let mut last = None;
    for _ in 0..100 {
        if let Some(status) = child.try_wait().expect("poll martin") {
            let mut stdout = String::new();
            let mut stderr = String::new();
            if let Some(mut s) = child.stdout.take() {
                let _ = s.read_to_string(&mut stdout).await;
            }
            if let Some(mut s) = child.stderr.take() {
                let _ = s.read_to_string(&mut stderr).await;
            }
            panic!("martin exited early: {status}; stdout={stdout}; stderr={stderr}");
        }
        if let Ok(r) = http_get(martin_port, "/catalog").await {
            last = Some(r);
            if last.as_ref().unwrap().0 == 200 {
                break;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    let Some(catalog) = last else {
        let _ = child.kill().await;
        panic!("martin did not respond on 127.0.0.1:{martin_port}");
    };
    println!(
        "catalog status={} headers={} body={}",
        catalog.0,
        catalog.2,
        String::from_utf8_lossy(&catalog.1)
    );

    let candidates = [
        "/points/0/0/0",
        "/points/0/0/0.pbf",
        "/main.points/0/0/0",
        "/main.points/0/0/0.pbf",
        "/main.points.geom/0/0/0",
        "/main.points.geom/0/0/0.pbf",
    ];
    let mut ok = None;
    for path in candidates {
        let r = http_get(martin_port, path).await.unwrap();
        println!(
            "GET {path} -> status={} len={} headers={}",
            r.0,
            r.1.len(),
            r.2
        );
        if r.0 >= 400 {
            println!("GET {path} body={}", String::from_utf8_lossy(&r.1));
        }
        if r.0 == 200 && !r.1.is_empty() {
            ok = Some((path.to_string(), r));
            break;
        }
    }

    let _ = child.kill().await;
    let mut stdout = String::new();
    let mut stderr = String::new();
    if let Some(mut s) = child.stdout.take() {
        let _ = s.read_to_string(&mut stdout).await;
    }
    if let Some(mut s) = child.stderr.take() {
        let _ = s.read_to_string(&mut stderr).await;
    }
    let Some((path, (_status, body, _headers))) = ok else {
        panic!("no tile endpoint returned 200; stdout={stdout}; stderr={stderr}");
    };
    println!("tile path {path}, {} bytes", body.len());
}
