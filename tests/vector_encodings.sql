-- SPDX-License-Identifier: Apache-2.0
-- Regression: non-flat DuckDB input vectors must NOT segfault scalar ST_*
-- functions. DuckDB feeds dictionary/sequence/constant vectors into scalar
-- callbacks after ORDER BY, LIMIT, filter/projection, and constant folding.
-- All paths below previously segfaulted on a non-flat BLOB (WKB) vector; the
-- vendored binary-safe VectorReader::read_blob fixed it. This pins the fix.
--
-- Run with:
--   LD_LIBRARY_PATH=<gdal-lib> duckdb -unsigned -cmd "LOAD '<ext>';" < tests/vector_encodings.sql
.bail off
.mode list

-- (extension loaded via `duckdb -cmd "LOAD '<ext>';"` by tests/run_sql.sh)

CREATE TABLE t AS SELECT i::INTEGER AS id, st_point(i, i*2) AS geom, (i % 7 = 0) AS is_null
FROM range(0, 8000) t(i);
CREATE TABLE tn AS SELECT id, CASE WHEN is_null THEN NULL ELSE geom END AS geom FROM t;

-- sequence vector (ORDER BY + LIMIT on the geometry-feeding side)
SELECT CASE WHEN count(*) = 100 THEN 'PASS' ELSE 'FAIL' END AS p1_seq
FROM (SELECT st_astext(geom) FROM t ORDER BY id LIMIT 100);

-- dictionary/sequence vector (ORDER BY the geometry column itself)
SELECT CASE WHEN count(*) = 50 THEN 'PASS' ELSE 'FAIL' END AS p2_dict
FROM (SELECT st_area(st_buffer(geom,1.0)) FROM t ORDER BY geom DESC LIMIT 50);

-- NULL-bearing geometry column under a sequence vector
SELECT CASE WHEN count(*) = 200 THEN 'PASS' ELSE 'FAIL' END AS p3_nullseq
FROM (SELECT st_dimension(geom) FROM tn ORDER BY id LIMIT 200);

-- constant-folded geometry vector
SELECT CASE WHEN count(*) = 5 THEN 'PASS' ELSE 'FAIL' END AS p4_const
FROM (SELECT st_astext(st_point(1,2)) FROM t LIMIT 5);

-- literal SedonaDB bridge under the same non-flat vector encodings
SELECT CASE WHEN count(*) = 100 THEN 'PASS' ELSE 'FAIL' END AS p5_bridge_seq
FROM (SELECT sedona_st_astext(geom) FROM t ORDER BY id LIMIT 100);

SELECT CASE WHEN count(*) = 100 THEN 'PASS' ELSE 'FAIL' END AS p6_bridge_null
FROM (SELECT sedona_st_dimension(geom) FROM tn ORDER BY id DESC LIMIT 100);

-- aggregate over a geometry column (its own state-machine path)
SELECT CASE WHEN st_area(st_envelope_agg(geom)) > 0 THEN 'PASS' ELSE 'FAIL' END AS p7_agg FROM t;
