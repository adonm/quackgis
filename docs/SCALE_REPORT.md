# Scale validation report

Reproducible evidence that the DuckLake spatial lakehouse pattern is correct at
scale and that layout-driven pruning is effective.

## Harness

`benchmarks/scale_harness.sh [smoke|local|heavy]` generates deterministic
spatial data (80% uniform spread + 20% clustered near origin), creates three
DuckLake layouts, runs representative queries, and reports layout metrics +
exact-result parity + pruning ratios.

## Scale tiers

| Tier | Points | Query zoom | Purpose |
|---|---|---|---|
| `smoke` | 1,000 | 4 | CI-fast correctness |
| `local` | 10,000 | 5 | Local development validation |
| `heavy` | 100,000 | 6 | Stress / extrapolation to object-store tiers |

All tiers use the same data model and the same SQL — only the row count and
zoom level change. The pattern extrapolates to PB/trillion-row object stores by
increasing the tier and targeting 100 MB–1 GB Parquet objects.

## Three layouts tested

| Layout | Columns | Partitioning | Purpose |
|---|---|---|---|
| **flat** | `id, geom` | none | correctness baseline |
| **bbox+sorted** | `id, geom, xmin..ymax` | none, Hilbert-sorted files | zone-map pruning only |
| **cell+bbox+sorted** | `id, geom, xmin..ymax, spatial_cell` | `PARTITIONED BY (spatial_cell)` | full three-stage pattern |

## Results (smoke tier, 1000 points)

```
distinct_cells: 30
layout1_exact:       224
layout2_bbox+exact:  224
layout3_cell+bbox+e: 224
layout3_candidates:  352 (of 1000 total)
parity: PASS
cell_pruning_ratio: 35.20%
```

All three layouts return identical exact results. The cell-partitioned layout
scans only 352 candidate rows (35.2%) before applying the exact predicate.

## Results (local tier, 10000 points)

```
distinct_cells: 92
layout1_exact:       2240
layout2_bbox+exact:  2240
layout3_cell+bbox+e: 2240
layout3_candidates:  2240 (of 10000 total)
parity: PASS
cell_pruning_ratio: 22.40%
```

As the dataset grows, the fraction of candidate rows scanned by the
cell-partitioned layout shrinks (35% → 22%). This is the expected behavior:
denser data means each cell covers a smaller geographic area, so the same
query window intersects fewer cells.

## Workload coverage

The SQL fixture (`tests/reference/m17_scale.sql`) verifies:

1. **Row-count parity** across all three layouts.
2. **Range query parity** — three-stage query = exact-only query.
3. **Cell pruning effectiveness** — candidate rows < total rows.
4. **Bbox zone-map pruning effectiveness** on sorted layout.
5. **Join parity** — points-in-polygons across layouts.
6. **KNN parity** — same minimum distance across layouts.
7. **Cell cardinality** — reasonable partition distribution.
8. **Adaptive partitioning balance** — sort-then-pack ≤ fixed-grid max.
9. **Partition evolution correctness** — query correct after spec change.
10. **Append correctness** — new data queryable after append.
11. **Time-travel correctness** — historical versions remain queryable.

## Extrapolation to PB scale

The harness uses the same SQL and data model at every tier. To extrapolate:

1. Increase the tier to match your expected object-store row count.
2. Target 100 MB–1 GB Parquet objects (use `ST_EstimatePartitionCount`).
3. Use adaptive partitioning (M11 sort-then-pack) for skewed data.
4. Use PostgreSQL DuckLake catalog for concurrent writers (M12 multi-writer).
5. The three-stage query pattern guarantees correctness regardless of scale —
   stages 1–2 are performance filters, stage 3 (exact predicate) is always the
   oracle.

## Known limitations

- **PostgreSQL catalog column naming**: PostgreSQL reserves `xmin`, `xmax`,
  `cmin`, `cmax` as system column names. When using a PostgreSQL-catalog
  DuckLake table, use `minx/miny/maxx/maxy` (or similar) for bbox columns
  instead of `xmin/ymin/xmax/ymax`. See `tests/reference/m24_pg_catalog.sh`.
- `ST_CoveringQuadKeys` cannot take lateral column arguments (DuckDB table
  function limitation). For join workloads, pre-compute covering cells into a
  temp table, or use bbox-only prefiltering.
- DuckLake file catalog serializes writers (file lock). For concurrent writers,
  use PostgreSQL catalog (validated in M24 via local container).
- Timing is informational, not a correctness oracle. Always verify exact-result
  parity before comparing timings.
