// SPDX-License-Identifier: Apache-2.0
//! Local LayoutBench seed/query runner for spatial-layout development.
//!
//! This intentionally talks over pgwire to a running QuackGIS server so it
//! exercises the same SQL routing, hidden layout columns, and pruning rewrites
//! that QGIS/Martin/GeoServer clients use. It is not part of the fast CI gate;
//! `tests/layoutbench_sf0.rs` remains the deterministic oracle.

use std::io::Cursor;
use std::pin::pin;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use futures::SinkExt;
use tokio_postgres::NoTls;

#[derive(Debug, Clone)]
struct Config {
    host: String,
    port: u16,
    scale: Scale,
    prefix: String,
    query_iters: usize,
    ingest_order: IngestOrder,
    load_method: LoadMethod,
    transactional_load: bool,
    analyze: bool,
    compare_variants: bool,
    compact_and_rerun: bool,
    reset: bool,
}

#[derive(Debug, Clone, Copy)]
enum Scale {
    Sf0,
    Sf1,
    Factor(usize),
}

impl Scale {
    fn factor(self) -> usize {
        match self {
            Scale::Sf0 => 1,
            // Deliberately moderate: enough rows for local planner/IO smoke
            // without requiring a workstation-sized benchmark run.
            Scale::Sf1 => 100,
            Scale::Factor(factor) => factor.max(1),
        }
    }

    fn label(self) -> String {
        match self {
            Scale::Sf0 => "sf0".to_string(),
            Scale::Sf1 => "sf1".to_string(),
            Scale::Factor(factor) => format!("sf{factor}x"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IngestOrder {
    Generated,
    Shuffled,
    Layout,
}

impl IngestOrder {
    fn label(self) -> &'static str {
        match self {
            IngestOrder::Generated => "generated",
            IngestOrder::Shuffled => "shuffled",
            IngestOrder::Layout => "layout",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LoadMethod {
    Insert,
    Copy,
}

impl LoadMethod {
    fn label(self) -> &'static str {
        match self {
            LoadMethod::Insert => "insert",
            LoadMethod::Copy => "copy",
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Rect {
    minx: f64,
    miny: f64,
    maxx: f64,
    maxy: f64,
}

#[derive(Debug)]
struct AerialFrame {
    id: i32,
    mission: String,
    strip: i32,
    captured_minute: i32,
    gsd_cm: f64,
    altitude_m: f64,
    footprint: Rect,
}

#[derive(Debug)]
struct CadObject {
    id: i32,
    floor: i32,
    object_type: &'static str,
    source_object_id: String,
    z_min: f64,
    z_max: f64,
    tolerance_mm: f64,
    geom: Rect,
}

#[derive(Debug)]
struct AssetRow {
    id: i32,
    asset_type: &'static str,
    uri: String,
    acquired_minute: i32,
    resolution_cm: f64,
    horizontal_accuracy_cm: f64,
    footprint: Rect,
}

#[derive(Debug, Clone, Copy)]
struct PruningMetric {
    total: i64,
    base: i64,
    candidate: i64,
    exact: i64,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct ScanMetric {
    output_rows: Option<u64>,
    bytes_scanned: Option<u64>,
    row_groups_total: Option<u64>,
    row_groups_matched: Option<u64>,
    files_ranges_total: Option<u64>,
    files_ranges_matched: Option<u64>,
    pushdown_rows_pruned: Option<u64>,
    pushdown_rows_matched: Option<u64>,
    hidden_bbox_predicate: bool,
    parquet_predicate: bool,
}

impl ScanMetric {
    fn row_groups_pruned(&self) -> Option<u64> {
        subtract(self.row_groups_total, self.row_groups_matched)
    }

    fn files_ranges_pruned(&self) -> Option<u64> {
        subtract(self.files_ranges_total, self.files_ranges_matched)
    }

    fn summary(&self) -> String {
        format!(
            "output_rows={} bytes_scanned={} row_groups={}/{}/{} files_ranges={}/{}/{} pushdown_rows={}/{} hidden_bbox={} parquet_predicate={}",
            fmt_opt(self.output_rows),
            fmt_opt(self.bytes_scanned),
            fmt_opt(self.row_groups_total),
            fmt_opt(self.row_groups_matched),
            fmt_opt(self.row_groups_pruned()),
            fmt_opt(self.files_ranges_total),
            fmt_opt(self.files_ranges_matched),
            fmt_opt(self.files_ranges_pruned()),
            fmt_opt(self.pushdown_rows_matched),
            fmt_opt(self.pushdown_rows_pruned),
            self.hidden_bbox_predicate,
            self.parquet_predicate,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct LayoutKey {
    time_bucket: i64,
    space_bucket: i64,
    space_sort: i64,
    id: i32,
}

#[derive(Debug, Clone, Copy)]
struct TimedCount {
    count: i64,
    elapsed_ms: u128,
    avg_ms: f64,
}

#[derive(Debug, Clone, Copy)]
struct QueryCase<'a> {
    label: &'a str,
    table: &'a str,
    geom_column: &'a str,
    envelope: Rect,
    extra_predicate: &'a str,
}

#[derive(Debug, Clone)]
struct QueryVariant {
    label: &'static str,
    sql: String,
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

    let factor = config.scale.factor();
    if config.reset {
        drop_tables(&client, &config.prefix).await?;
    }

    let seed_start = Instant::now();
    let seed = LayoutBenchSeed::generate(factor);
    seed.load(
        &client,
        &config.prefix,
        config.ingest_order,
        config.load_method,
        config.transactional_load,
    )
    .await?;
    let seed_elapsed = seed_start.elapsed();

    let table_rows = seed.row_counts();
    println!(
        "layoutbench_seed scale={} factor={} prefix={} ingest_order={} load_method={} transactional_load={} rows aerial={} cad={} assets={} elapsed_ms={}",
        config.scale.label(),
        factor,
        config.prefix,
        config.ingest_order.label(),
        config.load_method.label(),
        config.transactional_load,
        table_rows.aerial,
        table_rows.cad,
        table_rows.assets,
        seed_elapsed.as_millis()
    );

    let cases = query_cases(&config.prefix);
    for case in &cases {
        let metric = pruning_metric(&client, case).await?;
        println!(
            "layoutbench_pruning label={} total={} base={} candidate={} exact={} false_positive={} candidate_pct={:.2}",
            case.label,
            metric.total,
            metric.base,
            metric.candidate,
            metric.exact,
            metric.candidate - metric.exact,
            pct(metric.candidate, metric.base),
        );
    }

    if config.compact_and_rerun {
        run_query_suite(&client, &config, &cases, Some("before_compact")).await?;
        compact_tables(&client, &config.prefix).await?;
        run_query_suite(&client, &config, &cases, Some("after_compact")).await?;
    } else {
        run_query_suite(&client, &config, &cases, None).await?;
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
        let mut scale = Scale::Sf0;
        let mut prefix = "layoutbench_local".to_string();
        let mut query_iters = 3_usize;
        let mut ingest_order = IngestOrder::Generated;
        let mut load_method = LoadMethod::Insert;
        let mut transactional_load = false;
        let mut analyze = true;
        let mut compare_variants = false;
        let mut compact_and_rerun = false;
        let mut reset = true;

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
                "--scale" => {
                    scale = parse_scale(&args.next().context("--scale requires a value")?)?
                }
                "--factor" => {
                    let factor = args
                        .next()
                        .context("--factor requires a value")?
                        .parse::<usize>()
                        .context("--factor must be a positive integer")?;
                    scale = Scale::Factor(factor);
                }
                "--prefix" => prefix = args.next().context("--prefix requires a value")?,
                "--ingest-order" => {
                    ingest_order = parse_ingest_order(
                        &args.next().context("--ingest-order requires a value")?,
                    )?
                }
                "--load-method" => {
                    load_method =
                        parse_load_method(&args.next().context("--load-method requires a value")?)?
                }
                "--query-iters" => {
                    query_iters = args
                        .next()
                        .context("--query-iters requires a value")?
                        .parse()
                        .context("--query-iters must be a positive integer")?;
                }
                "--compare-variants" => compare_variants = true,
                "--compact-and-rerun" => compact_and_rerun = true,
                "--transactional-load" => transactional_load = true,
                "--no-analyze" => analyze = false,
                "--no-reset" => reset = false,
                "--help" | "-h" => {
                    print_help();
                    std::process::exit(0);
                }
                other => bail!("unknown argument: {other}"),
            }
        }

        if !is_safe_prefix(&prefix) {
            bail!("--prefix must contain only ASCII letters, digits, and underscores");
        }
        Ok(Self {
            host,
            port,
            scale,
            prefix,
            query_iters,
            ingest_order,
            load_method,
            transactional_load,
            analyze,
            compare_variants,
            compact_and_rerun,
            reset,
        })
    }
}

fn parse_scale(value: &str) -> Result<Scale> {
    match value.to_ascii_lowercase().as_str() {
        "sf0" => Ok(Scale::Sf0),
        "sf1" => Ok(Scale::Sf1),
        other => other.parse::<usize>().map(Scale::Factor).with_context(|| {
            format!("unsupported --scale {value:?}; use sf0, sf1, or an integer factor")
        }),
    }
}

fn parse_ingest_order(value: &str) -> Result<IngestOrder> {
    match value.to_ascii_lowercase().as_str() {
        "generated" => Ok(IngestOrder::Generated),
        "shuffled" | "random" => Ok(IngestOrder::Shuffled),
        "layout" | "clustered" | "sorted" => Ok(IngestOrder::Layout),
        _ => bail!("unsupported --ingest-order {value:?}; use generated, shuffled, or layout"),
    }
}

fn parse_load_method(value: &str) -> Result<LoadMethod> {
    match value.to_ascii_lowercase().as_str() {
        "insert" | "values" => Ok(LoadMethod::Insert),
        "copy" | "copy-in" | "copy_in" => Ok(LoadMethod::Copy),
        _ => bail!("unsupported --load-method {value:?}; use insert or copy"),
    }
}

fn print_help() {
    println!(
        "Usage: cargo run -p quackgis-server --example layoutbench -- \
         [--host HOST] [--port PORT] [--scale sf0|sf1|N] [--factor N] \
         [--prefix NAME] [--query-iters N] [--ingest-order generated|shuffled|layout] \
         [--load-method insert|copy] \
         [--transactional-load] [--compare-variants] [--compact-and-rerun] \
         [--no-analyze] [--no-reset]"
    );
}

fn is_safe_prefix(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

#[derive(Debug)]
struct RowCounts {
    aerial: usize,
    cad: usize,
    assets: usize,
}

#[derive(Debug)]
struct LayoutBenchSeed {
    aerial_frames: Vec<AerialFrame>,
    cad_objects: Vec<CadObject>,
    assets: Vec<AssetRow>,
}

impl LayoutBenchSeed {
    fn generate(factor: usize) -> Self {
        Self {
            aerial_frames: generate_aerial_frames(factor),
            cad_objects: generate_cad_objects(factor),
            assets: generate_assets(factor),
        }
    }

    fn row_counts(&self) -> RowCounts {
        RowCounts {
            aerial: self.aerial_frames.len(),
            cad: self.cad_objects.len(),
            assets: self.assets.len(),
        }
    }

    async fn load(
        &self,
        client: &tokio_postgres::Client,
        prefix: &str,
        ingest_order: IngestOrder,
        load_method: LoadMethod,
        transactional_load: bool,
    ) -> Result<()> {
        create_tables(client, prefix).await?;
        load_rows(
            client,
            &format!("{} (id, mission, strip, captured_minute, gsd_cm, altitude_m, geom, minx, miny, maxx, maxy)", table_name(prefix, "aerial_frames")),
            &self.aerial_values(ingest_order),
            &self.aerial_copy_rows(ingest_order),
            load_method,
            transactional_load,
        )
        .await?;
        load_rows(
            client,
            &format!("{} (id, floor, object_type, source_object_id, z_min, z_max, tolerance_mm, geom, minx, miny, maxx, maxy)", table_name(prefix, "cad_objects")),
            &self.cad_values(ingest_order),
            &self.cad_copy_rows(ingest_order),
            load_method,
            transactional_load,
        )
        .await?;
        load_rows(
            client,
            &format!("{} (id, asset_type, uri, acquired_minute, resolution_cm, horizontal_accuracy_cm, footprint, minx, miny, maxx, maxy)", table_name(prefix, "assets")),
            &self.asset_values(ingest_order),
            &self.asset_copy_rows(ingest_order),
            load_method,
            transactional_load,
        )
        .await?;
        Ok(())
    }

    fn aerial_values(&self, ingest_order: IngestOrder) -> Vec<String> {
        self.ordered_aerial(ingest_order)
            .into_iter()
            .map(|row| {
                format!(
                    "({}, '{}', {}, {}, {}, {}, {}, {}, {}, {}, {})",
                    row.id,
                    escape_sql(&row.mission),
                    row.strip,
                    row.captured_minute,
                    row.gsd_cm,
                    row.altitude_m,
                    rect_wkb_sql(row.footprint),
                    row.footprint.minx,
                    row.footprint.miny,
                    row.footprint.maxx,
                    row.footprint.maxy,
                )
            })
            .collect()
    }

    fn aerial_copy_rows(&self, ingest_order: IngestOrder) -> Vec<String> {
        self.ordered_aerial(ingest_order)
            .into_iter()
            .map(|row| {
                copy_line(&[
                    row.id.to_string(),
                    copy_text(&row.mission),
                    row.strip.to_string(),
                    row.captured_minute.to_string(),
                    row.gsd_cm.to_string(),
                    row.altitude_m.to_string(),
                    copy_bytea_hex(&rect_wkb(row.footprint)),
                    row.footprint.minx.to_string(),
                    row.footprint.miny.to_string(),
                    row.footprint.maxx.to_string(),
                    row.footprint.maxy.to_string(),
                ])
            })
            .collect()
    }

    fn ordered_aerial(&self, ingest_order: IngestOrder) -> Vec<&AerialFrame> {
        ordered_refs(
            &self.aerial_frames,
            ingest_order,
            0xa11e_a1a1_f00d_0001,
            |row| row.id,
            |row| layout_key(row.captured_minute, row.footprint, row.id),
        )
    }

    fn cad_values(&self, ingest_order: IngestOrder) -> Vec<String> {
        self.ordered_cad(ingest_order)
            .into_iter()
            .map(|row| {
                format!(
                    "({}, {}, '{}', '{}', {}, {}, {}, {}, {}, {}, {}, {})",
                    row.id,
                    row.floor,
                    escape_sql(row.object_type),
                    escape_sql(&row.source_object_id),
                    row.z_min,
                    row.z_max,
                    row.tolerance_mm,
                    rect_wkb_sql(row.geom),
                    row.geom.minx,
                    row.geom.miny,
                    row.geom.maxx,
                    row.geom.maxy,
                )
            })
            .collect()
    }

    fn cad_copy_rows(&self, ingest_order: IngestOrder) -> Vec<String> {
        self.ordered_cad(ingest_order)
            .into_iter()
            .map(|row| {
                copy_line(&[
                    row.id.to_string(),
                    row.floor.to_string(),
                    copy_text(row.object_type),
                    copy_text(&row.source_object_id),
                    row.z_min.to_string(),
                    row.z_max.to_string(),
                    row.tolerance_mm.to_string(),
                    copy_bytea_hex(&rect_wkb(row.geom)),
                    row.geom.minx.to_string(),
                    row.geom.miny.to_string(),
                    row.geom.maxx.to_string(),
                    row.geom.maxy.to_string(),
                ])
            })
            .collect()
    }

    fn ordered_cad(&self, ingest_order: IngestOrder) -> Vec<&CadObject> {
        ordered_refs(
            &self.cad_objects,
            ingest_order,
            0xcad0_b1ec_7000_0002,
            |row| row.id,
            |row| layout_key(0, row.geom, row.id),
        )
    }

    fn asset_values(&self, ingest_order: IngestOrder) -> Vec<String> {
        self.ordered_assets(ingest_order)
            .into_iter()
            .map(|row| {
                format!(
                    "({}, '{}', '{}', {}, {}, {}, {}, {}, {}, {}, {})",
                    row.id,
                    escape_sql(row.asset_type),
                    escape_sql(&row.uri),
                    row.acquired_minute,
                    row.resolution_cm,
                    row.horizontal_accuracy_cm,
                    rect_wkb_sql(row.footprint),
                    row.footprint.minx,
                    row.footprint.miny,
                    row.footprint.maxx,
                    row.footprint.maxy,
                )
            })
            .collect()
    }

    fn asset_copy_rows(&self, ingest_order: IngestOrder) -> Vec<String> {
        self.ordered_assets(ingest_order)
            .into_iter()
            .map(|row| {
                copy_line(&[
                    row.id.to_string(),
                    copy_text(row.asset_type),
                    copy_text(&row.uri),
                    row.acquired_minute.to_string(),
                    row.resolution_cm.to_string(),
                    row.horizontal_accuracy_cm.to_string(),
                    copy_bytea_hex(&rect_wkb(row.footprint)),
                    row.footprint.minx.to_string(),
                    row.footprint.miny.to_string(),
                    row.footprint.maxx.to_string(),
                    row.footprint.maxy.to_string(),
                ])
            })
            .collect()
    }

    fn ordered_assets(&self, ingest_order: IngestOrder) -> Vec<&AssetRow> {
        ordered_refs(
            &self.assets,
            ingest_order,
            0xa55e_7000_f007_0003,
            |row| row.id,
            |row| layout_key(row.acquired_minute, row.footprint, row.id),
        )
    }
}

async fn drop_tables(client: &tokio_postgres::Client, prefix: &str) -> Result<()> {
    for suffix in ["aerial_frames", "cad_objects", "assets"] {
        let table = table_name(prefix, suffix);
        let _ = client
            .batch_execute(&format!("DROP TABLE IF EXISTS public.{table}"))
            .await;
    }
    Ok(())
}

async fn create_tables(client: &tokio_postgres::Client, prefix: &str) -> Result<()> {
    let aerial = table_name(prefix, "aerial_frames");
    let cad = table_name(prefix, "cad_objects");
    let assets = table_name(prefix, "assets");
    client
        .batch_execute(&format!(
            "CREATE TABLE IF NOT EXISTS public.{aerial} (\
                id INT, mission TEXT, strip INT, captured_minute INT, \
                gsd_cm DOUBLE, altitude_m DOUBLE, geom BINARY, \
                minx DOUBLE, miny DOUBLE, maxx DOUBLE, maxy DOUBLE);\
             CREATE TABLE IF NOT EXISTS public.{cad} (\
                id INT, floor INT, object_type TEXT, source_object_id TEXT, \
                z_min DOUBLE, z_max DOUBLE, tolerance_mm DOUBLE, geom BINARY, \
                minx DOUBLE, miny DOUBLE, maxx DOUBLE, maxy DOUBLE);\
             CREATE TABLE IF NOT EXISTS public.{assets} (\
                id INT, asset_type TEXT, uri TEXT, acquired_minute INT, \
                resolution_cm DOUBLE, horizontal_accuracy_cm DOUBLE, footprint BINARY, \
                minx DOUBLE, miny DOUBLE, maxx DOUBLE, maxy DOUBLE);"
        ))
        .await
        .context("create LayoutBench tables")?;
    Ok(())
}

async fn load_rows(
    client: &tokio_postgres::Client,
    table_with_columns: &str,
    insert_values: &[String],
    copy_rows: &[String],
    load_method: LoadMethod,
    transactional: bool,
) -> Result<()> {
    if transactional {
        client
            .batch_execute("BEGIN")
            .await
            .with_context(|| format!("begin transactional load for {table_with_columns}"))?;
    }
    let result = match load_method {
        LoadMethod::Insert => insert_rows(client, table_with_columns, insert_values).await,
        LoadMethod::Copy => copy_rows_into_table(client, table_with_columns, copy_rows).await,
    };
    if let Err(err) = result {
        if transactional {
            let _ = client.batch_execute("ROLLBACK").await;
        }
        return Err(err);
    }
    if transactional {
        client
            .batch_execute("COMMIT")
            .await
            .with_context(|| format!("commit transactional load for {table_with_columns}"))?;
    }
    Ok(())
}

async fn insert_rows(
    client: &tokio_postgres::Client,
    table_with_columns: &str,
    rows: &[String],
) -> Result<()> {
    for chunk in rows.chunks(500) {
        let sql = format!(
            "INSERT INTO public.{table_with_columns} VALUES {}",
            chunk.join(",")
        );
        client
            .batch_execute(&sql)
            .await
            .with_context(|| format!("insert {} rows into {table_with_columns}", chunk.len()))?;
    }
    Ok(())
}

async fn copy_rows_into_table(
    client: &tokio_postgres::Client,
    table_with_columns: &str,
    rows: &[String],
) -> Result<()> {
    let sink = client
        .copy_in::<_, Cursor<Vec<u8>>>(&format!("COPY public.{table_with_columns} FROM STDIN"))
        .await
        .with_context(|| format!("start COPY into {table_with_columns}"))?;
    let mut sink = pin!(sink);
    for chunk in rows.chunks(5_000) {
        sink.as_mut()
            .send(Cursor::new(chunk.concat().into_bytes()))
            .await
            .with_context(|| format!("send {} COPY rows into {table_with_columns}", chunk.len()))?;
    }
    let copied = sink
        .as_mut()
        .finish()
        .await
        .with_context(|| format!("finish COPY into {table_with_columns}"))?;
    if copied != rows.len() as u64 {
        bail!(
            "COPY row count mismatch for {table_with_columns}: sent {}, server reported {copied}",
            rows.len()
        );
    }
    Ok(())
}

fn ordered_refs<T, Id, Key>(
    rows: &[T],
    ingest_order: IngestOrder,
    shuffle_salt: u64,
    id: Id,
    layout: Key,
) -> Vec<&T>
where
    Id: Fn(&T) -> i32,
    Key: Fn(&T) -> LayoutKey,
{
    let mut out = rows.iter().collect::<Vec<_>>();
    match ingest_order {
        IngestOrder::Generated => {}
        IngestOrder::Shuffled => {
            out.sort_by_key(|row| deterministic_shuffle_key(id(row), shuffle_salt));
        }
        IngestOrder::Layout => {
            out.sort_by_key(|row| layout(row));
        }
    }
    out
}

fn deterministic_shuffle_key(id: i32, salt: u64) -> u64 {
    splitmix64(id as u64 ^ salt)
}

fn layout_key(time_minute: i32, rect: Rect, id: i32) -> LayoutKey {
    LayoutKey {
        time_bucket: (time_minute as f64 / 60.0).floor() as i64,
        space_bucket: space_bucket(rect),
        space_sort: space_sort(rect),
        id,
    }
}

const SPACE_BUCKET_SIZE: f64 = 1024.0;

fn space_bucket(rect: Rect) -> i64 {
    let (center_x, center_y) = rect_center(rect);
    morton_signed(
        quantized_coord(center_x, SPACE_BUCKET_SIZE),
        quantized_coord(center_y, SPACE_BUCKET_SIZE),
    )
}

fn space_sort(rect: Rect) -> i64 {
    let (center_x, center_y) = rect_center(rect);
    morton_signed(
        quantized_coord(center_x, 1.0),
        quantized_coord(center_y, 1.0),
    )
}

fn rect_center(rect: Rect) -> (f64, f64) {
    ((rect.minx + rect.maxx) / 2.0, (rect.miny + rect.maxy) / 2.0)
}

fn quantized_coord(value: f64, cell_size: f64) -> i64 {
    if !value.is_finite() || cell_size <= 0.0 {
        return 0;
    }
    (value / cell_size).floor() as i64
}

fn morton_signed(x: i64, y: i64) -> i64 {
    let x = zigzag_i32(x) as u64;
    let y = zigzag_i32(y) as u64;
    (split_by_1(x) | (split_by_1(y) << 1)) as i64
}

fn zigzag_i32(value: i64) -> u32 {
    let value = value.clamp(i32::MIN as i64, i32::MAX as i64);
    ((value << 1) ^ (value >> 31)) as u32
}

fn split_by_1(mut value: u64) -> u64 {
    value &= 0x0000_0000_ffff_ffff;
    value = (value | (value << 16)) & 0x0000_ffff_0000_ffff;
    value = (value | (value << 8)) & 0x00ff_00ff_00ff_00ff;
    value = (value | (value << 4)) & 0x0f0f_0f0f_0f0f_0f0f;
    value = (value | (value << 2)) & 0x3333_3333_3333_3333;
    (value | (value << 1)) & 0x5555_5555_5555_5555
}

fn splitmix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

fn generate_aerial_frames(factor: usize) -> Vec<AerialFrame> {
    let mut rows = Vec::new();
    let mut id = 1;
    for block in 0..factor {
        for mission_idx in 0..3 {
            let mission = format!("mission_{block:03}_{mission_idx}");
            for strip in 0..3 {
                for frame in 0..12 {
                    let offset_x = block as f64 * 420.0;
                    let center_x = offset_x + 35.0 + frame as f64 * 27.0 + mission_idx as f64 * 4.0;
                    let center_y = 55.0 + strip as f64 * 48.0 + mission_idx as f64 * 3.0;
                    let width = 52.0 + (frame % 3) as f64 * 4.0;
                    let height = 36.0 + (strip % 2) as f64 * 6.0;
                    rows.push(AerialFrame {
                        id,
                        mission: mission.clone(),
                        strip: strip + (block as i32 * 10),
                        captured_minute: block as i32 * 1_440
                            + mission_idx * 480
                            + strip * 40
                            + frame * 9,
                        gsd_cm: 2.5 + mission_idx as f64 * 0.5,
                        altitude_m: 120.0 + strip as f64 * 8.0,
                        footprint: Rect {
                            minx: center_x - width / 2.0,
                            miny: center_y - height / 2.0,
                            maxx: center_x + width / 2.0,
                            maxy: center_y + height / 2.0,
                        },
                    });
                    id += 1;
                }
            }
        }
    }
    rows
}

fn generate_cad_objects(factor: usize) -> Vec<CadObject> {
    let mut rows = Vec::new();
    let mut id = 1;
    let origin_x = 1_000_000.0;
    let origin_y = 2_000_000.0;
    for block in 0..factor {
        let block_x = block as f64 * 150.0;
        for floor in 0..4 {
            for bay_x in 0..6 {
                for bay_y in 0..4 {
                    let minx = origin_x + block_x + bay_x as f64 * 18.5 + floor as f64 * 0.125;
                    let miny = origin_y + bay_y as f64 * 14.25 + floor as f64 * 0.075;
                    let object_type = if (bay_x + bay_y + floor) % 5 == 0 {
                        "column"
                    } else if bay_y % 2 == 0 {
                        "wall"
                    } else {
                        "room"
                    };
                    rows.push(CadObject {
                        id,
                        floor,
                        object_type,
                        source_object_id: format!(
                            "IFC-{block:03}-{floor:02}-{bay_x:02}-{bay_y:02}"
                        ),
                        z_min: floor as f64 * 3.6,
                        z_max: floor as f64 * 3.6 + 3.4,
                        tolerance_mm: 2.0 + ((bay_x + bay_y) % 3) as f64 * 0.25,
                        geom: Rect {
                            minx,
                            miny,
                            maxx: minx + 12.0 + (bay_x % 2) as f64 * 3.0,
                            maxy: miny + 8.0 + (bay_y % 2) as f64 * 4.0,
                        },
                    });
                    id += 1;
                }
            }
        }
    }
    rows
}

fn generate_assets(factor: usize) -> Vec<AssetRow> {
    let asset_types = ["copc", "cog", "3dtiles", "ifc", "e57", "citygml"];
    let mut rows = Vec::new();
    let mut id = 1;
    for block in 0..factor {
        for (asset_idx, asset_type) in asset_types.iter().enumerate() {
            for tile_idx in 0..4 {
                let minx =
                    block as f64 * 520.0 + 40.0 + asset_idx as f64 * 70.0 + tile_idx as f64 * 15.0;
                let miny = 35.0 + tile_idx as f64 * 42.0;
                rows.push(AssetRow {
                    id,
                    asset_type,
                    uri: format!("s3://layoutbench/{block:03}/{asset_type}/{tile_idx:02}"),
                    acquired_minute: block as i32 * 1_440 + asset_idx as i32 * 120 + tile_idx * 15,
                    resolution_cm: 5.0 + asset_idx as f64 * 2.0 + tile_idx as f64,
                    horizontal_accuracy_cm: 2.0 + (asset_idx % 3) as f64 * 2.0,
                    footprint: Rect {
                        minx,
                        miny,
                        maxx: minx + 85.0,
                        maxy: miny + 70.0,
                    },
                });
                id += 1;
            }
        }
    }
    rows
}

fn query_cases(prefix: &str) -> Vec<QueryCase<'static>> {
    let aerial_table = Box::leak(table_name(prefix, "aerial_frames").into_boxed_str());
    let cad_table = Box::leak(table_name(prefix, "cad_objects").into_boxed_str());
    let assets_table = Box::leak(table_name(prefix, "assets").into_boxed_str());
    vec![
        QueryCase {
            label: "aerial",
            table: aerial_table,
            geom_column: "geom",
            envelope: Rect {
                minx: 95.0,
                miny: 95.0,
                maxx: 290.0,
                maxy: 185.0,
            },
            extra_predicate: "captured_minute BETWEEN 40 AND 170",
        },
        QueryCase {
            label: "cad",
            table: cad_table,
            geom_column: "geom",
            envelope: Rect {
                minx: 1_000_020.0,
                miny: 2_000_010.0,
                maxx: 1_000_075.0,
                maxy: 2_000_055.0,
            },
            extra_predicate: "floor = 2",
        },
        QueryCase {
            label: "assets",
            table: assets_table,
            geom_column: "footprint",
            envelope: Rect {
                minx: 150.0,
                miny: 90.0,
                maxx: 430.0,
                maxy: 255.0,
            },
            extra_predicate: "resolution_cm <= 15.0 AND horizontal_accuracy_cm <= 7.0",
        },
    ]
}

async fn pruning_metric(
    client: &tokio_postgres::Client,
    case: &QueryCase<'_>,
) -> Result<PruningMetric> {
    let table_ref = format!("quackgis.main.{}", case.table);
    let envelope_sql = format!("ST_GeomFromWKB({})", envelope_wkb_sql(case.envelope));
    let total_sql = format!("SELECT COUNT(*) FROM {table_ref}");
    let base_sql = format!(
        "SELECT COUNT(*) FROM {table_ref} WHERE {}",
        case.extra_predicate
    );
    let candidate_sql = format!(
        "SELECT COUNT(*) FROM {table_ref} \
         WHERE {extra_predicate} \
           AND _qg_minx <= {maxx} AND _qg_maxx >= {minx} \
           AND _qg_miny <= {maxy} AND _qg_maxy >= {miny}",
        extra_predicate = case.extra_predicate,
        minx = case.envelope.minx,
        miny = case.envelope.miny,
        maxx = case.envelope.maxx,
        maxy = case.envelope.maxy,
    );
    let exact_sql = format!(
        "SELECT COUNT(*) FROM {table_ref} \
         WHERE {extra_predicate} \
           AND ST_Intersects(ST_GeomFromWKB({geom_column}), {envelope_sql})",
        extra_predicate = case.extra_predicate,
        geom_column = case.geom_column,
    );
    Ok(PruningMetric {
        total: query_count(client, &total_sql).await?,
        base: query_count(client, &base_sql).await?,
        candidate: query_count(client, &candidate_sql).await?,
        exact: query_count(client, &exact_sql).await?,
    })
}

fn public_exact_sql(case: &QueryCase<'_>) -> String {
    format!(
        "SELECT COUNT(*) AS n FROM public.{table} \
         WHERE {extra_predicate} \
           AND ST_Intersects(ST_GeomFromWKB({geom_column}), ST_GeomFromWKB({envelope_sql}))",
        table = case.table,
        extra_predicate = case.extra_predicate,
        geom_column = case.geom_column,
        envelope_sql = envelope_wkb_sql(case.envelope),
    )
}

fn query_variants(case: &QueryCase<'_>) -> Vec<QueryVariant> {
    vec![
        QueryVariant {
            label: "internal_exact",
            sql: internal_exact_no_rewrite_sql(case),
        },
        QueryVariant {
            label: "internal_bbox_exact",
            sql: internal_bbox_exact_sql(case),
        },
        QueryVariant {
            label: "internal_bbox_only",
            sql: internal_bbox_only_sql(case),
        },
    ]
}

fn internal_exact_no_rewrite_sql(case: &QueryCase<'_>) -> String {
    format!(
        "SELECT COUNT(*) AS n FROM quackgis.main.{table} \
         WHERE (_qg_minx IS NULL OR _qg_minx IS NOT NULL) \
           AND {extra_predicate} \
           AND ST_Intersects(ST_GeomFromWKB({geom_column}), ST_GeomFromWKB({envelope_sql}))",
        table = case.table,
        extra_predicate = case.extra_predicate,
        geom_column = case.geom_column,
        envelope_sql = envelope_wkb_sql(case.envelope),
    )
}

fn internal_bbox_exact_sql(case: &QueryCase<'_>) -> String {
    format!(
        "SELECT COUNT(*) AS n FROM quackgis.main.{table} \
         WHERE {extra_predicate} \
           AND {bbox_predicate} \
           AND ST_Intersects(ST_GeomFromWKB({geom_column}), ST_GeomFromWKB({envelope_sql}))",
        table = case.table,
        extra_predicate = case.extra_predicate,
        bbox_predicate = bbox_predicate(case.envelope),
        geom_column = case.geom_column,
        envelope_sql = envelope_wkb_sql(case.envelope),
    )
}

fn internal_bbox_only_sql(case: &QueryCase<'_>) -> String {
    format!(
        "SELECT COUNT(*) AS n FROM quackgis.main.{table} \
         WHERE {extra_predicate} \
           AND {bbox_predicate}",
        table = case.table,
        extra_predicate = case.extra_predicate,
        bbox_predicate = bbox_predicate(case.envelope),
    )
}

fn bbox_predicate(envelope: Rect) -> String {
    format!(
        "_qg_minx <= {maxx} AND _qg_maxx >= {minx} \
         AND _qg_miny <= {maxy} AND _qg_maxy >= {miny}",
        minx = envelope.minx,
        miny = envelope.miny,
        maxx = envelope.maxx,
        maxy = envelope.maxy,
    )
}

async fn run_query_suite(
    client: &tokio_postgres::Client,
    config: &Config,
    cases: &[QueryCase<'_>],
    phase: Option<&str>,
) -> Result<()> {
    let phase = phase_attr(phase);
    for case in cases {
        let sql = public_exact_sql(case);
        let timed = timed_count(client, &sql, config.query_iters.max(1)).await?;
        println!(
            "layoutbench_query label={}{} iters={} count={} elapsed_ms={} avg_ms={:.2}",
            case.label,
            phase,
            config.query_iters.max(1),
            timed.count,
            timed.elapsed_ms,
            timed.avg_ms,
        );

        if config.analyze {
            let plan = explain_analyze(client, &sql).await?;
            let scan = scan_metric_from_explain(&plan);
            println!(
                "layoutbench_scan label={}{} {}",
                case.label,
                phase,
                scan.summary()
            );
        }

        if config.compare_variants {
            for variant in query_variants(case) {
                let timed = timed_count(client, &variant.sql, config.query_iters.max(1)).await?;
                let scan = if config.analyze {
                    Some(scan_metric_from_explain(
                        &explain_analyze(client, &variant.sql).await?,
                    ))
                } else {
                    None
                };
                println!(
                    "layoutbench_variant label={}{} variant={} iters={} count={} elapsed_ms={} avg_ms={:.2}{}",
                    case.label,
                    phase,
                    variant.label,
                    config.query_iters.max(1),
                    timed.count,
                    timed.elapsed_ms,
                    timed.avg_ms,
                    scan.as_ref()
                        .map(|scan| format!(" {}", scan.summary()))
                        .unwrap_or_default(),
                );
            }
        }
    }
    Ok(())
}

fn phase_attr(phase: Option<&str>) -> String {
    phase
        .map(|phase| format!(" phase={phase}"))
        .unwrap_or_default()
}

async fn compact_tables(client: &tokio_postgres::Client, prefix: &str) -> Result<()> {
    let start = Instant::now();
    for suffix in ["aerial_frames", "cad_objects", "assets"] {
        let table = format!("{prefix}_{suffix}");
        client
            .batch_execute(&format!("CALL quackgis_compact_table('public.{table}')"))
            .await
            .with_context(|| format!("compact public.{table}"))?;
    }
    let elapsed = start.elapsed();
    println!(
        "layoutbench_compact prefix={} tables=3 elapsed_ms={}",
        prefix,
        elapsed.as_millis()
    );
    Ok(())
}

async fn query_count(client: &tokio_postgres::Client, sql: &str) -> Result<i64> {
    Ok(client
        .query_one(sql, &[])
        .await
        .with_context(|| format!("count query failed: {sql}"))?
        .get(0))
}

async fn timed_count(
    client: &tokio_postgres::Client,
    sql: &str,
    query_iters: usize,
) -> Result<TimedCount> {
    let mut last_count = None;
    let start = Instant::now();
    for _ in 0..query_iters {
        last_count = Some(query_count(client, sql).await?);
    }
    let elapsed = start.elapsed();
    Ok(TimedCount {
        count: last_count.unwrap_or_default(),
        elapsed_ms: elapsed.as_millis(),
        avg_ms: elapsed.as_secs_f64() * 1000.0 / query_iters as f64,
    })
}

async fn explain_analyze(client: &tokio_postgres::Client, sql: &str) -> Result<String> {
    let messages = client
        .simple_query(&format!("EXPLAIN ANALYZE {sql}"))
        .await
        .with_context(|| format!("EXPLAIN ANALYZE failed: {sql}"))?;
    let rendered = messages
        .iter()
        .filter_map(|message| match message {
            tokio_postgres::SimpleQueryMessage::Row(row) => Some(
                row.columns()
                    .iter()
                    .filter_map(|column| row.get(column.name()))
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    if rendered.is_empty() {
        bail!("EXPLAIN ANALYZE returned no plan rows for: {sql}");
    }
    Ok(rendered)
}

fn scan_metric_from_explain(plan: &str) -> ScanMetric {
    let mut metric = ScanMetric {
        output_rows: metric_value(plan, "output_rows"),
        bytes_scanned: metric_value(plan, "bytes_scanned"),
        pushdown_rows_pruned: metric_value(plan, "pushdown_rows_pruned"),
        pushdown_rows_matched: metric_value(plan, "pushdown_rows_matched"),
        hidden_bbox_predicate: plan.contains("_qg_minx")
            && plan.contains("_qg_maxx")
            && plan.contains("_qg_miny")
            && plan.contains("_qg_maxy"),
        parquet_predicate: plan.contains("DataSourceExec") && plan.contains("predicate="),
        ..ScanMetric::default()
    };

    if let Some((total, matched)) = pruning_pair(plan, "row_groups_pruned_statistics") {
        metric.row_groups_total = Some(total);
        metric.row_groups_matched = Some(matched);
    }
    if let Some((total, matched)) = pruning_pair(plan, "files_ranges_pruned_statistics") {
        metric.files_ranges_total = Some(total);
        metric.files_ranges_matched = Some(matched);
    }
    metric
}

fn metric_value(plan: &str, metric_name: &str) -> Option<u64> {
    let needle = format!("{metric_name}=");
    let start = plan.find(&needle)? + needle.len();
    parse_u64_prefix(&plan[start..])
}

fn pruning_pair(plan: &str, metric_name: &str) -> Option<(u64, u64)> {
    let needle = format!("{metric_name}=");
    let start = plan.find(&needle)? + needle.len();
    let rest = &plan[start..];
    let total = parse_u64_prefix(rest)?;
    let matched_needle = " matched";
    let matched_end = rest.find(matched_needle)?;
    let before_matched = &rest[..matched_end];
    let matched = parse_last_u64(before_matched)?;
    Some((total, matched))
}

fn parse_u64_prefix(value: &str) -> Option<u64> {
    let digits = value
        .chars()
        .skip_while(|ch| ch.is_whitespace())
        .take_while(|ch| ch.is_ascii_digit() || *ch == ',')
        .filter(|ch| *ch != ',')
        .collect::<String>();
    (!digits.is_empty()).then(|| digits.parse().ok()).flatten()
}

fn parse_last_u64(value: &str) -> Option<u64> {
    let mut end = None;
    for (idx, ch) in value.char_indices().rev() {
        if ch.is_ascii_digit() || ch == ',' {
            end = Some(idx + ch.len_utf8());
            break;
        }
    }
    let end = end?;
    let start = value[..end]
        .char_indices()
        .rev()
        .find(|(_, ch)| !ch.is_ascii_digit() && *ch != ',')
        .map(|(idx, ch)| idx + ch.len_utf8())
        .unwrap_or(0);
    value[start..end].replace(',', "").parse().ok()
}

fn subtract(total: Option<u64>, matched: Option<u64>) -> Option<u64> {
    total
        .zip(matched)
        .map(|(total, matched)| total.saturating_sub(matched))
}

fn fmt_opt(value: Option<u64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "NA".to_string())
}

fn pct(part: i64, whole: i64) -> f64 {
    if whole == 0 {
        0.0
    } else {
        part as f64 * 100.0 / whole as f64
    }
}

fn table_name(prefix: &str, suffix: &str) -> String {
    quote_ident(&format!("{prefix}_{suffix}"))
}

fn envelope_wkb_sql(rect: Rect) -> String {
    format!(
        "ST_MakeEnvelope({}, {}, {}, {}, 3857)",
        rect.minx, rect.miny, rect.maxx, rect.maxy
    )
}

fn rect_wkb_sql(rect: Rect) -> String {
    format!("X'{}'", hex_encode(&rect_wkb(rect)))
}

fn rect_wkb(rect: Rect) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(1 + 4 + 4 + 4 + 5 * 16);
    bytes.push(1);
    bytes.extend_from_slice(&3_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u32.to_le_bytes());
    bytes.extend_from_slice(&5_u32.to_le_bytes());
    for (x, y) in [
        (rect.minx, rect.miny),
        (rect.maxx, rect.miny),
        (rect.maxx, rect.maxy),
        (rect.minx, rect.maxy),
        (rect.minx, rect.miny),
    ] {
        bytes.extend_from_slice(&x.to_le_bytes());
        bytes.extend_from_slice(&y.to_le_bytes());
    }
    bytes
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

fn escape_sql(value: &str) -> String {
    value.replace('\'', "''")
}

fn copy_line(fields: &[String]) -> String {
    let mut out = fields.join("\t");
    out.push('\n');
    out
}

fn copy_text(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ => out.push(ch),
        }
    }
    out
}

fn copy_bytea_hex(bytes: &[u8]) -> String {
    format!("\\\\x{}", hex_encode(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_datafusion_explain_analyze_scan_metrics() {
        let plan = "Plan with Metrics\n\
            DataSourceExec: file_groups={1 group: [[file.parquet]]}, \
            projection=[geom], predicate=_qg_minx@1 <= 290 AND _qg_maxx@2 >= 95, \
            metrics=[output_rows=18, bytes_scanned=12,345, \
            row_groups_pruned_statistics=4 total → 2 matched, \
            files_ranges_pruned_statistics=3 total → 1 matched, \
            pushdown_rows_matched=18, pushdown_rows_pruned=12] \
            FilterExec: _qg_miny@3 <= 185 AND _qg_maxy@4 >= 95";

        let metric = scan_metric_from_explain(plan);

        assert_eq!(metric.output_rows, Some(18));
        assert_eq!(metric.bytes_scanned, Some(12_345));
        assert_eq!(metric.row_groups_total, Some(4));
        assert_eq!(metric.row_groups_matched, Some(2));
        assert_eq!(metric.row_groups_pruned(), Some(2));
        assert_eq!(metric.files_ranges_total, Some(3));
        assert_eq!(metric.files_ranges_matched, Some(1));
        assert_eq!(metric.files_ranges_pruned(), Some(2));
        assert_eq!(metric.pushdown_rows_matched, Some(18));
        assert_eq!(metric.pushdown_rows_pruned, Some(12));
        assert!(metric.hidden_bbox_predicate);
        assert!(metric.parquet_predicate);
    }

    #[test]
    fn missing_scan_metrics_render_as_unknowns() {
        let metric = scan_metric_from_explain("ProjectionExec: expr=[count(*)]");

        assert_eq!(metric, ScanMetric::default());
        assert_eq!(fmt_opt(metric.bytes_scanned), "NA");
    }
}
