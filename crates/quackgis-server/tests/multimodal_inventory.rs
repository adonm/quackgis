// SPDX-License-Identifier: Apache-2.0
//! Deterministic real-artifact companion gate for raster/point-cloud inventories.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio_postgres::NoTls;
use url::Url;

use common::ServerHandle;

#[derive(Debug, Deserialize)]
struct Manifest {
    manifest_version: u32,
    collection_id: String,
    artifacts: Vec<Artifact>,
}

#[derive(Debug, Deserialize)]
struct Artifact {
    asset_id: String,
    family: String,
    source_object_id: String,
    asset_version: i32,
    path: String,
    source_uri: String,
    media_type: String,
    format_profile: String,
    byte_size: u64,
    sha256: String,
    bounds: [f64; 4],
    z_bounds: Option<[f64; 2]>,
    srid: i32,
    coordinate_epoch: f64,
    vertical_datum: String,
    transform_pipeline: String,
    lineage: String,
    quality_status: String,
    lifecycle_state: String,
    acquired_minute: i32,
    resolution_or_spacing_cm: f64,
    sidecars: Vec<Sidecar>,
}

#[derive(Debug, Deserialize)]
struct Sidecar {
    path: String,
    byte_size: u64,
    sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Bounds {
    minx: f64,
    miny: f64,
    maxx: f64,
    maxy: f64,
    minz: Option<f64>,
    maxz: Option<f64>,
}

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/data/multimodal")
}

fn read_manifest() -> Manifest {
    let raw = fs::read_to_string(fixtures_dir().join("inventory-v1.json")).expect("manifest");
    serde_json::from_str(&raw).expect("valid inventory manifest")
}

fn sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn validate_source_uri(raw: &str) -> Result<(), String> {
    let uri = Url::parse(raw).map_err(|error| error.to_string())?;
    if !matches!(uri.scheme(), "fixture" | "s3" | "https") {
        return Err(format!(
            "unsupported durable source URI scheme: {}",
            uri.scheme()
        ));
    }
    if !uri.username().is_empty()
        || uri.password().is_some()
        || uri.query().is_some()
        || uri.fragment().is_some()
    {
        return Err("durable source URI must not contain credentials, query, or fragment".into());
    }
    Ok(())
}

fn parse_ascii_grid(bytes: &[u8]) -> Bounds {
    let text = std::str::from_utf8(bytes).expect("ASCII grid is UTF-8");
    let mut lines = text.lines();
    let mut header = std::collections::HashMap::new();
    for _ in 0..6 {
        let line = lines.next().expect("six-line ASCII grid header");
        let mut parts = line.split_whitespace();
        let key = parts.next().expect("header key").to_ascii_lowercase();
        let value = parts
            .next()
            .expect("header value")
            .parse::<f64>()
            .expect("numeric header value");
        header.insert(key, value);
    }
    let ncols = header["ncols"];
    let nrows = header["nrows"];
    let minx = header["xllcorner"];
    let miny = header["yllcorner"];
    let cellsize = header["cellsize"];
    let cells = lines
        .flat_map(str::split_whitespace)
        .map(|value| value.parse::<f64>().expect("numeric cell"))
        .collect::<Vec<_>>();
    assert_eq!(cells.len(), (ncols * nrows) as usize);
    Bounds {
        minx,
        miny,
        maxx: minx + ncols * cellsize,
        maxy: miny + nrows * cellsize,
        minz: None,
        maxz: None,
    }
}

fn parse_ascii_ply(bytes: &[u8]) -> Bounds {
    let text = std::str::from_utf8(bytes).expect("ASCII PLY is UTF-8");
    let lines = text.lines().collect::<Vec<_>>();
    assert_eq!(lines.first(), Some(&"ply"));
    assert_eq!(lines.get(1), Some(&"format ascii 1.0"));
    let vertex_count = lines
        .iter()
        .find_map(|line| line.strip_prefix("element vertex "))
        .expect("vertex count")
        .parse::<usize>()
        .expect("numeric vertex count");
    let data_start = lines
        .iter()
        .position(|line| *line == "end_header")
        .expect("end_header")
        + 1;
    let points = lines[data_start..]
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let values = line
                .split_whitespace()
                .map(|value| value.parse::<f64>().expect("numeric PLY coordinate"))
                .collect::<Vec<_>>();
            assert_eq!(values.len(), 3);
            [values[0], values[1], values[2]]
        })
        .collect::<Vec<_>>();
    assert_eq!(points.len(), vertex_count);
    Bounds {
        minx: points
            .iter()
            .map(|point| point[0])
            .fold(f64::INFINITY, f64::min),
        miny: points
            .iter()
            .map(|point| point[1])
            .fold(f64::INFINITY, f64::min),
        maxx: points
            .iter()
            .map(|point| point[0])
            .fold(f64::NEG_INFINITY, f64::max),
        maxy: points
            .iter()
            .map(|point| point[1])
            .fold(f64::NEG_INFINITY, f64::max),
        minz: Some(
            points
                .iter()
                .map(|point| point[2])
                .fold(f64::INFINITY, f64::min),
        ),
        maxz: Some(
            points
                .iter()
                .map(|point| point[2])
                .fold(f64::NEG_INFINITY, f64::max),
        ),
    }
}

fn bounds_for(artifact: &Artifact, bytes: &[u8]) -> Bounds {
    match artifact.family.as_str() {
        "raster" => parse_ascii_grid(bytes),
        "point_cloud" => parse_ascii_ply(bytes),
        family => panic!("unsupported fixture family {family}"),
    }
}

fn validate_artifact(artifact: &Artifact) -> Vec<u8> {
    validate_source_uri(&artifact.source_uri).expect("stable non-secret source URI");
    let bytes = fs::read(fixtures_dir().join(&artifact.path)).expect("source fixture exists");
    assert_eq!(bytes.len() as u64, artifact.byte_size);
    assert_eq!(sha256(&bytes), artifact.sha256);
    let bounds = bounds_for(artifact, &bytes);
    assert_eq!(
        [bounds.minx, bounds.miny, bounds.maxx, bounds.maxy],
        artifact.bounds
    );
    assert_eq!(
        bounds
            .minz
            .zip(bounds.maxz)
            .map(|(minz, maxz)| [minz, maxz]),
        artifact.z_bounds
    );
    for sidecar in &artifact.sidecars {
        let sidecar_bytes = fs::read(fixtures_dir().join(&sidecar.path)).expect("sidecar exists");
        assert_eq!(sidecar_bytes.len() as u64, sidecar.byte_size);
        assert_eq!(sha256(&sidecar_bytes), sidecar.sha256);
        if sidecar.path.ends_with(".prj") {
            let wkt = std::str::from_utf8(&sidecar_bytes).expect("PRJ is UTF-8 WKT");
            assert!(
                wkt.contains(&format!("AUTHORITY[\"EPSG\",\"{}\"]", artifact.srid)),
                "PRJ authority must match manifest SRID"
            );
        }
    }
    bytes
}

fn rect_wkb_hex(bounds: [f64; 4]) -> String {
    let [minx, miny, maxx, maxy] = bounds;
    let points = [
        (minx, miny),
        (maxx, miny),
        (maxx, maxy),
        (minx, maxy),
        (minx, miny),
    ];
    let mut bytes = Vec::with_capacity(1 + 4 + 4 + points.len() * 16);
    bytes.push(1);
    bytes.extend_from_slice(&3_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u32.to_le_bytes());
    bytes.extend_from_slice(&(points.len() as u32).to_le_bytes());
    for (x, y) in points {
        bytes.extend_from_slice(&x.to_le_bytes());
        bytes.extend_from_slice(&y.to_le_bytes());
    }
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn sql_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn inventory_ddl(table: &str) -> String {
    format!(
        "CREATE TABLE public.{table} (
            collection_id TEXT,
            asset_id TEXT,
            source_object_id TEXT,
            asset_version INT,
            source_uri TEXT,
            media_type TEXT,
            format_profile TEXT,
            byte_size BIGINT,
            checksum_sha256 TEXT,
            srid INT,
            coordinate_epoch DOUBLE,
            vertical_datum TEXT,
            transform_pipeline TEXT,
            lineage TEXT,
            quality_status TEXT,
            lifecycle_state TEXT,
            acquired_minute INT,
            resolution_or_spacing_cm DOUBLE,
            z_min DOUBLE,
            z_max DOUBLE,
            footprint BINARY
        );"
    )
}

fn inventory_insert(table: &str, collection: &str, artifact: &Artifact) -> String {
    inventory_insert_version(
        table,
        collection,
        artifact,
        artifact.asset_version,
        &artifact.source_object_id,
        &artifact.source_uri,
        &artifact.lineage,
        &artifact.lifecycle_state,
    )
}

#[allow(clippy::too_many_arguments)]
fn inventory_insert_version(
    table: &str,
    collection: &str,
    artifact: &Artifact,
    asset_version: i32,
    source_object_id: &str,
    source_uri: &str,
    lineage: &str,
    lifecycle_state: &str,
) -> String {
    let z_min = artifact
        .z_bounds
        .map_or("NULL".to_string(), |z| z[0].to_string());
    let z_max = artifact
        .z_bounds
        .map_or("NULL".to_string(), |z| z[1].to_string());
    format!(
        "INSERT INTO public.{table} VALUES (
            {collection}, {asset_id}, {source_object_id}, {asset_version},
            {source_uri}, {media_type}, {format_profile}, {byte_size}, {sha256},
            {srid}, {coordinate_epoch}, {vertical_datum}, {transform_pipeline},
            {lineage}, {quality_status}, {lifecycle_state}, {acquired_minute},
            {scale}, {z_min}, {z_max}, X'{footprint}'
        );",
        collection = sql_string(collection),
        asset_id = sql_string(&artifact.asset_id),
        source_object_id = sql_string(source_object_id),
        asset_version = asset_version,
        source_uri = sql_string(source_uri),
        media_type = sql_string(&artifact.media_type),
        format_profile = sql_string(&artifact.format_profile),
        byte_size = artifact.byte_size,
        sha256 = sql_string(&artifact.sha256),
        srid = artifact.srid,
        coordinate_epoch = artifact.coordinate_epoch,
        vertical_datum = sql_string(&artifact.vertical_datum),
        transform_pipeline = sql_string(&artifact.transform_pipeline),
        lineage = sql_string(lineage),
        quality_status = sql_string(&artifact.quality_status),
        lifecycle_state = sql_string(lifecycle_state),
        acquired_minute = artifact.acquired_minute,
        scale = artifact.resolution_or_spacing_cm,
        footprint = rect_wkb_hex(artifact.bounds),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn real_artifact_sidecar_inventories_roundtrip_and_prune_exactly() {
    let manifest = read_manifest();
    assert_eq!(manifest.manifest_version, 1);
    assert_eq!(manifest.artifacts.len(), 2);
    for artifact in &manifest.artifacts {
        let original = validate_artifact(artifact);
        let mut corrupt = original.clone();
        corrupt[0] ^= 1;
        assert_ne!(
            sha256(&corrupt),
            artifact.sha256,
            "one-byte corruption detected"
        );
    }
    assert!(!fixtures_dir().join("missing-object.copc").exists());
    assert!(validate_source_uri("s3://bucket/key?X-Amz-Signature=secret").is_err());
    assert!(validate_source_uri("https://user:secret@example.test/object").is_err());

    let raster = manifest
        .artifacts
        .iter()
        .find(|artifact| artifact.family == "raster")
        .expect("raster artifact");
    let point_cloud = manifest
        .artifacts
        .iter()
        .find(|artifact| artifact.family == "point_cloud")
        .expect("point-cloud artifact");

    let server = ServerHandle::start().await;
    let (client, connection) = tokio_postgres::connect(&server.conn_str(), NoTls)
        .await
        .expect("connect");
    let _connection = tokio::spawn(connection);
    client
        .batch_execute(&format!(
            "{} {} {} {}",
            inventory_ddl("raster_inventory"),
            inventory_ddl("pointcloud_inventory"),
            inventory_insert("raster_inventory", &manifest.collection_id, raster),
            inventory_insert("pointcloud_inventory", &manifest.collection_id, point_cloud),
        ))
        .await
        .expect("create and seed sidecar inventories");

    let geometry_rows = client
        .query(
            "SELECT f_table_name, f_geometry_column
             FROM geometry_columns
             WHERE f_table_name IN ('raster_inventory', 'pointcloud_inventory')
             ORDER BY f_table_name",
            &[],
        )
        .await
        .expect("discover inventory footprints");
    assert_eq!(geometry_rows.len(), 2);
    assert!(
        geometry_rows
            .iter()
            .all(|row| row.get::<_, String>(1) == "footprint")
    );

    let raster_layout = client
        .query_one(
            "SELECT _qg_minx, _qg_miny, _qg_maxx, _qg_maxy
             FROM quackgis.main.raster_inventory",
            &[],
        )
        .await
        .expect("raster hidden bounds");
    assert_eq!(raster_layout.get::<_, f64>(0), raster.bounds[0]);
    assert_eq!(raster_layout.get::<_, f64>(1), raster.bounds[1]);
    assert_eq!(raster_layout.get::<_, f64>(2), raster.bounds[2]);
    assert_eq!(raster_layout.get::<_, f64>(3), raster.bounds[3]);

    let exact_raster: i64 = client
        .query_one(
            "SELECT COUNT(*) FROM public.raster_inventory
             WHERE quality_status = 'validated'
               AND ST_Intersects(
                 ST_GeomFromWKB(footprint),
                 ST_GeomFromWKB(ST_MakeEnvelope(-1, -1, 5, 4, 3857))
               )",
            &[],
        )
        .await
        .expect("exact raster inventory query")
        .get(0);
    let pruned_exact_raster: i64 = client
        .query_one(
            "SELECT COUNT(*) FROM quackgis.main.raster_inventory
             WHERE quality_status = 'validated'
               AND _qg_minx <= 5 AND _qg_maxx >= -1
               AND _qg_miny <= 4 AND _qg_maxy >= -1
               AND ST_Intersects(
                 ST_GeomFromWKB(footprint),
                 ST_GeomFromWKB(ST_MakeEnvelope(-1, -1, 5, 4, 3857))
               )",
            &[],
        )
        .await
        .expect("pruned exact raster inventory query")
        .get(0);
    assert_eq!(exact_raster, 1);
    assert_eq!(pruned_exact_raster, exact_raster);

    let provenance = client
        .query_one(
            "SELECT asset_id, z_min, z_max, coordinate_epoch, vertical_datum, lineage
             FROM public.pointcloud_inventory
             WHERE resolution_or_spacing_cm <= 50
               AND ST_Intersects(
                 ST_GeomFromWKB(footprint),
                 ST_GeomFromWKB(ST_MakeEnvelope(99, 199, 111, 216, 3857))
               )",
            &[],
        )
        .await
        .expect("point-cloud quality/provenance query");
    assert_eq!(provenance.get::<_, String>(0), point_cloud.asset_id);
    assert_eq!(provenance.get::<_, f64>(1), 5.0);
    assert_eq!(provenance.get::<_, f64>(2), 12.0);
    assert_eq!(provenance.get::<_, f64>(3), 2021.5);
    assert_eq!(provenance.get::<_, String>(4), "ellipsoidal-WGS84");
    assert_eq!(provenance.get::<_, String>(5), "ply-vertex-envelope-v1");

    let replacement_uri = "fixture:///multimodal/test_dem-v2.asc";
    validate_source_uri(replacement_uri).expect("stable replacement URI");
    client
        .batch_execute(&format!(
            "UPDATE public.raster_inventory
             SET lifecycle_state = 'superseded'
             WHERE asset_id = {asset_id} AND asset_version = 1;
             {replacement}",
            asset_id = sql_string(&raster.asset_id),
            replacement = inventory_insert_version(
                "raster_inventory",
                &manifest.collection_id,
                raster,
                2,
                "test-dem-asc-v2",
                replacement_uri,
                "repack-of:test-dem-asc-v1",
                "active",
            ),
        ))
        .await
        .expect("version object while preserving logical asset identity");
    let lifecycle = client
        .query(
            "SELECT asset_version, lifecycle_state, source_object_id
             FROM public.raster_inventory
             WHERE asset_id = 'dem-logical-1'
             ORDER BY asset_version",
            &[],
        )
        .await
        .expect("inventory lifecycle query");
    assert_eq!(lifecycle.len(), 2);
    assert_eq!(lifecycle[0].get::<_, i32>(0), 1);
    assert_eq!(lifecycle[0].get::<_, String>(1), "superseded");
    assert_eq!(lifecycle[1].get::<_, i32>(0), 2);
    assert_eq!(lifecycle[1].get::<_, String>(1), "active");
    assert_eq!(lifecycle[1].get::<_, String>(2), "test-dem-asc-v2");
}
