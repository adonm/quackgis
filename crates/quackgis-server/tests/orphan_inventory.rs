// SPDX-License-Identifier: Apache-2.0
//! Safe operator-visible orphan inventory over the maintained local profile.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use chrono::{Duration, Utc};
use tokio_postgres::NoTls;

use common::ServerHandle;

#[tokio::test(flavor = "multi_thread")]
async fn dry_run_reports_only_unreferenced_old_parquet() {
    let server = ServerHandle::start().await;
    let (client, connection) = tokio_postgres::connect(&server.conn_str(), NoTls)
        .await
        .expect("connect");
    let _connection = tokio::spawn(connection);
    client
        .batch_execute(
            "CREATE TABLE public.orphan_inventory_points (id INT, geom BINARY);
             INSERT INTO public.orphan_inventory_points VALUES
               (1, X'010100000000000000000000000000000000000000');",
        )
        .await
        .expect("seed referenced Parquet file");

    let data_path = server.tmp_dir().join("data");
    let orphan_dir = data_path.join("manual-prewrite");
    fs::create_dir_all(&orphan_dir).expect("orphan fixture directory");
    let orphan = orphan_dir.join("aborted-write.parquet");
    let ignored = orphan_dir.join("README.txt");
    fs::write(&orphan, b"uncommitted parquet candidate").expect("orphan fixture");
    fs::write(&ignored, b"not parquet").expect("ignored fixture");

    let paths = server.storage_paths();
    let candidates = paths
        .orphan_candidates_before(Utc::now() + Duration::minutes(1))
        .await
        .expect("dry-run orphan inventory");
    assert_eq!(candidates.len(), 1, "only the stray Parquet is a candidate");
    assert!(candidates[0].ends_with("manual-prewrite/aborted-write.parquet"));
    assert!(
        orphan.exists(),
        "dry-run inventory must not delete candidates"
    );
    assert!(ignored.exists(), "non-Parquet files are outside the sweep");

    let recent_candidates = paths
        .orphan_candidates_before(Utc::now() - Duration::hours(1))
        .await
        .expect("age-gated orphan inventory");
    assert!(
        recent_candidates.is_empty(),
        "the mandatory age cutoff must exclude recent prewrites"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn quarantine_requires_explicit_apply_and_stays_outside_live_prefix() {
    let server = ServerHandle::start().await;
    let (client, connection) = tokio_postgres::connect(&server.conn_str(), NoTls)
        .await
        .expect("connect");
    let _connection = tokio::spawn(connection);
    client
        .batch_execute(
            "CREATE TABLE public.orphan_quarantine_points (id INT, geom BINARY);
             INSERT INTO public.orphan_quarantine_points VALUES
               (1, X'010100000000000000000000000000000000000000');",
        )
        .await
        .expect("seed referenced Parquet file");

    let data_path = server.tmp_dir().join("data");
    let referenced_before = parquet_paths(&data_path);
    assert!(
        !referenced_before.is_empty(),
        "the seed table should create at least one referenced parquet file"
    );

    let orphan_dir = data_path.join("manual-prewrite");
    fs::create_dir_all(&orphan_dir).expect("orphan fixture directory");
    let orphan = orphan_dir.join("aborted-write.parquet");
    let recent = orphan_dir.join("recent-write.parquet");
    fs::write(&orphan, b"uncommitted parquet candidate").expect("orphan fixture");
    fs::write(&recent, b"recent parquet candidate").expect("recent fixture");

    let paths = server.storage_paths();
    let quarantine = server.tmp_dir().join("quarantine");
    let dry_report = paths
        .quarantine_orphan_candidates_before(
            Utc::now() - Duration::hours(1),
            quarantine.to_str().expect("quarantine path"),
            false,
        )
        .await
        .expect("dry-run quarantine");
    assert!(dry_report.dry_run);
    assert!(
        dry_report.candidates.is_empty(),
        "recent candidates must stay age-gated out of quarantine plans"
    );
    assert!(
        orphan.exists(),
        "dry-run quarantine must not move candidates"
    );

    let plan = paths
        .quarantine_orphan_candidates_before(
            Utc::now() + Duration::minutes(1),
            quarantine.to_str().expect("quarantine path"),
            false,
        )
        .await
        .expect("quarantine plan");
    assert_eq!(
        plan.candidates.len(),
        2,
        "both manual old candidates are planned"
    );
    assert!(orphan.exists(), "planning remains non-destructive");

    let live_prefix = data_path.join("bad-quarantine");
    let rejected = paths
        .quarantine_orphan_candidates_before(
            Utc::now() + Duration::minutes(1),
            live_prefix.to_str().expect("live quarantine path"),
            false,
        )
        .await;
    assert!(
        rejected.is_err(),
        "quarantine destinations inside the live data path must fail closed"
    );

    let applied = paths
        .quarantine_orphan_candidates_before(
            Utc::now() + Duration::minutes(1),
            quarantine.to_str().expect("quarantine path"),
            true,
        )
        .await
        .expect("apply quarantine");
    assert!(!applied.dry_run);
    assert_eq!(applied.copied_count, 2);
    assert_eq!(applied.deleted_count, 2);
    assert!(
        !orphan.exists(),
        "applied quarantine removes the orphan source"
    );
    assert!(
        !recent.exists(),
        "the second planned orphan source is also removed"
    );
    for entry in &applied.candidates {
        assert!(
            absolute_style_path(&entry.quarantine).exists(),
            "quarantine copy should exist for {entry:?}"
        );
    }

    let referenced_after = parquet_paths(&data_path);
    assert_eq!(
        referenced_after, referenced_before,
        "quarantine must not remove referenced DuckLake files"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn missing_catalog_fails_without_creating_it() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let catalog = tmp.path().join("missing.db");
    let data = tmp.path().join("data");
    let paths = quackgis_server::context::StoragePaths::new(
        catalog.to_str().expect("catalog path"),
        data.to_str().expect("data path"),
    )
    .expect("storage paths");

    let result = paths
        .orphan_candidates_before(Utc::now() - Duration::hours(1))
        .await;
    assert!(result.is_err(), "missing catalog must fail closed");
    assert!(!catalog.exists(), "inventory must not create a catalog");
}

fn absolute_style_path(path: &str) -> PathBuf {
    Path::new(path).to_path_buf()
}

fn parquet_paths(root: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    collect_parquet_paths(root, &mut paths);
    paths.sort();
    paths
}

fn collect_parquet_paths(path: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_parquet_paths(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "parquet") {
            out.push(path);
        }
    }
}
