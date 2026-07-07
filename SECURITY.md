# Security policy

QuackGIS is pre-1.0 developer software. Please report security issues privately
through GitHub security advisories when available, or by opening an issue that
omits exploit details and requests a private channel.

## Current dependency exception

GitHub Dependabot alert `GHSA-2f9f-gq7v-9h6m` / `CVE-2026-43868` reports
`thrift <= 0.22.0`. In this workspace it is transitive through:

```text
datafusion 54 -> parquet 58.3.0 -> thrift 0.17.0
```

`cargo update -p thrift --precise 0.23.0` is blocked by `parquet 58.3.0`'s
`thrift = ^0.17` requirement, and `cargo update -p parquet` has no compatible
patched release for DataFusion 54. `parquet 59` removes the vulnerable `thrift`
dependency, but no compatible DataFusion release is available in the current
stack yet.

Local mitigation: QuackGIS does not expose Apache Thrift as a network protocol;
the affected code is in the Parquet dependency path behind DataFusion/DuckLake.
Revisit this exception when the DataFusion stack can move to a Parquet line that
no longer depends on `thrift 0.17`.
