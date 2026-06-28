#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Generate a catalog audit from src/registry.rs.

Parses the macro/builder registrations and groups every SQL function name by
provenance: literal SedonaDB bridge, local geo, GEOS, PROJ, GDAL/raster,
aggregate, table function, or extension-specific.

Usage:
    python3 tools/catalog_audit.py [--markdown] [path/to/registry.rs]

With --markdown, emits a markdown table suitable for docs.  Without it, emits
plain text counts.
"""
from __future__ import annotations

import re
import sys
from collections import defaultdict
from dataclasses import dataclass, field
from pathlib import Path


@dataclass
class CatalogEntry:
    name: str
    provenance: str  # "literal-sedonadb", "local-geo", "geos", "proj", "gdal-raster", "aggregate", "table-function", "extension"
    signature: str = ""
    sedona_name: str = ""  # the SedonaDB kernel name (for bridge entries)


def classify(
    name: str,
    macro: str,
    sedona_name: str,
    is_aggregate: bool = False,
    is_table_fn: bool = False,
) -> str:
    if is_aggregate:
        return "aggregate"
    if is_table_fn:
        if "raster" in name or "pixeldata" in name:
            return "gdal-raster"
        if name == "sedona_join":
            return "extension"
        return "table-function"
    if sedona_name:
        return "literal-sedonadb"
    # Local registrations — classify by backend hint from comments / known names.
    if name in ("st_transform",):
        return "proj"
    if name in (
        "st_node", "st_polygonize", "st_buildarea", "st_voronoipolygons",
        "st_snap", "st_makevalid",
    ):
        return "geos"
    return "local-geo"


# Regex patterns for all registration macros.
# Each captures the SQL name (first quoted string) and optionally the SedonaDB
# kernel name (second quoted string, only for register_sedona_* macros).
MACRO_PATTERNS: list[tuple[str, re.Pattern[str]]] = [
    # SedonaDB bridge macros (two-arg: sql_name, sedona_kernel)
    ("sedona", re.compile(
        r'register_sedona_\w+!\(\s*"([^"]+)"\s*,\s*"([^"]+)"'
    )),
    # Local macros (one-arg: sql_name)
    ("local", re.compile(
        r'register_(?:unary_geom|binary_geom|predicate|geom_double|geom_varchar|'
        r'geom_int|geom_bool|binary_double|geom_int_to_geom|geom_double_to_geom|'
        r'geom_double2_to_geom|geom_double6_to_geom|geom_int2_to_geom|'
        r'geom_int_to_varchar|str_geom|doubles2_geom|doubles4_geom|'
        r'binary_geom_varchar|geom_geom_str_predicate|'
        r'geom_int_geom_to_geom|geom_double3_to_geom)!'
        r'\(\s*"([^"]+)"'
    )),
]

# Inline builder patterns (for manually-registered functions).
BUILDER_RE = re.compile(r'(?:Scalar|Aggregate|Table)FunctionBuilder::new\(\s*"([^"]+)"')


def parse_registry(path: Path) -> list[CatalogEntry]:
    """Parse registry.rs and return all catalog entries."""
    text = path.read_text()

    # Track which lines are inside the bridge section (after the bridge comment).
    bridge_section = False

    # First pass: find all inline builder registrations and their context.
    # These are GEOS, aggregate, table-function, and a few special cases.
    entries: list[CatalogEntry] = []
    seen_names: set[str] = set()

    lines = text.splitlines()
    i = 0
    while i < len(lines):
        line = lines[i]

        # Detect bridge section start.
        if "Literal Apache SedonaDB bridge" in line:
            bridge_section = True

        # Check for macro-based registrations.
        # SedonaDB bridge macros (two-arg).
        for pattern_name, pattern in MACRO_PATTERNS:
            m = pattern.search(line)
            if m:
                if pattern_name == "sedona":
                    sql_name = m.group(1)
                    sedona_name = m.group(2)
                    prov = "literal-sedonadb"
                else:
                    sql_name = m.group(1)
                    sedona_name = ""
                    prov = classify(sql_name, "", "")

                if sql_name not in seen_names:
                    entries.append(CatalogEntry(
                        name=sql_name,
                        provenance=prov,
                        sedona_name=sedona_name,
                    ))
                    seen_names.add(sql_name)
                break
        else:
            # Check for inline builder registrations.
            bm = BUILDER_RE.search(line)
            if bm:
                name = bm.group(1)
                if name in seen_names:
                    i += 1
                    continue
                # Look at context to classify.
                is_agg = "AggregateFunctionBuilder" in line
                is_table = "TableFunctionBuilder" in line
                prov = classify(name, "", "", is_aggregate=is_agg, is_table_fn=is_table)
                entries.append(CatalogEntry(name=name, provenance=prov))
                seen_names.add(name)

        i += 1

    return entries


def group_by_provenance(entries: list[CatalogEntry]) -> dict[str, list[str]]:
    groups: dict[str, list[str]] = defaultdict(list)
    for e in entries:
        groups[e.provenance].append(e.name)
    for k in groups:
        groups[k].sort()
    return groups


def print_summary(entries: list[CatalogEntry], markdown: bool = False) -> None:
    groups = group_by_provenance(entries)

    # Count routed public st_* (functions that share a literal kernel).
    st_names = {e.name for e in entries if e.name.startswith("st_") and not e.name.startswith("sedona_")}
    sedona_st_names = {e.name for e in entries if e.name.startswith("sedona_st_")}
    # A routed function has both st_X and sedona_st_X.
    routed = set()
    for e in entries:
        if e.provenance == "literal-sedonadb" and e.name.startswith("st_") and not e.name.startswith("sedona_"):
            routed.add(e.name)

    total = len(entries)
    st_count = len(st_names)
    sedona_count = len([e for e in entries if e.name.startswith("sedona_")])
    routed_count = len(routed)

    if markdown:
        print(f"| Metric | Count |")
        print(f"|--------|-------|")
        print(f"| Total SQL functions | {total} |")
        print(f"| Public `st_*` | {st_count} |")
        print(f"| Literal `sedona_st_*` | {len(sedona_st_names)} |")
        print(f"| Extension-specific | {total - st_count - len(sedona_st_names)} |")
        print(f"| `st_*` routed to literal kernel | {routed_count} |")
        print()
        print("### By backend")
        print(f"| Backend | Functions | Count |")
        print(f"|---------|-----------|-------|")
        for prov in ["literal-sedonadb", "local-geo", "geos", "proj", "gdal-raster", "aggregate", "table-function", "extension"]:
            if prov in groups:
                label = prov.replace("-", " ").title()
                fns = ", ".join(f"`{f}`" for f in groups[prov])
                print(f"| {label} | {fns} | {len(groups[prov])} |")
        print()
        if routed:
            print("### `st_*` functions routed to literal SedonaDB kernel")
            print(", ".join(f"`{f}`" for f in sorted(routed)))
    else:
        print(f"Catalog audit: {total} functions")
        print(f"  st_*:       {st_count}")
        print(f"  sedona_st_*: {len(sedona_st_names)}")
        print(f"  routed:     {routed_count}")
        print()
        for prov in ["literal-sedonadb", "local-geo", "geos", "proj", "gdal-raster", "aggregate", "table-function", "extension"]:
            if prov in groups:
                print(f"  {prov} ({len(groups[prov])}):")
                for f in groups[prov]:
                    print(f"    {f}")
                print()


def get_counts(entries: list[CatalogEntry]) -> dict[str, int]:
    """Return the canonical count dict derived from registry entries."""
    st_names = {e.name for e in entries if e.name.startswith("st_") and not e.name.startswith("sedona_")}
    sedona_st_names = {e.name for e in entries if e.name.startswith("sedona_st_")}
    extension_names = {e.name for e in entries if e.name.startswith("sedona_") and not e.name.startswith("sedona_st_")}
    routed = {
        e.name for e in entries
        if e.provenance == "literal-sedonadb"
        and e.name.startswith("st_")
        and not e.name.startswith("sedona_")
    }
    return {
        "total": len(entries),
        "st": len(st_names),
        "sedona": len(sedona_st_names),
        "extension": len(extension_names),
        "routed": len(routed),
    }


# --check mode: verify committed doc counts match the live registry.

_COUNT_RE = re.compile(r"\*?\*?(\d+)\*?\*?\s+SQL\s+functions", re.IGNORECASE)
# Match "159 public `st_*`" or "159 `st_*`" but NOT "36 public `st_*` functions route"
_ST_COUNT_RE = re.compile(r"(\d+)\s+(?:public\s+)?`st_\*`(?![^.]*rout)", re.IGNORECASE)
_SEDONA_COUNT_RE = re.compile(r"(\d+)\s+`sedona_st_\*`", re.IGNORECASE)
_ROUTED_COUNT_RE = re.compile(r"(\d+)\s+public\s+`st_\*`\s+functions?\s+route", re.IGNORECASE)

DOC_FILES = ["README.md", "COMPATIBILITY.md", "ROADMAP.md"]


def check_drift(entries: list[CatalogEntry], root: Path) -> int:
    """Check that committed counts in docs match the live registry.

    Returns 0 on success, 1 on drift.
    """
    actual = get_counts(entries)
    errors: list[str] = []

    for doc_name in DOC_FILES:
        doc_path = root / doc_name
        if not doc_path.exists():
            continue
        text = doc_path.read_text()

        for label, pattern, key in [
            ("total SQL functions", _COUNT_RE, "total"),
            ("st_* count", _ST_COUNT_RE, "st"),
            ("sedona_st_* count", _SEDONA_COUNT_RE, "sedona"),
            ("routed count", _ROUTED_COUNT_RE, "routed"),
        ]:
            m = pattern.search(text)
            if m:
                committed = int(m.group(1))
                if committed != actual[key]:
                    errors.append(
                        f"  {doc_name}: {label} = {committed} (expected {actual[key]})"
                    )

    if errors:
        print("Catalog drift detected — committed doc counts do not match registry:")
        for e in errors:
            print(e)
        print()
        print("Run: python3 tools/catalog_audit.py")
        print("Then update the counts in README.md, COMPATIBILITY.md, ROADMAP.md.")
        return 1

    print(
        f"Catalog check OK: {actual['total']} functions "
        f"({actual['st']} st_*, {actual['sedona']} sedona_st_*, "
        f"{actual['routed']} routed)."
    )
    return 0


# Functions that are intentionally local despite a literal SedonaDB twin existing.
# Each entry maps the st_* name to a short reason for keeping the local impl.
INTENTIONALLY_LOCAL: dict[str, str] = {
    "st_geomfromwkb": "trust-boundary constructor (input validation)",
    "st_geomfromewkb": "trust-boundary constructor (input validation)",
    "st_geomfromewkt": "trust-boundary constructor (input validation)",
    "st_geometryn": "bridge returns NULL for per-row varying integer index",
    "st_pointn": "bridge returns NULL for per-row varying integer index",
    "st_interiorringn": "bridge returns NULL for per-row varying integer index",
    "st_dimension": "local returns -1 for EMPTY (PostGIS parity); bridge returns 0",
    "st_setsrid": "writes EWKB SRID tag on the blob; bridge models SRID at type level and drops it",
    "st_srid": "reads EWKB SRID tag from the blob; bridge always returns 0 for plain WKB",
}

# ---------------------------------------------------------------------------
# Upstream Apache SedonaDB scalar/aggregate function inventory.
# Sourced from sedona-functions/src/register.rs (scalar_udfs + aggregate_udfs).
# Each entry: (sql_name, classification, reason_if_not_bridgeable)
# Classifications: bridged, bridge-only, not-bridgeable
# ---------------------------------------------------------------------------

UPSTREAM_SEDONADB: list[tuple[str, str, str]] = [
    # Scalars that are bridged and have a routed public st_* counterpart.
    ("st_affine", "routed", ""),
    ("st_asbinary", "routed", ""),
    ("st_asewkb", "routed", ""),
    ("st_astext", "routed", ""),
    ("st_azimuth", "routed", ""),
    ("st_dimension", "routed", ""),
    ("st_envelope", "routed", ""),
    ("st_flipcoordinates", "routed", ""),
    ("st_force2d", "routed", ""),
    ("st_force3d", "routed", ""),
    ("st_force3dm", "bridge-only", "PostGIS delta: explicit z/m parameter required"),
    ("st_force4d", "bridge-only", "PostGIS delta: explicit z/m parameters required"),
    ("st_geometryn", "intentionally-local", "bridge returns NULL for per-row varying integer index"),
    ("st_geometrytype", "routed", ""),
    ("st_geomfromewkb", "intentionally-local", "trust-boundary constructor (input validation)"),
    ("st_geogfromwkb", "bridge-only", "geography-type constructor, no public counterpart"),
    ("st_geomfromwkb", "intentionally-local", "trust-boundary constructor (input validation)"),
    ("st_geomfromwkbunchecked", "bridge-only", "unvalidated constructor, literal-only for debugging"),
    ("st_geogfromwkt", "bridge-only", "geography-type constructor, no public counterpart"),
    ("st_geomcollfromtext", "bridge-only", "typed constructor, no public counterpart"),
    ("st_geomfromewkt", "intentionally-local", "trust-boundary constructor (input validation)"),
    ("st_geomfromwkt", "routed", ""),
    ("st_linefromtext", "routed", ""),
    ("st_mlinefromtext", "bridge-only", "typed constructor, no public counterpart"),
    ("st_mpointfromtext", "bridge-only", "typed constructor, no public counterpart"),
    ("st_mpolyfromtext", "bridge-only", "typed constructor, no public counterpart"),
    ("st_pointfromtext", "routed", ""),
    ("st_polygonfromtext", "routed", ""),
    ("st_hasm", "routed", ""),
    ("st_hasz", "routed", ""),
    ("st_interiorringn", "intentionally-local", "bridge returns NULL for per-row varying integer index"),
    ("st_isclosed", "routed", ""),
    ("st_iscollection", "routed", ""),
    ("st_isempty", "routed", ""),
    ("st_knn", "not-bridgeable", "special KNN operator, not a scalar UDF"),
    ("st_makeline", "routed", ""),
    ("st_numgeometries", "routed", ""),
    ("st_geogpoint", "bridge-only", "geography-type constructor"),
    ("st_point", "routed", ""),
    ("st_pointn", "intentionally-local", "bridge returns NULL for per-row varying integer index"),
    ("st_numpoints", "routed", ""),
    ("st_points", "routed", ""),
    ("st_pointm", "bridge-only", "Z/M point constructor, no public counterpart"),
    ("st_pointz", "bridge-only", "Z/M point constructor, no public counterpart"),
    ("st_pointzm", "bridge-only", "Z/M point constructor, no public counterpart"),
    ("st_reverse", "routed", ""),
    ("st_rotate", "routed", ""),
    ("st_rotate_x", "bridge-only", "3D rotation, no public counterpart"),
    ("st_rotate_y", "bridge-only", "3D rotation, no public counterpart"),
    ("st_scale", "routed", ""),
    ("st_segmentize", "routed", ""),
    ("st_setcrs", "bridge-only", "CRS-tagged variant"),
    ("st_setsrid", "routed", ""),
    ("st_crs", "bridge-only", "CRS-tagged variant"),
    ("st_srid", "routed", ""),
    ("st_endpoint", "routed", ""),
    ("st_startpoint", "routed", ""),
    ("st_togeography", "not-bridgeable", "geography type conversion, no DuckDB geography type"),
    ("st_togeometry", "not-bridgeable", "geography type conversion, no DuckDB geography type"),
    ("st_translate", "routed", ""),
    ("st_mmax", "bridge-only", "Z/M accessor, no public counterpart"),
    ("st_mmin", "bridge-only", "Z/M accessor, no public counterpart"),
    ("st_xmax", "routed", ""),
    ("st_xmin", "routed", ""),
    ("st_ymax", "routed", ""),
    ("st_ymin", "routed", ""),
    ("st_zmax", "bridge-only", "Z/M accessor, no public counterpart"),
    ("st_zmin", "bridge-only", "Z/M accessor, no public counterpart"),
    ("st_m", "routed", ""),
    ("st_x", "routed", ""),
    ("st_y", "routed", ""),
    ("st_z", "routed", ""),
    ("st_zmflag", "routed", ""),
    ("st_linesubstring", "routed", ""),
    # Aggregates.
    ("st_analyze_agg", "not-bridgeable", "aggregate — local st_envelope_agg / st_union_agg serve this role"),
    ("st_collect_agg", "not-bridgeable", "aggregate — local st_collect aggregate"),
    ("st_envelope_agg", "not-bridgeable", "aggregate — local st_envelope_agg aggregate"),
    # Extension-specific (not st_*).
    ("sd_format", "not-bridgeable", "SedonaDB extension function, not spatial SQL"),
    ("sd_order", "not-bridgeable", "SedonaDB extension function"),
    ("sd_simplifystorage", "not-bridgeable", "SedonaDB extension function"),
    # Table functions (not scalar).
    ("st_dump", "not-bridgeable", "table function — local ST_Dump with different return shape"),
]


def compat_check(entries: list[CatalogEntry], root: Path) -> int:
    """Cross-reference sedona_st_* bridge functions against public st_*.

    Reports:
      - Bridge-only: sedona_st_* with no public st_* counterpart.
      - Routing candidates: sedona_st_* where a local st_* exists but isn't routed
        and isn't in the INTENTIONALLY_LOCAL allowlist.

    Returns 0 on success, 1 on undocumented routing decisions.
    """
    all_names = {e.name for e in entries}
    routed = {
        e.name for e in entries
        if e.provenance == "literal-sedonadb"
        and e.name.startswith("st_")
        and not e.name.startswith("sedona_")
    }
    sedona_kernel_names = {
        e.sedona_name for e in entries
        if e.name.startswith("sedona_st_") and e.sedona_name
    }
    local_st = {
        e.name for e in entries
        if e.provenance == "local-geo"
        and e.name.startswith("st_")
        and not e.name.startswith("sedona_")
    }

    errors: list[str] = []

    # 1. Routing candidates: local st_* where a sedona_st_* twin exists
    #    and the function is neither routed nor intentionally local.
    undocumented = []
    for name in sorted(local_st):
        if name in routed:
            continue
        if name in INTENTIONALLY_LOCAL:
            continue
        # Check if a sedona_st_* twin with the same base name exists.
        sedona_twin = "sedona_" + name
        if sedona_twin in all_names:
            undocumented.append(name)

    if undocumented:
        errors.append("Undocumented routing decisions (local st_* with literal twin):")
        for name in undocumented:
            twin = "sedona_" + name
            errors.append(f"  {name}: has literal twin {twin} but is local-geo")
            errors.append(f"    → route it or add to INTENTIONALLY_LOCAL with a reason")

    # 2. Bridge-only: sedona_st_* with no public st_* counterpart.
    bridge_only = []
    for e in sorted(entries, key=lambda x: x.name):
        if not e.name.startswith("sedona_st_"):
            continue
        base = e.name.removeprefix("sedona_")
        if base not in all_names:
            bridge_only.append(e.name)

    # Report (informational, not an error).
    if bridge_only:
        print("Bridge-only functions (no public st_* counterpart):")
        for name in bridge_only:
            print(f"  {name}")
        print()

    if errors:
        print("\n".join(errors))
        print()
        print("Fix: either route the function to the literal kernel, or add it to")
        print("INTENTIONALLY_LOCAL in tools/catalog_audit.py with a reason.")
        return 1

    print(
        f"Compat check OK: {len(routed)} routed, "
        f"{len(INTENTIONALLY_LOCAL)} intentionally local, "
        f"{len(bridge_only)} bridge-only."
    )
    return 0


def generate_ledger(entries: list[CatalogEntry], root: Path) -> int:
    """Generate docs/SEDONA_LEDGER.md — the stable, classified function table.

    This is the mechanically-audited compatibility contract. It cross-references
    the live registry against the known upstream SedonaDB inventory. Run in CI
    to detect drift between registry, upstream classification, and docs.
    """
    counts = get_counts(entries)
    all_names = {e.name for e in entries}
    routed = sorted(
        e.name for e in entries
        if e.provenance == "literal-sedonadb"
        and e.name.startswith("st_")
        and not e.name.startswith("sedona_")
    )
    local_st = sorted(
        e.name for e in entries
        if e.provenance == "local-geo"
        and e.name.startswith("st_")
        and not e.name.startswith("sedona_")
    )
    bridge_only = sorted(
        e.name for e in entries
        if e.name.startswith("sedona_st_")
        and e.name.removeprefix("sedona_") not in all_names
    )

    lines: list[str] = []
    lines.append("# SedonaDB compatibility ledger")
    lines.append("")
    lines.append(
        "<!-- AUTO-GENERATED by `python3 tools/catalog_audit.py --generate-ledger`. -->\n"
        "<!-- Do not edit by hand; update the registry and re-run the generator. -->"
    )
    lines.append("")
    lines.append(f"**Total functions:** {counts['total']}  ")
    lines.append(f"**Public `st_*`:** {counts['st']}  ")
    lines.append(f"**Literal `sedona_st_*`:** {counts['sedona']}  ")
    lines.append(f"**Routed to literal kernel:** {counts['routed']}  ")
    lines.append(
        f"**Intentionally local:** {len(INTENTIONALLY_LOCAL)}  "
    )
    lines.append("")

    # Upstream classification.
    lines.append("## Upstream Apache SedonaDB classification")
    lines.append("")
    lines.append(
        "Every kernel in `sedona-functions/src/register.rs` is classified as "
        "one of: **routed** (public `st_*` routes to the literal kernel), "
        "**intentionally-local** (has a bridge twin but kept local for a "
        "documented reason), **bridge-only** (exposed only as `sedona_st_*`), "
        "or **not-bridgeable** (table fn, aggregate, geography type, or special "
        "operator)."
    )
    lines.append("")

    for cls, label in [
        ("routed", "Routed to literal kernel"),
        ("intentionally-local", "Intentionally local"),
        ("bridge-only", "Bridge-only (literal `sedona_st_*`, no public counterpart)"),
        ("not-bridgeable", "Not bridgeable"),
    ]:
        items = [(n, r) for (n, c, r) in UPSTREAM_SEDONADB if c == cls]
        if not items:
            continue
        lines.append(f"### {label} ({len(items)})")
        lines.append("")
        lines.append("| Function | Reason |")
        lines.append("|---|---|")
        for name, reason in items:
            lines.append(f"| `{name}` | {reason or "—"} |")
        lines.append("")

    # Routed functions (live registry).
    lines.append("## Routed public `st_*` (live registry)")
    lines.append("")
    lines.append(
        f"These {len(routed)} public `st_*` functions route to the literal "
        "Apache SedonaDB kernel:"
    )
    lines.append("")
    for name in routed:
        lines.append(f"- `{name}`")
    lines.append("")

    # Intentionally local (live registry).
    lines.append("## Intentionally local `st_*` with literal twin")
    lines.append("")
    if INTENTIONALLY_LOCAL:
        lines.append("| Function | Reason |")
        lines.append("|---|---|")
        for name in sorted(INTENTIONALLY_LOCAL):
            lines.append(f"| `{name}` | {INTENTIONALLY_LOCAL[name]} |")
    else:
        lines.append("_(none)_")
    lines.append("")

    # Bridge-only.
    lines.append("## Bridge-only `sedona_st_*` (no public counterpart)")
    lines.append("")
    if bridge_only:
        for name in bridge_only:
            lines.append(f"- `{name}`")
    else:
        lines.append("_(none)_")
    lines.append("")

    out_path = root / "docs" / "SEDONA_LEDGER.md"
    out_path.write_text("\n".join(lines) + "\n")
    print(f"Ledger written to {out_path.relative_to(root)}")
    return 0


def export_json(entries: list[CatalogEntry], root: Path) -> int:
    """Export machine-readable JSON compatibility data.

    Shares the same source as the Markdown ledger so docs, tools, and release
    notes always agree.
    """
    import json

    counts = get_counts(entries)
    all_names = {e.name for e in entries}
    routed_list = sorted(
        e.name for e in entries
        if e.provenance == "literal-sedonadb"
        and e.name.startswith("st_")
        and not e.name.startswith("sedona_")
    )
    bridge_only_list = sorted(
        e.name for e in entries
        if e.name.startswith("sedona_st_")
        and e.name.removeprefix("sedona_") not in all_names
    )

    data = {
        "counts": counts,
        "intentionally_local": {
            name: INTENTIONALLY_LOCAL[name]
            for name in sorted(INTENTIONALLY_LOCAL)
        },
        "upstream_sedonadb": [
            {"name": n, "classification": c, "reason": r}
            for n, c, r in UPSTREAM_SEDONADB
        ],
        "routed": routed_list,
        "bridge_only": bridge_only_list,
    }

    out_path = root / "docs" / "sedonadb_compat.json"
    out_path.write_text(json.dumps(data, indent=2) + "\n")
    print(f"JSON export written to {out_path.relative_to(root)}")
    return 0


def main() -> None:
    check_mode = "--check" in sys.argv
    compat_mode = "--compat-check" in sys.argv
    ledger_mode = "--generate-ledger" in sys.argv
    json_mode = "--export-json" in sys.argv
    markdown = "--markdown" in sys.argv
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    registry_path = Path(args[0]) if args else Path("src/registry.rs")

    root = Path(__file__).resolve().parent.parent

    if not registry_path.exists():
        # Try relative to script location.
        registry_path = root / "src" / "registry.rs"

    if not registry_path.exists():
        print(f"Error: cannot find registry.rs at {registry_path}", file=sys.stderr)
        sys.exit(1)

    entries = parse_registry(registry_path)

    if check_mode:
        sys.exit(check_drift(entries, root))

    if compat_mode:
        sys.exit(compat_check(entries, root))

    if ledger_mode:
        sys.exit(generate_ledger(entries, root))

    if json_mode:
        sys.exit(export_json(entries, root))

    print_summary(entries, markdown=markdown)


if __name__ == "__main__":
    main()
