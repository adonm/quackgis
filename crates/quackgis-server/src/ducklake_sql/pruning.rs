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
    if !is_rewritable_statement(statement) {
        return None;
    }

    let sql = statement.to_string();
    let sql_lower = sql.to_ascii_lowercase();
    if sql_lower.contains("_qg_") || sql_lower.contains(" union ") || !sql_lower.contains(" where ")
    {
        return None;
    }

    let envelope = extract_envelope(&sql)?;
    for target in rewrite_targets(&sql) {
        if projection_has_wildcard_for_target(&sql, &target)
            || target_has_same_level_join(&sql, &target)
        {
            continue;
        }
        let table_schema = ducklake_table_schema(session_context, &target.table).await?;
        if !has_layout_columns(table_schema.as_ref())
            || !mentions_spatial_predicate(&sql, table_schema.as_ref())
        {
            continue;
        }

        let bbox_predicate = bbox_predicate(envelope, target.qualifier.as_deref());
        if let Some(rewritten) = inject_bbox_predicate(&sql, &target, &bbox_predicate) {
            return Some(rewritten);
        }
    }
    None
}

fn is_rewritable_statement(statement: &Statement) -> bool {
    match statement {
        Statement::Query(_) => true,
        Statement::Explain { statement, .. } => matches!(statement.as_ref(), Statement::Query(_)),
        _ => false,
    }
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

fn rewrite_targets(sql: &str) -> Vec<RewriteTarget> {
    static FROM_RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r#"(?is)\bfrom\s+(?P<table>(?:"[^"]+"|[a-z_][\w$]*)(?:\s*\.\s*(?:"[^"]+"|[a-z_][\w$]*)){0,2})"#,
        )
        .expect("valid FROM regex")
    });

    FROM_RE
        .captures_iter(sql)
        .filter_map(|captures| {
            let table_match = captures.name("table")?;
            if !is_code_at(sql, table_match.start()) {
                return None;
            }
            let table_parts = object_parts(table_match.as_str());
            let table = ducklake_table_name(&table_parts)?;
            let qualifier =
                alias_after_table(&sql[table_match.end()..]).map(quote_ident_or_passthrough);
            Some(RewriteTarget {
                table,
                table_span: table_match.start()..table_match.end(),
                qualifier,
            })
        })
        .collect()
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

fn projection_has_wildcard_for_target(sql: &str, target: &RewriteTarget) -> bool {
    let depth = depth_at(sql, target.table_span.start);
    let Some(select_start) =
        last_keyword_at_depth_before(sql, "select", target.table_span.start, depth)
    else {
        return false;
    };
    let Some(from_start) = find_keyword_at_depth(sql, "from", select_start + "select".len(), depth)
    else {
        return false;
    };
    span_has_wildcard_at_depth(sql, select_start + "select".len(), from_start, depth)
}

#[cfg(test)]
fn projection_has_top_level_wildcard(sql: &str) -> bool {
    let Some(select_start) = find_keyword_at_depth(sql, "select", 0, 0) else {
        return false;
    };
    let Some(from_start) = find_keyword_at_depth(sql, "from", select_start + "select".len(), 0)
    else {
        return false;
    };
    span_has_wildcard_at_depth(sql, select_start + "select".len(), from_start, 0)
}

fn span_has_wildcard_at_depth(sql: &str, start: usize, end: usize, target_depth: i32) -> bool {
    let bytes = sql.as_bytes();
    let mut state = ScanState::default();
    let mut i = 0;
    while i < end.min(bytes.len()) {
        if i >= start && state.is_code() && state.depth == target_depth && bytes[i] == b'*' {
            return true;
        }
        i = state.advance(bytes, i);
    }
    false
}

fn target_has_same_level_join(sql: &str, target: &RewriteTarget) -> bool {
    let depth = depth_at(sql, target.table_span.start);
    let start = target.table_span.end;
    let end = where_start_after_target(sql, target, depth)
        .or_else(|| statement_segment_end(sql, start, depth))
        .unwrap_or(sql.len());
    ["join", "left", "right", "inner", "outer", "cross", "full"]
        .into_iter()
        .any(|keyword| {
            find_keyword_at_depth(sql, keyword, start, depth).is_some_and(|idx| idx < end)
        })
}

#[derive(Debug, Default, Clone, Copy)]
struct ScanState {
    depth: i32,
    in_single_quote: bool,
    in_double_quote: bool,
    in_line_comment: bool,
    in_block_comment: bool,
}

impl ScanState {
    fn is_code(self) -> bool {
        !self.in_single_quote
            && !self.in_double_quote
            && !self.in_line_comment
            && !self.in_block_comment
    }

    fn advance(&mut self, bytes: &[u8], i: usize) -> usize {
        if i >= bytes.len() {
            return i + 1;
        }
        if self.in_line_comment {
            if bytes[i] == b'\n' {
                self.in_line_comment = false;
            }
            return i + 1;
        }
        if self.in_block_comment {
            if bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'/') {
                self.in_block_comment = false;
                return i + 2;
            }
            return i + 1;
        }
        if self.in_single_quote {
            if bytes[i] == b'\'' {
                if bytes.get(i + 1) == Some(&b'\'') {
                    return i + 2;
                }
                self.in_single_quote = false;
            }
            return i + 1;
        }
        if self.in_double_quote {
            if bytes[i] == b'"' {
                if bytes.get(i + 1) == Some(&b'"') {
                    return i + 2;
                }
                self.in_double_quote = false;
            }
            return i + 1;
        }

        match bytes[i] {
            b'-' if bytes.get(i + 1) == Some(&b'-') => {
                self.in_line_comment = true;
                i + 2
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                self.in_block_comment = true;
                i + 2
            }
            b'\'' => {
                self.in_single_quote = true;
                i + 1
            }
            b'"' => {
                self.in_double_quote = true;
                i + 1
            }
            b'(' => {
                self.depth += 1;
                i + 1
            }
            b')' => {
                self.depth = (self.depth - 1).max(0);
                i + 1
            }
            _ => i + 1,
        }
    }
}

fn depth_at(sql: &str, idx: usize) -> i32 {
    let bytes = sql.as_bytes();
    let mut state = ScanState::default();
    let mut i = 0;
    while i < idx.min(bytes.len()) {
        i = state.advance(bytes, i);
    }
    state.depth
}

fn is_code_at(sql: &str, idx: usize) -> bool {
    let bytes = sql.as_bytes();
    let mut state = ScanState::default();
    let mut i = 0;
    while i < idx.min(bytes.len()) {
        i = state.advance(bytes, i);
    }
    state.is_code()
}

fn find_keyword_at_depth(
    sql: &str,
    keyword: &str,
    start: usize,
    target_depth: i32,
) -> Option<usize> {
    let lower = sql.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let keyword = keyword.as_bytes();
    let mut state = ScanState::default();
    let mut i = 0;
    while i + keyword.len() <= bytes.len() {
        if i >= start
            && state.is_code()
            && state.depth == target_depth
            && bytes[i..].starts_with(keyword)
            && is_keyword_boundary(bytes.get(i.wrapping_sub(1)).copied())
            && is_keyword_boundary(bytes.get(i + keyword.len()).copied())
        {
            return Some(i);
        }
        i = state.advance(bytes, i);
    }
    None
}

fn last_keyword_at_depth_before(
    sql: &str,
    keyword: &str,
    before: usize,
    target_depth: i32,
) -> Option<usize> {
    let mut last = None;
    let mut start = 0;
    while let Some(idx) = find_keyword_at_depth(sql, keyword, start, target_depth) {
        if idx >= before {
            break;
        }
        last = Some(idx);
        start = idx + keyword.len();
    }
    last
}

fn is_keyword_boundary(ch: Option<u8>) -> bool {
    ch.is_none_or(|ch| !ch.is_ascii_alphanumeric() && ch != b'_')
}

fn statement_segment_end(sql: &str, start: usize, target_depth: i32) -> Option<usize> {
    let bytes = sql.as_bytes();
    let mut state = ScanState::default();
    let mut i = 0;
    while i < bytes.len() {
        if i >= start && state.is_code() && state.depth < target_depth {
            return Some(i);
        }
        if i >= start && state.is_code() && state.depth == target_depth && bytes[i] == b')' {
            return Some(i);
        }
        i = state.advance(bytes, i);
    }
    None
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
    let depth = depth_at(sql, target.table_span.start);
    let where_start = where_start_after_target(sql, target, depth)?;
    let after_where = where_start + "where".len();
    let suffix_start = predicate_end(sql, after_where, depth);
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

fn where_start_after_target(sql: &str, target: &RewriteTarget, depth: i32) -> Option<usize> {
    let segment_end = statement_segment_end(sql, target.table_span.end, depth).unwrap_or(sql.len());
    find_keyword_at_depth(sql, "where", target.table_span.end, depth)
        .filter(|idx| *idx < segment_end)
}

fn predicate_end(sql: &str, predicate_start: usize, depth: i32) -> usize {
    let clause_keywords = ["group", "order", "having", "limit", "offset", "fetch"];
    clause_keywords
        .into_iter()
        .filter_map(|keyword| find_keyword_at_depth(sql, keyword, predicate_start, depth))
        .chain(statement_segment_end(sql, predicate_start, depth))
        .min()
        .unwrap_or(sql.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn injects_bbox_and_internal_table_for_make_envelope() {
        let sql = "SELECT COUNT(*) FROM public.frames WHERE mission = 'a' AND \
                   ST_Intersects(ST_GeomFromWKB(geom), \
                   ST_GeomFromWKB(ST_MakeEnvelope(1, 2, 3, 4, 3857)))";
        let target = rewrite_targets(sql).pop().expect("target");
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

    #[test]
    fn wildcard_projection_is_not_rewritten() {
        assert!(projection_has_top_level_wildcard(
            "SELECT f.* FROM frames AS f WHERE ST_Intersects(geom, ST_MakeEnvelope(1,2,3,4,3857))"
        ));
        assert!(!projection_has_top_level_wildcard(
            "SELECT COUNT(*) FROM frames WHERE ST_Intersects(geom, ST_MakeEnvelope(1,2,3,4,3857))"
        ));
    }

    #[test]
    fn injects_inside_derived_query_at_matching_depth() {
        let sql = "SELECT COUNT(*) FROM (SELECT id FROM public.frames AS f \
                   WHERE f.geom && ST_TileEnvelope(0, 0, 0) ORDER BY id) AS q";
        let target = rewrite_targets(sql).pop().expect("target");
        let envelope = extract_envelope(sql).expect("envelope");
        let rewritten = inject_bbox_predicate(
            sql,
            &target,
            &bbox_predicate(envelope, target.qualifier.as_deref()),
        )
        .expect("rewrite");
        assert!(rewritten.contains("FROM quackgis.\"main\".\"frames\" AS f"));
        assert!(rewritten.contains("\"f\".\"_qg_minx\""));
        assert!(rewritten.contains(" ORDER BY id"));
    }

    #[test]
    fn scanner_ignores_comments_and_string_literals() {
        let sql = "SELECT COUNT('* from nope') FROM public.frames -- FROM ignored\n\
                   WHERE geom && ST_TileEnvelope(0, 0, 0) /* WHERE ignored */";
        let targets = rewrite_targets(sql);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].table, "frames");
        assert!(!projection_has_top_level_wildcard(sql));
    }

    #[test]
    fn join_detection_stops_before_where_predicate() {
        let sql = "SELECT COUNT(*) FROM public.frames \
                   WHERE left(label, 1) = 'a' \
                   AND geom && ST_TileEnvelope(0, 0, 0)";
        let target = rewrite_targets(sql).pop().expect("target");
        assert!(!target_has_same_level_join(sql, &target));
    }
}
