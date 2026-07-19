FROM docker.io/library/golang:1.21.6-alpine@sha256:a6a7f1fcf12f5efa9e04b1e75020931a616cd707f14f62ab5262bfbe109aa84a AS build

ARG PG_FEATURESERV_COMMIT=2f1bd809f2c745d6d332495ee336f604f3ad9063

RUN apk add --no-cache git
COPY patches/quackgis.patch /tmp/quackgis.patch
RUN git clone https://github.com/CrunchyData/pg_featureserv.git /src \
    && git -C /src checkout --detach "$PG_FEATURESERV_COMMIT" \
    && git -C /src apply /tmp/quackgis.patch \
    && cd /src \
    && CGO_ENABLED=0 go test ./internal/data ./internal/service \
    && CGO_ENABLED=0 go build -trimpath -ldflags "-s -w" -o /pg_featureserv .

FROM docker.io/library/alpine:3.22@sha256:14358309a308569c32bdc37e2e0e9694be33a9d99e68afb0f5ff33cc1f695dce

RUN apk add --no-cache ca-certificates
WORKDIR /app
COPY --from=build /pg_featureserv /app/pg_featureserv
COPY --from=build /src/assets /app/assets

USER 1001:1001
EXPOSE 9000
ENTRYPOINT ["/app/pg_featureserv"]
