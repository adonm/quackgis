# Multi-modal inventory fixtures

These tiny, deterministic source artifacts exercise sidecar inventory behavior
without adding GDAL/PDAL or format decoders to the QuackGIS runtime:

- `test_dem.asc` + `test_dem.prj`: valid 4×3 ESRI ASCII Grid in EPSG:3857;
- `test_cloud.ply`: valid ASCII PLY point cloud with five XYZ vertices.

`inventory-v1.json` pins bytes, SHA-256, source-derived bounds, CRS/epoch,
vertical datum, provenance, and stable non-secret fixture URIs. The Rust
`multimodal_inventory` test parses these small headers/records itself, validates
the manifest, and inserts only sidecar footprint/provenance rows through pgwire.

This is a local real-artifact companion gate. It is not COG, COPC/LAZ, regional
inventory, object-store lifecycle, or format-decoding support.
