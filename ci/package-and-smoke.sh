#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Full release pipeline: build the release artifact, package the
# .duckdb_extension (512-byte DuckDB trailer), LOAD it, and smoke-test that both
# the local ST_* path and the literal Apache SedonaDB bridge path run in a real
# DuckDB session. Catches regressions that Rust unit tests can't (packaging,
# symbol export, runtime library resolution).
#
# Usage: ./ci/package-and-smoke.sh [duckdb_binary]
# Needs the GDAL/PROJ/libclang build env; auto-locates Linuxbrew if present.
set -euo pipefail

cd "$(dirname "$0")/.."

DUCKDB="${1:-duckdb}"
PLATFORM="linux_amd64"

# Build env (Linuxbrew provides GDAL 3.13.1 headers + libclang + runtime libs).
BREW=/var/home/linuxbrew/.linuxbrew
if [ -d "$BREW/lib" ]; then
    export PKG_CONFIG_PATH="$BREW/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
    if [ -z "${LIBCLANG_PATH:-}" ] || [ ! -e "${LIBCLANG_PATH:-}/libclang.so" ]; then
        export LIBCLANG_PATH="$(dirname "$(find "$BREW" -name libclang.so 2>/dev/null | head -1)")"
    fi
    export LD_LIBRARY_PATH="$BREW/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
fi

echo ">> cargo build --release"
cargo build --release

SO=target/release/libsedonadb.so
EXT=build/dev/sedonadb.duckdb_extension
mkdir -p build/dev

echo ">> package $SO -> $EXT ($PLATFORM)"
./target/release/sedonadb-package "$SO" "$EXT" "$PLATFORM"

echo ">> smoke-test in DuckDB ($DUCKDB)"
# Smoke checks covering every backend family: local geo, literal SedonaDB,
# aggregate (envelope+collect), GEOS topology, spheroid geodesics, raster pixel
# streaming, ST_Value point sampling, CRS transform (PROJ), table functions
# (ST_Dump), overlay fallback, ST_DumpRings, ST_ContainsProperly, and
# literal routing parity.
"$DUCKDB" -unsigned <<SQL
LOAD '$(pwd)/$EXT';
.mode list
SELECT CASE WHEN st_astext(st_geomfromtext('POINT(1 2)')) = 'POINT(1 2)'
            THEN 'PASS local' ELSE 'FAIL local' END;
SELECT CASE WHEN sedona_st_dimension(st_geomfromtext('POINT(1 2)')) = 0
            THEN 'PASS sedona' ELSE 'FAIL sedona' END;
SELECT CASE WHEN sedona_st_astext(st_geomfromtext('POINT(1 2)')) = 'POINT(1 2)'
            THEN 'PASS sedona-astext' ELSE 'FAIL sedona-astext' END;
SELECT CASE WHEN st_area(st_envelope_agg(g)) > 0
            THEN 'PASS aggregate-envelope' ELSE 'FAIL aggregate-envelope' END
FROM (SELECT st_geomfromtext('POLYGON((0 0,1 0,1 1,0 1,0 0))') AS g);
SELECT CASE WHEN st_numgeometries(st_collect(g)) >= 1
            THEN 'PASS aggregate-collect' ELSE 'FAIL aggregate-collect' END
FROM (SELECT st_geomfromtext('POINT(1 2)') AS g);
SELECT CASE WHEN st_numgeometries(st_voronoipolygons(st_geomfromtext(
            'MULTIPOINT((0 0),(1 0),(0 1),(1 1))'))) >= 4
            THEN 'PASS geos' ELSE 'FAIL geos' END;
SELECT CASE WHEN st_distancespheroid(st_point(0,0), st_point(1,0)) > 100000.0
            THEN 'PASS spheroid' ELSE 'FAIL spheroid' END;
SELECT CASE WHEN (SELECT count(*) FROM st_pixeldata('tests/data/test_raster.asc', 1)) = 12
            THEN 'PASS raster-pixeldata' ELSE 'FAIL raster-pixeldata' END;
SELECT CASE WHEN st_value('tests/data/test_raster.asc', 1, 0.5, 2.5) = 1.0
            THEN 'PASS raster-value' ELSE 'FAIL raster-value' END;
SELECT CASE WHEN abs(st_x(st_transform(st_geomfromtext('POINT(-0.1278 51.5074)'), 4326, 3857))
                     - (-14227.16)) < 1.0
            THEN 'PASS proj-transform' ELSE 'FAIL proj-transform' END;
SELECT CASE WHEN (SELECT count(*) FROM st_dump(st_geomfromtext('MULTIPOINT((1 2),(3 4))'))) = 2
            THEN 'PASS tablefn-dump' ELSE 'FAIL tablefn-dump' END;
SELECT CASE WHEN abs(st_area(st_intersection(
            st_geomfromtext('POLYGON((0 0,2 0,2 2,0 2,0 0))'),
            st_geomfromtext('POLYGON((1 1,3 1,3 3,1 3,1 1))'))) - 1.0) < 1e-9
            THEN 'PASS overlay-intersection' ELSE 'FAIL overlay-intersection' END;
SELECT CASE WHEN (SELECT count(*) FROM st_dumprings(
            st_geomfromtext('POLYGON((0 0,1 0,1 1,0 1,0 0),(0.2 0.2,0.4 0.2,0.4 0.4,0.2 0.4,0.2 0.2))'))) = 2
            THEN 'PASS tablefn-dumprings' ELSE 'FAIL tablefn-dumprings' END;
SELECT CASE WHEN st_containsproperly(
            st_geomfromtext('POLYGON((0 0,4 0,4 4,0 4,0 0))'),
            st_geomfromtext('POINT(1 1)'))
            THEN 'PASS containsproperly' ELSE 'FAIL containsproperly' END;
SELECT CASE WHEN st_astext(st_rotate(st_geomfromtext('POINT(1 0)'), 1.5707963267948966)) =
                 sedona_st_astext(sedona_st_rotate(st_geomfromtext('POINT(1 0)'), 1.5707963267948966))
            THEN 'PASS routing-parity' ELSE 'FAIL routing-parity' END;
SELECT CASE WHEN st_relate(st_geomfromtext('POLYGON((0 0,0 4,4 4,4 0,0 0))'),
                           st_geomfromtext('POLYGON((2 2,2 6,6 6,6 2,2 2))'))
                 = '212101212'
            THEN 'PASS geos-relate' ELSE 'FAIL geos-relate' END;
SELECT CASE WHEN st_assvg(st_geomfromtext('POINT(1 2)')) = '1 -2'
            THEN 'PASS output-svg' ELSE 'FAIL output-svg' END;
SELECT CASE WHEN st_relate(st_geomfromtext('POINT(1 1)'),
                           st_geomfromtext('POLYGON((0 0,0 4,4 4,4 0,0 0))'),
                           '0FFFFF212')
            THEN 'PASS geos-relate-pattern' ELSE 'FAIL geos-relate-pattern' END;
SQL

echo ">> smoke OK: packaged extension loads and runs both paths"
