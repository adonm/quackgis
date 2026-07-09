# QuackGIS project history

This is the contributor-facing explanation of how QuackGIS reached its current
architecture. It is intentionally selective: Git records every change, while this
document explains the decisions, dead ends, retained ideas, and safe places to
continue.

For current truth, use:

- [PROJECT_DIRECTION.md](./PROJECT_DIRECTION.md) for the product charter;
- [../ARCHITECTURE.md](../ARCHITECTURE.md) for design and invariants;
- [../ROADMAP.md](../ROADMAP.md) for future outcomes;
- [ROADMAP_STATUS.md](./ROADMAP_STATUS.md) for implemented evidence; and
- [../CHANGELOG.md](../CHANGELOG.md) for release-facing changes.

## The short version

QuackGIS started by asking how to expose Sedona-style spatial analytics through
familiar PostGIS workflows. Three prototypes answered different parts of that
question:

1. a DuckDB WKB/GeoRust spatial extension proved the vectorized execution shape;
2. a DuckLake experiment added literal SedonaDB bridging and showed snapshots/
   layout should be the storage contract;
3. a PostgreSQL facade explored client compatibility while exposing that a
   PostgreSQL + DuckDB + C-extension stack was too heavy and fragile before it
   reached end-to-end validation.

The current architecture keeps the lessons and discards the coupling: one Rust
pgwire service, DataFusion + SedonaDB execution, DuckLake/Parquet storage, and
small explicit compatibility boundaries.

## Timeline and commit anchors

The first three commits are large prototype snapshots rather than ordinary small
changes. They are useful archaeological anchors, not a sequence to replay.

| Date | Anchor | Era | Read it for |
|---|---|---|---|
| 2026-06-28 | `8788859` | DuckDB WKB/GeoRust extension | Original PostGIS-like surface, benchmark ideas, and vectorized WKB execution; real SedonaDB bridging was still future work. |
| 2026-06-29 | `e602e49` | DuckLake + Sedona bridge exploration | Expanded the initial local DuckLake benchmark into a literal SedonaDB bridge and broader storage/layout/snapshot design. |
| 2026-07-04 | `bc2b835` | PostgreSQL facade | Unfinished client-compatibility experiments, SQL wrappers, container/Kubernetes work, and the stack cost that triggered redesign before end-to-end validation. |
| 2026-07-05 | `1acd774` | Rust pgwire pivot | Start of the current architecture and `quackgis-server` crate. |
| 2026-07-05 | `351a92c` | DuckLake writer path | First current-architecture persistence/restart contract. |
| 2026-07-06 | `18b33fb` | QGIS read path | Shift from abstract compatibility to an end-to-end client gate. |
| 2026-07-07 | `67eecf7` | Editing/transactions | First narrow transaction semantics driven by real edit workflows. |
| 2026-07-08 | `e1f24a2` | Spatial layout oracle | Layout projection became a tested write/read contract; automatic hiding followed in `2babfa9`. |
| 2026-07-08 | `5177636` | Shared lake profile | Start of PostgreSQL/object-storage-shaped integration evidence. |
| 2026-07-09 | `760606d` | Atomic mutation API | One-snapshot delete/append/retire publication became a storage invariant. |
| 2026-07-09 | `80feb98` | Process metrics | Operational evidence became part of the product surface. |
| 2026-07-09 | `ba81c90` | Snapshot reads | First narrow query-scoped DuckLake time-travel path. |
| 2026-07-10 | `ae1b212` | Pre-parse SQL boundary | `AS OF SNAPSHOT` proved syntax compatibility sometimes belongs before AST hooks. |

Use `git show <anchor>` to inspect an era. Always compare it with current
architecture/status before reusing code or claims.

## Era 1 — spatial engine prototype

### Question

Can a PostGIS-like vectorized WKB spatial surface run efficiently in DuckDB, and
can that path evolve toward SedonaDB rather than a permanent local kernel fork?

### What was built

The first prototype was a DuckDB extension using GeoRust algorithms behind a
Sedona/PostGIS-like function surface, with benchmarks and compatibility
experiments. The following DuckLake exploration added literal SedonaDB bridge
experiments; the current native SedonaDB integration arrived after the Rust pgwire
pivot.

### What survived

- SedonaDB remains the exact spatial engine.
- Columnar fanout, grouped analytics, and candidate narrowing remain core goals.
- WKB is still the durable compatibility format.
- Focused semantic fixtures are more valuable than a raw function count.

### Why it changed

An engine extension did not solve pgwire clients, PostgreSQL catalog expectations,
shared snapshot storage, authorization, deployment, or recovery. DuckDB also
created an unnecessary engine boundary once DataFusion-native Sedona and DuckLake
were available.

## Era 2 — DuckLake and spatial-layout exploration

### Question

Can open Parquet storage, snapshot metadata, and deterministic spatial layout
replace a mutable PostGIS table/index as the durable data plane?

### What was learned

- Layout must be computed at write time and remain correctness-neutral.
- Exact spatial recheck is mandatory; bbox/layout predicates may only over-select.
- COPY/grouped writes and compaction are layout primitives, not just convenience
  features.
- Snapshot publication is the natural mutation/release boundary.
- Plans that mention billions, assets, or partitions need measurable gates before
  they become claims.

### What was discarded

Early documents mixed current behavior, multi-year ambition, and speculative
partition/index APIs. Current docs separate implemented ordinary hidden columns
from future true DuckLake partitioning.

## Era 3 — PostgreSQL facade prototype

### Question

Could QuackGIS get PostGIS clients “for free” by running PostgreSQL in front of
DuckDB/DuckLake?

### What was built

A PostgreSQL container combined pg_ducklake, a DuckDB/Sedona bridge, SQL wrappers,
a C geometry type, client tests, Helm/Kubernetes assets, and backup experiments.
Old internal documents called parts of this `v0.1` and `v0.2`; no Git release tags
were published.

### What survived

- QGIS/OGR/GeoServer/driver workflows define compatibility.
- `geometry_columns`, OIDs, row descriptions, parameters, cursors, and catalog
  shapes are real product interfaces.
- Backup, deployment, and client evidence belong beside query features.
- Useful tests should be black-box and workflow-shaped.

### Why it was retired

The stack required a PostgreSQL server, pg_ducklake table access, DuckDB, native
GIS libraries, C-extension ABI compatibility, SQL stubs, and several routing
layers. It optimized for borrowing PostgreSQL internals instead of building the
spatial lakehouse service QuackGIS wanted to operate.

Do not restore the retired `container/init.d`, pg_ducklake, C geometry extension,
or DuckDB bridge architecture. Recover only a focused test or compatibility idea
whose current owner is clear.

## Era 4 — current Rust pgwire architecture

The July 5 pivot made each responsibility explicit:

- `datafusion-postgres` owns pgwire parsing/protocol plumbing;
- `datafusion-pg-catalog` and QuackGIS shims provide PostgreSQL/PostGIS metadata;
- SedonaDB owns exact spatial kernels;
- DataFusion owns SQL planning/vectorized execution;
- `datafusion-ducklake` owns snapshot catalog and Parquet IO; and
- QuackGIS owns integration policy, safety checks, layout, maintenance,
  authorization, and evidence.

### Compatibility became boundary-oriented

Client traces showed four different extension points:

1. raw SQL preprocessing before parsing;
2. parsed statement/planning hooks;
3. catalog/type/parameter/result encoding; and
4. stateful COPY/cursor/portal handlers.

Put a fix at the narrowest correct boundary. Do not solve a wire-state problem
with string rewriting or a catalog-shape problem inside a spatial kernel.

### Storage became invariant-oriented

Native DELETE/UPDATE/compaction work established the central write rule: object
files may be prewritten, but all visible metadata for one operation publishes in
one snapshot. Physical row lineage also requires a deliberately isolated scan
mode; normal optimization is unsafe if it can renumber positions.

### Evidence became part of delivery

Local tests expanded into Kind multi-pod, QPS/OLAP, Linkerd, real OSM, client,
failure-injection, metrics, and release-artifact gates. The important lesson is
not that Kind proves production—it does not—but that every expensive external or
scale run needs a deterministic companion and machine-readable budgets.

## Lessons contributors inherit

1. **Correctness before compatibility illusion.** Fail closed when PostgreSQL-like
   behavior cannot be represented safely.
2. **Exact recheck before pruning cleverness.** False negatives are unacceptable.
3. **One publication before mutation speed.** Half-visible DML is a correctness
   bug; orphan cleanup is a separate operation.
4. **Trace before shim.** Capture the client behavior and classify the reusable
   protocol/catalog surface first.
5. **Current storage truth must be explicit.** PostgreSQL multicatalog is
   non-standard; SQLite is spec-oriented but not drop-in DuckDB-writable;
   geometry identity still has heuristic debt.
6. **Plans are not evidence.** Name rows, bytes, profile, client version, hardware,
   source SHA, and artifacts.
7. **Forks are tools, not architecture goals.** Preserve a small tested capability
   and a migration trigger in `DIVERGENCE.md`.
8. **Heavy assets stay sidecar-first.** Add real inventory/lifecycle evidence
   before adding format decoders to the server.

## Where new contributors should start

### To understand a request

| Request type | Start here |
|---|---|
| Product priority or milestone | `ROADMAP.md`, then `docs/ROADMAP_STATUS.md` |
| Architecture or invariant | `ARCHITECTURE.md` |
| pgwire server assembly | `crates/quackgis-server/src/pgwire_server.rs` |
| DuckLake SQL, DML, snapshots, COPY | `crates/quackgis-server/src/ducklake_sql.rs` and its submodules |
| PostgreSQL/PostGIS catalog behavior | `crates/quackgis-server/src/catalog_compat.rs` and `postgis_compat.rs` |
| Spatial aliases/kernels | `crates/quackgis-server/src/spatial_udfs.rs`, then SedonaDB upstream |
| Storage fork behavior | `vendor/datafusion-ducklake`, `docs/NATIVE_DML_FORK_PLAN.md`, `DIVERGENCE.md` |
| Pgwire fork behavior | `vendor/datafusion-postgres`, `DIVERGENCE.md` |
| Current client claim | `docs/COMPATIBILITY.md` and `docs/COMPATIBILITY_MATRIX.md` |
| Operations/security | `docs/OPERATIONS.md`, `docs/SECURITY_RBAC.md` |

### Before coding

1. Identify the trust boundary and current evidence ring.
2. Find the focused test/probe that owns the behavior.
3. Check whether the behavior is QuackGIS policy or an upstream/fork concern.
4. Preserve exact spatial and one-snapshot mutation invariants.
5. Add the cheapest deterministic gate before a Kind/external gate.
6. Update current status or divergence; do not paste completed implementation into
   the forward roadmap.

### Useful archaeology commands

```sh
git show 1acd774                 # current architecture pivot
git log -- <path>               # history of one current surface
git blame <path>                # why a current line exists; follow with git show
git log --reverse --oneline     # era order, including the three prototype snapshots
```

If an old commit contradicts current architecture, the current architecture and
tests win. History explains decisions; it does not override maintained behavior.
