#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# QuackGIS PostGIS compatibility test (Milestone 3).
#
# Tests real PostGIS SQL patterns through psql: constructors, casts, operators,
# predicates, measurements, transforms, and rewrite diagnostics.
#
# Usage:
#   ./container/test-compat.sh
#
# Environment:
#   PG_HOST      Postgres host (default: 127.0.0.1)
#   PG_PORT      Postgres port (default: 55432)
#   PG_USER      Postgres user (default: postgres)
#   PG_PASSWORD  Postgres password (default: quackgis)
#   PG_DB        Postgres database (default: postgres)
set -euo pipefail

PG_HOST="${PG_HOST:-127.0.0.1}"
PG_PORT="${PG_PORT:-55432}"
PG_USER="${PG_USER:-postgres}"
PG_PASSWORD="${PG_PASSWORD:-quackgis}"
PG_DB="${PG_DB:-postgres}"

export PGPASSWORD="$PG_PASSWORD"
PSQL="psql -h $PG_HOST -p $PG_PORT -U $PG_USER -d $PG_DB -tA"

PASS=0
FAIL=0
FAILED_CHECKS=()

check() {
    local label="$1"
    local expected="$2"
    local actual="$3"
    if [ "$actual" = "$expected" ]; then
        echo "PASS $label"
        PASS=$((PASS + 1))
    else
        echo "FAIL $label (expected='$expected' got='$actual')"
        FAIL=$((FAIL + 1))
        FAILED_CHECKS+=("$label")
    fi
}

echo "── QuackGIS PostGIS compatibility test ──────────────────────"
echo

# ══ Compatibility surface check ══════════════════════════════════════════════

echo "── Feature surface ──────────────────────────────────────────"
$PSQL -c "SELECT feature, status, detail FROM quackgis.compat_check();" 2>&1
echo

# ══ Constructors ═════════════════════════════════════════════════════════════

echo "── Constructors ─────────────────────────────────────────────"

RESULT=$($PSQL -c "SELECT st_astext(st_geomfromtext('POINT(1 2)'));" 2>&1)
check "st_geomfromtext+st_astext" "POINT(1 2)" "$RESULT"

RESULT=$($PSQL -c "SELECT st_astext(st_point(3, 4));" 2>&1)
check "st_point" "POINT(3 4)" "$RESULT"

RESULT=$($PSQL -c "SELECT st_astext(st_makeenvelope(0, 0, 1, 1));" 2>&1)
check "st_makeenvelope" "POLYGON((0 0,1 0,1 1,0 1,0 0))" "$RESULT"

# ══ Casts ════════════════════════════════════════════════════════════════════

echo
echo "── Casts ────────────────────────────────────────────────────"

RESULT=$($PSQL -c "SELECT st_astext('POINT(5 6)'::geometry);" 2>&1)
check "text::geometry cast" "POINT(5 6)" "$RESULT"

# ══ Accessors ════════════════════════════════════════════════════════════════

echo
echo "── Accessors ────────────────────────────────────────────────"

RESULT=$($PSQL -c "SELECT st_area(st_geomfromtext('POLYGON((0 0,4 0,4 4,0 4,0 0))'));" 2>&1)
check "st_area" "16" "$RESULT"

RESULT=$($PSQL -c "SELECT st_x(st_point(7, 8));" 2>&1)
check "st_x" "7" "$RESULT"

RESULT=$($PSQL -c "SELECT st_geometrytype(st_geomfromtext('POINT(0 0)'));" 2>&1)
check "st_geometrytype" "ST_Point" "$RESULT"

RESULT=$($PSQL -c "SELECT st_isempty(st_geomfromtext('POINT(0 0)'));" 2>&1)
check "st_isempty false" "f" "$RESULT"

RESULT=$($PSQL -c "SELECT st_srid(st_setsrid(st_point(0,0), 4326));" 2>&1)
check "st_srid+st_setsrid" "4326" "$RESULT"

# ══ Predicates ═══════════════════════════════════════════════════════════════

echo
echo "── Predicates ───────────────────────────────────────────────"

RESULT=$($PSQL -c "SELECT st_intersects(
    st_geomfromtext('POLYGON((0 0,2 0,2 2,0 2,0 0))'),
    st_geomfromtext('POLYGON((1 1,3 1,3 3,1 3,1 1))'));" 2>&1)
check "st_intersects overlap" "t" "$RESULT"

RESULT=$($PSQL -c "SELECT st_contains(
    st_geomfromtext('POLYGON((0 0,4 0,4 4,0 4,0 0))'),
    st_geomfromtext('POINT(1 1)'));" 2>&1)
check "st_contains" "t" "$RESULT"

RESULT=$($PSQL -c "SELECT st_disjoint(
    st_geomfromtext('POINT(0 0)'),
    st_geomfromtext('POINT(10 10)'));" 2>&1)
check "st_disjoint" "t" "$RESULT"

RESULT=$($PSQL -c "SELECT st_dwithin(
    st_geomfromtext('POINT(0 0)'),
    st_geomfromtext('POINT(3 4)'), 6.0);" 2>&1)
check "st_dwithin" "t" "$RESULT"

# ══ Measurements ═════════════════════════════════════════════════════════════

echo
echo "── Measurements ─────────────────────────────────────────────"

RESULT=$($PSQL -c "SELECT st_distance(st_point(0,0), st_point(3,4));" 2>&1)
check "st_distance 3-4-5" "5" "$RESULT"

RESULT=$($PSQL -c "SELECT st_distance(st_point(0,0), st_point(0,1)) > 100000
    FROM (SELECT st_distancesphere(st_point(0,0), st_point(0,1)) AS d) x
    WHERE true;" 2>&1 || true)
# Distancesphere check — test the function directly
RESULT=$($PSQL -c "SELECT st_distancesphere(st_point(0,0), st_point(0,1)) > 100000;" 2>&1)
check "st_distancesphere" "t" "$RESULT"

# ══ Operators ════════════════════════════════════════════════════════════════

echo
echo "── Operators ────────────────────────────────────────────────"

RESULT=$($PSQL -c "SELECT st_geomfromtext('POLYGON((0 0,2 0,2 2,0 2,0 0))') &&
                        st_geomfromtext('POLYGON((1 1,3 1,3 3,1 3,1 1))');" 2>&1)
check "&& bbox overlap" "t" "$RESULT"

RESULT=$($PSQL -c "SELECT st_geomfromtext('POINT(0 0)') <->
                        st_geomfromtext('POINT(3 4)');" 2>&1)
check "<-> distance" "5" "$RESULT"

# ══ Set operations ════════════════════════════════════════════════════════════

echo
echo "── Set operations ───────────────────────────────────────────"

RESULT=$($PSQL -c "SELECT st_area(st_intersection(
    st_geomfromtext('POLYGON((0 0,2 0,2 2,0 2,0 0))'),
    st_geomfromtext('POLYGON((1 1,3 1,3 3,1 3,1 1))')));" 2>&1)
check "st_intersection area" "1" "$RESULT"

RESULT=$($PSQL -c "SELECT abs(st_area(st_union(
    st_geomfromtext('POLYGON((0 0,2 0,2 2,0 2,0 0))'),
    st_geomfromtext('POLYGON((1 0,3 0,3 2,1 2,1 0))'))) - 6.0) < 0.001;" 2>&1)
check "st_union area" "t" "$RESULT"

# ══ Transforms ═══════════════════════════════════════════════════════════════

echo
echo "── Transforms ───────────────────────────────────────────────"

RESULT=$($PSQL -c "SELECT abs(st_x(st_transform(
    st_geomfromtext('POINT(-0.1278 51.5074)'), 4326, 3857))
    - (-14227.16)) < 2.0;" 2>&1)
check "st_transform CRS" "t" "$RESULT"

# ══ SRID handling ═════════════════════════════════════════════════════════════

echo
echo "── SRID ─────────────────────────────────────────────────────"

RESULT=$($PSQL -c "SELECT st_asewkt(st_setsrid(st_point(1,2), 4326));" 2>&1)
check "st_asewkt with SRID" "SRID=4326;POINT(1 2)" "$RESULT"

# ══ Rewrite diagnostics ══════════════════════════════════════════════════════

echo
echo "── Rewrite diagnostics ──────────────────────────────────────"

RESULT=$($PSQL -c "SELECT quackgis.rewrite_sql(
    'SELECT * FROM a JOIN b ON a.geom && b.geom');" 2>&1)
if echo "$RESULT" | grep -qi "st_intersects"; then
    echo "PASS rewrite && → st_intersects"
    PASS=$((PASS + 1))
else
    echo "FAIL rewrite && → st_intersects (got: $RESULT)"
    FAIL=$((FAIL + 1))
    FAILED_CHECKS+=("rewrite_overlap")
fi

RESULT=$($PSQL -c "SELECT quackgis.rewrite_sql(
    'SELECT st_memunion(geom) FROM t');" 2>&1)
if echo "$RESULT" | grep -qi "st_union_agg"; then
    echo "PASS rewrite ST_MemUnion → ST_Union_Agg"
    PASS=$((PASS + 1))
else
    echo "FAIL rewrite ST_MemUnion (got: $RESULT)"
    FAIL=$((FAIL + 1))
    FAILED_CHECKS+=("rewrite_memunion")
fi

# ══ PostGIS compatibility functions ══════════════════════════════════════════

echo
echo "── PostGIS compat functions ─────────────────────────────────"

RESULT=$($PSQL -c "SELECT postgis_version();" 2>&1)
if [ -n "$RESULT" ] && echo "$RESULT" | grep -qi "QUACKGIS"; then
    echo "PASS postgis_version()"
    PASS=$((PASS + 1))
else
    echo "FAIL postgis_version() (got: $RESULT)"
    FAIL=$((FAIL + 1))
    FAILED_CHECKS+=("postgis_version")
fi

# ══ Summary ══════════════════════════════════════════════════════════════════

echo
echo "── Summary ──────────────────────────────────────────────────"
echo "PASS=$PASS FAIL=$FAIL"
if [ "$FAIL" -gt 0 ]; then
    echo "Failed: ${FAILED_CHECKS[*]}"
    exit 1
fi
echo "ALL COMPATIBILITY TESTS PASSED"
