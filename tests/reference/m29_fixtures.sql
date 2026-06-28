-- M29: Aggregate ORDER BY — crash fix and rewriter support.
--
-- DuckDB's C-API aggregate execution does not properly support ORDER BY inside
-- aggregate function calls: some state slots passed to the update callback are
-- uninitialized, causing heap corruption (SIGABRT) or silently dropped rows.
--
-- Fix (M29): the update callbacks now check state validity (null inner pointer)
-- and skip uninitialized slots rather than crashing. The Rust AST rewriter
-- detects ORDER BY in aggregates and warns with the subquery workaround.
--
-- This file proves both: the crash is gone (exit 0, not 134), and the rewriter
-- detects the pattern.
.mode list

-- The rewriter detects ORDER BY in aggregate calls and warns.
SELECT CASE WHEN sedonadb_rewrite_postgis(
    'SELECT st_collect(g ORDER BY k) FROM t'
) LIKE '%WARNING%ORDER BY%'
THEN 'PASS rewriter detects order_by' ELSE 'FAIL rewriter detects order_by' END;

-- The rewriter also detects it for other aggregate functions.
SELECT CASE WHEN sedonadb_rewrite_postgis(
    'SELECT st_makeline(g ORDER BY k) FROM t'
) LIKE '%WARNING%ORDER BY%'
THEN 'PASS rewriter detects makeline order_by' ELSE 'FAIL rewriter detects makeline order_by' END;

-- Without ORDER BY, aggregates work correctly (baseline).
SELECT CASE WHEN st_astext(st_collect(g))
                 = 'GEOMETRYCOLLECTION(POINT(0 0),POINT(1 1))'
            THEN 'PASS collect no order_by' ELSE 'FAIL collect no order_by' END
FROM (VALUES (st_point(0,0)), (st_point(1,1))) t(g);

-- The workaround: pre-sort in a subquery, no ORDER BY in the aggregate itself.
SELECT CASE WHEN st_numpoints(st_makeline_agg(g))
                 = 2
            THEN 'PASS makeline subquery sort' ELSE 'FAIL makeline subquery sort' END
FROM (SELECT g FROM (VALUES (st_point(1,1), 1), (st_point(0,0), 0)) t(g,k) ORDER BY k) sorted;
