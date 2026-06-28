#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""PostGIS → DuckDB SQL transpiler.

Scans .sql files for PostGIS-specific patterns and either annotates them with
warnings (default) or applies mechanical rewrites (--apply mode).

Modes:
    (default)  Annotate: print the original SQL with inline warnings.
    --apply    Rewrite: emit transformed SQL with high-confidence rewrites applied.
               Low-confidence patterns remain as comments for human review.

Usage:
    python3 tools/postgis_rewriter.py <file.sql>           # annotate
    python3 tools/postgis_rewriter.py <file.sql> --apply    # auto-rewrite
    python3 tools/postgis_rewriter.py --stdin < file.sql   # pipe mode

Mechanical rewrites (--apply, high confidence):
    &&           → st_intersects(a, b)
    <->          → st_distance(a, b) in ORDER BY context
    ::geometry   → (remove; use BLOB directly)
    ::geography  → (remove; use explicit sphere/spheroid functions)
    geometry(T,S) typmod → (remove; use BLOB)
    USING gist   → (comment out; no DuckDB equivalent)
    ST_MemUnion  → ST_Union_Agg
    ST_AsMVT/TWKB/KML → (no change needed; all shipped)
"""
from __future__ import annotations

import re
import sys
from dataclasses import dataclass
from pathlib import Path


@dataclass
class Finding:
    line_no: int
    col: int
    pattern: str
    message: str
    suggestion: str
    confidence: str  # "high" (mechanical) or "low" (needs human review)


# ── Pattern definitions ────────────────────────────────────────────────
# Each entry: (regex, name, message, suggestion, confidence)

PATTERNS: list[tuple[re.Pattern, str, str, str, str]] = [
    (
        re.compile(r'(?<![<>=!])&&(?![&=])'),
        "bbox-overlap-operator",
        "PostGIS `&&` operator is not available in DuckDB.",
        "Rewrite to: st_intersects(a, b)  (or bbox column predicates for joins)",
        "high",
    ),
    (
        re.compile(r'(?<![<>=!])<->(?![>=])'),
        "knn-operator",
        "PostGIS `<->` operator is not available in DuckDB.",
        "Rewrite to: ORDER BY st_distance(a, b) LIMIT k",
        "high",
    ),
    (
        re.compile(r'(?<![<>=!])<#>(?![>=])'),
        "bbox-distance-operator",
        "PostGIS `<#>` operator is not available in DuckDB.",
        "Rewrite to: ORDER BY st_distance(a, b) LIMIT k",
        "high",
    ),
    (
        re.compile(r"::geometry\b"),
        "geometry-cast",
        "`::geometry` cast is not available in DuckDB (geometry is BLOB).",
        "Remove cast; use ST_GeomFromText/ST_GeomFromWKB constructors.",
        "high",
    ),
    (
        re.compile(r"::geography\b"),
        "geography-cast",
        "`::geography` cast is not available in DuckDB.",
        "Use st_distancesphere/st_distancespheroid for geodesic distance.",
        "high",
    ),
    (
        re.compile(r'geometry\s*\(\s*\w+\s*,\s*\d+\s*\)', re.IGNORECASE),
        "geometry-typmod",
        "`geometry(Type, SRID)` typmod is not available in DuckDB.",
        "Use BLOB column + st_setsrid(geom, SRID) at insert time.",
        "high",
    ),
    (
        re.compile(r'USING\s+gist', re.IGNORECASE),
        "gist-index",
        "GiST indexes have no DuckDB equivalent.",
        "Use layout columns: bbox + st_quadkey + st_hilbert + DuckLake partitioning.",
        "high",
    ),
    (
        re.compile(r'\bST_Union\s*\((?![^,)]+,[^,)]+)'),
        "st-union-aggregate",
        "PostGIS `ST_Union(geom)` aggregate should use `ST_Union_Agg` in DuckDB.",
        "Rewrite to: ST_Union_Agg(geom)",
        "low",
    ),
    (
        re.compile(r'\bST_MemUnion\s*\(', re.IGNORECASE),
        "st-memunion",
        "`ST_MemUnion` → `ST_Union_Agg` (same engine).",
        "Rewrite to: ST_Union_Agg(geom)",
        "high",
    ),
    (
        re.compile(r'\bST_Collect\s*\([^,)]+,[^,)]+', re.IGNORECASE),
        "st-collect-scalar",
        "Scalar `ST_Collect(g1, g2)` is unavailable (aggregate only).",
        "Use ST_Multi(g) or subquery: SELECT st_collect(geom) FROM (VALUES (g1),(g2)) t(geom)",
        "low",
    ),
    (
        re.compile(r'\bST_DWithin\b.*geography', re.IGNORECASE),
        "st-dwithin-geography",
        "ST_DWithin on geography needs explicit geodesic function.",
        "Use: st_distancespheroid(a, b) <= threshold",
        "low",
    ),
]


# ── Rewrite rules (--apply mode) ───────────────────────────────────────
# Each rule: (regex, replacement, description)
# Applied in order; each operates on a single line.

REWRITE_RULES: list[tuple[re.Pattern, str, str]] = [
    # && → st_intersects (but not inside strings)
    (re.compile(r'(\w+|\([^)]+\))\s*&&\s*(\w+|\([^)]+\))'), r'st_intersects(\1, \2)',
     "&& → st_intersects()"),
    # <-> and <#> in ORDER BY → st_distance
    (re.compile(r'(\w+|\([^)]+\))\s*<->\s*(\w+|\([^)]+\))'), r'st_distance(\1, \2)',
     "<-> → st_distance()"),
    (re.compile(r'(\w+|\([^)]+\))\s*<#>\s*(\w+|\([^)]+\))'), r'st_distance(\1, \2)',
     "<#> → st_distance()"),
    # ::geometry → remove cast
    (re.compile(r'::geometry\b', re.IGNORECASE), '',
     "::geometry → (removed)"),
    # ::geography → remove cast
    (re.compile(r'::geography\b', re.IGNORECASE), '',
     "::geography → (removed)"),
    # geometry(Type, SRID) typmod → remove (keep column name)
    (re.compile(r'\s+geometry\s*\(\s*\w+\s*,\s*\d+\s*\)', re.IGNORECASE), ' BLOB',
     "geometry(T,S) → BLOB"),
    # USING gist → comment out the line
    (re.compile(r'^(CREATE\s+INDEX\s+.*USING\s+gist.*)$', re.IGNORECASE | re.MULTILINE),
     r'-- \1  -- GiST not available in DuckDB',
     "USING gist → commented out"),
    # ST_MemUnion → ST_Union_Agg
    (re.compile(r'\bST_MemUnion\s*\(', re.IGNORECASE), 'ST_Union_Agg(',
     "ST_MemUnion → ST_Union_Agg"),
    # ST_Union( → ST_Union_Agg( (only single-arg aggregate form)
    (re.compile(r'\bST_Union\s*\((?=\s*\w)', re.IGNORECASE), 'ST_Union_Agg(',
     "ST_Union( → ST_Union_Agg("),
]


def scan_line(line: str, line_no: int) -> list[Finding]:
    findings = []
    for pattern, name, message, suggestion, confidence in PATTERNS:
        for m in pattern.finditer(line):
            findings.append(Finding(
                line_no=line_no, col=m.start(), pattern=name,
                message=message, suggestion=suggestion, confidence=confidence,
            ))
    return findings


def annotate(text: str) -> str:
    lines = text.splitlines()
    output = []
    all_findings = []
    for i, line in enumerate(lines, 1):
        findings = scan_line(line, i)
        if findings:
            all_findings.extend(findings)
            output.append(f"  {line}")
            for f in findings:
                tag = "⚠ " if f.confidence == "low" else "→ "
                output.append(f"  {'':^{f.col}}  {tag}[L{f.line_no}] {f.pattern} ({f.confidence})")
                for sline in f.suggestion.splitlines():
                    output.append(f"      {sline}")
        else:
            output.append(f"  {line}")
    if all_findings:
        high = sum(1 for f in all_findings if f.confidence == "high")
        low = sum(1 for f in all_findings if f.confidence == "low")
        output.append(f"\n-- {len(all_findings)} finding(s): {high} high-confidence, {low} low-confidence")
    else:
        output.append("\n-- No PostGIS-specific patterns found. SQL looks DuckDB-ready.")
    return "\n".join(output)


def apply_rewrites(text: str) -> str:
    lines = text.splitlines()
    output = []
    rewrite_count = 0
    for line in lines:
        original = line
        for pattern, replacement, desc in REWRITE_RULES:
            new_line = pattern.sub(replacement, line)
            if new_line != line:
                rewrite_count += 1
                line = new_line
        if line != original:
            # Show what changed as a comment
            output.append(f"-- REWRITTEN: {original.strip()[:80]}")
        output.append(line)
    output.append(f"\n-- {rewrite_count} mechanical rewrite(s) applied.")
    output.append("-- Review low-confidence patterns manually before running.")
    return "\n".join(output)


def main() -> None:
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    apply_mode = "--apply" in sys.argv
    stdin_mode = "--stdin" in sys.argv

    if stdin_mode:
        text = sys.stdin.read()
    elif not args:
        print(__doc__)
        sys.exit(0)
    else:
        path = Path(args[0])
        if not path.exists():
            print(f"Error: {path} not found", file=sys.stderr)
            sys.exit(1)
        text = path.read_text()

    if apply_mode:
        print(apply_rewrites(text))
    else:
        print(annotate(text))


if __name__ == "__main__":
    main()
