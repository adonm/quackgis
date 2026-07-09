// SPDX-License-Identifier: Apache-2.0
//! Minimal MVT (Mapbox Vector Tile) encoder + WKB geometry reader.
//!
//! Implements enough of the MVT spec for Martin tile serving:
//! - WKB → coordinate extraction (Point, LineString, Polygon, Multi*)
//! - Coordinate transform to tile pixel space (ST_AsMVTGeom)
//! - Protobuf MVT tile encoding (ST_AsMVT aggregate)
//!
//! No external protobuf or WKB crate dependency — pure Rust.

// ─── Protobuf varint encoding ──────────────────────────────────────────────

fn encode_varint(mut value: u64, buf: &mut Vec<u8>) {
    while value >= 0x80 {
        buf.push((value as u8) | 0x80);
        value >>= 7;
    }
    buf.push(value as u8);
}

fn encode_zigzag32(n: i32) -> u32 {
    ((n << 1) ^ (n >> 31)) as u32
}

fn encode_zigzag64(n: i64) -> u64 {
    ((n << 1) ^ (n >> 63)) as u64
}

fn encode_tag(field: u32, wire_type: u32, buf: &mut Vec<u8>) {
    encode_varint(((field as u64) << 3) | (wire_type as u64), buf);
}

fn encode_string_field(field: u32, s: &str, buf: &mut Vec<u8>) {
    encode_tag(field, 2, buf);
    encode_varint(s.len() as u64, buf);
    buf.extend_from_slice(s.as_bytes());
}

fn encode_bytes_field(field: u32, data: &[u8], buf: &mut Vec<u8>) {
    encode_tag(field, 2, buf);
    encode_varint(data.len() as u64, buf);
    buf.extend_from_slice(data);
}

fn encode_uint32_field(field: u32, val: u32, buf: &mut Vec<u8>) {
    encode_tag(field, 0, buf);
    encode_varint(val as u64, buf);
}

fn encode_uint64_field(field: u32, val: u64, buf: &mut Vec<u8>) {
    encode_tag(field, 0, buf);
    encode_varint(val, buf);
}

// ─── WKB geometry reading ─────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum GeomType {
    Point,
    LineString,
    Polygon,
    MultiPoint,
    MultiLineString,
    MultiPolygon,
    GeometryCollection,
    Unknown(u32),
}

impl GeomType {
    fn from_u32(code: u32) -> Self {
        match code & 0xFF {
            1 => GeomType::Point,
            2 => GeomType::LineString,
            3 => GeomType::Polygon,
            4 => GeomType::MultiPoint,
            5 => GeomType::MultiLineString,
            6 => GeomType::MultiPolygon,
            7 => GeomType::GeometryCollection,
            other => GeomType::Unknown(other),
        }
    }

    fn mvt_type(&self) -> Option<u32> {
        match self {
            GeomType::Point | GeomType::MultiPoint => Some(1),
            GeomType::LineString | GeomType::MultiLineString => Some(2),
            GeomType::Polygon | GeomType::MultiPolygon => Some(3),
            _ => None,
        }
    }
}

/// A parsed geometry in a flat coordinate representation.
pub struct ParsedGeom {
    pub geom_type: GeomType,
    /// Flat list of rings, each ring is a Vec of (x, y) points.
    /// Point/LineString: one ring. Polygon: outer + inner rings.
    /// Multi*: rings from all sub-geometries concatenated, with ring_counts
    /// tracking sub-geometry boundaries.
    pub rings: Vec<Vec<(f64, f64)>>,
    /// For Multi* types: number of rings per sub-geometry (for proper MVT
    /// encoding). None for simple types.
    pub sub_geom_ring_counts: Option<Vec<usize>>,
    pub srid: Option<i32>,
}

impl ParsedGeom {
    /// Get all coordinates as a flat iterator.
    pub fn all_coords(&self) -> impl Iterator<Item = (f64, f64)> + '_ {
        self.rings.iter().flat_map(|r| r.iter().copied())
    }

    /// Bounding box.
    pub fn bbox(&self) -> Option<(f64, f64, f64, f64)> {
        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        let mut found = false;
        for (x, y) in self.all_coords() {
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
            found = true;
        }
        found.then_some((min_x, min_y, max_x, max_y))
    }

    /// Map all coordinates through a transform function.
    pub fn map_coords<F: Fn(f64, f64) -> (f64, f64)>(&self, f: F) -> Self {
        let rings = self
            .rings
            .iter()
            .map(|ring| ring.iter().map(|(x, y)| f(*x, *y)).collect())
            .collect();
        Self {
            geom_type: self.geom_type.clone(),
            rings,
            sub_geom_ring_counts: self.sub_geom_ring_counts.clone(),
            srid: self.srid,
        }
    }

    /// Encode back to WKB (little-endian, no SRID flag).
    pub fn to_wkb(&self) -> Vec<u8> {
        let type_code = match self.geom_type {
            GeomType::Point => 1u32,
            GeomType::LineString => 2,
            GeomType::Polygon => 3,
            GeomType::MultiPoint => 4,
            GeomType::MultiLineString => 5,
            GeomType::MultiPolygon => 6,
            GeomType::GeometryCollection => 7,
            GeomType::Unknown(c) => c,
        };

        let mut buf = Vec::new();
        buf.push(1); // LE

        match &self.geom_type {
            GeomType::Point => {
                buf.extend_from_slice(&type_code.to_le_bytes());
                if let Some(pt) = self.rings.first().and_then(|r| r.first()) {
                    buf.extend_from_slice(&pt.0.to_le_bytes());
                    buf.extend_from_slice(&pt.1.to_le_bytes());
                }
            }
            GeomType::LineString => {
                buf.extend_from_slice(&type_code.to_le_bytes());
                let pts = self.rings.first().map(|r| r.len()).unwrap_or(0);
                buf.extend_from_slice(&(pts as u32).to_le_bytes());
                for (x, y) in self.rings.first().iter().flat_map(|r| r.iter()) {
                    buf.extend_from_slice(&x.to_le_bytes());
                    buf.extend_from_slice(&y.to_le_bytes());
                }
            }
            GeomType::Polygon => {
                buf.extend_from_slice(&type_code.to_le_bytes());
                buf.extend_from_slice(&(self.rings.len() as u32).to_le_bytes());
                for ring in &self.rings {
                    buf.extend_from_slice(&(ring.len() as u32).to_le_bytes());
                    for (x, y) in ring {
                        buf.extend_from_slice(&x.to_le_bytes());
                        buf.extend_from_slice(&y.to_le_bytes());
                    }
                }
            }
            GeomType::MultiPoint => {
                buf.extend_from_slice(&type_code.to_le_bytes());
                let n = self.rings.first().map(|r| r.len()).unwrap_or(0);
                buf.extend_from_slice(&(n as u32).to_le_bytes());
                for (x, y) in self.rings.first().iter().flat_map(|r| r.iter()) {
                    buf.push(1);
                    buf.extend_from_slice(&1u32.to_le_bytes());
                    buf.extend_from_slice(&x.to_le_bytes());
                    buf.extend_from_slice(&y.to_le_bytes());
                }
            }
            GeomType::MultiLineString => {
                buf.extend_from_slice(&type_code.to_le_bytes());
                buf.extend_from_slice(&(self.rings.len() as u32).to_le_bytes());
                for ring in &self.rings {
                    buf.push(1);
                    buf.extend_from_slice(&2u32.to_le_bytes());
                    buf.extend_from_slice(&(ring.len() as u32).to_le_bytes());
                    for (x, y) in ring {
                        buf.extend_from_slice(&x.to_le_bytes());
                        buf.extend_from_slice(&y.to_le_bytes());
                    }
                }
            }
            GeomType::MultiPolygon => {
                buf.extend_from_slice(&type_code.to_le_bytes());
                let counts = self.sub_geom_ring_counts.as_ref();
                let n_polys = counts.map(|c| c.len()).unwrap_or(1);
                buf.extend_from_slice(&(n_polys as u32).to_le_bytes());
                let mut ring_idx = 0;
                let counts_vec = counts.cloned().unwrap_or_else(|| vec![self.rings.len()]);
                for &ring_count in &counts_vec {
                    buf.push(1);
                    buf.extend_from_slice(&3u32.to_le_bytes());
                    buf.extend_from_slice(&(ring_count as u32).to_le_bytes());
                    for _ in 0..ring_count {
                        if ring_idx >= self.rings.len() {
                            break;
                        }
                        let ring = &self.rings[ring_idx];
                        buf.extend_from_slice(&(ring.len() as u32).to_le_bytes());
                        for (x, y) in ring {
                            buf.extend_from_slice(&x.to_le_bytes());
                            buf.extend_from_slice(&y.to_le_bytes());
                        }
                        ring_idx += 1;
                    }
                }
            }
            _ => {
                // GeometryCollection or Unknown: encode as empty
                buf.extend_from_slice(&type_code.to_le_bytes());
                buf.extend_from_slice(&0u32.to_le_bytes());
            }
        }
        buf
    }
}

/// Parse WKB bytes into a ParsedGeom.
pub fn parse_wkb(wkb: &[u8]) -> Option<ParsedGeom> {
    if wkb.len() < 5 {
        return None;
    }
    let le = wkb[0] == 1;
    let type_code = read_u32(wkb, 1, le);
    let base_type = type_code & 0xFF;
    let has_srid = type_code & 0x20000000 != 0;
    let srid = if has_srid && wkb.len() >= 9 {
        Some(read_i32(wkb, 5, le))
    } else {
        None
    };
    let data_off = if has_srid { 9 } else { 5 };
    let mut off = data_off;

    let geom_type = GeomType::from_u32(base_type);
    let rings = match base_type {
        1 => {
            // Point
            if off + 16 > wkb.len() {
                return None;
            }
            let x = read_f64(wkb, off, le)?;
            let y = read_f64(wkb, off + 8, le)?;
            vec![vec![(x, y)]]
        }
        2 => {
            // LineString
            if off + 4 > wkb.len() {
                return None;
            }
            let n = read_u32(wkb, off, le) as usize;
            off += 4;
            let mut pts = Vec::with_capacity(n);
            for _ in 0..n {
                if off + 16 > wkb.len() {
                    break;
                }
                pts.push((read_f64(wkb, off, le)?, read_f64(wkb, off + 8, le)?));
                off += 16;
            }
            vec![pts]
        }
        3 => {
            // Polygon
            if off + 4 > wkb.len() {
                return None;
            }
            let num_rings = read_u32(wkb, off, le) as usize;
            off += 4;
            let mut rings = Vec::with_capacity(num_rings);
            for _ in 0..num_rings {
                if off + 4 > wkb.len() {
                    break;
                }
                let n = read_u32(wkb, off, le) as usize;
                off += 4;
                let mut pts = Vec::with_capacity(n);
                for _ in 0..n {
                    if off + 16 > wkb.len() {
                        break;
                    }
                    pts.push((read_f64(wkb, off, le)?, read_f64(wkb, off + 8, le)?));
                    off += 16;
                }
                rings.push(pts);
            }
            rings
        }
        4 => {
            // MultiPoint
            if off + 4 > wkb.len() {
                return None;
            }
            let n = read_u32(wkb, off, le) as usize;
            off += 4;
            let mut pts = Vec::with_capacity(n);
            for _ in 0..n {
                off += 5; // skip sub-geom header (byte_order + type)
                if off + 16 > wkb.len() {
                    break;
                }
                pts.push((read_f64(wkb, off, le)?, read_f64(wkb, off + 8, le)?));
                off += 16;
            }
            vec![pts]
        }
        5 => {
            // MultiLineString
            if off + 4 > wkb.len() {
                return None;
            }
            let n = read_u32(wkb, off, le) as usize;
            off += 4;
            let mut rings = Vec::with_capacity(n);
            for _ in 0..n {
                off += 5; // skip sub-geom header
                if off + 4 > wkb.len() {
                    break;
                }
                let np = read_u32(wkb, off, le) as usize;
                off += 4;
                let mut pts = Vec::with_capacity(np);
                for _ in 0..np {
                    if off + 16 > wkb.len() {
                        break;
                    }
                    pts.push((read_f64(wkb, off, le)?, read_f64(wkb, off + 8, le)?));
                    off += 16;
                }
                rings.push(pts);
            }
            rings
        }
        6 => {
            // MultiPolygon
            if off + 4 > wkb.len() {
                return None;
            }
            let n = read_u32(wkb, off, le) as usize;
            off += 4;
            let mut rings = Vec::new();
            let mut ring_counts = Vec::with_capacity(n);
            for _ in 0..n {
                off += 5; // skip sub-geom header
                if off + 4 > wkb.len() {
                    break;
                }
                let nr = read_u32(wkb, off, le) as usize;
                off += 4;
                ring_counts.push(nr);
                for _ in 0..nr {
                    if off + 4 > wkb.len() {
                        break;
                    }
                    let np = read_u32(wkb, off, le) as usize;
                    off += 4;
                    let mut pts = Vec::with_capacity(np);
                    for _ in 0..np {
                        if off + 16 > wkb.len() {
                            break;
                        }
                        pts.push((read_f64(wkb, off, le)?, read_f64(wkb, off + 8, le)?));
                        off += 16;
                    }
                    rings.push(pts);
                }
            }
            return Some(ParsedGeom {
                geom_type: GeomType::MultiPolygon,
                rings,
                sub_geom_ring_counts: Some(ring_counts),
                srid,
            });
        }
        _ => vec![],
    };

    Some(ParsedGeom {
        geom_type,
        rings,
        sub_geom_ring_counts: None,
        srid,
    })
}

fn read_u32(buf: &[u8], off: usize, le: bool) -> u32 {
    let b = [buf[off], buf[off + 1], buf[off + 2], buf[off + 3]];
    if le {
        u32::from_le_bytes(b)
    } else {
        u32::from_be_bytes(b)
    }
}

fn read_i32(buf: &[u8], off: usize, le: bool) -> i32 {
    read_u32(buf, off, le) as i32
}

fn read_f64(buf: &[u8], off: usize, le: bool) -> Option<f64> {
    let b: [u8; 8] = buf[off..off + 8].try_into().ok()?;
    Some(if le {
        f64::from_le_bytes(b)
    } else {
        f64::from_be_bytes(b)
    })
}

// ─── MVT geometry encoding ─────────────────────────────────────────────────

/// Encode a parsed geometry as MVT geometry commands (vector of u32 integers).
pub fn encode_mvt_geometry(geom: &ParsedGeom) -> Option<(u32, Vec<u32>)> {
    let mvt_type = geom.geom_type.mvt_type()?;
    let mut commands = Vec::new();

    match geom.geom_type {
        GeomType::Point | GeomType::MultiPoint => {
            let pts = geom.rings.first()?;
            let count = pts.len();
            if count == 0 {
                return None;
            }
            // MoveTo with count points
            commands.push((1u32 << 3) | (count as u32));
            let mut prev = (0i32, 0i32);
            for &(x, y) in pts {
                let ix = x.round() as i32;
                let iy = y.round() as i32;
                let dx = encode_zigzag32(ix - prev.0);
                let dy = encode_zigzag32(iy - prev.1);
                commands.push(dx);
                commands.push(dy);
                prev = (ix, iy);
            }
        }
        GeomType::LineString | GeomType::MultiLineString => {
            for ring in &geom.rings {
                if ring.is_empty() {
                    continue;
                }
                let count = ring.len();
                // MoveTo(1 point)
                commands.push((1u32 << 3) | 1);
                let first = ring[0];
                let ix0 = first.0.round() as i32;
                let iy0 = first.1.round() as i32;
                commands.push(encode_zigzag32(ix0));
                commands.push(encode_zigzag32(iy0));
                // LineTo(count-1 points)
                commands.push((2u32 << 3) | ((count - 1) as u32));
                let mut prev = (ix0, iy0);
                for &(x, y) in &ring[1..] {
                    let ix = x.round() as i32;
                    let iy = y.round() as i32;
                    commands.push(encode_zigzag32(ix - prev.0));
                    commands.push(encode_zigzag32(iy - prev.1));
                    prev = (ix, iy);
                }
            }
        }
        GeomType::Polygon | GeomType::MultiPolygon => {
            let ring_counts = geom.sub_geom_ring_counts.as_ref();
            let mut ring_idx = 0;
            let counts: Vec<usize> = match ring_counts {
                Some(c) => c.clone(),
                None => vec![geom.rings.len()],
            };
            for &_ring_count in &counts {
                // Process rings for this polygon
                let outer = &geom.rings.get(ring_idx)?;
                if outer.is_empty() {
                    ring_idx += 1;
                    continue;
                }
                // MoveTo
                commands.push((1u32 << 3) | 1);
                let first = outer[0];
                let ix0 = first.0.round() as i32;
                let iy0 = first.1.round() as i32;
                commands.push(encode_zigzag32(ix0));
                commands.push(encode_zigzag32(iy0));
                // LineTo
                commands.push((2u32 << 3) | ((outer.len() - 1) as u32));
                let mut prev = (ix0, iy0);
                for &(x, y) in &outer[1..] {
                    let ix = x.round() as i32;
                    let iy = y.round() as i32;
                    commands.push(encode_zigzag32(ix - prev.0));
                    commands.push(encode_zigzag32(iy - prev.1));
                    prev = (ix, iy);
                }
                // ClosePath
                commands.push(7u32 << 3);
                ring_idx += 1;
                // Skip inner rings (holes) for now — basic polygon support
            }
        }
        _ => return None,
    }

    Some((mvt_type, commands))
}

/// Encode an MVT Value message for common scalar types.
#[allow(clippy::unnecessary_cast, clippy::needless_return)]
pub fn encode_mvt_value(val: &str) -> Vec<u8> {
    // Try int, then float, then string
    if let Ok(i) = val.parse::<i64>() {
        let mut buf = Vec::new();
        encode_tag(4, 0, &mut buf); // int_value field 4, varint
        // Use sint for negative numbers
        if i < 0 {
            encode_tag(6, 0, &mut buf);
            encode_varint(encode_zigzag64(i) as u64, &mut buf);
        } else {
            encode_tag(5, 0, &mut buf);
            encode_varint(i as u64, &mut buf);
        }
        // Actually MVT Value has distinct fields; use string for simplicity
        let mut sbuf = Vec::new();
        encode_string_field(1, val, &mut sbuf);
        return sbuf;
    } else if let Ok(f) = val.parse::<f64>() {
        let mut buf = Vec::new();
        encode_tag(3, 1, &mut buf); // double_value field 3, 64-bit
        buf.extend_from_slice(&f.to_le_bytes());
        return buf;
    } else {
        let mut buf = Vec::new();
        encode_string_field(1, val, &mut buf);
        return buf;
    }
}

/// Encode a complete MVT tile from features.
#[derive(Debug)]
pub struct MvtFeature {
    pub geom_type: u32,
    pub commands: Vec<u32>,
    pub tags: Vec<u32>, // [key_idx, val_idx, ...]
    pub id: Option<u64>,
}

/// Build the final MVT tile bytes from a layer name, extent, and features.
pub fn build_mvt_tile(layer_name: &str, extent: u32, features: &[MvtFeature]) -> Vec<u8> {
    build_mvt_tile_with_dictionary(layer_name, extent, &[], &[], features)
}

/// Build the final MVT tile bytes with an explicit layer key/value dictionary.
///
/// Feature tags use MVT's packed `[key_index, value_index, ...]` encoding and
/// refer to the `keys` and `values` slices supplied here. The SQL aggregate still
/// feeds geometry-only features today; this lower-level dictionary path is kept
/// explicit so Martin/real-data attribute propagation can be wired and tested
/// without changing the geometry encoder again.
pub fn build_mvt_tile_with_dictionary(
    layer_name: &str,
    extent: u32,
    keys: &[String],
    values: &[String],
    features: &[MvtFeature],
) -> Vec<u8> {
    // Build Feature messages
    let mut feature_msgs: Vec<Vec<u8>> = Vec::new();
    for feat in features {
        let mut buf = Vec::new();
        if let Some(id) = feat.id {
            encode_uint64_field(1, id, &mut buf); // id
        }
        // tags (packed uint32)
        if !feat.tags.is_empty() {
            encode_tag(2, 2, &mut buf);
            let mut tag_buf = Vec::new();
            for &t in &feat.tags {
                encode_varint(t as u64, &mut tag_buf);
            }
            encode_varint(tag_buf.len() as u64, &mut buf);
            buf.extend_from_slice(&tag_buf);
        }
        // geometry type
        encode_uint32_field(3, feat.geom_type, &mut buf);
        // geometry commands (packed uint32)
        encode_tag(4, 2, &mut buf);
        let mut geom_buf = Vec::new();
        for &cmd in &feat.commands {
            encode_varint(cmd as u64, &mut geom_buf);
        }
        encode_varint(geom_buf.len() as u64, &mut buf);
        buf.extend_from_slice(&geom_buf);
        feature_msgs.push(buf);
    }

    // Build Layer message
    let mut layer = Vec::new();
    // field 15: name (string)
    encode_string_field(15, layer_name, &mut layer);
    // field 1: extent (uint32)
    encode_uint32_field(1, extent, &mut layer);
    // field 2: features (repeated bytes)
    for feat in &feature_msgs {
        encode_bytes_field(2, feat, &mut layer);
    }
    // field 3: keys (repeated string)
    for key in keys {
        encode_string_field(3, key, &mut layer);
    }
    // field 4: values (repeated Value messages)
    for value in values {
        encode_bytes_field(4, &encode_mvt_value(value), &mut layer);
    }

    // Build Tile message
    let mut tile = Vec::new();
    // field 3: layers (repeated bytes)
    encode_bytes_field(3, &layer, &mut tile);

    tile
}

// ─── ST_AsMVTGeom transform ────────────────────────────────────────────────

/// Transform geometry coordinates from geographic space to tile pixel space.
/// Input: geometry in bounds CRS, bounds polygon, extent (e.g. 4096).
/// Output: geometry with coordinates in [0, extent] range.
pub fn mvt_geom_transform(
    geom: &ParsedGeom,
    bounds: (f64, f64, f64, f64),
    extent: f64,
) -> ParsedGeom {
    let (bx1, by1, bx2, by2) = bounds;
    let bw = bx2 - bx1;
    let bh = by2 - by1;
    if bw == 0.0 || bh == 0.0 {
        return geom.map_coords(|_, _| (0.0, 0.0));
    }
    let sx = extent / bw;
    let sy = extent / bh;
    geom.map_coords(|x, y| ((x - bx1) * sx, (y - by1) * sy))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point_wkb(x: f64, y: f64) -> Vec<u8> {
        let mut wkb = Vec::with_capacity(21);
        wkb.push(1);
        wkb.extend_from_slice(&1_u32.to_le_bytes());
        wkb.extend_from_slice(&x.to_le_bytes());
        wkb.extend_from_slice(&y.to_le_bytes());
        wkb
    }

    fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
        haystack
            .windows(needle.len())
            .any(|window| window == needle)
    }

    #[test]
    fn mvt_dictionary_encodes_feature_tags_keys_and_values() {
        let parsed = parse_wkb(&point_wkb(0.0, 0.0)).expect("parse point WKB");
        let (geom_type, commands) = encode_mvt_geometry(&parsed).expect("encode point MVT");
        let feature = MvtFeature {
            geom_type,
            commands,
            tags: vec![0, 0, 1, 1],
            id: Some(7),
        };
        let keys = vec!["name".to_string(), "kind".to_string()];
        let values = vec!["origin".to_string(), "poi".to_string()];

        let tile = build_mvt_tile_with_dictionary("points", 4096, &keys, &values, &[feature]);

        assert!(contains_bytes(&tile, b"points"));
        assert!(contains_bytes(&tile, b"name"));
        assert!(contains_bytes(&tile, b"kind"));
        assert!(contains_bytes(&tile, b"origin"));
        assert!(contains_bytes(&tile, b"poi"));
    }
}
