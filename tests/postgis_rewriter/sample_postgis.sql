-- Sample PostGIS SQL for rewriter testing.
-- Contains a mix of high-confidence and low-confidence patterns.

-- 1. Bbox overlap join (high confidence)
SELECT a.* FROM parcels a JOIN zones z ON a.geom && z.geom;

-- 2. KNN query (high confidence)
SELECT * FROM coffee_shops
ORDER BY geom <-> st_point(-122.4, 37.7)
LIMIT 5;

-- 3. Cast and typmod (high confidence)
SELECT 'POINT(1 2)'::geometry;
SELECT 'POINT(1 2)'::geometry(Point, 4326);

-- 4. Geography distance (high confidence)
SELECT st_distance(a.geom::geography, b.geom::geography)
FROM cities a, cities b WHERE a.id = 1 AND b.id = 2;

-- 5. GiST index (high confidence)
CREATE INDEX idx_geom ON my_table USING gist(geom);

-- 6. ST_Union aggregate (low confidence - could be scalar)
SELECT st_union(geom) FROM polygons GROUP BY region;

-- 7. ST_MemUnion (high confidence)
SELECT st_memunion(geom) FROM polygons;

-- 8. Not-yet-shipped output formats (low confidence)
SELECT st_asmvt(geom, 'layer') FROM tiles;
SELECT st_astwkb(geom) FROM points;
SELECT st_askml(geom) FROM regions;

-- 9. Scalar ST_Collect (low confidence)
SELECT st_collect(g1, g2) FROM pairs;

-- 10. DWithin on geography (low confidence)
SELECT * FROM pts WHERE st_dwithin(a.geom::geography, b.geom::geography, 1000);

-- 11. Clean DuckDB SQL (no findings expected)
SELECT st_area(st_geomfromtext('POLYGON((0 0,0 1,1 1,0 0))'));
SELECT st_intersects(a.geom, b.geom) FROM a JOIN b ON a.id = b.id;
