# Security and authorization

QuackGIS currently implements a small service-identity model, not PostgreSQL RBAC.

## Implemented controls

| Control | Evidence |
|---|---|
| development trust mode | local only; explicit status/logging |
| SCRAM-SHA-256 password startup | real DuckDB pgwire workflow |
| read/write and optional read-only identity | auth unit + real workflow |
| normalized read/write table allowlists | structural policy unit + denied real cases |
| separate opt-in maintenance identity and table policy | auth/parser unit + real pgwire compaction workflow |
| fail-closed TLS material configuration | startup validation |
| bounded/redacted auth and authorization audit events | audit/policy tests |
| optional private metrics endpoint | metrics unit tests |
| native driver digest/version validation | storage unit/native tests |

Broad catalog filtering, administrative permissions, mandatory TLS, channel
binding, secret rotation, and production failure drills remain open.

## Trust boundaries

1. **Client → pgwire:** use SCRAM and deployment-enforced TLS/network policy outside
   local development. Configuring TLS does not currently forbid plaintext.
2. **Server → native DuckDB:** the driver path loads native code and must remain
   operator-controlled; startup verifies exact digest/version.
3. **SQL → ADBC:** exactly one structurally parsed statement is authorized before
   prepare/schema access. Unknown write/read shapes fail closed.
4. **Storage:** local data roots carry one authority marker. Remote credentials and
   shared profiles are disabled.
5. **Metrics/audit:** never include SQL text, parameters, WKB, credentials, signed
   URIs, or sensitive paths.

## Authorization model

- `QUACKGIS_WRITE_ALLOWLIST` restricts write-capable identities.
- `QUACKGIS_READ_ALLOWLIST` restricts table reads and denies broad metadata access
  until filtered metadata surfaces exist.
- Object identity is normalized from the parsed statement, never a raw SQL prefix.
- Compatibility rewrites may shape SQL/results but may not change the underlying
  table authorization decision.
- `QUACKGIS_MAINTENANCE_USER` grants one existing read/write identity the bounded
  compaction call; the write table allowlist still applies. Backup/restore,
  retention, and future shared storage remain offline/operator capabilities and
  do not inherit SQL access.

## Required Local 1.0 evidence

- malformed/half-configured TLS fails startup;
- wrong password never falls back to trust;
- deployment plaintext-denial is verified;
- read-only and allowlist denials return stable SQLSTATE `42501` before ADBC;
- query timeout/cancel and connection quarantine do not bypass policy;
- secret rotation and revocation have a documented deployment behavior;
- online backup/restore and all maintenance actions emit redacted administrative
  events (the current offline backup tool runs outside the server audit stream); and
- metadata visible to restricted identities is trace-tested and filtered.

Do not claim full PostgreSQL object privileges. Add only the permissions required
by maintained service/client workflows.
