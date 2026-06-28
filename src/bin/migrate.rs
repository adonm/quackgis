// SPDX-License-Identifier: Apache-2.0
//
//! `sedonadb-migrate` — PostGIS → DuckDB SQL migration assistant.
//!
//! Reads PostGIS SQL, applies the shared Rust AST rewriter, and emits:
//!   - rewritten DuckDB SQL (`--out`, or stdout if omitted)
//!   - a Markdown review report (`--report`)
//!
//! The report includes per-line confidence levels, items requiring manual
//! review, and DuckLake layout hints for spatial tables.
//!
//! Usage:
//! ```sh
//! sedonadb-migrate input.sql --out output.sql --report report.md
//! sedonadb-migrate input.sql                    # rewritten SQL to stdout
//! ```

// Include the rewriter module directly (no library link) — it depends only on
// `sqlparser`, keeping the binary build light and independent of the extension's
// GDAL/GEOS/PROJ stack.
#[path = "../rewriter.rs"]
mod rewriter;

use rewriter::{Confidence, RewriteKind};
use std::env;
use std::fmt::Write;
use std::fs;
use std::process::ExitCode;

fn print_usage() {
    eprintln!(
        "Usage: sedonadb-migrate <input.sql> [--out <output.sql>] [--report <report.md>]"
    );
    eprintln!(
        "\nRewrites PostGIS SQL to DuckDB-compatible SQL using the Rust AST engine."
    );
    eprintln!("Outputs rewritten SQL to stdout when --out is omitted.");
}

struct Args {
    input: String,
    out: Option<String>,
    report: Option<String>,
}

fn parse_args() -> Result<Args, ExitCode> {
    let mut args = env::args().skip(1);
    let input = match args.next() {
        Some(s) if !s.starts_with("--") => s,
        _ => {
            print_usage();
            return Err(ExitCode::from(1));
        }
    };
    let mut out = None;
    let mut report = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--out" => {
                out = args.next();
            }
            "--report" => {
                report = args.next();
            }
            "-h" | "--help" => {
                print_usage();
                return Err(ExitCode::from(0));
            }
            _ => {
                eprintln!("Unknown argument: {arg}");
                print_usage();
                return Err(ExitCode::from(1));
            }
        }
    }
    Ok(Args { input, out, report })
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(code) => return code,
    };

    let sql = match fs::read_to_string(&args.input) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Error reading {}: {e}", args.input);
            return ExitCode::from(2);
        }
    };

    let result = rewriter::rewrite_postgis_detailed(&sql);

    // Emit rewritten SQL.
    let output = if result.parse_error.is_some() {
        eprintln!(
            "Warning: parse error in input ({}); original SQL preserved.",
            result.parse_error.as_ref().unwrap()
        );
        sql.clone()
    } else {
        result.rewritten_sql.clone()
    };

    match &args.out {
        Some(path) => {
            if let Err(e) = fs::write(path, &output) {
                eprintln!("Error writing {path}: {e}");
                return ExitCode::from(3);
            }
        }
        None => {
            println!("{output}");
        }
    }

    // Emit report.
    if let Some(path) = &args.report {
        let report = generate_report(&args.input, &sql, &result);
        if let Err(e) = fs::write(path, &report) {
            eprintln!("Error writing {path}: {e}");
            return ExitCode::from(3);
        }
    }

    // Summary to stderr.
    let mech = result.mechanical_count();
    let review = result.review_events().len();
    eprintln!(
        "sedonadb-migrate: {mech} mechanical rewrite(s), {review} item(s) need review."
    );
    if review > 0 {
        ExitCode::from(0) // success, but user should review the report
    } else {
        ExitCode::from(0)
    }
}

/// Generate a Markdown review report.
fn generate_report(input: &str, original_sql: &str, result: &rewriter::RewriteResult) -> String {
    let mut r = String::new();
    let mech = result.mechanical_count();
    let review = result.review_events().len();
    let stmt_count = original_sql.lines().filter(|l| {
        let t = l.trim();
        !t.is_empty() && !t.starts_with("--")
    }).count();

    writeln!(r, "# sedonadb-migrate report").unwrap();
    writeln!(r).unwrap();
    writeln!(r, "**Input:** `{input}`").unwrap();
    writeln!(r, "**Statements:** ~{stmt_count}").unwrap();
    writeln!(r, "**Mechanical rewrites:** {mech} (high confidence)").unwrap();
    writeln!(r, "**Needs review:** {review}").unwrap();
    writeln!(r).unwrap();

    if let Some(e) = &result.parse_error {
        writeln!(r, "## ⚠️ Parse error").unwrap();
        writeln!(r).unwrap();
        writeln!(r, "The input could not be parsed as PostgreSQL SQL:").unwrap();
        writeln!(r, "> {e}").unwrap();
        writeln!(r).unwrap();
        writeln!(r, "Original SQL is preserved in the output. Fix the syntax and re-run.").unwrap();
        return r;
    }

    // Events table.
    if !result.events.is_empty() {
        writeln!(r, "## Rewrites").unwrap();
        writeln!(r).unwrap();
        writeln!(r, "| Line | Kind | Confidence | Description |").unwrap();
        writeln!(r, "|------|------|------------|-------------|").unwrap();
        for e in &result.events {
            let kind = kind_label(e.kind);
            let conf = match e.confidence {
                Confidence::High => "✅ High",
                Confidence::NeedsReview => "⚠️ Review",
            };
            let line = if e.line > 0 { e.line.to_string() } else { "?".to_string() };
            writeln!(r, "| {line} | {kind} | {conf} | {} |", e.description).unwrap();
        }
        writeln!(r).unwrap();
    }

    // Review section.
    let reviews = result.review_events();
    if !reviews.is_empty() {
        writeln!(r, "## Items requiring manual review").unwrap();
        writeln!(r).unwrap();
        for (i, e) in reviews.iter().enumerate() {
            let line = if e.line > 0 { e.line.to_string() } else { "?".to_string() };
            writeln!(r, "{}. **Line {line}:** {}", i + 1, e.description).unwrap();
        }
        writeln!(r).unwrap();
    }

    // DuckLake layout hints.
    let hints = layout_hints(original_sql);
    if !hints.is_empty() {
        writeln!(r, "## DuckLake layout hints").unwrap();
        writeln!(r).unwrap();
        for h in &hints {
            writeln!(r, "- {h}").unwrap();
        }
        writeln!(r).unwrap();
    }

    if result.events.is_empty() {
        writeln!(r, "No rewrites needed — the SQL is already DuckDB-compatible.").unwrap();
    }

    r
}

fn kind_label(kind: RewriteKind) -> &'static str {
    match kind {
        RewriteKind::OperatorOverlap => "Operator → Function",
        RewriteKind::OperatorDistance => "Operator → Function",
        RewriteKind::CastGeometry => "Cast removal",
        RewriteKind::CastGeography => "Cast removal",
        RewriteKind::FunctionRename => "Function rename",
        RewriteKind::GiSTIndex => "Index",
        RewriteKind::AggregateOrderBy => "Aggregate ORDER BY",
    }
}

/// Heuristic DuckLake layout recommendations based on SQL text scanning.
fn layout_hints(sql: &str) -> Vec<String> {
    let mut hints = Vec::new();
    let lower = sql.to_lowercase();

    // Detect CREATE TABLE with geometry/geography columns.
    let has_spatial_col = lower.contains("geometry") || lower.contains("geography");
    let has_create = lower.contains("create table");

    if has_create && has_spatial_col {
        hints.push(
            "Detected spatial column in CREATE TABLE. Add bbox prefilter columns for zone-map pruning:\n\
             ```sql\n\
             ALTER TABLE t ADD COLUMN minx DOUBLE, maxx DOUBLE, miny DOUBLE, maxy DOUBLE;\n\
             UPDATE t SET minx = st_xmin(geom), maxx = st_xmax(geom),\n\
                    miny = st_ymin(geom), maxy = st_ymax(geom);\n\
             ```"
                .to_string(),
        );
    }

    if lower.contains("using gist") {
        hints.push(
            "GiST index detected. DuckDB has no GiST — use bbox columns + DuckDB IEJoin for spatial\n\
             overlap, or `sedona_join()` for spatial joins. For large tables, partition by a\n\
             spatial key (geohash/Hilbert) and sort within partitions for file-level pruning."
                .to_string(),
        );
    }

    // Detect ORDER BY ... <-> (KNN) which needs a different approach in DuckDB.
    if lower.contains("<->") || lower.contains("<#>") {
        hints.push(
            "KNN distance operator detected. DuckDB cannot register binary operators from\n\
             extensions — `sedonadb_rewrite_postgis()` maps `<->` to `st_distance()`. For\n\
             ORDER BY ... LIMIT K queries, ensure bbox columns are present for prefiltering."
                .to_string(),
        );
    }

    hints
}
