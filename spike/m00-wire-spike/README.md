# M0 wire spike

Throwaway spike to validate the make-or-break assumption of the redesign:
**`psql` can talk to datafusion-postgres over a SedonaDB `SessionContext`,
and spatial functions execute end-to-end.**

Lives alongside v0.1 (do not retire v0.1 until this passes). Not part of any
workspace; standalone crate.

## What it tests

- G9: SedonaDB is consumable as a git dep (not on crates.io) — `sedona`
  umbrella crate registers its full function set via
  `SedonaContext::new_from_context`.
- Wire stack: `datafusion-postgres` master (DataFusion 53) served over pgwire
  on top of a Sedona-enabled context.
- G11: SedonaDB `quackgis/df53` fork-bump aligns DF/Arrow with
  datafusion-postgres so both upstreams can be tracked at HEAD (see
  `DIVERGENCE.md` upstream of this repo).
- End-to-end: `SELECT ST_AsText(ST_GeomFromText('POINT(1 2)'))` from psql.

## What it does NOT test (yet)

- pg_catalog introspection (QGIS layer discovery) — needs the v0.15
  `setup_pg_catalog(ctx, name, auth_provider)` call with a context provider.
- DuckLake storage (M1).
- BINARY cursor format (the gap ledger G3 follow-up — machinery exists
  upstream but `CursorStatementHook` ignores the BINARY keyword).
- Geometry wire encoding as a real PG type OID (G1).

## Run

```sh
cargo run --release          # listens on 0.0.0.0:5433
# in another shell:
psql -h 127.0.0.1 -p 5433 -U postgres -c "SELECT ST_AsText(ST_GeomFromText('POINT(1 2)'))"
```

## Findings

See `FINDINGS.md` after the probe completes (recorded into the gap ledger in
`ROADMAP.md`).
