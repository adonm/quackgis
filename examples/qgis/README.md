# QGIS example

Start and seed a local demo server:

```sh
just demo-local
```

In QGIS, create a new PostgreSQL connection:

```text
Name: QuackGIS local demo
Host: 127.0.0.1
Port: 5434
Database: quackgis
Username: postgres
Password: <empty>
SSL mode: Prefer/Disable for local dev
```

Add these layers from schema `public`:

- `demo_points`
- `demo_polygons`

Quick validation query in DB Manager:

```sql
SELECT name, ST_AsText(ST_GeomFromWKB(geom)) AS wkt
FROM public.demo_points
ORDER BY id;
```

Expected point names: `origin`, `one`.
