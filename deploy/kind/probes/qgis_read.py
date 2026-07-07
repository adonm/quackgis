# SPDX-License-Identifier: Apache-2.0
import sys

from probe_common import pg_connect, quackgis_host, quackgis_port, require, seed_point_table, table_name
from qgis.PyQt.QtCore import QSize
from qgis.PyQt.QtGui import QColor
from qgis.core import (
    QgsApplication,
    QgsDataSourceUri,
    QgsFeatureRequest,
    QgsMapRendererSequentialJob,
    QgsMapSettings,
    QgsRectangle,
    QgsVectorLayer,
)


def main() -> int:
    host = quackgis_host()
    port = quackgis_port()
    table = table_name("qgis_probe")

    conn = pg_connect()
    conn.autocommit = True
    try:
        seed_point_table(conn, table)
    finally:
        conn.close()

    QgsApplication.setPrefixPath("/usr", True)
    app = QgsApplication([], False)
    app.initQgis()
    try:
        uri = QgsDataSourceUri()
        uri.setConnection(host, str(port), "quackgis", "postgres", "")
        uri.setDataSource("public", table, "geom", "", "id")
        layer = QgsVectorLayer(uri.uri(False), table, "postgres")
        print("valid", layer.isValid())
        print("provider", layer.providerType())
        print("error", layer.error().message())
        require(layer.isValid(), "QGIS read layer did not open")

        if layer.dataProvider():
            print("provider_error", layer.dataProvider().error().message())
            print("feature_count", layer.featureCount())
            print("fields", [f.name() for f in layer.fields()])

        features = list(layer.getFeatures())
        print("features_read", len(features))
        print("feature_attrs", [f.attributes() for f in features])
        wkts = [f.geometry().asWkt() if f.hasGeometry() else "" for f in features]
        print("feature_wkts", wkts)
        require(len(features) == 2 and all(wkts), f"unexpected features: {wkts!r}")

        filter_request = QgsFeatureRequest().setFilterExpression('"name" = \'one\'')
        filtered = list(layer.getFeatures(filter_request))
        filter_names = [f["name"] for f in filtered]
        print("filter_names", filter_names)
        require(filter_names == ["one"], f"unexpected filter_names: {filter_names!r}")

        target = next(f for f in features if f["name"] == "one")
        identify_request = QgsFeatureRequest().setFilterFid(target.id())
        identified = list(layer.getFeatures(identify_request))
        identify_names = sorted(f["name"] for f in identified)
        print("identify_names", identify_names)
        require(identify_names == ["one"], f"unexpected identify_names: {identify_names!r}")

        settings = QgsMapSettings()
        settings.setLayers([layer])
        settings.setExtent(QgsRectangle(-1, -1, 2, 2))
        settings.setOutputSize(QSize(128, 128))
        settings.setBackgroundColor(QColor(255, 255, 255))
        render_job = QgsMapRendererSequentialJob(settings)
        render_job.start()
        render_job.waitForFinished()
        image = render_job.renderedImage()
        render_ok = (not image.isNull()) and image.width() == 128 and image.height() == 128
        print("render_ok", render_ok)
        require(render_ok, "QGIS render failed")
        return 0
    finally:
        app.exitQgis()


if __name__ == "__main__":
    sys.exit(main())
