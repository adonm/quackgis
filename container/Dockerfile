# SPDX-License-Identifier: Apache-2.0
#
# QuackGIS facade container — built from vendored pg_ducklake source with
# sedonadb extension loading support.
#
# Runtime base is the osgeo GDAL image (has all GDAL/PROJ/GEOS libraries).
# PostgreSQL 18 is installed from the PGDG apt repository.
#
# Usage:
#   docker build -t quackgis:dev -f container/Dockerfile .
#   docker run -e POSTGRES_PASSWORD=quackgis -p 5432:5432 quackgis:dev

# ════════════════════════════════════════════════════════════════════════════
# Stage 1: Build sedonadb DuckDB extension (against Debian Bookworm GDAL)
# ════════════════════════════════════════════════════════════════════════════
FROM postgres:18-bookworm AS sedonadb-builder

ARG RUST_VERSION=1.88.0
ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update && apt-get install -y --no-install-recommends \
        clang libclang-dev llvm pkg-config cmake \
        build-essential curl ca-certificates \
        libgdal-dev libgeos-dev libproj-dev libsqlite3-dev \
    && rm -rf /var/lib/apt/lists/*

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --default-toolchain ${RUST_VERSION} --profile minimal
ENV PATH=/root/.cargo/bin:${PATH}

WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
COPY sql/ sql/

RUN cargo build --release
RUN ./target/release/sedonadb-package \
    target/release/libsedonadb.so \
    /tmp/sedonadb.duckdb_extension linux_amd64

# ════════════════════════════════════════════════════════════════════════════
# Stage 2: Build pg_ducklake from vendored source
# ════════════════════════════════════════════════════════════════════════════
FROM postgres:18-bookworm AS pg-ducklake-builder

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update && apt-get install -y \
    postgresql-server-dev-18 \
    build-essential libreadline-dev zlib1g-dev flex bison libxml2-dev libxslt-dev \
    libssl-dev libxml2-utils xsltproc pkg-config libc++-dev libc++abi-dev libglib2.0-dev \
    libtinfo6 cmake libstdc++-12-dev liblz4-dev ccache ninja-build git libcurl4-openssl-dev \
    curl zip unzip \
    && rm -rf /var/lib/apt/lists/*

ENV VCPKG_FORCE_SYSTEM_BINARIES=1
RUN git clone https://github.com/microsoft/vcpkg.git /vcpkg && \
    git -C /vcpkg checkout 84bab45d415d22042bd0b9081aea57f362da3f35 && \
    /vcpkg/bootstrap-vcpkg.sh -disableMetrics
ENV VCPKG_TOOLCHAIN_PATH=/vcpkg/scripts/buildsystems/vcpkg.cmake
RUN echo x64-linux > /vcpkg/triplet

WORKDIR /build

COPY vendor/pg_ducklake/Makefile vendor/pg_ducklake/Makefile.pgxs ./
COPY vendor/pg_ducklake/libpgduckdb libpgduckdb
COPY vendor/pg_ducklake/pg_ducklake pg_ducklake

RUN git clone --depth 1 --branch v1.5.4 https://github.com/duckdb/duckdb.git duckdb && \
    mkdir -p .git/modules/duckdb && touch .git/modules/duckdb/HEAD

RUN git clone https://github.com/duckdb/ducklake.git pg_ducklake/third_party/ducklake && \
    git -C pg_ducklake/third_party/ducklake checkout 93cc490d9b5554f6fd5322dbef23f41d4fa91bb8 && \
    for patch in pg_ducklake/third_party/ducklake-*.patch; do \
        git -C pg_ducklake/third_party/ducklake apply "$(readlink -f "$patch")"; \
    done && \
    mkdir -p .git/modules/pg_ducklake/third_party/ducklake && \
    touch .git/modules/pg_ducklake/third_party/ducklake/HEAD && \
    touch pg_ducklake/third_party/ducklake/.patched

RUN --mount=type=cache,target=/ccache/ \
    VCPKG_TARGET_TRIPLET=x64-linux make -j$(nproc) -C pg_ducklake duckdb

RUN --mount=type=cache,target=/ccache/ \
    VCPKG_TARGET_TRIPLET=x64-linux make -j$(nproc) -C pg_ducklake

RUN mkdir /out && \
    VCPKG_TARGET_TRIPLET=x64-linux DESTDIR=/out make -C pg_ducklake install

# ════════════════════════════════════════════════════════════════════════════
# Stage 3: Runtime — PostgreSQL base + pg_ducklake + sedonadb + GDAL libs
# ════════════════════════════════════════════════════════════════════════════
FROM postgres:18-bookworm AS runtime

ENV DEBIAN_FRONTEND=noninteractive

# GDAL/PROJ/GEOS runtime comes from Debian Bookworm (matches build env).
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates libcurl4 libgdal32 libgeos-c1v5 libproj25 \
    && rm -rf /var/lib/apt/lists/*

RUN ldconfig

# Copy pg_ducklake from builder.
COPY --from=pg-ducklake-builder /out/ /

# Place sedonadb extension in the DuckDB extension directory.
RUN mkdir -p /opt/quackgis/duckdb-extensions/v1.5.4/linux_amd64
COPY --from=sedonadb-builder /tmp/sedonadb.duckdb_extension \
     /opt/quackgis/duckdb-extensions/v1.5.4/linux_amd64/sedonadb.duckdb_extension

# Copy init scripts.
COPY container/init.d/ /docker-entrypoint-initdb.d/

# Entrypoint.
COPY container/entrypoint.sh /opt/quackgis/entrypoint.sh
RUN chmod +x /opt/quackgis/entrypoint.sh

ENV QUACKGIS_EXTENSION_DIR=/opt/quackgis/extensions \
    QUACKGIS_DUCKLAKE_DIR=/var/lib/quackgis \
    QUACKGIS_DUCKLAKE_DATA_PATH= \
    QUACKGIS_DEFAULT_SCHEMA=public \
    QUACKGIS_REWRITE_MODE=warn \
    QUACKGIS_DUCKDB_MEMORY_LIMIT=4096 \
    QUACKGIS_DUCKDB_THREADS=-1 \
    HOME=/var/lib/postgresql

RUN mkdir -p /var/lib/quackgis/data && \
    chown -R postgres:postgres /var/lib/quackgis /opt/quackgis

USER postgres

ENTRYPOINT ["/opt/quackgis/entrypoint.sh"]
CMD ["postgres"]

LABEL org.opencontainers.image.title="QuackGIS" \
      org.opencontainers.image.description="PostGIS-compatible spatial database facade" \
      org.opencontainers.image.source="https://github.com/adonm/quackgis" \
      org.opencontainers.image.licenses="Apache-2.0"

HEALTHCHECK --interval=10s --timeout=5s --start-period=60s --retries=3 \
    CMD pg_isready -U "$POSTGRES_USER" -d "$POSTGRES_DB" || exit 1
