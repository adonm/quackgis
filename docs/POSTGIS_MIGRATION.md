# Offline PostGIS migration

`quackgis-migrate` implements the first executable G0 slice: it inventories an
exactly pinned PostgreSQL/PostGIS source in one read-only repeatable-read
transaction, classifies the maintained inventory categories across selected
schemas, streams accepted tables through PostgreSQL text COPY to QuackGIS, and verifies canonical source and
target checksums before one target transaction commits.

This is executable preview evidence, not a complete migration product. The
provenance-pinned runtime now includes the migrator and a packaged Kind gate uses
a dedicated credential, role, client CA, and mutual-TLS iroh tiny client.
Automatic staging-root promotion, runtime identity inside the report, role/grant
application, restart verification, keys, nonzero CRS, geography, non-Point
geometry, named-client post-migration gates, and operator cutover remain open.

## Maintained smoke

```sh
mise exec -- just postgis-migration-smoke
```

The recipe pulls the digest-pinned PostgreSQL 18/PostGIS 3.6 image, starts a fresh
actual QuackGIS/DuckLake server, and runs three cases:

1. copy release scalars plus Point/NULL WKB while a concurrent source write commits
   outside the held snapshot;
2. reject an invalid target date and prove the complete target DDL/COPY transaction
   leaves no tables; and
3. reject a primary-key source before attempting to reach an unavailable target.

The smoke uses plaintext only on literal loopback addresses. It is not the
release-network contract.

The packaged target-path gate is separate because it pulls an external pinned
PostGIS fixture image:

```sh
mise exec -- just kind-up-local
mise exec -- just kind-postgis-migration-gate
```

The normal K0 client/REST and direct-worker denial matrix runs first. An optional
Job then starts digest-pinned PostgreSQL 18.4/PostGIS 3.6.4 as a native sidecar,
executes the non-root migrator from the immutable runtime image, and transfers
10,002 scalar rows plus two Point/NULL rows through a credential-bound
`migration_operator` lease. The migration Service trusts a separate migration
client CA: the ordinary K0 client certificate is explicitly denied. The PostGIS
sidecar and source trust mode exist only inside this one Job's network namespace;
they are absent from normal topology startup.

## Configuration

The JSON configuration is bounded to 1 MiB, rejects unknown fields, and requires
an explicit source scope and source-to-target table list:

```json
{
  "format_version": 1,
  "source": {
    "postgres_version_num": 180004,
    "postgis_version": "3.6.4"
  },
  "source_schemas": ["public", "survey"],
  "tables": [
    {
      "source_schema": "public",
      "source_table": "places",
      "target_schema": "main",
      "target_table": "migrated_places",
      "column_mappings": {"location": "geom_wkb"}
    },
    {
      "source_schema": "survey",
      "source_table": "readings",
      "target_schema": "main",
      "target_table": "migrated_readings"
    }
  ]
}
```

Every configured target table must be absent and its target schema must already
exist. A geometry target must use one of the maintained WKB geometry names so
COPY and RowDescription cannot silently expose it as ordinary `bytea`.

The inventory covers all base/partitioned tables, columns, constraints, views,
materialized views, sequences, foreign tables, indexes, non-extension functions,
triggers, extensions, owner roles, and visible table/column grants in the selected
schemas. Unselected objects remain in the report with a `reject` disposition; they
do not become implicit migration input. Inventory cardinalities have explicit
table/column/constraint/object/role/grant ceilings.

## Preflight

Connection URLs must not contain passwords. Use an owner-only password file and a
CA; client certificate/key options are paired when source mTLS is required:

```sh
export QUACKGIS_MIGRATE_SOURCE_URL='postgresql://migration@source.example/source_db'
export QUACKGIS_MIGRATE_SOURCE_PASSWORD_FILE=/run/secrets/source-password
export QUACKGIS_MIGRATE_SOURCE_CA=/run/secrets/source-ca.crt

mise exec -- cargo run -p quackgis-migrate -- preflight \
  --config migration.json \
  --out preflight-report.json
```

A source version mismatch or missing PostGIS capability fails before a report can
claim compatibility. A semantic rejection writes the report and exits nonzero.
The target is neither configured nor contacted by `preflight`.

The first accepted data contract is deliberately narrow:

| Source | Target | Disposition |
|---|---|---|
| `smallint`, `integer`, `bigint`, `boolean`, `real`, `double precision` | matching release scalar | exact |
| bounded `numeric(p,s)` with precision at most 38 | `DECIMAL(p,s)` | exact |
| `date`, timestamp without time zone at precision 0–6 | matching temporal type | exact within the maintained COPY range |
| `varchar(n)`, `text` | `VARCHAR(n)`, `VARCHAR` | exact bytes; `text` is an explicit map |
| `bytea` | `BLOB` | exact bytes |
| `geometry(Point,0)` with two dimensions | maintained `BLOB` WKB column | canonical NDR WKB map |

Literal and literal-cast defaults, NOT NULL, and table/column comments are
retained. Identity/generated columns, expressions or volatile defaults, keys and
other constraints, RLS, triggers, partitioned tables, special replica identity,
geography, nonzero/mixed SRID, Z/M, non-Point geometry, and every unlisted scalar
reject the selected table.

## Copy and verification

Configure the target URL to the dedicated packaged tiny-client listener. Its
credential is mapped to a bounded operator-provisioned LOGIN role and its client
certificate should come from a migration-only CA. The target connection supports
a CA and paired client certificate/key:

```sh
export QUACKGIS_MIGRATE_TARGET_URL='postgresql://migration_operator@127.0.0.1:5432/quackgis'
export QUACKGIS_MIGRATE_TARGET_CA=/run/secrets/tiny-client-ca.crt
export QUACKGIS_MIGRATE_TARGET_CLIENT_CERT=/run/secrets/migration.crt
export QUACKGIS_MIGRATE_TARGET_CLIENT_KEY=/run/secrets/migration.key

mise exec -- cargo run -p quackgis-migrate -- run \
  --config migration.json \
  --out migration-report.json
```

The migrator holds the source snapshot from identity/preflight through final COPY
and verification. It creates every target table and performs every COPY inside
one target transaction. Source `CopyOut` chunks are forwarded directly into the
bounded QuackGIS `CopyIn`; no full table or request is collected. A pre-commit
error explicitly rolls back the target transaction.

For each table, source and target rows are normalized by type and accumulated into
order-independent SHA-256 multiset checksums for the complete row and every
column. The report records row and NULL counts, wire bytes and wire SHA-256, table
and column checksums, source snapshot identity, target PostgreSQL profile identity,
durations, all mappings/rejections, errors, and the final decision. It contains no
connection URL, password, certificate path, local target path, or row value. A
fresh target pgwire connection recomputes all checksums and counts after commit.

Report states are operationally significant:

| State | Meaning |
|---|---|
| `rejected` | preflight failed; target was not contacted |
| `failed_rolled_back` | target preparation failed and the transaction rollback completed |
| `commit_indeterminate` | commit did not return success; reconcile target state before retry |
| `committed_unverified` | commit succeeded but fresh-session verification failed |
| `verified` | fresh-session counts and every canonical checksum match |

Only `verified` exits successfully. Even then, `final_decision` says the snapshot
is prepared for operator cutover; it does not claim an atomic release promotion.
Use a fresh isolated target root for this preview, keep PostGIS as the rollback
source, and do not direct clients to the target until separate cutover checks pass.

## Explicit cleanup

Cleanup is a separate destructive command and requires an explicit confirmation.
It drops only the exact target tables named by the validated configuration, in
one target transaction, and writes a path-free cleanup report:

```sh
mise exec -- cargo run -p quackgis-migrate -- cleanup \
  --config migration.json \
  --out cleanup-report.json \
  --confirm-drop-configured-targets
```

`DROP TABLE` admission is limited to one configured table at a time and requires
the same exact ownership decision as creation/comments. Views, multiple targets,
`CASCADE`, and non-owner roles fail closed. The packaged gate runs cleanup first
so it is repeatable. This is not report-bound release rollback or staging
promotion; those remain separate G0 work.

## Security boundary

The source database and its catalogs are untrusted migration input. Identifiers
are always quoted, only classified column expressions are generated, arbitrary
source defaults are not executed, and source credentials remain in the migrator.
The QuackGIS worker receives only ordinary target pgwire traffic and never receives
the source URL, password, TLS key, or direct DuckDB/DuckLake credentials.

K0 maps a separate iroh credential only to `migration_operator`, whose immutable
role configuration owns only the two maintained migration fixture targets. A
separate tiny-client listener trusts only migration-client certificates. The
ordinary client certificate cannot connect to that listener, and the migration
certificate is not trusted by the ordinary pgwire listener.

TLS is required unless the operator explicitly enables plaintext for a literal
loopback TCP or Unix-socket host. Password files must be non-symlink, owner-only,
NUL-free regular files no larger than 4 KiB. Certificate/key files are bounded,
and target/source client certificate settings must be paired.
