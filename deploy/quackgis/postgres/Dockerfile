FROM docker.io/postgis/postgis:18-3.6

ARG DUCKDB_FDW_COMMIT
ARG DUCKDB_VERSION

COPY patches/quackgis.patch /tmp/quackgis.patch

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
    && DUCKDB_VERSION="$DUCKDB_VERSION" ./download_libduckdb.sh \
    && make -j"$(nproc)" USE_PGXS=1 \
    && make install USE_PGXS=1 \
    && ldconfig \
    && rm -rf /tmp/duckdb_fdw /tmp/quackgis.patch /var/lib/apt/lists/*
