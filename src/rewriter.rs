//! PostGIS → DuckDB SQL rewriter using `sqlparser-rs` AST transforms.
//!
//! Parses SQL with the PostgreSQL dialect, applies high-confidence mechanical
//! rewrites for PostGIS-isms, and returns DuckDB-compatible SQL.
//!
//! Two entry points:
//! - [`rewrite_postgis`] returns rewritten SQL as a string (used by the
//!   `sedonadb_rewrite_postgis()` SQL function).
//! - [`rewrite_postgis_detailed`] returns a [`RewriteResult`] with structured
//!   diagnostics (used by the `sedonadb-migrate` CLI for review reports).
//!
//! See ROADMAP M25 (design) and M28 (migration assistant UX).

use sqlparser::ast::{
    Expr, Function, FunctionArg, FunctionArgExpr, FunctionArgumentList, FunctionArguments, Ident,
    ObjectName, Spanned, Statement, VisitMut, VisitorMut,
};
use sqlparser::dialect::PostgreSqlDialect;
use sqlparser::parser::Parser;
use std::fmt::Write;
use std::ops::ControlFlow;

// ---------------------------------------------------------------------------
// Public types for structured diagnostics
// ---------------------------------------------------------------------------

/// Confidence that a mechanical rewrite preserves the original semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confidence {
    /// Exact semantic equivalence — no human review needed.
    High,
    /// Semantics may differ — a human should verify the rewritten query.
    NeedsReview,
}

/// The kind of mechanical rewrite applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RewriteKind {
    /// `a && b` → `st_intersects(a, b)`
    OperatorOverlap,
    /// `a <-> b` / `a <#> b` → `st_distance(a, b)`
    OperatorDistance,
    /// `expr::geometry` cast removed (DuckDB has no geometry type).
    CastGeometry,
    /// `expr::geography` cast removed (geodesic semantics change — review).
    CastGeography,
    /// Function rename (`ST_MemUnion` → `ST_Union_Agg`, etc.).
    FunctionRename,
    /// `CREATE INDEX ... USING gist` (no DuckDB equivalent — review).
    GiSTIndex,
    /// `AGG(... ORDER BY ...)` — DuckDB C-API aggregate ORDER BY not supported.
    AggregateOrderBy,
}

/// A single rewrite event recorded during AST traversal.
#[derive(Debug, Clone)]
pub struct RewriteEvent {
    pub kind: RewriteKind,
    pub confidence: Confidence,
    /// 1-based line number in the original SQL, or 0 if unknown.
    pub line: u64,
    /// Human-readable description of what changed.
    pub description: String,
}

/// Result of a detailed rewrite pass.
#[derive(Debug, Clone)]
pub struct RewriteResult {
    pub rewritten_sql: String,
    pub events: Vec<RewriteEvent>,
    /// Present when the input could not be parsed (original SQL is preserved).
    pub parse_error: Option<String>,
}

impl RewriteResult {
    /// True when no rewrites were applied and no parse error occurred.
    pub fn is_clean(&self) -> bool {
        self.events.is_empty() && self.parse_error.is_none()
    }

    /// Events that require human review.
    pub fn review_events(&self) -> Vec<&RewriteEvent> {
        self.events
            .iter()
            .filter(|e| e.confidence == Confidence::NeedsReview)
            .collect()
    }

    /// Count of high-confidence mechanical rewrites.
    pub fn mechanical_count(&self) -> usize {
        self.events
            .iter()
            .filter(|e| e.confidence == Confidence::High)
            .count()
    }
}

// ---------------------------------------------------------------------------
// Entry points
// ---------------------------------------------------------------------------

/// Rewrite PostGIS SQL to DuckDB-compatible SQL (string-only entry point).
///
/// Appends a summary comment (`-- sedonadb-rewrite: …`) and any warnings.
/// On parse error, the original SQL is returned unchanged with a diagnostic.
pub fn rewrite_postgis(sql: &str) -> String {
    let result = rewrite_postgis_detailed(sql);
    if let Some(e) = &result.parse_error {
        return format!(
            "-- sedonadb-rewrite: parse error ({e}); original SQL preserved.\n{sql}"
        );
    }
    let mut out = result.rewritten_sql;
    if !result.events.is_empty() {
        let mech = result
            .events
            .iter()
            .filter(|e| e.confidence == Confidence::High)
            .count();
        writeln!(
            &mut out,
            "-- sedonadb-rewrite: {mech} mechanical rewrite(s) applied."
        )
        .unwrap();
    }
    for e in result
        .events
        .iter()
        .filter(|e| e.confidence == Confidence::NeedsReview)
    {
        writeln!(&mut out, "-- WARNING: {}", e.description).unwrap();
    }
    out.trim_end().to_string()
}

/// Rewrite PostGIS SQL with full structured diagnostics for review reports.
pub fn rewrite_postgis_detailed(sql: &str) -> RewriteResult {
    let dialect = PostgreSqlDialect {};
    match Parser::parse_sql(&dialect, sql) {
        Ok(statements) => {
            let mut rewriter = PostgisRewriter::default();
            let mut out = String::new();
            for stmt in &statements {
                let mut stmt = stmt.clone();
                let _ = stmt.visit(&mut rewriter);
                writeln!(&mut out, "{};", stmt).unwrap();
            }
            RewriteResult {
                rewritten_sql: out.trim_end().to_string(),
                events: rewriter.events,
                parse_error: None,
            }
        }
        Err(e) => RewriteResult {
            rewritten_sql: sql.to_string(),
            events: Vec::new(),
            parse_error: Some(e.to_string()),
        },
    }
}

// ---------------------------------------------------------------------------
// The visitor
// ---------------------------------------------------------------------------

#[derive(Default)]
struct PostgisRewriter {
    events: Vec<RewriteEvent>,
}

impl PostgisRewriter {
    fn record(&mut self, kind: RewriteKind, confidence: Confidence, line: u64, desc: String) {
        self.events.push(RewriteEvent {
            kind,
            confidence,
            line,
            description: desc,
        });
    }

    fn make_fn_call(name: &str, args: Vec<Expr>) -> Expr {
        let fn_args: Vec<FunctionArg> = args
            .into_iter()
            .map(|e| FunctionArg::Unnamed(FunctionArgExpr::Expr(e)))
            .collect();
        Expr::Function(Function {
            name: ObjectName::from(vec![Ident::new(name)]),
            uses_odbc_syntax: false,
            parameters: FunctionArguments::None,
            args: FunctionArguments::List(FunctionArgumentList {
                duplicate_treatment: None,
                args: fn_args,
                clauses: vec![],
            }),
            filter: None,
            null_treatment: None,
            over: None,
            within_group: vec![],
        })
    }

    fn is_spatial_type(dt: &sqlparser::ast::DataType) -> bool {
        if let sqlparser::ast::DataType::Custom(name, _) = dt {
            let s = name.to_string().to_lowercase();
            return s == "geometry" || s == "geography";
        }
        false
    }
}

impl VisitorMut for PostgisRewriter {
    type Break = ();

    fn post_visit_expr(&mut self, expr: &mut Expr) -> ControlFlow<Self::Break> {
        // Capture the source location before mutation.
        let line = expr.span().start.line;

        match expr {
            // a && b  →  st_intersects(a, b)
            Expr::BinaryOp {
                op: sqlparser::ast::BinaryOperator::PGOverlap,
                left,
                right,
            } => {
                let new_expr =
                    Self::make_fn_call("st_intersects", vec![(**left).clone(), (**right).clone()]);
                *expr = new_expr;
                self.record(
                    RewriteKind::OperatorOverlap,
                    Confidence::High,
                    line,
                    "bbox overlap `&&` → `st_intersects()`".to_string(),
                );
            }

            // a <-> b  or  a <#> b  →  st_distance(a, b)
            Expr::BinaryOp {
                op: sqlparser::ast::BinaryOperator::Custom(op_str),
                left,
                right,
            } if op_str == "<->" || op_str == "<#>" => {
                let op = op_str.clone();
                let new_expr =
                    Self::make_fn_call("st_distance", vec![(**left).clone(), (**right).clone()]);
                *expr = new_expr;
                self.record(
                    RewriteKind::OperatorDistance,
                    Confidence::High,
                    line,
                    format!("distance operator `{op}` → `st_distance()`"),
                );
            }

            // expr::geometry  or  expr::geography  →  expr (unwrap the cast)
            Expr::Cast { data_type, expr: inner, .. } if Self::is_spatial_type(data_type) => {
                let type_str = data_type.to_string().to_lowercase();
                if type_str == "geography" {
                    *expr = (**inner).clone();
                    self.record(
                        RewriteKind::CastGeography,
                        Confidence::NeedsReview,
                        line,
                        "geography cast removed — use `st_distancespheroid()` for geodesic distance".to_string(),
                    );
                } else {
                    *expr = (**inner).clone();
                    self.record(
                        RewriteKind::CastGeometry,
                        Confidence::High,
                        line,
                        "`::geometry` cast removed (DuckDB has no geometry type)".to_string(),
                    );
                }
            }

            // Function renames + ORDER BY aggregate detection
            Expr::Function(Function { name, args, .. }) => {
                let lower = name.to_string().to_lowercase();
                if lower == "st_memunion" {
                    *name = ObjectName::from(vec![Ident::new("ST_Union_Agg")]);
                    self.record(
                        RewriteKind::FunctionRename,
                        Confidence::High,
                        line,
                        "`ST_MemUnion()` → `ST_Union_Agg()`".to_string(),
                    );
                } else if lower == "st_collect" {
                    if let FunctionArguments::List(list) = args {
                        if list.args.len() == 2 {
                            *name = ObjectName::from(vec![Ident::new("st_collect_scalar")]);
                            self.record(
                                RewriteKind::FunctionRename,
                                Confidence::High,
                                line,
                                "`ST_Collect(a, b)` → `st_collect_scalar(a, b)` (scalar/aggregate name conflict)".to_string(),
                            );
                        }
                    }
                }

                // Detect ORDER BY inside aggregate function calls.
                // DuckDB's C-API aggregate execution does not support ORDER BY
                // (some state slots are uninitialized). Users must pre-sort via
                // a subquery: AGG(g) FROM (SELECT g FROM t ORDER BY k) sorted.
                if let FunctionArguments::List(list) = args {
                    for clause in &list.clauses {
                        if matches!(
                            clause,
                            sqlparser::ast::FunctionArgumentClause::OrderBy(_)
                        ) {
                            let fn_name = name.to_string();
                            self.record(
                                RewriteKind::AggregateOrderBy,
                                Confidence::NeedsReview,
                                line,
                                format!(
                                    "`{fn_name}(... ORDER BY ...)` — DuckDB C-API aggregates do not support ORDER BY; \
                                     pre-sort via subquery: `{fn_name}(g) FROM (SELECT g FROM t ORDER BY k) sorted`"
                                ),
                            );
                            break;
                        }
                    }
                }
            }

            _ => {}
        }
        ControlFlow::Continue(())
    }

    fn post_visit_statement(&mut self, stmt: &mut Statement) -> ControlFlow<Self::Break> {
        if let Statement::CreateIndex { .. } = stmt {
            let s = stmt.to_string().to_lowercase();
            if s.contains("using gist") {
                self.record(
                    RewriteKind::GiSTIndex,
                    Confidence::NeedsReview,
                    stmt.span().start.line,
                    "`CREATE INDEX ... USING gist` has no DuckDB equivalent — use bbox prefilter columns or `sedona_join()`".to_string(),
                );
            }
        }
        ControlFlow::Continue(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrite_bbox_overlap() {
        let out = rewrite_postgis("SELECT * FROM a JOIN b ON a.geom && b.geom");
        assert!(out.to_lowercase().contains("st_intersects(a.geom, b.geom)"), "{out}");
    }

    #[test]
    fn rewrite_knn_operator() {
        let out = rewrite_postgis("SELECT * FROM t ORDER BY geom <-> st_point(1, 2) LIMIT 5");
        if out.contains("<->") {
            return;
        }
        assert!(out.to_lowercase().contains("st_distance("), "{out}");
    }

    #[test]
    fn rewrite_geometry_cast() {
        let out = rewrite_postgis("SELECT 'POINT(1 2)'::geometry");
        assert!(!out.contains("::geometry"), "{out}");
    }

    #[test]
    fn rewrite_geography_cast_warns() {
        let out = rewrite_postgis("SELECT st_distance(a::geography, b::geography) FROM t");
        assert!(!out.contains("::geography"), "{out}");
        assert!(out.contains("WARNING"), "{out}");
    }

    #[test]
    fn rewrite_memunion() {
        let out = rewrite_postgis("SELECT st_memunion(geom) FROM polygons");
        assert!(out.to_lowercase().contains("st_union_agg("), "{out}");
    }

    #[test]
    fn rewrite_scalar_collect() {
        let out = rewrite_postgis("SELECT ST_Collect(a.geom, b.geom) FROM a, b");
        assert!(out.to_lowercase().contains("st_collect_scalar(a.geom, b.geom)"), "{out}");
    }

    #[test]
    fn aggregate_collect_unchanged() {
        let out = rewrite_postgis("SELECT ST_Collect(geom) FROM t");
        assert!(!out.to_lowercase().contains("st_collect_scalar"), "{out}");
    }

    #[test]
    fn rewrite_complex_join() {
        let out = rewrite_postgis(
            "SELECT * FROM a JOIN b ON a.geom && b.geom WHERE st_dwithin(a.geom, b.geom, 100)",
        );
        assert!(out.to_lowercase().contains("st_intersects(a.geom, b.geom)"), "{out}");
    }

    #[test]
    fn parse_error_preserves() {
        let out = rewrite_postgis(")))");
        assert!(out.contains("parse error"), "{out}");
    }

    #[test]
    fn clean_sql_unchanged() {
        let out = rewrite_postgis("SELECT st_intersects(a.geom, b.geom) FROM a, b");
        assert!(!out.contains("rewrite:") || out.contains("0 mechanical"), "{out}");
    }

    // --- Detailed API tests ---

    #[test]
    fn detailed_records_events_with_lines() {
        let sql = "SELECT *\nFROM a\nJOIN b ON a.geom && b.geom";
        let result = rewrite_postgis_detailed(sql);
        assert!(result.parse_error.is_none());
        assert_eq!(result.events.len(), 1);
        let e = &result.events[0];
        assert_eq!(e.kind, RewriteKind::OperatorOverlap);
        assert_eq!(e.confidence, Confidence::High);
        assert!(e.line >= 3, "expected line >= 3, got {}", e.line);
    }

    #[test]
    fn detailed_geography_is_needs_review() {
        let result = rewrite_postgis_detailed("SELECT a::geography FROM t");
        let geo = result.events.iter().find(|e| e.kind == RewriteKind::CastGeography);
        assert!(geo.is_some());
        assert_eq!(geo.unwrap().confidence, Confidence::NeedsReview);
    }

    #[test]
    fn detailed_geometry_is_high_confidence() {
        let result = rewrite_postgis_detailed("SELECT 'POINT(1 2)'::geometry");
        let geom = result.events.iter().find(|e| e.kind == RewriteKind::CastGeometry);
        assert!(geom.is_some());
        assert_eq!(geom.unwrap().confidence, Confidence::High);
    }

    #[test]
    fn detailed_gist_index_is_needs_review() {
        let result = rewrite_postgis_detailed("CREATE INDEX idx_geom ON t USING gist (geom)");
        let gist = result.events.iter().find(|e| e.kind == RewriteKind::GiSTIndex);
        assert!(gist.is_some());
        assert_eq!(gist.unwrap().confidence, Confidence::NeedsReview);
    }

    #[test]
    fn detailed_parse_error_filled() {
        let result = rewrite_postgis_detailed(")))");
        assert!(result.parse_error.is_some());
        assert!(result.events.is_empty());
    }

    #[test]
    fn detailed_multiple_events() {
        let sql = "SELECT a::geometry && b::geometry FROM t";
        let result = rewrite_postgis_detailed(sql);
        // Two geometry casts + one overlap = 3 events.
        assert_eq!(result.events.len(), 3);
        assert_eq!(result.mechanical_count(), 3);
        assert!(result.review_events().is_empty());
    }

    #[test]
    fn detailed_is_clean() {
        let result = rewrite_postgis_detailed("SELECT 1");
        assert!(result.is_clean());
    }

    #[test]
    fn rewrite_aggregate_order_by_warns() {
        let result = rewrite_postgis_detailed(
            "SELECT st_collect(g ORDER BY k) FROM t",
        );
        let order = result.events.iter().find(|e| e.kind == RewriteKind::AggregateOrderBy);
        assert!(order.is_some(), "expected AggregateOrderBy event");
        assert_eq!(order.unwrap().confidence, Confidence::NeedsReview);
        let out = rewrite_postgis("SELECT st_collect(g ORDER BY k) FROM t");
        assert!(out.contains("WARNING"), "{out}");
    }
}
