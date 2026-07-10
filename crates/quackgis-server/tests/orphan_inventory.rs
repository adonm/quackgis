// SPDX-License-Identifier: Apache-2.0
//! Safe operator-visible orphan inventory over the maintained local profile.

mod common;

use std::fs;

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
