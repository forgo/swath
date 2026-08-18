# Recipes — Swath in the tools you already use

Meet-them-where-they-are bridges: no plugin, no SDK, no migration. Each
recipe is smoke-tested in CI against the compose stack (`swath-e2e`'s
`qgis_xyz_template_serves_png`), so a documented URL that stops working
fails the build.

## QGIS — XYZ Tiles connection (zero code)

Swath's tile route is already XYZ-shaped
(`GET /tilesets/{layerId}/tiles/{tileMatrix}/{tileRow}/{tileCol}`,
ENDPOINTS.md), so QGIS consumes it as a stock **XYZ Tiles** layer:

1. Start a stack (`just demo`, or `swath serve` — QUICKSTART.md).
2. In QGIS: *Browser panel → XYZ Tiles → right-click → New Connection…*
3. Name it, and set the URL template (tile row is `{y}` — placeholders
   substitute wherever they appear, so OGC `z/row/col` order just works):

   ```text
   http://localhost:8080/tilesets/truecolor/tiles/{z}/{y}/{x}
   ```

4. Set *Min/Max zoom* to the layer's tile matrix range (the fixture
   layers serve z9–z13), then drag the connection onto the canvas.

Any layer id from `GET /tilesets` works in place of `truecolor`
(`ndvi` computes on the fly — nothing pre-baked).

**Dated layers** (the time dimension, ADR 0015): pin a frame by adding
`datetime=` to the template — QGIS passes the query string through
unchanged:

```text
http://localhost:8080/tilesets/park-fire-ndvi/tiles/{z}/{y}/{x}?datetime=2024-08-01T00:00:00Z
```

Absent `datetime`, tiles follow the newest ingested granule — a live
QGIS canvas updates as granules arrive (refresh the layer). The
granules a layer can pin are listed by `GET /tilesets/{layerId}/granules`.

Remote stacks: substitute the host; nothing else changes.

## ArcGIS — parked

Deliberately not written until the QGIS recipe proves the
meet-them-where-they-are pattern with real users: ArcGIS Pro consumes
XYZ the same way (*Add Data → Data From Path → Tile Layer*), but a
recipe we haven't exercised is a support liability, not a bridge.
Revisit on the first real request.

## Jupyter / openEO — arrives with #195

The openeo-python-client notebook loop (connect → build graph →
`POST /result` → PNG inline) lands here with issue #195.
