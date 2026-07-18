# Offline PostGIS migration

`quackgis-migrate` implements the first executable G0 slice: it inventories an
exactly pinned PostgreSQL/PostGIS source in one read-only repeatable-read
transaction, classifies the maintained inventory categories across selected
schemas, streams accepted tables through PostgreSQL text COPY to QuackGIS, and verifies canonical source and
target checksums before one target transaction commits.

This is executable preview evidence, not a complete migration product. The
provenance-pinned runtime includes the migrator; reports bind the executable,
artifact manifest, clean source SHA, and immutable target runtime image digest.
Fresh staging, exact-report verification, explicit atomic promotion, restart
verification, and psql/psycopg/OGR/QGIS qualification pass through a dedicated
credential, role, client CA, and mutual-TLS iroh tiny client. Exact source roles
and grants can be mapped to independently provisioned immutable target policy;
passwords and role DDL are never copied. Bounded progress checkpoints, richer
spatial report dimensions, keys, nonzero CRS, geography, non-Point geometry, and
general operator cutover remain open.

## Maintained smoke

```sh
mise exec -- just postgis-migration-smoke
```

The recipe pulls the digest-pinned PostgreSQL 18/PostGIS 3.6 image, starts a fresh
actual QuackGIS/DuckLake server, and runs these cases:

1. copy release scalars plus Point/NULL WKB while a concurrent source write commits
   outside the held snapshot;
2. reject an invalid target date and prove the complete target DDL/COPY transaction
   leaves no tables; and
3. reject a primary-key source before attempting to reach an unavailable target;
4. reject a wrong report digest before target access, verify and clean an exact
   staged report, retry into a fresh stage with identical checksums, and atomically
   promote only the second verified report; and
5. prove the promoted release has 100,003 rows and no staging-table residue.

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
10,002 scalar rows plus two Point/NULL rows into a fresh `g0stage__*` namespace
through a credential-bound `migration_operator` lease. The Job hashes the
path-free migration report, reverifies that exact digest, hashes the verification
report, and promotes only that exact pair in one transaction. Its report records
the clean source and artifact identities plus target runtime image digest. The
gate restarts the complete K0 Pod, then pinned psql 18.3, psycopg 3.2.13, GDAL/OGR
3.11.5, and QGIS 3.44.11 recheck promoted counts, release scalars, Point/NULL,
extent/filter/identify, and rendering through the same migration Service. The
ordinary K0 client certificate is explicitly denied. The PostGIS sidecar and
source trust mode exist only inside this one Job's network namespace; they are
absent from normal topology startup.

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
  "role_mappings": {"source_reader": "reader"},
  "grant_mappings": [
    {
      "source_role": "source_reader",
      "source_schema": "public",
      "source_table": "places",
      "source_column": "location",
      "privilege": "SELECT",
      "target_role": "reader"
    }
  ],
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

`role_mappings` maps an exact inventoried source owner/grantee role to an existing
target role. `grant_mappings` then accepts only an exact inventoried `SELECT`,
`INSERT`, `UPDATE`, or `DELETE` grant on a selected table or column, derives its
target object through the table/column mappings, and requires the same target
role. A configured role or grant absent from the held snapshot rejects preflight.
Unmapped source roles/grants remain explicit `reject` dispositions. These fields
do not create roles, copy passwords, or execute `GRANT`: operators provision the
target's immutable role policy separately, and the path-free report records the
source-to-target decision for audit.

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
  --out migration-report.json \
  --staging-id release_1 \
  --runtime-manifest /opt/quackgis/artifact-manifest.json \
  --target-runtime-image registry.example/quackgis@sha256:0000000000000000000000000000000000000000000000000000000000000000
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
staging-to-release mappings, explicit role/grant mappings, migrator/artifact/source/image
digests, durations, all mappings/rejections, errors, and the final decision. It contains no
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

Only `verified` exits `run` successfully. Even then, `final_decision` says the
snapshot is prepared for explicit promotion; release names are still absent.

Bind each next operation to the exact report bytes, not merely a filename:

```sh
report_sha256=$(sha256sum migration-report.json | cut -d' ' -f1)
mise exec -- cargo run -p quackgis-migrate -- verify \
  --report migration-report.json \
  --report-sha256 "$report_sha256" \
  --out verification-report.json \
  --runtime-manifest /opt/quackgis/artifact-manifest.json \
  --target-runtime-image registry.example/quackgis@sha256:0000000000000000000000000000000000000000000000000000000000000000

verification_sha256=$(sha256sum verification-report.json | cut -d' ' -f1)
mise exec -- cargo run -p quackgis-migrate -- promote \
  --report migration-report.json \
  --report-sha256 "$report_sha256" \
  --verification-report verification-report.json \
  --verification-report-sha256 "$verification_sha256" \
  --out promotion-report.json \
  --confirm-promote \
  --runtime-manifest /opt/quackgis/artifact-manifest.json \
  --target-runtime-image registry.example/quackgis@sha256:0000000000000000000000000000000000000000000000000000000000000000
```

`verify` requires the same runtime and target identity and recomputes every count,
NULL count, and checksum on a new connection. `promote` rejects any mismatched
report, verification, runtime, or target before mutation. It reverifies staging
inside one target transaction, creates absent release tables, copies and verifies
them, removes staging, commits once, reconnects, and verifies release names again.
Failure before commit retains verified staging and publishes no release;
response-loss is `commit_indeterminate`, and post-commit verification failure is
`committed_unverified`. Keep PostGIS as the rollback source until a separate
operator retirement decision.

## Explicit cleanup

Cleanup is a separate destructive command and requires an explicit confirmation.
It accepts only an exact verified migration report and drops only that report's
staging targets in one transaction. Release targets are never cleanup inputs:

```sh
mise exec -- cargo run -p quackgis-migrate -- cleanup \
  --report migration-report.json \
  --report-sha256 "$report_sha256" \
  --out cleanup-report.json \
  --confirm-cleanup-staging \
  --runtime-manifest /opt/quackgis/artifact-manifest.json \
  --target-runtime-image registry.example/quackgis@sha256:0000000000000000000000000000000000000000000000000000000000000000
```

`DROP TABLE` admission is limited to one configured table at a time and requires
the same exact ownership decision as creation/comments. Views, multiple targets,
`CASCADE`, and non-owner roles fail closed. `reset-configured-targets` is a
separately named preview/test operation requiring
`--confirm-drop-configured-targets`; the repeatable Kind fixture uses it to remove
only its statically owned release and staging tables before a new oracle. It is
not release rollback and is not part of the operator promotion path.

## Security boundary

The source database and its catalogs are untrusted migration input. Identifiers
are always quoted, only classified column expressions are generated, arbitrary
source defaults are not executed, and source credentials remain in the migrator.
The QuackGIS worker receives only ordinary target pgwire traffic and never receives
the source URL, password, TLS key, or direct DuckDB/DuckLake credentials.

K0 maps a separate iroh credential only to `migration_operator`, whose immutable
role configuration owns only the two maintained release and two staging fixture
targets. A
separate tiny-client listener trusts only migration-client certificates. The
ordinary client certificate cannot connect to that listener, and the migration
certificate is not trusted by the ordinary pgwire listener.

TLS is required unless the operator explicitly enables plaintext for a literal
loopback TCP or Unix-socket host. Password files must be non-symlink, owner-only,
NUL-free regular files no larger than 4 KiB. Certificate/key files are bounded,
and target/source client certificate settings must be paired.
