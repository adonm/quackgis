# Security policy

QuackGIS is pre-1.0 developer software. Please report security issues privately
through GitHub security advisories when available, or by opening an issue that
omits exploit details and requests a private channel.

## Native runtime trust boundary

The configured DuckDB ADBC library is native code loaded into the server process.
Treat its path as operator-controlled configuration, never client input. Startup
requires the exact committed SHA-256 and DuckDB SQL version before claiming or
opening a data root. Production packaging must include version-matched signed
`spatial` and `ducklake` extensions and run with extension installation disabled.

## Network and authentication limits

Trust authentication is development-only. Password mode uses SCRAM-SHA-256, but
configured TLS is currently optional rather than mandatory and does not implement
channel binding. Deploy behind explicit network policy, configure certificate and
PKCS#8 key together, and use read/write allowlists for service identities. Remote
catalog/object-store profiles remain disabled.
