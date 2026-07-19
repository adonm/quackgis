#!/usr/bin/env sh
set -eu

image=${QUACKGIS_QGIS_IMAGE:-docker.io/qgis/qgis@sha256:aa55ce7f4b87d8fd28accc51658fe550667865c2ed088778c35915c2b4347587}
network=${QUACKGIS_COMPOSE_NETWORK:-quackgis_default}

if [ -n "${CONTAINER_ENGINE:-}" ]; then
    engine=$CONTAINER_ENGINE
elif command -v docker >/dev/null 2>&1; then
    engine=docker
elif command -v podman >/dev/null 2>&1; then
    engine=podman
else
    echo "Docker or Podman is required" >&2
    exit 2
fi

ogr_output=$(
    "$engine" run --rm \
        --network "$network" \
        --entrypoint /usr/bin/ogrinfo \
        "$image" \
        -ro -q \
        -spat -123.2 49.1 -123.0 49.3 \
        'PG:host=postgres port=5432 dbname=quackgis user=quackgis_reader password=quackgis-reader-dev sslmode=disable' \
        public.features
)
printf '%s\n' "$ogr_output"
printf '%s\n' "$ogr_output" | grep -F 'id (Integer64) = 1' >/dev/null
printf '%s\n' "$ogr_output" | grep -F 'name (String) = west' >/dev/null
printf '%s\n' "$ogr_output" | grep -F 'POINT (-123.1 49.2)' >/dev/null
if printf '%s\n' "$ogr_output" | grep -F 'id (Integer64) = 2' >/dev/null; then
    echo "OGR viewport returned an out-of-bounds feature" >&2
    exit 1
fi

qgis_script=$(cat <<'SCRIPT'
python3 - <<'PY'
from qgis.PyQt.QtCore import QSize
from qgis.PyQt.QtGui import QColor
from qgis.core import (
    Qgis,
    QgsApplication,
    QgsDataSourceUri,
    QgsFeatureRequest,
    QgsMapRendererParallelJob,
    QgsMapSettings,
    QgsMarkerSymbol,
    QgsRectangle,
    QgsSingleSymbolRenderer,
    QgsVectorLayer,
)

assert Qgis.QGIS_VERSION == "3.44.11-Solothurn", Qgis.QGIS_VERSION
app = QgsApplication([], False)
app.initQgis()
try:
    uri = QgsDataSourceUri()
    uri.setConnection(
        "postgres",
        "5432",
        "quackgis",
        "quackgis_reader",
        "quackgis-reader-dev",
        QgsDataSourceUri.SslDisable,
    )
    uri.setUseEstimatedMetadata(True)
    uri.setDataSource("public", "features", "geom", "", "id")
    layer = QgsVectorLayer(uri.uri(False), "features", "postgres")
    assert layer.isValid(), layer.error().message()
    assert [field.name() for field in layer.fields()] == ["id", "name"]

    features = list(layer.getFeatures())
    values = sorted(
        (
            feature["id"],
            feature["name"],
            round(feature.geometry().asPoint().x(), 2),
            round(feature.geometry().asPoint().y(), 2),
        )
        for feature in features
    )
    assert values == [
        (1, "west", -123.1, 49.2),
        (2, "east", -122.9, 49.25),
        (3, "south", -123.05, 48.9),
    ], values

    extent = layer.extent()
    extent_values = tuple(
        round(value, 2)
        for value in (
            extent.xMinimum(),
            extent.yMinimum(),
            extent.xMaximum(),
            extent.yMaximum(),
        )
    )
    assert extent_values == (-123.1, 48.9, -122.9, 49.25), extent_values

    viewport = list(
        layer.getFeatures(
            QgsFeatureRequest().setFilterRect(
                QgsRectangle(-123.2, 49.1, -123.0, 49.3)
            )
        )
    )
    assert [(feature["id"], feature["name"]) for feature in viewport] == [
        (1, "west")
    ], viewport

    assert layer.setSubsetString('"id" = 2')
    subset = list(layer.getFeatures())
    assert [(feature["id"], feature["name"]) for feature in subset] == [
        (2, "east")
    ], subset
    assert layer.setSubsetString("")

    layer.setRenderer(
        QgsSingleSymbolRenderer(
            QgsMarkerSymbol.createSimple(
                {"name": "circle", "color": "255,0,0", "size": "8"}
            )
        )
    )
    settings = QgsMapSettings()
    settings.setLayers([layer])
    settings.setExtent(QgsRectangle(-123.2, 48.8, -122.8, 49.4))
    settings.setOutputSize(QSize(128, 128))
    settings.setBackgroundColor(QColor("white"))
    renderer = QgsMapRendererParallelJob(settings)
    renderer.start()
    renderer.waitForFinished()
    image = renderer.renderedImage()
    assert not image.isNull()
    assert renderer.errors() == [], renderer.errors()
    nonwhite = sum(
        image.pixelColor(x, y) != QColor("white")
        for x in range(image.width())
        for y in range(image.height())
    )
    assert nonwhite > 0, nonwhite
    print(
        "QGIS client passed version=%s rows=3 viewport=west subset=east render_pixels=%d"
        % (Qgis.QGIS_VERSION, nonwhite)
    )
finally:
    app.exitQgis()
PY
SCRIPT
)

"$engine" run --rm \
    --network "$network" \
    --entrypoint /bin/bash \
    -e HOME=/tmp/qgis-home \
    -e QT_QPA_PLATFORM=offscreen \
    "$image" \
    -ceu "$qgis_script"

echo "QuackGIS GDAL/OGR and QGIS PostgreSQL client smoke passed"
