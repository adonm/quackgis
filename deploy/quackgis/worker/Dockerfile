FROM docker.io/duckdb/duckdb:1.5.4@sha256:d5e66353428256453574ddfd4ee446ef37510e61619bb5a8f63b988165bd70b8 AS duckdb

FROM docker.io/library/debian:bookworm-slim@sha256:7b140f374b289a7c2befc338f42ebe6441b7ea838a042bbd5acbfca6ec875818

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=duckdb /duckdb /duckdb

RUN test "$(uname -m)" = x86_64 \
    && /duckdb -c "INSTALL httpfs; INSTALL quack FROM core_nightly; LOAD quack; INSTALL spatial;" \
    && printf '%s  %s\n' \
        2e020c7e89b11364e413ddfab8b9ca870b1fc0a16eaf612c9445773ef68989ef \
        /root/.duckdb/extensions/v1.5.4/linux_amd64/httpfs.duckdb_extension \
        65d72788a701595f3cd263221b5a8d76c570312e9b8bd989a6b7e0b12c525ae0 \
        /root/.duckdb/extensions/v1.5.4/linux_amd64/quack.duckdb_extension \
        819a0fa94d2e8257371bb4d97f32c47586b42c85e24bab1bcbf8712a88b8ccfa \
        /root/.duckdb/extensions/v1.5.4/linux_amd64/spatial.duckdb_extension \
        | sha256sum --check --strict
