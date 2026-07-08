# QuackGIS client examples

These examples use the stable demo layers seeded by `just demo-local` or
`just demo-kind`:

- `public.demo_points`
- `public.demo_polygons`

Local connection:

```text
host=127.0.0.1 port=5434 dbname=quackgis user=postgres password=<empty>
```

Kind in-cluster connection:

```text
host=quackgis.quackgis.svc.cluster.local port=5434 dbname=quackgis user=postgres password=<empty>
```

See the client-specific examples:

- [QGIS](./qgis/README.md)
- [GDAL/OGR](./ogr/README.md)
- [GeoServer](./geoserver/README.md)

For the single-command developer-preview smoke (CREATE, COPY, spatial query,
compact), see [../docs/DEVELOPER_PREVIEW.md](../docs/DEVELOPER_PREVIEW.md) or run
`just preview-smoke` from the repository root.
