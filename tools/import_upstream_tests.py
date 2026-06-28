#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Import and port upstream PostGIS/SedonaDB test coverage.

Clones upstream repos, selects high-value test files, applies the PostGIS
rewriter, and generates DuckDB-ready SQL test files. This is the bridge
between external test corpora and our extension.

Usage:
    python3 tools/import_upstream_tests.py --postgis     # import PostGIS regress
    python3 tools/import_upstream_tests.py --sedonadb    # import SedonaDB tests
    python3 tools/import_upstream_tests.py --list        # list available tests
    python3 tools/import_upstream_tests.py --all         # import everything

Output: tests/upstream/ directory with ported SQL files.

Requirements: git (for cloning), python3.
"""
from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
UPSTREAM_DIR = REPO_ROOT / ".tmp" / "ref"
OUTPUT_DIR = REPO_ROOT / "tests" / "upstream"

POSTGIS_REPO = "https://github.com/postgis/postgis.git"
POSTGIS_TEST_DIR = "regress/sql"
# Curated high-value PostGIS regress tests that map to our function surface.
# These cover core geometry functions, predicates, measurements, and editing.
POSTGIS_TARGETS = [
    "regress/sql/regress.sql",          # core geometry functions
    "regress/sql/boundary.sql",         # ST_Boundary
    "regress/sql/centroid.sql",         # ST_Centroid
    "regress/sql/contains.sql",         # ST_Contains
    "regress/sql/convexhull.sql",       # ST_ConvexHull
    "regress/sql/equals.sql",           # ST_Equals
    "regress/sql/distance.sql",         # ST_Distance
    "regress/sql/intersection.sql",     # ST_Intersection
    "regress/sql/isvalid.sql",          # ST_IsValid
    "regress/sql/within.sql",           # ST_Within
    "regress/sql/union.sql",            # ST_Union
    "regress/sql/difference.sql",       # ST_Difference
    "regress/sql/symdifference.sql",    # ST_SymDifference
    "regress/sql/intersects.sql",       # ST_Intersects
    "regress/sql/touches.sql",          # ST_Touches
    "regress/sql/crosses.sql",          # ST_Crosses
    "regress/sql/overlaps.sql",         # ST_Overlaps
    "regress/sql/relate.sql",           # ST_Relate
    "regress/sql/buffer.sql",           # ST_Buffer
    "regress/sql/simplify.sql",         # ST_Simplify
    "regress/sql/affine.sql",           # ST_Affine
    "regress/sql/translate.sql",        # ST_Translate
    "regress/sql/scale.sql",            # ST_Scale
    "regress/sql/rotate.sql",           # ST_Rotate
    "regress/sql/reverse.sql",          # ST_Reverse
    "regress/sql/segmentize.sql",       # ST_Segmentize
    "regress/sql/azimuth.sql",          # ST_Azimuth
    "regress/sql/project.sql",          # ST_Project
    "regress/sql/force_dims.sql",       # ST_Force*D
    "regress/sql/force_collection.sql", # ST_ForceCollection
    "regress/sql/multi.sql",            # ST_Multi
    "regress/sql/normalize.sql",        # ST_Normalize
    "regress/sql/summary.sql",          # ST_Summary (metadata)
]

SEDONADB_REPO = "https://github.com/apache/sedona-db.git"
SEDONADB_TEST_DIRS = [
    "sedona-functions/src/test",         # kernel unit/integration tests
    "sedona-sql/src/test",               # SQL-level tests
]


def clone_shallow(repo_url: str, dest: Path) -> bool:
    """Shallow-clone a repo into dest. Returns True on success."""
    if dest.exists():
        print(f"  Already cloned: {dest}")
        return True
    dest.parent.mkdir(parents=True, exist_ok=True)
    print(f"  Cloning {repo_url} (depth=1)...")
    result = subprocess.run(
        ["git", "clone", "--depth", "1", repo_url, str(dest)],
        capture_output=True, text=True, timeout=120,
    )
    if result.returncode != 0:
        print(f"  FAILED: {result.stderr[:200]}", file=sys.stderr)
        return False
    return True


def rewrite_postgis_sql(text: str) -> str:
    """Apply mechanical PostGIS→DuckDB rewrites to test SQL."""
    import re

    # Comment out lines that can't be mechanically rewritten
    lines = text.splitlines()
    output = []
    for line in lines:
        original = line

        # Skip empty lines and comments
        stripped = line.strip()
        if not stripped or stripped.startswith('--'):
            output.append(line)
            continue

        # Rewrite ::geometry casts (remove)
        line = re.sub(r'::geometry\b', '', line, flags=re.IGNORECASE)

        # Rewrite ::geography casts (remove)
        line = re.sub(r'::geography\b', '', line, flags=re.IGNORECASE)

        # Rewrite geometry(Type, SRID) typmods → BLOB
        line = re.sub(r'\s+geometry\s*\(\s*\w+\s*,\s*\d+\s*\)', ' BLOB', line, flags=re.IGNORECASE)

        # Rewrite && operator → st_intersects
        # Simple case: a && b → st_intersects(a, b)
        line = re.sub(r'(\w+)\s*&&\s*(\w+)', r'st_intersects(\1, \2)', line)

        # Rewrite <-> → st_distance
        line = re.sub(r'(\w+)\s*<->\s*(\w+)', r'st_distance(\1, \2)', line)

        # ST_MemUnion → ST_Union_Agg
        line = re.sub(r'\bST_MemUnion\b', 'ST_Union_Agg', line, flags=re.IGNORECASE)

        # Comment out CREATE INDEX ... USING gist
        if re.search(r'CREATE\s+INDEX.*USING\s+gist', line, re.IGNORECASE):
            line = '-- ' + line + '  -- GiST not available'

        # Comment out lines with PostgreSQL-specific syntax we can't rewrite
        if re.search(r'\bCREATE\s+(OR\s+REPLACE\s+)?FUNCTION\b', line, re.IGNORECASE):
            line = '-- ' + line + '  -- PG function definition not portable'
        if re.search(r'\bLANGUAGE\s+\'?(plpgsql|c)\'?', line, re.IGNORECASE):
            line = '-- ' + line
        if re.search(r'\bSELECT\s+setlimit\b', line, re.IGNORECASE):
            line = '-- ' + line
        if re.search(r'\bALTER\s+TABLE.*ADD\s+CONSTRAINT', line, re.IGNORECASE):
            line = '-- ' + line
        if 'enable_function' in line.lower():
            line = '-- ' + line
        if 'drop_table' in line.lower():
            line = '-- ' + line
        if line.strip().startswith('DROP '):
            line = '-- ' + line
        if line.strip().startswith('CREATE TABLE'):
            # Keep CREATE TABLE but fix geometry columns
            pass

        # Fix table creation with geometry columns
        line = re.sub(r'\bgeometry\b(?!\s*\()', 'BLOB', line, flags=re.IGNORECASE)

        # Fix ewkt → text casts
        line = re.sub(r'::ewkt\b', '', line, flags=re.IGNORECASE)
        line = re.sub(r'::text\b', '', line, flags=re.IGNORECASE)
        line = re.sub(r'::double\s+precision\b', '', line, flags=re.IGNORECASE)
        line = re.sub(r'::int(eger)?\b', '', line, flags=re.IGNORECASE)
        line = re.sub(r'::bool(ean)?\b', '', line, flags=re.IGNORECASE)

        # Remove PostgreSQL-specific functions
        line = re.sub(r'\bsetlimit\s*\([^)]*\)', '0', line, flags=re.IGNORECASE)
        line = re.sub(r'\bastext\s*\(', 'st_astext(', line, flags=re.IGNORECASE)
        line = re.sub(r'\basbinary\s*\(', 'st_asbinary(', line, flags=re.IGNORECASE)
        line = re.sub(r'\bnpoints\b', 'st_npoints', line, flags=re.IGNORECASE)
        line = re.sub(r'\bmemcollect\b', 'st_collect', line, flags=re.IGNORECASE)

        # Rewrite PG expected-output annotations
        # PostGIS regress uses '1|...' format for expected output in comments
        # These don't affect execution but we keep them for reference

        if line != original:
            output.append(f"-- REWRITTEN from: {original.strip()[:80]}")
        output.append(line)

    return "\n".join(output)


def import_postgis() -> None:
    """Clone PostGIS, select tests, rewrite, and output ported SQL files."""
    dest = UPSTREAM_DIR / "postgis"
    if not clone_shallow(POSTGIS_REPO, dest):
        print("Failed to clone PostGIS repo", file=sys.stderr)
        sys.exit(1)

    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)
    imported = 0
    skipped = 0

    for target in POSTGIS_TARGETS:
        src = dest / target
        if not src.exists():
            skipped += 1
            continue

        text = src.read_text()
        rewritten = rewrite_postgis_sql(text)

        # Write ported file
        name = src.stem  # e.g., "regress" from "regress.sql"
        out_path = OUTPUT_DIR / f"postgis_{name}.sql"

        header = f"""-- Auto-ported from PostGIS regress: {target}
-- Generated by tools/import_upstream_tests.py
-- Run with: duckdb -unsigned -cmd "LOAD '<ext>';" < {out_path.name}
--
-- WARNING: These tests use PostGIS expected-output format (not CASE WHEN).
-- They validate that functions run and produce output without error.
-- Exact value comparison requires manual review of PostGIS-isms.
.mode list
.bail off

"""
        out_path.write_text(header + rewritten)
        imported += 1
        print(f"  Imported: {target} → {out_path.name}")

    print(f"\nImported {imported} PostGIS test file(s), skipped {skipped}.")
    print(f"Output: {OUTPUT_DIR}/")
    print(f"\nTo run: for f in {OUTPUT_DIR}/postgis_*.sql; do")
    print(f"  duckdb -unsigned -cmd \"LOAD '$EXT';\" < \"$f\" 2>&1 | grep -cE '^(Error|PASS)'")
    print(f"done")


def import_sedonadb() -> None:
    """Clone SedonaDB, find SQL tests, and output them as-is (they use the same
    geo/wkb stack). These validate bridge parity."""
    dest = UPSTREAM_DIR / "sedonadb"
    if not clone_shallow(SEDONADB_REPO, dest):
        print("Failed to clone SedonaDB repo", file=sys.stderr)
        sys.exit(1)

    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)
    imported = 0

    for test_dir_rel in SEDONADB_TEST_DIRS:
        test_dir = dest / test_dir_rel
        if not test_dir.exists():
            continue
        for sql_file in test_dir.rglob("*.sql"):
            # Skip non-test files
            if sql_file.name in ("mod.rs",):
                continue
            text = sql_file.read_text(errors='ignore')
            name = sql_file.stem.replace(" ", "_")
            out_path = OUTPUT_DIR / f"sedonadb_{name}.sql"
            header = f"-- Auto-ported from SedonaDB: {sql_file.relative_to(dest)}\n-- Validates bridge parity.\n.mode list\n.bail off\n\n"
            out_path.write_text(header + text)
            imported += 1
            print(f"  Imported: {sql_file.relative_to(dest)} → {out_path.name}")

    print(f"\nImported {imported} SedonaDB test file(s).")
    print(f"Output: {OUTPUT_DIR}/")


def list_available() -> None:
    """List available upstream tests without importing."""
    print("PostGIS regress targets:")
    for t in POSTGIS_TARGETS:
        print(f"  {t}")
    print(f"\nTotal: {len(POSTGIS_TARGETS)} PostGIS test files")
    print(f"\nSedonaDB test directories:")
    for d in SEDONADB_TEST_DIRS:
        print(f"  {d}")


def main() -> None:
    if "--list" in sys.argv:
        list_available()
        return
    if "--postgis" in sys.argv or "--all" in sys.argv:
        print("=== Importing PostGIS regress tests ===")
        import_postgis()
        print()
    if "--sedonadb" in sys.argv or "--all" in sys.argv:
        print("=== Importing SedonaDB tests ===")
        import_sedonadb()
        print()
    if not any(a.startswith("--") for a in sys.argv[1:]):
        print(__doc__)
        sys.exit(0)


if __name__ == "__main__":
    main()
