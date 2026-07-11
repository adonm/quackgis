# Runtime third-party licenses

The QuackGIS runtime bundle contains the following DuckDB artifacts:

| Artifact | Upstream source | License |
|---|---|---|
| DuckDB CLI and shared library | <https://github.com/duckdb/duckdb> | MIT |
| DuckDB Spatial extension | <https://github.com/duckdb/duckdb-spatial> | MIT |
| DuckLake extension | <https://github.com/duckdb/ducklake> | MIT |

Exact versions and SHA-256 values are recorded in `artifact-manifest.json`.

The Spatial binary also bundles third-party native libraries, including GEOS,
GDAL, PROJ, OpenSSL, curl, expat, zlib, and SQLite. The repository-generated image
is therefore a local verification artifact, not a redistribution-ready release.
Local 1.0 packaging must produce the complete versioned notices, corresponding-
source/relinking materials where required (including LGPL obligations), and a
license review for the exact pinned Spatial binary before publishing it.

## MIT License

Copyright 2018-2025 Stichting DuckDB Foundation

Permission is hereby granted, free of charge, to any person obtaining a copy of
this software and associated documentation files (the "Software"), to deal in
the Software without restriction, including without limitation the rights to
use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of
the Software, and to permit persons to whom the Software is furnished to do so,
subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS
FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR
COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER
IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN
CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

The Rust server also statically links crates recorded in `Cargo.lock`. Local 1.0
release packaging must add a generated, version-pinned dependency notice inventory
before public distribution.
