// SPDX-License-Identifier: Apache-2.0
//! SQL-level spatial pruning rewrites.
//!
//! This keeps correctness in the original SedonaDB predicate and only injects a
//! hidden-layout bbox prefilter for simple single-table DuckLake queries where
//! the query envelope is statically visible.

use std::sync::LazyLock;

use datafusion::arrow::datatypes::Schema;
use datafusion::prelude::SessionContext;
use datafusion::sql::sqlparser::ast::Statement;
use regex::Regex;

use crate::context::DUCKLAKE_CATALOG;

use super::layout;
use super::names::{ducklake_table_ref, quote_ident};

const WEB_MERCATOR_WORLD: f64 = 20_037_508.342_789_244;

#[derive(Debug, Clone)]
struct RewriteTarget {
    table: String,
    table_span: std::ops::Range<usize>,
    qualifier: Option<String>,
}

#[derive(Debug, Clone, Copy)]
struct Envelope {
    minx: f64,
    miny: f64,
    maxx: f64,
    maxy: f64,
}

pub(super) async fn rewrite_spatial_pruning_query(
    statement: &Statement,
    session_context: &SessionContext,
) -> Option<String> {
    if !matches!(statement, Statement::Query(_)) {
        return None;
    }

    let sql = statement.to_string();
    let sql_lower = sql.to_ascii_lowercase();
    if sql_lower.contains("_qg_")
        || sql_lower.contains(" join ")
        || sql_lower.contains(" union ")
        || !sql_lower.contains(" where ")
    {
        return None;
    }

    let envelope = extract_envelope(&sql)?;
    let target = rewrite_target(&sql)?;
    let table_schema = ducklake_table_schema(session_context, &target.table).await?;
    if !has_layout_columns(table_schema.as_ref())
        || !mentions_spatial_predicate(&sql, table_schema.as_ref())
    {
        return None;
    }

    let bbox_predicate = bbox_predicate(envelope, target.qualifier.as_deref());
    inject_bbox_predicate(&sql, &target, &bbox_predicate)
}

async fn ducklake_table_schema(
    session_context: &SessionContext,
    table: &str,
) -> Option<datafusion::arrow::datatypes::SchemaRef> {
    let catalog = session_context.catalog(DUCKLAKE_CATALOG)?;
    let schema = catalog.schema("main")?;
    let table = schema.table(table).await.ok().flatten()?;
    Some(table.schema())
}

fn has_layout_columns(schema: &Schema) -> bool {
    [layout::MINX, layout::MINY, layout::MAXX, layout::MAXY]
        .into_iter()
        .all(|name| {
            schema
                .fields()
                .iter()
                .any(|field| field.name().eq_ignore_ascii_case(name))
        })
}

fn mentions_spatial_predicate(sql: &str, schema: &Schema) -> bool {
    let sql_lower = sql.to_ascii_lowercase();
    if !(sql_lower.contains("st_intersects") || sql_lower.contains("&&")) {
        return false;
    }
    schema.fields().iter().any(|field| {
        if layout::is_layout_column(field.name()) {
            return false;
        }
        (crate::geometry_columns::is_geometry_column_name(field.name())
            || field.name().eq_ignore_ascii_case("footprint"))
            && mentions_column(&sql_lower, field.name())
    })
}

fn mentions_column(sql_lower: &str, column: &str) -> bool {
    let pattern = format!(
        r#"(?i)(?:\bst_geomfromwkb\s*\(\s*)?(?:(?:"[^"]+"|[a-z_][\w$]*)\s*\.\s*)?(?:"{}"|\b{}\b)"#,
        regex::escape(column),
        regex::escape(&column.to_ascii_lowercase())
    );
    Regex::new(&pattern)
        .expect("valid geometry column regex")
        .is_match(sql_lower)
}

fn rewrite_target(sql: &str) -> Option<RewriteTarget> {
    static FROM_RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r#"(?is)\bfrom\s+(?P<table>(?:"[^"]+"|[a-z_][\w$]*)(?:\s*\.\s*(?:"[^"]+"|[a-z_][\w$]*)){0,2})"#,
        )
        .expect("valid FROM regex")
    });

    let captures = FROM_RE.captures(sql)?;
    let table_match = captures.name("table")?;
    let table_parts = object_parts(table_match.as_str());
    let table = ducklake_table_name(&table_parts)?;
    let qualifier = alias_after_table(&sql[table_match.end()..]).map(quote_ident_or_passthrough);
    Some(RewriteTarget {
        table,
        table_span: table_match.start()..table_match.end(),
        qualifier,
    })
}

fn object_parts(table_ref: &str) -> Vec<String> {
    table_ref
        .split('.')
        .map(|part| part.trim().trim_matches('"').to_string())
        .collect()
}

fn ducklake_table_name(parts: &[String]) -> Option<String> {
    match parts {
        [table] => Some(table.clone()),
        [schema, table] if is_ducklake_schema(schema) => Some(table.clone()),
        [catalog, schema, table]
            if catalog.eq_ignore_ascii_case(DUCKLAKE_CATALOG) && is_ducklake_schema(schema) =>
        {
            Some(table.clone())
        }
        _ => None,
    }
}

fn is_ducklake_schema(schema: &str) -> bool {
    schema.eq_ignore_ascii_case("main") || schema.eq_ignore_ascii_case("public")
}

fn alias_after_table(rest: &str) -> Option<&str> {
    static ALIAS_RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?is)^\s+(?:as\s+)?(?P<alias>"[^"]+"|[a-z_][\w$]*)"#)
            .expect("valid alias regex")
    });
    let alias = ALIAS_RE.captures(rest)?.name("alias")?.as_str();
    (!is_sql_keyword(alias)).then_some(alias)
}

fn is_sql_keyword(token: &str) -> bool {
    matches!(
        token.trim_matches('"').to_ascii_lowercase().as_str(),
        "where"
            | "join"
            | "left"
            | "right"
            | "inner"
            | "outer"
            | "cross"
            | "group"
            | "order"
            | "limit"
            | "offset"
            | "fetch"
            | "having"
            | "union"
    )
}

fn quote_ident_or_passthrough(alias: &str) -> String {
    if alias.starts_with('"') && alias.ends_with('"') {
        alias.to_string()
    } else {
        quote_ident(alias)
    }
}

fn extract_envelope(sql: &str) -> Option<Envelope> {
    extract_tile_envelope(sql).or_else(|| extract_make_envelope(sql))
}

fn extract_make_envelope(sql: &str) -> Option<Envelope> {
    static MAKE_ENVELOPE_RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(&format!(
            r#"(?is)\bst_makeenvelope\s*\(\s*({n})\s*,\s*({n})\s*,\s*({n})\s*,\s*({n})(?:\s*,[^)]*)?\)"#,
            n = NUMBER_RE
        ))
        .expect("valid ST_MakeEnvelope regex")
    });
    let captures = MAKE_ENVELOPE_RE.captures(sql)?;
    Some(Envelope {
        minx: parse_capture(&captures, 1)?,
        miny: parse_capture(&captures, 2)?,
        maxx: parse_capture(&captures, 3)?,
        maxy: parse_capture(&captures, 4)?,
    })
}

fn extract_tile_envelope(sql: &str) -> Option<Envelope> {
    static TILE_ENVELOPE_RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(&format!(
            r#"(?is)\bst_tileenvelope\s*\(\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)(?:\s*,\s*st_makeenvelope\s*\(\s*({n})\s*,\s*({n})\s*,\s*({n})\s*,\s*({n})(?:\s*,[^)]*)?\))?(?:\s*,\s*(?:margin\s*=>\s*)?({n}))?\s*\)"#,
            n = NUMBER_RE
        ))
        .expect("valid ST_TileEnvelope regex")
    });
    let captures = TILE_ENVELOPE_RE.captures(sql)?;
    let z = parse_capture::<i32>(&captures, 1)?;
    let x = parse_capture::<i64>(&captures, 2)?;
    let y = parse_capture::<i64>(&captures, 3)?;
    if z < 0 {
        return None;
    }
    let bounds = if captures.get(4).is_some() {
        Envelope {
            minx: parse_capture(&captures, 4)?,
            miny: parse_capture(&captures, 5)?,
            maxx: parse_capture(&captures, 6)?,
            maxy: parse_capture(&captures, 7)?,
        }
    } else {
        Envelope {
            minx: -WEB_MERCATOR_WORLD,
            miny: -WEB_MERCATOR_WORLD,
            maxx: WEB_MERCATOR_WORLD,
            maxy: WEB_MERCATOR_WORLD,
        }
    };
    let margin = captures
        .get(8)
        .and_then(|m| m.as_str().parse::<f64>().ok())
        .unwrap_or(0.0);
    tile_envelope(z, x, y, bounds, margin)
}

const NUMBER_RE: &str = r#"[-+]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][-+]?\d+)?"#;

fn parse_capture<T: std::str::FromStr>(captures: &regex::Captures<'_>, idx: usize) -> Option<T> {
    captures.get(idx)?.as_str().parse::<T>().ok()
}

fn tile_envelope(z: i32, x: i64, y: i64, bounds: Envelope, margin: f64) -> Option<Envelope> {
    let tiles = 2.0_f64.powi(z);
    if !tiles.is_finite() || tiles <= 0.0 {
        return None;
    }
    let width = (bounds.maxx - bounds.minx) / tiles;
    let height = (bounds.maxy - bounds.miny) / tiles;
    let expand_x = margin * width;
    let expand_y = margin * height;
    Some(Envelope {
        minx: bounds.minx + (x as f64 * width) - expand_x,
        maxx: bounds.minx + ((x + 1) as f64 * width) + expand_x,
        miny: bounds.maxy - ((y + 1) as f64 * height) - expand_y,
        maxy: bounds.maxy - (y as f64 * height) + expand_y,
    })
}

fn bbox_predicate(envelope: Envelope, qualifier: Option<&str>) -> String {
    let col = |name: &str| {
        qualifier
            .map(|qualifier| format!("{qualifier}.{}", quote_ident(name)))
            .unwrap_or_else(|| quote_ident(name))
    };
    format!(
        "{} <= {} AND {} >= {} AND {} <= {} AND {} >= {}",
        col(layout::MINX),
        envelope.maxx,
        col(layout::MAXX),
        envelope.minx,
        col(layout::MINY),
        envelope.maxy,
        col(layout::MAXY),
        envelope.miny,
    )
}

fn inject_bbox_predicate(
    sql: &str,
    target: &RewriteTarget,
    bbox_predicate: &str,
) -> Option<String> {
    static WHERE_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"(?is)\bwhere\b"#).expect("valid WHERE regex"));
    static CLAUSE_END_RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?is)\b(group\s+by|order\s+by|having|limit|offset|fetch)\b"#)
            .expect("valid clause-end regex")
    });

    let where_match = WHERE_RE.find(sql)?;
    let after_where = where_match.end();
    let suffix_start = CLAUSE_END_RE
        .find(&sql[after_where..])
        .map(|m| after_where + m.start())
        .unwrap_or(sql.len());
    let existing_predicate = sql[after_where..suffix_start].trim();
    if existing_predicate.is_empty() {
        return None;
    }

    let mut rewritten = sql.to_string();
    rewritten.replace_range(
        target.table_span.clone(),
        &ducklake_table_ref("main", &target.table),
    );
    let adjusted_where = if target.table_span.end <= after_where {
        let new_table_ref_len = ducklake_table_ref("main", &target.table).len();
        let replaced_len = target.table_span.end - target.table_span.start;
        (after_where as isize + new_table_ref_len as isize - replaced_len as isize) as usize
    } else {
        after_where
    };
    let adjusted_suffix = if target.table_span.end <= suffix_start {
        let new_table_ref_len = ducklake_table_ref("main", &target.table).len();
        let replaced_len = target.table_span.end - target.table_span.start;
        (suffix_start as isize + new_table_ref_len as isize - replaced_len as isize) as usize
    } else {
        suffix_start
    };
    rewritten.replace_range(
        adjusted_where..adjusted_suffix,
        &format!(" ({existing_predicate}) AND ({bbox_predicate}) "),
    );
    Some(rewritten)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn injects_bbox_and_internal_table_for_make_envelope() {
        let sql = "SELECT COUNT(*) FROM public.frames WHERE mission = 'a' AND \
                   ST_Intersects(ST_GeomFromWKB(geom), \
                   ST_GeomFromWKB(ST_MakeEnvelope(1, 2, 3, 4, 3857)))";
        let target = rewrite_target(sql).expect("target");
        let envelope = extract_envelope(sql).expect("envelope");
        let rewritten =
            inject_bbox_predicate(sql, &target, &bbox_predicate(envelope, None)).expect("rewrite");
        assert!(rewritten.contains("quackgis.\"main\".\"frames\""));
        assert!(rewritten.contains("\"_qg_minx\" <= 3"));
        assert!(rewritten.contains("\"_qg_maxy\" >= 2"));
        assert!(rewritten.contains("ST_Intersects"));
    }

    #[test]
    fn extracts_default_tile_envelope() {
        let envelope = extract_envelope("SELECT * FROM t WHERE geom && ST_TileEnvelope(0, 0, 0)")
            .expect("tile envelope");
        assert_eq!(envelope.minx, -WEB_MERCATOR_WORLD);
        assert_eq!(envelope.maxx, WEB_MERCATOR_WORLD);
        assert_eq!(envelope.miny, -WEB_MERCATOR_WORLD);
        assert_eq!(envelope.maxy, WEB_MERCATOR_WORLD);
    }
}
