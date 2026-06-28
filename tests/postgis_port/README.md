# PostGIS workload portability harness

These files test how real PostGIS SQL ports to the `sedonadb` extension. Each
case shows the **original PostGIS SQL** as a comment, the **expected PostGIS
result** (known-good or generated from a pinned PostGIS), and the **ported
DuckDB SQL** that verifies the result.

## Running

The port cases are included in the main SQL regression suite
(`tests/run_sql.sh`). They use the same `CASE WHEN … THEN 'PORT …' ELSE 'FAIL …'`
convention as the milestone fixtures.

## Generating expected outputs from real PostGIS

When Docker is available, `generate_expected.sh` starts a pinned PostGIS
container, runs the PostGIS source SQL from each case file, and emits the
expected results to `expected/`. This is **optional** — the case files carry
hand-verified expected values for standard geometries. The Docker generator is
for full regression coverage and is skippable in CI without Docker.

```bash
# Optional: generate expected outputs from real PostGIS
./tests/postgis_port/generate_expected.sh

# Always: run port cases against the extension
./tests/run_sql.sh
```

## Case file conventions

- `.mode list` at the top to keep DuckDB output parseable.
- `-- PG:` prefix for the original PostGIS SQL (for the generator to extract).
- `-- Expected:` for the known-good result.
- `-- Rewrite:` when a mechanical change is needed (cast, operator, etc.).
- `CASE WHEN … THEN 'PORT <family> <description>' ELSE 'FAIL …'` for assertion.

## Status ledger

Each case is one of:

| Status | Meaning |
|---|---|
| `PORT` | Extension matches PostGIS result (exact or within tolerance). |
| `DELTA` | Extension matches with a documented semantic difference. |
| `SKIP` | Unsupported; documented in COMPATIBILITY.md with rationale. |
