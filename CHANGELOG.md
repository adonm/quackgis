# Changelog

QuackGIS has not published a stable release. Prototype-era details and commit
anchors live in [docs/HISTORY.md](./docs/HISTORY.md) and Git history.

## Unreleased — DuckDB-only local preview

### Added

- Native work is now ordered through N0. One executable candidate
  DuckDB/DuckLake/Spatial/QuackGIS bundle manifest pins exact compatible source commits,
  ordered digest-pinned patch queues, one central DuckDB build, an additive
  QuackGIS extension, upstream/product test matrices, immutable artifact trust,
  licenses/SBOM, and upgrade/rollback evidence. Exact source preparation and
  result-tree validation pass; bootstrap, the current DuckLake artifact builder,
  runtime static checks, Rust digest assertions, and package assembly consume the
  common authority, and runtime manifests carry path-free bundle/source/patch/
  toolchain identity. Deterministic SPDX 2.3 and native-license outputs are hashed
  into runtime manifests and copied into images; unresolved Spatial transitive
  details remain explicitly incomplete and release-blocking. The current artifact
  remains marked as a legacy separate build until the central build reproduces it.
  A machine-readable upstream review and opt-in live ref check now require every
  native capability/patch to be adopted, retained for an exact gap, or scheduled
  for deletion. Current evidence selects latest DuckDB 1.5.4/current DuckLake 1.5,
  directs S0 to adopt existing Spatial CRS support, and requires M4 to reevaluate
  newer upstream R-tree work before retaining local bbox machinery. S0 then qualifies official
  CRS-aware geometry and DuckLake persistence before any narrow type-fidelity
  patch; Q0 separately decides and proves validated keys before OGR-created tables
  or direct-table QGIS can be claimed. Metadata-only keys and a Spatial fork for
  PostgreSQL catalog presentation are explicitly rejected.
- The first executable G0 offline PostGIS migration slice adds a dedicated
  `quackgis-migrate` client with exact PostgreSQL/PostGIS version pins, bounded
  selected-schema inventory, complete migrate/map/reject dispositions, one
  read-only repeatable-read source snapshot, and bounded source-to-target pgwire
  COPY. All target DDL/COPY/comments share one transaction; canonical complete-
  row and per-column checksums pass before commit and on a fresh target session.
  The digest-pinned actual-process smoke proves 100,002 snapshot rows exclude a
  concurrent source commit, release scalars plus Point/NULL WKB survive, invalid
  data leaves zero target tables, and key semantics reject before target access.
  TLS/mTLS and owner-only password files are supported. The provenance-pinned
  runtime packages the migrator, and a Kind gate moves 10,004 exact rows through
  a dedicated `migration_operator` credential, migration-only client CA,
  mutual-TLS tiny client, and iroh worker while denying the ordinary K0 client
  certificate. Runs now use a bounded fresh staging namespace; exact SHA-256-bound
  verification gates one explicit atomic promotion, and report-bound cleanup can
  remove staging without naming release tables. Reports bind the migrator,
  artifact manifest, clean source SHA, and immutable target image digest. The
  packaged gate promotes, restarts K0, and passes pinned psql, psycopg, OGR, and
  QGIS reads against the promoted 10,004 rows. Exact source owner/grantee roles
  and maintained table/column grants can now map to independently provisioned
  immutable target policy; missing configured mappings fail preflight, while
  credentials and role DDL remain excluded. An optional atomic path-free progress
  checkpoint records bounded preflight totals, per-table transfer/verification,
  16 MiB wire progress, commit-boundary state, rollback, and terminal decisions;
  it is neither a resume nor promotion token. Maintained Point columns now report
  family, SRID, dimensions, structural NDR WKB validity, empty/invalid counts,
  and finite 2D extents without row samples. Broader semantic validity and
  spatial/key support remain open.
- Owner-authorized `COMMENT ON TABLE` and `COMMENT ON COLUMN` now pass the pgwire
  structural admission boundary; all other comment targets and non-owner roles
  fail closed.
- Pinned offscreen QGIS 3.44.11 now qualifies expression and provider subset
  filters, exact extent, a spatial viewport identify request, and a 128×128
  rendered Point image through the packaged query layer. The pgwire edge
  structurally converts only QGIS's literal SRID-0 `geom_wkb &&
  ST_MakeEnvelope(...)` shape to an exact DuckDB `ST_Intersects` predicate;
  dynamic/nonzero-SRID/non-WKB and generic overlap shapes fail closed.
- Pinned GDAL/OGR 3.11.5 now authors a bounded text COPY into a predeclared
  official-DuckLake table through the packaged mutual-TLS tiny client, then a
  fresh psycopg connection verifies exact scalar, Point, and NULL values. The
  COPY decoder accepts GDAL's plain PostGIS EWKB hex only for fields already
  classified as geometry/geography; ordinary binary fields still require
  PostgreSQL `bytea` `\\x` syntax.
- Pgwire startup now advertises the frozen PostgreSQL 18.4 profile consistently
  in trust, SCRAM, and edge-preauthenticated modes. Structural `version()`,
  `SHOW server_version[_num]`, and PostgreSQL-typed
  `pg_is_in_recovery=false` agree. Simple/extended idle transaction end,
  failed-transaction `COMMIT`/`ROLLBACK`, and `25P02` precedence now pass the
  native workflow and QGIS-shaped pinned-catalog inquiry without client-specific
  branches.
- Local 1.0 now owns a tracked, read-only DuckLake identity patch. Exact upstream,
  DuckDB, patch, vcpkg, tool, platform, and accepted artifact pins drive a
  reproducible build gate. The server accepts only its compile-time SHA-256 at an
  absolute non-symlink path; the immutable runtime image packages that artifact,
  keeps client `LOAD`/`INSTALL` denied, and passes network-disabled extension and
  server-start smokes. `quackgis-rest --version` is now available to the runtime
  artifact gate.
- K0 now packages two role-aware REST replicas. Bootstrap maps separate proven
  credentials to exact `postgres` and `authenticator` leases without changing the
  edge protocol; the core server accepts only configured `LOGIN` roles on a
  loopback edge-preauthenticated listener. Each REST Pod has a unique transport
  key, a passwordless loopback tiny client, and no database storage mount.
  Rootless-Podman Kind proves two ready endpoints, exact reader/denied behavior,
  one-Pod failover, core reconnect, and replacement mTLS/edge/authenticator/JWT
  rotation with old-certificate, old-lease, and old-token denial.
- The runtime image now includes `quackgis-rest`; a stable internal UDP Service
  and bounded first-worker-response timeout let long-lived tiny clients discard
  stale QUIC sessions after core replacement.
- The checksum-pinned catalog identity lane now persists and exposes shared
  monotonic schema/security epochs as bounded `BIGINT` pgwire functions. REST
  keys role caches by the pair plus connection generation, refreshes atomically
  when either changes, and retains exact per-request role-filtered revision
  validation when signed-only startup reports the capability unavailable.
- REST database credentials now come from a bounded owner-only non-symlink file,
  never the pgwire URL. Atomic replacement invalidates the old connection;
  readiness fails while REST and database credentials disagree, then REST
  reconnects with the new password after a same-state database restart without a
  REST restart. Actual pgwire evidence proves committed-state preservation,
  transaction-local cleanup, and old-password denial.
- Bounded read-only PostgreSQL SQL cursors now support simple and extended
  transaction/`DECLARE`/metadata-only and forward `FETCH`/`CLOSE` workflows with
  session/fetch ceilings, role and catalog-epoch validation, text/binary format
  pinning, and clean bounded drain on close. The pinned GDAL/OGR 3.11.5 Kind job
  uses that unmodified SQL-result path to read the psycopg-created fixture and
  requires exact Point/NULL GeoJSON through mTLS, replacement, and key rotation.
  Direct OGR discovery, no-FID behavior, and predeclared-target COPY now pass;
  OGR-created tables and authoritative CRS metadata remain unsupported and are
  now ordered under Q0 and S0 after N0.
- The pinned psycopg 3.2.13 Kind job now runs a copied-data workflow through the
  mutual-TLS tiny-client ingress: create/reuse an official-DuckLake table, clear
  it, stream PostgreSQL COPY with exact WKB and NULL rows, close/reconnect, and
  verify exact scalar/spatial readback. It remains green after a 5.042-second
  ordered replacement and packaged mTLS/iroh key rotation with old-client denial;
  the existing direct/plaintext/client-certificate denial jobs remain green.
- The REST sidecar now re-reads its bounded regular-file HS256 key for every
  verification and readiness check. Atomic key replacement accepts newly signed
  tokens immediately, rejects the old key, and makes `/ready` fail while key
  material is missing or invalid.
- REST schema/OpenAPI caches now validate a bounded SHA-256 revision of the exact
  role-filtered, REST-exposed PostgreSQL catalog before every authenticated API
  request. Live column changes and stale over-broad role caches invalidate
  automatically; catalog validation failure returns `503`, while database grants
  remain authoritative across the validation/execution race.
- Local readiness now verifies readable DuckLake snapshots, a synced/removable
  4 KiB data-root write, and mandatory rollback of unique internal DuckLake DDL;
  native evidence requires zero table, file, or snapshot residue.
- The REST edge now validates bounded HS256 JWT signature, issuer, audience,
  expiry/not-before, and a static role allowlist, then uses one SCRAM
  authenticator with transaction-local role/claims. Per-role PostgreSQL catalog
  caches drive matching OpenAPI visibility and direct read denial.
- REST schema discovery now consumes the role-filtered PostgreSQL `public`
  information schema instead of DuckDB-internal `main` names and type
  translation, with actual SCRAM/grant-backed pgwire evidence.
- A shared `quackgis-edge` I0 protocol foundation with versioned control/edge
  ALPNs, five-minute bootstrap-signed one-worker leases, transport-bound
  registered-key refresh proofs, fresh worker challenges, typed application
  streams, mandatory uncompressed negotiation, 16 KiB control-message bounds,
  and fail-closed public-default/custom relay policy validation.
- Executable `quackgis-bootstrap`, `quackgis-worker-edge`, and tiny
  `quackgis-client` binaries with owner-only key loading/generation, bounded
  endpoint/stream admission, nonce replay protection, loopback-only backend and
  client boundaries, leased pgwire startup-role enforcement, nested-TLS denial,
  backend credential-challenge denial, lease refresh, and typed cancellation. A
  real local-direct iroh gate proves concurrent multiplexed sessions and the
  local bridge against a deterministic fake trust-mode pgwire backend.
- A registered native direct-path gate carries typed and spatial queries, atomic
  text COPY, rollback, cancellation/quarantine, and a fresh reconnect through the
  bootstrap/tiny-client/worker path to the current DuckDB/official-DuckLake
  server, with exact row/count/sum and SQLSTATE oracles.
- The native iroh gate now runs one differential oracle unchanged over direct TCP
  and the tiny-client path, requiring equal result values/types, errors,
  parameters, portals, transaction/disconnect behavior, successful and malformed
  COPY atomicity, cancellation/quarantine, concurrent sessions, and reconnect
  state.
- Evidence-selected adaptive LZ4 transport framing with mandatory raw fallback,
  independent 64 KiB per-direction blocks, minimum-savings selection,
  incompressible sampling backoff, expansion/allocation/corruption bounds, and
  payload-free byte/block/CPU/failure metrics. Startup/auth/control/cancellation
  remain raw.
- Deterministic custom-relay evidence proves relay-only application traffic,
  unusable-direct fallback, compressed and raw blocks, typed cancellation,
  same-identity worker restart, credential rotation, old-key denial, and native
  DuckDB differential parity. Opt-in public-preset gates also pass reconnect and
  the full native differential oracle.
- Clean 8/32/64 MiB TCP/direct/forced-relay `off`/`auto` profiles publish
  three-sample latency/throughput distributions, CPU, RSS, cancellation, stream,
  byte, ratio, and codec metrics. The source-`93c68be` Ryzen 7 7700X reference
  passes the +64 MiB transport allowance and committed raw/auto budgets; automatic
  LZ4 saves 99.57% on the maintained compressible profile while incompressible
  blocks stay raw.
- The K0 runtime image now packages bootstrap, worker, tiny-client, and keygen
  binaries beside the complete DuckDB server. One ordered rootless-Podman Kind
  Pod keeps backend pgwire loopback/trust-only and exposes only a mutual-TLS tiny
  client; fixed direct iroh routes require no outbound relay. Pinned psql 18.3,
  psycopg 3.2.13, and OGR 3.11.5 pass through the bridge, while direct worker TCP,
  plaintext, and certificate-free Jobs fail. Ordered replacement reconnects in
  4.07 seconds, and packaged mTLS/edge-key rotation denies the old client
  certificate before all current gates reconnect.
- Local-first smoke/local/reference/external roadmap levels with explicit host
  performance, minimal-Kind topology, and managed-service claim boundaries.
- A common profile evidence envelope with source dirty hashes, checksum-only native
  provenance, host/cgroup fingerprint, data/oracle/measurement/budget sections,
  atomic publication, and an independent CI validator.
- A parameterized smoke/local/reference transport entrypoint that runs one
  deterministic DuckDB/ADBC/pgwire scenario and exact-result oracle from 100k
  through the configured row count; the first dirty-tree 1M local run passes.
- A registered M4 spatial scan profile compares maintained WKB/bbox with native
  `GEOMETRY` over ordered official-DuckLake files using exact pgwire results,
  visible exact rechecks, DuckDB row-group metrics, conservative compressed-byte
  bounds, timed queries, process RSS, DuckDB memory/spill, and fragmented-file
  compaction. The M4-complete v5 workload adds grouped aggregates, bounded
  spatial joins, nine-column/1 KiB-payload wide projections, first-row timing,
  point/line/polygon mutation, and explicit 1 GiB file/default-row-group policy.
  Two clean 10M runs precede two clean 100M runs on source `8490ed7`; every
  exact-result, plan, load, first-row, p50/p95/p99, RSS, memory, zero-spill,
  scan-byte, file, row-group, and compaction budget passes.
- Warm, interleaved ADBC/pgwire transport sampling with an exact-result oracle and
  fail-closed reference enforcement of the one-second eligibility and 15% p50
  overhead limits.
- Scale-safe transport fixture names retain complete IDs above six digits; the
  exact byte-count oracle now has boundary coverage through 10M rows.
- A clean 50M-row transport reference on source `fc0b6069` records a 1200.05 ms
  direct-ADBC p50 and 0.999 pgwire/ADBC p50 ratio, passing the one-second
  eligibility floor and 1.15 overhead ceiling on the named reference host.
- A separately registered result-stream profile with two-millisecond process RSS
  sampling, time-to-first-row, throughput, row/sum oracle, and Arrow batch
  high-water evidence; its first 1M local run passes with one in-flight batch.
- Clean serial 1M and 10M generated-BIGINT reference runs on source `12817bcd`
  pass exact cardinality/sum, first-row-before-completion, one-batch high water,
  and the +128 MiB RSS budget with approximately 1.72/2.36 MiB RSS growth.
- A reusable fresh DuckLake/server/client profile fixture and cancellation profile
  covering request-to-`57014` latency, native counters, explicit same-session
  quarantine, and fresh-session reuse; the first 25-sample local run passes at
  1.41 ms p95.
- A clean 100-sample cancellation reference on source `8b0d1e46` passes the 500 ms
  M1 budget at 1.51 ms p95 with 100 completed native calls, zero failures, and
  deterministic quarantine/fresh-session behavior.
- Parameterized nullable VARCHAR/BLOB result profiling with every-row value/NULL
  checks, native batch counts, first-row timing, throughput, and RSS; its first
  dirty-tree 100k local run crosses 49 batches at 9 MiB RSS delta.
- The clean 1M wide-result reference on source `b240507e` checks every value over
  489 native batches with one in flight, zero rejections, and 19.17 MiB RSS delta.
- Parameterized direct streaming ADBC versus bounded pgwire COPY profiling with
  exact count/sum/WKB publication, RSS, rows/bytes/batches, commit timing, and
  throughput ratio; its first dirty-tree 1M local run passes at 64 MiB RSS delta
  and a 0.272 pgwire/direct ratio.
- COPY text decoding now stores one contiguous bounded batch plus compact field
  ranges, borrows unescaped values, builds Arrow text/binary columns directly,
  finds delimiters with bounded slice scanning, and parses hex/integer values
  without per-field/per-row temporary buffers.
- The clean 10M-row COPY reference on source `9e4611ed` processes 647,777,780
  wire bytes with 126 MiB RSS delta and a 0.528 pgwire/direct throughput ratio,
  passing the 256 MiB and 0.50 M2 budgets with exact count/sum/WKB publication.
- Project tooling now reports installed/missing core, Kind, container, and named
  client tools through `just doctor`, auto-selects usable Podman before Docker
  unless explicitly overridden, and provides a pinned rootless local Kind flow
  that builds and loads digest-addressed runtime and psql/psycopg/OGR images.
- The initial direct-worker rootless-Podman baseline imported local images through portable
  archives, aliases their containerd manifest digests, recreates stale named
  clusters, bound pgwire beyond loopback, and passed TLS/SCRAM smoke
  Jobs with psql 18.3, psycopg 3.2.13, and GDAL/OGR 3.11.5. The pgwire adapter
  handles those clients' bounded encoding and string-mode SET/SHOW probes without
  executing PostgreSQL session syntax in DuckDB.
- Added the authenticated, stateless `quackgis-rest` preview as a separate
  read-only pgwire client. It pins and extends `pg-rest-server`'s parser/query
  engine, supports PostgREST-style projection/filter/order/pagination, OpenAPI and
  schema reload, uses typed text parameters and bounded errors/timeouts, and has
  an actual DuckDB/DuckLake pgwire suite covering auth, reads, denials, and WKB.
- The initial direct-worker Kind topology: one TLS-required StatefulSet,
  retained node-local PV/PVC, generated TLS/auth Secrets, health probes, and
  opt-in psql/psycopg/OGR Jobs that reject mutable image references at render time.
- Owned Rust pgwire/TLS/SCRAM edge over DuckDB ADBC.
- Official local DuckLake create, Arrow ingest/query, transaction, snapshot
  inspection, adjacent-file merge, and reopen workflows.
- Structural single-statement admission and parsed read/write table policy.
- Parameterized reads/mutations, incremental bounded PostgreSQL text COPY with
  atomic publication, independent client sessions, transaction cleanup, and
  portal paging.
- Failed explicit transactions reject subsequent simple/extended work with
  `25P02`; `COMMIT` rolls back prior writes and returns the session to service.
- DuckDB Spatial execution with 43 curated original-PostGIS expressions routed
  through native functions or bounded server-owned rewrites/macros.
- Role-bound generic `geometry_columns`, discoverable `geometry_columns` and
  typed-empty `spatial_ref_sys` metadata relations, CRS/SRID and DuckDB version
  probes, plus textual 2D/3D extent aggregates over WKB. Subtype/dimension/SRID
  claims are not inferred, and SRID assignment remains fail-closed.
- Checksum/version validation for `libduckdb` and signed `spatial`/`ducklake`
  extensions, plus a load-only runtime image contract.
- DataFusion-free Arrow-to-pgwire encoder with maintained WKB sentinel identity.
- Generated Arrow encoder properties for geometry WKB payload/null identity and
  fixed-size binary values, plus fail-closed invalid JSON/unsupported list shapes
  and null-safe interval encoding.
- Configured fail-closed Arrow result-batch ceiling with batch byte/in-flight
  metrics.
- Fixed native blocking-worker budget with a reserved cancellation/control slot
  and active/queued/high-water metrics.
- Global plus reader/writer/maintenance-class admission limits and per-class
  active/queued/high-water metrics, with a 32-contender ceiling regression.
- A registered mixed-class native profile that saturates admission with retained
  reader/COPY work, observes reader, writer, and maintenance queueing together,
  and completes without rejection or timeout.
- A duration-controlled actual-process M5 mixed release profile now combines
  concurrent exact reads, atomic COPY, parameterized mutation, repeated native
  cancellation/quarantine, official compaction, process RSS sampling, idle-state
  metrics, same-path restart, and a post-restart write. The clean three-second
  smoke publishes 16,900 rows, validates 281 reads and 385 cancellations, runs 33
  compactions, stays within 118.05 MiB RSS growth, and restarts in 131.45 ms; the
  same oracle requires exactly 24 hours in reference mode.
- Opt-in maintenance identity and a literal-only server-owned adjacent-file
  compaction call with write-policy enforcement, maintenance admission, audit
  events, transaction rejection, and pgwire/reopen evidence.
- Opt-in `/healthz` and startup/drain-aware `/readyz` responses beside `/metrics`,
  plus active-transaction and explicit session-quarantine metrics.
- Bounded SIGINT/SIGTERM connection drain that stops acceptance, rejects new
  transactions, permits established work to finish, and aborts at a configured
  deadline.
- Actual-process forced-drain/restart profiling that proves an explicit
  uncommitted transaction publishes zero rows, committed state remains exact,
  restart meets the 60-second smoke budget, and post-restart writes succeed.
- Checksummed offline local backup/restore with symlink/source-change rejection,
  exact-path enforcement, staged publication, and native snapshot/count recovery
  evidence.
- Backup format v2 now embeds a path-free projection of the exact selected
  DuckDB/library/DuckLake/Spatial runtime identity. Restore verifies that identity
  before creating either target and rejects a different bootstrap or release
  artifact manifest; both current manifest schemas have focused tests.
- Actual-process offline recovery profiling now checkpoints exact scalar/WKB
  state, stops for a checksum backup, writes later rows only to the original,
  deletes both durable paths, restores to the exact paths, and requires the
  checkpoint with no later rows plus a post-recovery write. The clean smoke
  restores three files and becomes queryable in 116.51 ms.
- Periodic aggregate DuckDB tracked-memory and temporary-storage samples with
  current/high-water gauges and sampler health counters.
- Structural compatibility for maintained PostgreSQL session settings,
  `SHOW search_path`, `public`→`quackgis.main` relation mapping, and quoted
  one-/two-/three-part COPY targets.
- A machine-readable `pg18-column-core-v1` contract with digest-pinned PostgreSQL
  18.4 result-description evidence and explicit pending named-client traces.
- A credential-free 21-query copied-point discovery trace from the exact OGR
  3.11.5 image against digest-pinned PostgreSQL 18.4/PostGIS, including observed
  geometry, FID, SRID, feature-count, and extent results.
- A credential-free 12-query `\d+` trace from exact psql 18.3 against the same
  oracle, including normalized relation/attribute/index/constraint/policy queries
  and rendered spatial table structure.
- A credential-free offscreen QGIS 3.44.11 PostgreSQL provider trace: 32 statements
  (26 unique families) for layer open, fields, CRS, privileges, ownership, count,
  3D extent, and a successful binary-cursor feature read.
- Exact QGIS 3.44 session bootstrap support: its four `SET` statements are accepted
  only as an all-allowlisted simple-query batch (maximum eight), with bounded,
  control-free `application_name`; mixed/general multi-statement SQL remains denied.
- A multi-process DuckDB 1.5.4 identity gate proving DuckLake table UUID/ID and
  column field-ID continuity across rename/reopen plus new identity on name reuse.
- A registered client-neutral catalog fixture for DuckDB-derived table/column
  metadata and relational namespace/type/range/collation/owner-role views. Explicit
  and implicit catalog references are structurally mapped; 24 full-row PostgreSQL
  18 built-ins and PostGIS-shaped geometry/geography scalar/arrays have complete
  namespace/owner/array/collation links. Provenance-bound wire metadata, inferred
  and caller-supplied OID parameters, alias safety, and spatial transport pass.
  Unimplemented catalog routing, reserved CTE shadowing, and unsupported wildcard/
  nested/set/derived/implicit-join/cross-database shapes fail closed rather than
  reaching DuckDB/user objects; private-schema and `TABLE` access are rejected.
- Stable single-logical-database discovery through relational `pg_database` plus
  structurally rewritten `current_database`, `current_schema`, and
  `current_schemas`; actual pgwire evidence checks owner references and exact
  PostgreSQL `oid`, `name`, and `name[]` result types.
- Role-bound PostgreSQL 18 `information_schema.schemata`, `tables`, `columns`,
  `table_privileges`, `role_table_grants`, `column_privileges`, and
  `role_column_grants` projections. They bind the effective role structurally,
  derive object existence from DuckDB, preserve `public` naming and exact
  `name`/`varchar` wire types, hide ungranted objects, expand table grants to
  eligible columns, and omit QuackGIS-only `MAINTAIN` from the standard views.
- Registry-backed DuckLake defaults and table/column comments through
  `pg_attrdef`, `pg_description`, `pg_get_expr`, `col_description`, and
  `obj_description`. Default/comment values advance the guarded catalog
  fingerprint; actual pgwire preserves PostgreSQL `oid`, `int2`, `pg_node_tree`,
  and `text` identity and proves effective-role visibility cannot exceed the
  login identity's legacy allowlist ceiling.
- Durable PostgreSQL 18 `pg_constraint` identity for DuckLake `NOT NULL`
  constraints, including rename continuity, resolving OID/attribute links,
  `int2[]` keys, and `pg_get_constraintdef`; a typed but empty `pg_index` plus
  NULL `pg_get_indexdef` truthfully represent DuckLake's lack of primary, unique,
  foreign-key, check, and index support.
- DuckDB-computed bbox maintenance during COPY for the explicit reserved-column
  layout contract, including NULL, exact-recheck, and reopen evidence.
- Fail-closed bbox layout validation rejects partial, wrong-type, caller-supplied,
  or ambiguous reserved columns before staging and keeps the pgwire session usable.
- Conservative one-table bbox candidate injection for mandatory exact
  `ST_Intersects` predicates over maintained WKB layouts. Bounded literal
  envelope/text and numbered-bound WKB probes gain four planner-visible overlap
  filters while retaining the original exact predicate; OR/NOT, joins,
  subqueries, multiple matches, and arbitrary/oversized probes stay unoptimized;
  malformed or ambiguous reserved layouts fail closed. Native
  hole/boundary/NULL/empty/invalid/bound/reopen/`EXPLAIN` exact-oracle comparisons
  and actual pgwire literal cases pass.
- COPY deadline evidence after flushed batches, with an explicit pre-publication
  cancellation check preventing aborted ADBC EOF normalization from publishing.
- A generated 220k-row, greater-than-20-MiB pgwire COPY regression using bounded
  60-KiB client chunks, proving the request is no longer capped at 16 MiB.
- COPY persistence checks for NULL/scalar/WKB values and official DuckLake
  adjacent-file compaction after fragmented loads.
- Stable `0A000` errors for the five maintained `ST_NDims`, `ST_CoordDim`, and
  `ST_GeometryN` extension-candidate cases, verified through simple and extended
  pgwire with session reuse.
- Fail-closed native query-stream cleanup: only observed EOF returns a connection;
  partial delivery, reader failure, or cancellation quarantines the session with
  an explicit engine error.
- Deep local readiness that binds pgwire and queries the DuckLake snapshot surface
  before reporting ready, with explicit starting, storage-unavailable, and draining
  states kept separate from process liveness.
- Schema-aware maintained-layout UPDATEs atomically recompute all four bbox
  columns when the geometry is assigned from one numbered bound parameter or
  `NULL`; malformed geometry rolls back, explicit rollback restores geometry and
  bounds together, and reopen preserves the refreshed values. Direct INSERT,
  arbitrary geometry expressions, tuple geometry assignment, and caller-written
  bbox values remain fail-closed; ordinary-column UPDATEs remain supported.
- A deterministic 32-client native pgwire admission regression: suspended portals
  retain eight reader permits and no ninth reader enters before release.
- Statement deadlines run native cancellation through reserved blocking-worker
  capacity, including a regression with all regular worker capacity occupied.
- Synchronous COPY cleanup for malformed final rows and oversized decoded chunks,
  with zero-row observer evidence and exact Arrow batch-memory splitting tests.
- Configurable pre-body pgwire frontend-frame enforcement. A header-only
  oversized `CopyData` declaration is rejected without reserving its body and the
  abandoned COPY publishes zero rows; post-decode chunk and Arrow limits remain
  defense in depth.
- Native pgwire evidence that a cancelled streaming client is explicitly
  quarantined while an independent session remains usable.
- Pgwire writes now execute in a cancellable pre-commit transaction. Cancelled
  autocommit writes roll back with `57014` and remain reusable; cancelled writes
  inside explicit transactions roll back and quarantine the session. Cancellation
  closes before commit begins, commit is deliberately non-cancellable, and commit
  failures are classified as indeterminate.
- Explicit CI execution of the vendored Arrow encoder suite; Float16 and UInt32
  OID scalar parity, Float16/fixed-binary list parity, fail-closed unsupported time
  units, and panic-free nested error propagation now have focused regressions.
- Narrow exact-AST-shape `pg_type` resolution for the maintained geometry/geography
  sentinel OIDs, with geometry/geography RowDescription, binary WKB, text hex-WKB,
  and NULL evidence through the native pgwire client. Near-miss projections, predicates,
  and constants remain native rather than being intercepted.
- Ledger-pinned `0A000` simple/extended protocol behavior and session-reuse
  evidence for all 10 deferred Rust-edge and five extension-candidate spatial gaps.
- Explicit `preferred`/`required` TLS policy: required mode needs paired valid
  material and rejects insecure startup before trust or SCRAM authentication.
- Actual-process TLS/SCRAM evidence with generated client trust: valid encrypted
  access succeeds, plaintext and an untrusted certificate fail, committed data
  survives restart-based certificate/password rotation, old trust and password
  fail afterward, and a post-rotation write succeeds.

### Changed

- DuckDB and official DuckLake are unconditional and are the sole engine/storage
  authority for new roots.
- The forward roadmap now separates G0 offline PostGIS snapshot migration from G1
  online logical catch-up/cutover. Offline migration uses complete preflight and
  packaged-client COPY; online migration waits for M6 durable control/storage,
  uses source-LSN idempotent batches, and makes no distributed exactly-once,
  dual-write, reverse-replication, or implicit DDL claim.
- Project direction now prioritizes bounded streaming, cancellation, admission,
  bulk ingest, focused named clients, and measured spatial performance before
  shared or broad compatibility claims.
- Missing capabilities follow the native DuckDB → SQL macro/rewrite → Rust edge →
  vectorized DuckDB extension decision ladder.
- CI bootstraps and verifies the pinned native runtime before native gates.
- Runtime artifacts package the complete DuckDB bundle rather than a bare server
  binary.

### Removed

- DataFusion, Sedona SQL execution, `datafusion-postgres`,
  `datafusion-pg-catalog`, and forked `datafusion-ducklake` runtime/vendor trees.
- Legacy engine modules, catalog hooks, native writer/mutation code, and associated
  tests.
- Optional backend selection and the `duckdb-adbc` feature gate.
- Unsupported shared PostgreSQL/S3 deployment automation and stale Kind/client
  probes.
- Nonfunctional examples, benchmark runners, migration plans, and scheduled jobs
  that depended on retired code or absent tests.

### Known limits

- Query and COPY streaming have strict scale/RSS/throughput evidence still open.
- Query cancellation/deadlines, classed admission/resource controls, sampled
  native memory/temporary storage, and a fixed blocking-worker budget are
  implemented; general write cancellation and native scale/concurrency evidence
  remain open.
- Local official DuckLake only.
- Incomplete catalogs, geometry identity, spatial gaps, and named GIS clients.
- No online/relocated production recovery, upgrade, soak, or shared-profile
  evidence.
