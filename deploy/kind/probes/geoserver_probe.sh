#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0

set -euo pipefail

source /opt/quackgis-probes/probe_common.sh

table="$(probe_table_from_uid geoserver_probe)"
keyless_table="$(probe_table_from_uid geoserver_keyless)"
workspace="quackgis"
store="quackgis"
base="http://127.0.0.1:8080/geoserver"

cleanup() {
  status=$?
  if [ -f /tmp/geoserver.log ]; then
    printf '%s\n' '--- geoserver log tail ---'
    tail -300 /tmp/geoserver.log || true
  fi
  if [ -n "${gs_pid:-}" ]; then
    kill "${gs_pid}" 2>/dev/null || true
  fi
  exit "${status}"
}
trap cleanup EXIT

/opt/startup.sh >/tmp/geoserver.log 2>&1 &
gs_pid=$!

probe_wait_geoserver "${base}" "${gs_pid}"
probe_curl_auth "${base}/rest/about/version.json" | tee /tmp/version.json

cat >/tmp/workspace.xml <<XML
<workspace><name>${workspace}</name></workspace>
XML
probe_curl_auth -H "Content-Type: text/xml" \
  -d @/tmp/workspace.xml \
  "${base}/rest/workspaces" >/tmp/workspace.out

cat >/tmp/store.xml <<XML
<dataStore>
  <name>${store}</name>
  <enabled>true</enabled>
  <connectionParameters>
    <entry key="dbtype">postgis</entry>
    <entry key="host">${QUACKGIS_HOST}</entry>
    <entry key="port">${QUACKGIS_PORT}</entry>
    <entry key="database">quackgis</entry>
    <entry key="schema">public</entry>
    <entry key="user">postgres</entry>
    <entry key="passwd"></entry>
    <entry key="namespace">${workspace}</entry>
    <entry key="Expose primary keys">true</entry>
    <entry key="Loose bbox">true</entry>
    <entry key="Estimated extends">false</entry>
    <entry key="validate connections">false</entry>
    <entry key="preparedStatements">true</entry>
    <entry key="fetch size">50</entry>
  </connectionParameters>
</dataStore>
XML
probe_curl_auth -H "Content-Type: text/xml" \
  -d @/tmp/store.xml \
  "${base}/rest/workspaces/${workspace}/datastores" >/tmp/store.out

cat >/tmp/featuretype.xml <<XML
<featureType>
  <name>${table}</name>
  <nativeName>${table}</nativeName>
  <title>${table}</title>
  <srs>EPSG:4326</srs>
  <nativeCRS>EPSG:4326</nativeCRS>
  <projectionPolicy>FORCE_DECLARED</projectionPolicy>
  <nativeBoundingBox>
    <minx>-1</minx><maxx>8</maxx>
    <miny>-1</miny><maxy>8</maxy>
    <crs>EPSG:4326</crs>
  </nativeBoundingBox>
  <latLonBoundingBox>
    <minx>-1</minx><maxx>8</maxx>
    <miny>-1</miny><maxy>8</maxy>
    <crs>EPSG:4326</crs>
  </latLonBoundingBox>
  <attributes>
    <attribute>
      <name>id</name>
      <binding>java.lang.Integer</binding>
      <minOccurs>0</minOccurs><maxOccurs>1</maxOccurs><nillable>true</nillable>
    </attribute>
    <attribute>
      <name>geom</name>
      <binding>org.locationtech.jts.geom.Point</binding>
      <minOccurs>0</minOccurs><maxOccurs>1</maxOccurs><nillable>true</nillable>
    </attribute>
    <attribute>
      <name>name</name>
      <binding>java.lang.String</binding>
      <minOccurs>0</minOccurs><maxOccurs>1</maxOccurs><nillable>true</nillable>
    </attribute>
  </attributes>
  <enabled>true</enabled>
</featureType>
XML
probe_curl_auth -H "Content-Type: text/xml" \
  -d @/tmp/featuretype.xml \
  "${base}/rest/workspaces/${workspace}/datastores/${store}/featuretypes" \
  >/tmp/featuretype.out

probe_curl_auth \
  "${base}/${workspace}/ows?service=WFS&version=1.0.0&request=GetFeature&typeName=${workspace}:${table}&outputFormat=application/json" \
  -o /tmp/features.json
grep -q '"name"[[:space:]]*:[[:space:]]*"origin"' /tmp/features.json
grep -q '"name"[[:space:]]*:[[:space:]]*"one"' /tmp/features.json
point_count="$(grep -o '"type"[[:space:]]*:[[:space:]]*"Point"' /tmp/features.json | wc -l | tr -d ' ')"
printf 'wfs_point_count %s\n' "${point_count}"
test "${point_count}" -ge 2

probe_curl_auth \
  "${base}/${workspace}/wms?service=WMS&version=1.1.1&request=GetMap&layers=${workspace}:${table}&styles=&bbox=-1,-1,8,8&width=128&height=128&srs=EPSG:4326&format=image/png" \
  -o /tmp/map.png
png_header="$(od -An -tx1 -N8 /tmp/map.png | tr -d ' \n')"
printf 'wms_png_header %s\n' "${png_header}"
test "${png_header}" = "89504e470d0a1a0a"

cat >/tmp/wfst-insert.xml <<XML
<wfs:Transaction service="WFS" version="1.0.0"
    xmlns:wfs="http://www.opengis.net/wfs"
    xmlns:gml="http://www.opengis.net/gml"
    xmlns:quackgis="http://quackgis">
  <wfs:Insert>
    <quackgis:${table}>
      <quackgis:id>3</quackgis:id>
      <quackgis:geom>
        <gml:Point srsName="EPSG:4326">
          <gml:coordinates decimal="." cs="," ts=" ">4,5</gml:coordinates>
        </gml:Point>
      </quackgis:geom>
      <quackgis:name>wfst-inserted</quackgis:name>
    </quackgis:${table}>
  </wfs:Insert>
</wfs:Transaction>
XML
probe_curl_auth -H "Content-Type: text/xml" -d @/tmp/wfst-insert.xml "${base}/${workspace}/wfs" -o /tmp/wfst-insert.out
probe_xml_success /tmp/wfst-insert.out
probe_curl_auth "${base}/${workspace}/ows?service=WFS&version=1.0.0&request=GetFeature&typeName=${workspace}:${table}&outputFormat=application/json" -o /tmp/features-after-insert.json
grep -q '"name"[[:space:]]*:[[:space:]]*"wfst-inserted"' /tmp/features-after-insert.json

cat >/tmp/wfst-update.xml <<XML
<wfs:Transaction service="WFS" version="1.0.0"
    xmlns:wfs="http://www.opengis.net/wfs"
    xmlns:gml="http://www.opengis.net/gml"
    xmlns:ogc="http://www.opengis.net/ogc"
    xmlns:quackgis="http://quackgis">
  <wfs:Update typeName="${workspace}:${table}">
    <wfs:Property>
      <wfs:Name>name</wfs:Name>
      <wfs:Value>wfst-updated</wfs:Value>
    </wfs:Property>
    <wfs:Property>
      <wfs:Name>geom</wfs:Name>
      <wfs:Value>
        <gml:Point srsName="EPSG:4326">
          <gml:coordinates decimal="." cs="," ts=" ">6,7</gml:coordinates>
        </gml:Point>
      </wfs:Value>
    </wfs:Property>
    <ogc:Filter>
      <ogc:PropertyIsEqualTo>
        <ogc:PropertyName>id</ogc:PropertyName>
        <ogc:Literal>3</ogc:Literal>
      </ogc:PropertyIsEqualTo>
    </ogc:Filter>
  </wfs:Update>
</wfs:Transaction>
XML
probe_curl_auth -H "Content-Type: text/xml" -d @/tmp/wfst-update.xml "${base}/${workspace}/wfs" -o /tmp/wfst-update.out
probe_xml_success /tmp/wfst-update.out
probe_curl_auth "${base}/${workspace}/ows?service=WFS&version=1.0.0&request=GetFeature&typeName=${workspace}:${table}&outputFormat=application/json" -o /tmp/features-after-update.json
grep -q '"name"[[:space:]]*:[[:space:]]*"wfst-updated"' /tmp/features-after-update.json
! grep -q '"name"[[:space:]]*:[[:space:]]*"wfst-inserted"' /tmp/features-after-update.json

cat >/tmp/wfst-delete.xml <<XML
<wfs:Transaction service="WFS" version="1.0.0"
    xmlns:wfs="http://www.opengis.net/wfs"
    xmlns:ogc="http://www.opengis.net/ogc"
    xmlns:quackgis="http://quackgis">
  <wfs:Delete typeName="${workspace}:${table}">
    <ogc:Filter>
      <ogc:PropertyIsEqualTo>
        <ogc:PropertyName>id</ogc:PropertyName>
        <ogc:Literal>3</ogc:Literal>
      </ogc:PropertyIsEqualTo>
    </ogc:Filter>
  </wfs:Delete>
</wfs:Transaction>
XML
probe_curl_auth -H "Content-Type: text/xml" -d @/tmp/wfst-delete.xml "${base}/${workspace}/wfs" -o /tmp/wfst-delete.out
probe_xml_success /tmp/wfst-delete.out
probe_curl_auth "${base}/${workspace}/ows?service=WFS&version=1.0.0&request=GetFeature&typeName=${workspace}:${table}&outputFormat=application/json" -o /tmp/features-after-delete.json
! grep -q '"name"[[:space:]]*:[[:space:]]*"wfst-updated"' /tmp/features-after-delete.json
printf '%s\n' 'wfst_transaction_ok True'

cat >/tmp/keyless-featuretype.xml <<XML
<featureType>
  <name>${keyless_table}</name>
  <nativeName>${keyless_table}</nativeName>
  <title>${keyless_table}</title>
  <srs>EPSG:4326</srs>
  <nativeCRS>EPSG:4326</nativeCRS>
  <projectionPolicy>FORCE_DECLARED</projectionPolicy>
  <nativeBoundingBox>
    <minx>-1</minx><maxx>8</maxx>
    <miny>-1</miny><maxy>8</maxy>
    <crs>EPSG:4326</crs>
  </nativeBoundingBox>
  <latLonBoundingBox>
    <minx>-1</minx><maxx>8</maxx>
    <miny>-1</miny><maxy>8</maxy>
    <crs>EPSG:4326</crs>
  </latLonBoundingBox>
  <attributes>
    <attribute>
      <name>_quackgis_rowid</name>
      <binding>java.lang.Long</binding>
      <minOccurs>1</minOccurs><maxOccurs>1</maxOccurs><nillable>false</nillable>
    </attribute>
    <attribute>
      <name>geom</name>
      <binding>org.locationtech.jts.geom.Point</binding>
      <minOccurs>0</minOccurs><maxOccurs>1</maxOccurs><nillable>true</nillable>
    </attribute>
    <attribute>
      <name>name</name>
      <binding>java.lang.String</binding>
      <minOccurs>0</minOccurs><maxOccurs>1</maxOccurs><nillable>true</nillable>
    </attribute>
  </attributes>
  <enabled>true</enabled>
</featureType>
XML
probe_curl_auth -H "Content-Type: text/xml" \
  -d @/tmp/keyless-featuretype.xml \
  "${base}/rest/workspaces/${workspace}/datastores/${store}/featuretypes" \
  >/tmp/keyless-featuretype.out

probe_curl_auth \
  "${base}/${workspace}/ows?service=WFS&version=1.0.0&request=GetFeature&typeName=${workspace}:${keyless_table}&outputFormat=application/json" \
  -o /tmp/keyless-features.json
grep -q '"name"[[:space:]]*:[[:space:]]*"keyless-origin"' /tmp/keyless-features.json
grep -q '"name"[[:space:]]*:[[:space:]]*"keyless-one"' /tmp/keyless-features.json
keyless_point_count="$(grep -o '"type"[[:space:]]*:[[:space:]]*"Point"' /tmp/keyless-features.json | wc -l | tr -d ' ')"
printf 'geoserver_keyless_wfs_point_count %s\n' "${keyless_point_count}"
test "${keyless_point_count}" -ge 2

cat >/tmp/keyless-wfst-update.xml <<XML
<wfs:Transaction service="WFS" version="1.0.0"
    xmlns:wfs="http://www.opengis.net/wfs"
    xmlns:ogc="http://www.opengis.net/ogc"
    xmlns:quackgis="http://quackgis">
  <wfs:Update typeName="${workspace}:${keyless_table}">
    <wfs:Property>
      <wfs:Name>name</wfs:Name>
      <wfs:Value>keyless-updated</wfs:Value>
    </wfs:Property>
    <ogc:Filter>
      <ogc:PropertyIsEqualTo>
        <ogc:PropertyName>_quackgis_rowid</ogc:PropertyName>
        <ogc:Literal>1</ogc:Literal>
      </ogc:PropertyIsEqualTo>
    </ogc:Filter>
  </wfs:Update>
</wfs:Transaction>
XML
probe_curl_auth -H "Content-Type: text/xml" -d @/tmp/keyless-wfst-update.xml "${base}/${workspace}/wfs" -o /tmp/keyless-wfst-update.out
probe_xml_success /tmp/keyless-wfst-update.out
probe_curl_auth "${base}/${workspace}/ows?service=WFS&version=1.0.0&request=GetFeature&typeName=${workspace}:${keyless_table}&outputFormat=application/json" -o /tmp/keyless-features-after-update.json
grep -q '"name"[[:space:]]*:[[:space:]]*"keyless-updated"' /tmp/keyless-features-after-update.json
grep -q '"name"[[:space:]]*:[[:space:]]*"keyless-one"' /tmp/keyless-features-after-update.json
! grep -q '"name"[[:space:]]*:[[:space:]]*"keyless-origin"' /tmp/keyless-features-after-update.json
printf '%s\n' 'geoserver_keyless_update_ok True'

printf '%s\n' 'geoserver_probe_ok True'
