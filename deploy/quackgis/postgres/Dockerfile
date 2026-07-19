FROM docker.io/duckdb/duckdb:1.5.4@sha256:d5e66353428256453574ddfd4ee446ef37510e61619bb5a8f63b988165bd70b8 AS duckdb

FROM docker.io/postgis/postgis:18-3.6@sha256:c893f6f2652d11e13f50f8623045b3523991b41d038b4d213dc040f42641f0d7

ARG DUCKDB_FDW_COMMIT
ARG DUCKDB_VERSION

COPY patches/quackgis.patch /tmp/quackgis.patch
COPY --from=duckdb /duckdb /usr/local/bin/duckdb

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential \
        ca-certificates \
        curl \
        git \
        postgresql-server-dev-18 \
        unzip \
    && git clone https://github.com/alitrack/duckdb_fdw.git /tmp/duckdb_fdw \
    && git -C /tmp/duckdb_fdw checkout --detach "$DUCKDB_FDW_COMMIT" \
    && git -C /tmp/duckdb_fdw apply /tmp/quackgis.patch \
    && cd /tmp/duckdb_fdw \
    && curl --fail --location --output duckdb-temp.zip \
        "https://github.com/duckdb/duckdb/releases/download/v${DUCKDB_VERSION}/libduckdb-linux-amd64.zip" \
    && echo "838d98a85e697bab9935010c88a8c67d3312ccedcab4cb4a0ba01da65113bb70  duckdb-temp.zip" \
        | sha256sum --check --strict \
    && unzip -o duckdb-temp.zip \
    && rm duckdb-temp.zip \
    && make -j"$(nproc)" USE_PGXS=1 \
    && make install USE_PGXS=1 \
    && mkdir -p /usr/local/share/quackgis \
    && HOME=/usr/local/share/quackgis /usr/local/bin/duckdb \
        -c "INSTALL httpfs; INSTALL quack FROM core_nightly; LOAD quack;" \
    && printf '%s  %s\n' \
        2e020c7e89b11364e413ddfab8b9ca870b1fc0a16eaf612c9445773ef68989ef \
        /usr/local/share/quackgis/.duckdb/extensions/v1.5.4/linux_amd64/httpfs.duckdb_extension \
        65d72788a701595f3cd263221b5a8d76c570312e9b8bd989a6b7e0b12c525ae0 \
        /usr/local/share/quackgis/.duckdb/extensions/v1.5.4/linux_amd64/quack.duckdb_extension \
        | sha256sum --check --strict \
    && ldconfig \
    && rm -rf /tmp/duckdb_fdw /tmp/quackgis.patch /var/lib/apt/lists/* \
    && rm /usr/local/bin/duckdb

COPY install-extensions.sh /docker-entrypoint-initdb.d/10_install_extensions.sh
