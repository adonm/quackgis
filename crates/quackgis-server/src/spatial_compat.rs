// SPDX-License-Identifier: Apache-2.0
//! Bounded PostGIS spelling and result-shape compatibility for DuckDB Spatial.
//!
//! Rewrites apply only to unquoted function identifiers. String literals,
//! quoted identifiers, dollar-quoted bodies, and comments are copied verbatim.

/// Macros installed into the DuckDB control database after loading `spatial`.
///
/// Names are QuackGIS-specific so extension upgrades cannot silently shadow or
/// replace this compatibility contract.
pub const DUCKDB_COMPATIBILITY_MACROS: &str = r#"
CREATE OR REPLACE MACRO quackgis_postgis_lib_version() AS '3.4.0';
CREATE OR REPLACE MACRO quackgis_postgis_version() AS '3.4.0 QUACKGIS';
CREATE OR REPLACE MACRO quackgis_st_geomfromewkt(ewkt) AS
    ST_GeomFromText(regexp_replace(CAST(ewkt AS VARCHAR), '^[sS][rR][iI][dD]=[0-9]+;', ''));
CREATE OR REPLACE MACRO quackgis_st_asbinary(g, byte_order) AS
    CASE WHEN upper(CAST(byte_order AS VARCHAR)) = 'NDR' THEN ST_AsWKB(g)
         ELSE error('QuackGIS ST_AsBinary supports NDR byte order only') END;
CREATE OR REPLACE MACRO quackgis_st_ashexewkb(g) AS hex(ST_AsWKB(g));
CREATE OR REPLACE MACRO quackgis_geometry_type(g) AS
    upper(CAST(ST_GeometryType(ST_GeomFromWKB(CAST(g AS BLOB))) AS VARCHAR));
CREATE OR REPLACE MACRO quackgis_st_geometry_type(g) AS
    CASE upper(CAST(ST_GeometryType(ST_GeomFromWKB(CAST(g AS BLOB))) AS VARCHAR))
        WHEN 'POINT' THEN 'ST_Point'
        WHEN 'LINESTRING' THEN 'ST_LineString'
        WHEN 'POLYGON' THEN 'ST_Polygon'
        WHEN 'MULTIPOINT' THEN 'ST_MultiPoint'
        WHEN 'MULTILINESTRING' THEN 'ST_MultiLineString'
        WHEN 'MULTIPOLYGON' THEN 'ST_MultiPolygon'
        WHEN 'GEOMETRYCOLLECTION' THEN 'ST_GeometryCollection'
        ELSE 'ST_' || CAST(ST_GeometryType(ST_GeomFromWKB(CAST(g AS BLOB))) AS VARCHAR)
    END;
CREATE OR REPLACE MACRO quackgis_st_curvetoline(g) AS g;
CREATE OR REPLACE MACRO quackgis_st_hasarc(g) AS false;
CREATE OR REPLACE MACRO quackgis_st_srid(g) AS
    CASE WHEN g IS NULL THEN NULL
         ELSE coalesce(try_cast(regexp_extract(
                  ST_CRS(ST_GeomFromWKB(CAST(g AS BLOB))),
                  '^EPSG:([0-9]+)$', 1) AS INTEGER), 0)
    END;
CREATE OR REPLACE MACRO quackgis_st_zmflag(g) AS
    CASE WHEN g IS NULL THEN NULL
         ELSE ST_ZMFlag(ST_GeomFromWKB(CAST(g AS BLOB))) END;
CREATE OR REPLACE MACRO quackgis_st_extent(g) AS
    replace(CAST(ST_Extent(ST_Extent_Agg(ST_GeomFromWKB(CAST(g AS BLOB)))) AS VARCHAR),
            ', ', ',');
CREATE OR REPLACE MACRO quackgis_st_3dextent(g) AS
    CASE WHEN count(g) = 0 THEN NULL ELSE
      'BOX3D(' ||
      printf('%.17g', min(ST_XMin(ST_GeomFromWKB(CAST(g AS BLOB))))) || ' ' ||
      printf('%.17g', min(ST_YMin(ST_GeomFromWKB(CAST(g AS BLOB))))) || ' ' ||
      printf('%.17g', coalesce(min(ST_ZMin(ST_GeomFromWKB(CAST(g AS BLOB)))), 0)) || ',' ||
      printf('%.17g', max(ST_XMax(ST_GeomFromWKB(CAST(g AS BLOB))))) || ' ' ||
      printf('%.17g', max(ST_YMax(ST_GeomFromWKB(CAST(g AS BLOB))))) || ' ' ||
      printf('%.17g', coalesce(max(ST_ZMax(ST_GeomFromWKB(CAST(g AS BLOB)))), 0)) || ')'
    END;
CREATE OR REPLACE MACRO quackgis_postgis_geos_version() AS 'QUACKGIS-DUCKDB';
CREATE OR REPLACE MACRO quackgis_postgis_proj_version() AS DuckDB_Proj_Version();
"#;

/// Rewrite PostGIS function spellings to DuckDB-native or QuackGIS-owned names.
pub fn rewrite_postgis_sql(sql: &str) -> String {
    let bytes = sql.as_bytes();
    let mut output = String::with_capacity(sql.len());
    let mut index = 0;

    while index < bytes.len() {
        match bytes[index] {
            b'\'' => copy_single_quoted(sql, &mut output, &mut index),
            b'"' => copy_double_quoted(sql, &mut output, &mut index),
            b'-' if bytes.get(index + 1) == Some(&b'-') => {
                copy_line_comment(sql, &mut output, &mut index);
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                copy_block_comment(sql, &mut output, &mut index);
            }
            b'$' => {
                if let Some(delimiter_end) = dollar_delimiter_end(bytes, index) {
                    copy_dollar_quoted(sql, &mut output, &mut index, delimiter_end);
                } else {
                    output.push('$');
                    index += 1;
                }
            }
            byte if is_identifier_start(byte) => {
                let start = index;
                index += 1;
                while index < bytes.len() && is_identifier_continue(bytes[index]) {
                    index += 1;
                }
                let identifier = &sql[start..index];
                let next = bytes[index..]
                    .iter()
                    .copied()
                    .find(|byte| !byte.is_ascii_whitespace());
                if next == Some(b'(') {
                    output.push_str(rewrite_function_name(identifier).unwrap_or(identifier));
                } else {
                    output.push_str(identifier);
                }
            }
            _ => {
                let character = sql[index..].chars().next().expect("valid UTF-8 suffix");
                output.push(character);
                index += character.len_utf8();
            }
        }
    }

    output
}

fn rewrite_function_name(identifier: &str) -> Option<&'static str> {
    if identifier.eq_ignore_ascii_case("postgis_lib_version") {
        Some("quackgis_postgis_lib_version")
    } else if identifier.eq_ignore_ascii_case("postgis_version") {
        Some("quackgis_postgis_version")
    } else if identifier.eq_ignore_ascii_case("st_geomfromewkt") {
        Some("quackgis_st_geomfromewkt")
    } else if identifier.eq_ignore_ascii_case("st_makepoint") {
        Some("ST_Point")
    } else if identifier.eq_ignore_ascii_case("st_asbinary") {
        Some("quackgis_st_asbinary")
    } else if identifier.eq_ignore_ascii_case("st_ashexewkb") {
        Some("quackgis_st_ashexewkb")
    } else if identifier.eq_ignore_ascii_case("geometrytype") {
        Some("quackgis_geometry_type")
    } else if identifier.eq_ignore_ascii_case("st_geometrytype") {
        Some("quackgis_st_geometry_type")
    } else if identifier.eq_ignore_ascii_case("st_numpoints") {
        Some("ST_NPoints")
    } else if identifier.eq_ignore_ascii_case("st_curvetoline") {
        Some("quackgis_st_curvetoline")
    } else if identifier.eq_ignore_ascii_case("st_hasarc") {
        Some("quackgis_st_hasarc")
    } else if identifier.eq_ignore_ascii_case("st_srid") {
        Some("quackgis_st_srid")
    } else if identifier.eq_ignore_ascii_case("st_zmflag") {
        Some("quackgis_st_zmflag")
    } else if identifier.eq_ignore_ascii_case("st_extent") {
        Some("quackgis_st_extent")
    } else if identifier.eq_ignore_ascii_case("st_3dextent") {
        Some("quackgis_st_3dextent")
    } else if identifier.eq_ignore_ascii_case("postgis_geos_version") {
        Some("quackgis_postgis_geos_version")
    } else if identifier.eq_ignore_ascii_case("postgis_proj_version") {
        Some("quackgis_postgis_proj_version")
    } else {
        None
    }
}

fn is_identifier_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

fn is_identifier_continue(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'$')
}

fn copy_single_quoted(sql: &str, output: &mut String, index: &mut usize) {
    copy_quoted(sql, output, index, b'\'');
}

fn copy_double_quoted(sql: &str, output: &mut String, index: &mut usize) {
    copy_quoted(sql, output, index, b'"');
}

fn copy_quoted(sql: &str, output: &mut String, index: &mut usize, quote: u8) {
    let bytes = sql.as_bytes();
    let start = *index;
    *index += 1;
    while *index < bytes.len() {
        if bytes[*index] == quote {
            *index += 1;
            if bytes.get(*index) == Some(&quote) {
                *index += 1;
                continue;
            }
            break;
        }
        *index += 1;
    }
    output.push_str(&sql[start..*index]);
}

fn copy_line_comment(sql: &str, output: &mut String, index: &mut usize) {
    let bytes = sql.as_bytes();
    let start = *index;
    *index += 2;
    while *index < bytes.len() && bytes[*index] != b'\n' {
        *index += 1;
    }
    output.push_str(&sql[start..*index]);
}

fn copy_block_comment(sql: &str, output: &mut String, index: &mut usize) {
    let bytes = sql.as_bytes();
    let start = *index;
    *index += 2;
    let mut depth = 1_u32;
    while *index < bytes.len() && depth > 0 {
        if bytes.get(*index) == Some(&b'/') && bytes.get(*index + 1) == Some(&b'*') {
            depth += 1;
            *index += 2;
        } else if bytes.get(*index) == Some(&b'*') && bytes.get(*index + 1) == Some(&b'/') {
            depth -= 1;
            *index += 2;
        } else {
            *index += 1;
        }
    }
    output.push_str(&sql[start..*index]);
}

fn dollar_delimiter_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut index = start + 1;
    if bytes.get(index) == Some(&b'$') {
        return Some(index + 1);
    }
    if !bytes.get(index).copied().is_some_and(is_identifier_start) {
        return None;
    }
    index += 1;
    while bytes
        .get(index)
        .copied()
        .is_some_and(is_identifier_continue)
    {
        index += 1;
    }
    (bytes.get(index) == Some(&b'$')).then_some(index + 1)
}

fn copy_dollar_quoted(sql: &str, output: &mut String, index: &mut usize, delimiter_end: usize) {
    let start = *index;
    let delimiter = &sql[start..delimiter_end];
    let body_start = delimiter_end;
    *index = sql[body_start..]
        .find(delimiter)
        .map_or(sql.len(), |offset| body_start + offset + delimiter.len());
    output.push_str(&sql[start..*index]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_only_unquoted_function_calls() {
        let sql = "SELECT ST_MakePoint(1, 2), ST_NumPoints(g), ST_GeometryType(g), \
                   'ST_MakePoint(3, 4)', \"ST_NumPoints\", $tag$ST_HasArc(g)$tag$ \
                   -- ST_GeometryType(g)\n+                   /* ST_CurveToLine(g) */";
        let rewritten = rewrite_postgis_sql(sql);
        assert!(rewritten.contains("ST_Point(1, 2)"));
        assert!(rewritten.contains("ST_NPoints(g)"));
        assert!(rewritten.contains("quackgis_st_geometry_type(g)"));
        assert!(rewritten.contains("'ST_MakePoint(3, 4)'"));
        assert!(rewritten.contains("\"ST_NumPoints\""));
        assert!(rewritten.contains("$tag$ST_HasArc(g)$tag$"));
        assert!(rewritten.contains("-- ST_GeometryType(g)"));
        assert!(rewritten.contains("/* ST_CurveToLine(g) */"));
    }

    #[test]
    fn rewrites_curated_compatibility_functions() {
        for (source, target) in [
            ("postgis_lib_version()", "quackgis_postgis_lib_version()"),
            ("ST_GeomFromEWKT($1)", "quackgis_st_geomfromewkt($1)"),
            ("st_asbinary(g, 'NDR')", "quackgis_st_asbinary(g, 'NDR')"),
            ("ST_AsHEXEWKB(g)", "quackgis_st_ashexewkb(g)"),
            ("GeometryType(g)", "quackgis_geometry_type(g)"),
            ("ST_CurveToLine(g)", "quackgis_st_curvetoline(g)"),
            ("ST_HasArc(g)", "quackgis_st_hasarc(g)"),
            ("ST_Zmflag(g)", "quackgis_st_zmflag(g)"),
        ] {
            assert_eq!(rewrite_postgis_sql(source), target);
        }
    }
}
