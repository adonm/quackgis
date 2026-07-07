# SPDX-License-Identifier: Apache-2.0
import sys

from probe_common import pg_connect, quackgis_host, quackgis_port, quote_ident, table_name
from qgis.core import QgsApplication, QgsDataSourceUri, QgsFeature, QgsGeometry, QgsVectorLayer


def main() -> int:
    host = quackgis_host()
    port = quackgis_port()
    table = table_name("qgis_edit_probe")

    conn = pg_connect()
    conn.autocommit = True
    try:
        with conn.cursor() as cur:
            cur.execute(f"CREATE TABLE public.{quote_ident(table)} (name TEXT, geom BINARY)")
            cur.execute(
                f"INSERT INTO public.{quote_ident(table)} (name, geom) VALUES "
                "('seed', X'010100000000000000000000000000000000000000') "
                "RETURNING \"_quackgis_rowid\", name"
            )
            print("seed_returning", cur.fetchall())
    finally:
        conn.close()

    QgsApplication.setPrefixPath("/usr", True)
    app = QgsApplication([], False)
    app.initQgis()

    def open_layer():
        uri = QgsDataSourceUri()
        uri.setConnection(host, str(port), "quackgis", "postgres", "")
        uri.setDataSource("public", table, "geom", "", "_quackgis_rowid")
        layer = QgsVectorLayer(uri.uri(False), table, "postgres")
        print("valid", layer.isValid())
        print("provider", layer.providerType())
        print("error", layer.error().message())
        if layer.dataProvider():
            print("provider_error", layer.dataProvider().error().message())
            print("capabilities", layer.dataProvider().capabilitiesString())
        if not layer.isValid():
            raise RuntimeError("QGIS edit layer did not open")
        return layer

    def snapshot(layer, label):
        layer.reload()
        feats = sorted(list(layer.getFeatures()), key=lambda f: (str(f["name"]), f.id()))
        rows = [
            (
                f.id(),
                f["_quackgis_rowid"] if "_quackgis_rowid" in f.fields().names() else None,
                f["name"],
                f.geometry().asWkt() if f.hasGeometry() else "",
            )
            for f in feats
        ]
        print(label, rows)
        return feats, rows

    def assert_commit(layer, label):
        if not layer.commitChanges():
            print(f"{label}_commit_errors", layer.commitErrors())
            raise RuntimeError(f"{label} commit failed")

    try:
        layer = open_layer()
        print("fields", [f.name() for f in layer.fields()])
        before, before_rows = snapshot(layer, "before")
        if len(before) != 1 or before[0]["name"] != "seed":
            raise RuntimeError(f"unexpected initial rows: {before_rows}")

        if not layer.startEditing():
            raise RuntimeError("startEditing insert failed")
        new_feature = QgsFeature(layer.fields())
        new_feature.setAttribute("name", "inserted")
        new_feature.setGeometry(QgsGeometry.fromWkt("POINT(1 1)"))
        if not layer.addFeature(new_feature):
            print("insert_errors", layer.commitErrors())
            raise RuntimeError("layer.addFeature failed")
        assert_commit(layer, "insert")

        layer = open_layer()
        inserted_features, inserted_rows = snapshot(layer, "after_insert")
        if sorted(f["name"] for f in inserted_features) != ["inserted", "seed"]:
            raise RuntimeError(f"unexpected rows after insert: {inserted_rows}")

        target = next(f for f in inserted_features if f["name"] == "inserted")
        if not layer.startEditing():
            raise RuntimeError("startEditing update failed")
        name_idx = layer.fields().indexFromName("name")
        if not layer.changeAttributeValue(target.id(), name_idx, "updated"):
            raise RuntimeError("changeAttributeValue failed")
        if not layer.changeGeometry(target.id(), QgsGeometry.fromWkt("POINT(2 2)")):
            raise RuntimeError("changeGeometry failed")
        assert_commit(layer, "update")

        layer = open_layer()
        updated_features, updated_rows = snapshot(layer, "after_update")
        if "updated" not in [f["name"] for f in updated_features]:
            raise RuntimeError(f"update not visible: {updated_rows}")

        seed = next(f for f in updated_features if f["name"] == "seed")
        if not layer.startEditing():
            raise RuntimeError("startEditing delete failed")
        if not layer.deleteFeature(seed.id()):
            raise RuntimeError("deleteFeature failed")
        assert_commit(layer, "delete")

        layer = open_layer()
        final_features, final_rows = snapshot(layer, "after_delete")
        ok = (
            len(final_features) == 1
            and final_features[0]["name"] == "updated"
            and final_features[0].geometry().asWkt().lower().startswith("point")
        )
        print("edit_ok", ok)
        if not ok:
            raise RuntimeError(f"unexpected final rows: {final_rows}")
        return 0
    finally:
        app.exitQgis()


if __name__ == "__main__":
    sys.exit(main())
