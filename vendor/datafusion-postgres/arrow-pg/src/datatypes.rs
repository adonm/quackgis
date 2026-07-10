use std::sync::Arc;

#[cfg(not(feature = "datafusion"))]
use arrow::{datatypes::*, record_batch::RecordBatch};
#[cfg(feature = "postgis")]
use arrow_schema::extension::ExtensionType;
#[cfg(feature = "datafusion")]
use datafusion::arrow::{datatypes::*, record_batch::RecordBatch};

use pgwire::api::Type;
use pgwire::api::portal::Format;
use pgwire::api::results::FieldInfo;
use pgwire::error::{ErrorInfo, PgWireError, PgWireResult};
use pgwire::messages::data::DataRow;
use pgwire::types::format::FormatOptions;
use postgres_types::Kind;

use crate::row_encoder::RowEncoder;

const OID_ALIAS_METADATA_KEY: &str = "pg.oid_alias";
const OID_COLUMN_NAMES: &[&str] = &[
    "oid",
    "typelem",
    "rngsubtype",
    "typbasetype",
    "typrelid",
    "typnamespace",
    "enumtypid",
    "attrelid",
    "atttypid",
    "relnamespace",
    "reltype",
    "reloftype",
    "reltoastrelid",
    "relrewrite",
];

/// PostgreSQL type OID advertised for PostGIS-style `geometry` columns.
///
/// PostGIS assigns the `geometry`/`geography` type OIDs dynamically when the
/// extension is created, so there is no universal constant. QuackGIS stores
/// geometry as WKB inside Arrow `Binary` columns and advertises a fixed,
/// collision-free sentinel OID on the wire so that clients (QGIS, GeoServer)
/// which key geometry handling off the RowDescription type OID see a distinct
/// type from `bytea`. The value is intentionally far outside the builtin
/// PostgreSQL type-OID range and the runtime `oid_counter` range used by
/// `datafusion-pg-catalog` for table OIDs.
pub const GEOMETRY_OID: u32 = 90_001;
pub const GEOGRAPHY_OID: u32 = 90_002;

/// Arrow field metadata key carrying QuackGIS' durable spatial family.
///
/// The physical Arrow type remains binary WKB/EWKB. This metadata records only
/// the SQL family (`geometry` or `geography`), not subtype, SRID, or dimensions.
pub const SPATIAL_FAMILY_METADATA_KEY: &str = "quackgis.spatial_family";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpatialFamily {
    Geometry,
    Geography,
}

impl SpatialFamily {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Geometry => "geometry",
            Self::Geography => "geography",
        }
    }

    fn from_metadata(value: &str) -> Option<Self> {
        match value {
            "geometry" => Some(Self::Geometry),
            "geography" => Some(Self::Geography),
            _ => None,
        }
    }
}

/// Column names treated as WKB-encoded PostGIS geometry/geography by
/// convention when the Arrow type is binary. Mirrors the names used by QGIS,
/// GDAL, and typical `CREATE TABLE` statements.
const GEOMETRY_COLUMN_NAMES: &[&str] = &[
    "geom",
    "geometry",
    "the_geom",
    "wkb_geometry",
    "wkb_geom",
    "shape",
    "footprint",
    "way", // OpenStreetMap convention
];

const GEOGRAPHY_COLUMN_NAMES: &[&str] = &["geog", "geography", "the_geog"];

/// Returns true if `name` matches the geometry-column naming convention.
pub fn is_geometry_column_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    GEOMETRY_COLUMN_NAMES.contains(&lower.as_str())
}

/// Returns true if `name` matches the geography-column naming convention.
pub fn is_geography_column_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    GEOGRAPHY_COLUMN_NAMES.contains(&lower.as_str())
}

/// Return a validated explicit family annotation.
///
/// Metadata on non-binary fields and unknown metadata values is ignored.
pub fn explicit_spatial_family(field: &Field) -> Option<SpatialFamily> {
    if !is_binary_arrow_type(field.data_type()) {
        return None;
    }
    field
        .metadata()
        .get(SPATIAL_FAMILY_METADATA_KEY)
        .and_then(|value| SpatialFamily::from_metadata(value))
}

/// Classify a spatial field, preferring validated metadata over legacy names.
/// Only Binary, LargeBinary, and BinaryView physical fields qualify.
pub fn classify_spatial_field(field: &Field) -> Option<SpatialFamily> {
    if !is_binary_arrow_type(field.data_type()) {
        return None;
    }
    explicit_spatial_family(field).or_else(|| {
        if is_geometry_column_name(field.name()) {
            Some(SpatialFamily::Geometry)
        } else if is_geography_column_name(field.name()) {
            Some(SpatialFamily::Geography)
        } else {
            None
        }
    })
}

/// Set or clear the recognized spatial-family key while preserving other field
/// metadata. A family is only written for a binary physical field.
pub fn with_spatial_family_metadata(field: Field, family: Option<SpatialFamily>) -> Field {
    let mut metadata = field.metadata().clone();
    metadata.remove(SPATIAL_FAMILY_METADATA_KEY);
    if is_binary_arrow_type(field.data_type())
        && let Some(family) = family
    {
        metadata.insert(
            SPATIAL_FAMILY_METADATA_KEY.to_string(),
            family.as_str().to_string(),
        );
    }
    field.with_metadata(metadata)
}

fn is_oid_column_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    OID_COLUMN_NAMES.contains(&lower.as_str())
}

pub fn is_binary_arrow_type(dt: &DataType) -> bool {
    matches!(
        dt,
        DataType::Binary | DataType::LargeBinary | DataType::BinaryView
    )
}

/// Construct the custom PostGIS-compatible `geometry` type used in
/// RowDescription/FieldInfo. `Kind::Simple` matches PostGIS' base type so
/// libpq treats the column as a binary-coercible scalar (bytes on the wire).
pub fn geometry_pg_type() -> Type {
    Type::new(
        "geometry".to_string(),
        GEOMETRY_OID,
        Kind::Simple,
        String::new(),
    )
}

/// Construct the custom PostGIS-compatible `geography` type.
pub fn geography_pg_type() -> Type {
    Type::new(
        "geography".to_string(),
        GEOGRAPHY_OID,
        Kind::Simple,
        String::new(),
    )
}

#[cfg(feature = "datafusion")]
pub mod df;

pub fn into_pg_type(arrow_type: &DataType) -> PgWireResult<Type> {
    let datatype = match arrow_type {
        DataType::Null => Type::UNKNOWN,
        DataType::Boolean => Type::BOOL,
        // PostgreSQL has no SQL-standard one-byte integer type. Its internal
        // catalog "char" type (OID 18) is represented by tokio-postgres as
        // i8 and is used by pg_catalog columns such as pg_class.relkind.
        DataType::Int8 => Type::CHAR,
        DataType::Int16 | DataType::UInt8 => Type::INT2,
        DataType::Int32 | DataType::UInt16 => Type::INT4,
        DataType::Int64 | DataType::UInt32 => Type::INT8,
        DataType::UInt64 => Type::NUMERIC,
        DataType::Timestamp(_, tz) => {
            if tz.is_some() {
                Type::TIMESTAMPTZ
            } else {
                Type::TIMESTAMP
            }
        }
        DataType::Time32(_) | DataType::Time64(_) => Type::TIME,
        DataType::Date32 | DataType::Date64 => Type::DATE,
        DataType::Interval(_) | DataType::Duration(_) => Type::INTERVAL,
        DataType::Binary
        | DataType::FixedSizeBinary(_)
        | DataType::LargeBinary
        | DataType::BinaryView => Type::BYTEA,
        DataType::Float16 | DataType::Float32 => Type::FLOAT4,
        DataType::Float64 => Type::FLOAT8,
        DataType::Decimal128(_, _) => Type::NUMERIC,
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View => Type::TEXT,
        DataType::List(field)
        | DataType::FixedSizeList(field, _)
        | DataType::LargeList(field)
        | DataType::ListView(field)
        | DataType::LargeListView(field) => match field.data_type() {
            DataType::Boolean => Type::BOOL_ARRAY,
            DataType::Int8 => Type::INT2_ARRAY,
            DataType::Int16 | DataType::UInt8 => Type::INT2_ARRAY,
            DataType::Int32 | DataType::UInt16 => Type::INT4_ARRAY,
            DataType::Int64 | DataType::UInt32 => Type::INT8_ARRAY,
            DataType::UInt64 | DataType::Decimal128(_, _) => Type::NUMERIC_ARRAY,
            DataType::Timestamp(_, tz) => {
                if tz.is_some() {
                    Type::TIMESTAMPTZ_ARRAY
                } else {
                    Type::TIMESTAMP_ARRAY
                }
            }
            DataType::Time32(_) | DataType::Time64(_) => Type::TIME_ARRAY,
            DataType::Date32 | DataType::Date64 => Type::DATE_ARRAY,
            DataType::Interval(_) | DataType::Duration(_) => Type::INTERVAL_ARRAY,
            DataType::FixedSizeBinary(_)
            | DataType::Binary
            | DataType::LargeBinary
            | DataType::BinaryView => Type::BYTEA_ARRAY,
            DataType::Float16 | DataType::Float32 => Type::FLOAT4_ARRAY,
            DataType::Float64 => Type::FLOAT8_ARRAY,
            DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View => Type::TEXT_ARRAY,
            DataType::Struct(_) => Type::new(
                Type::RECORD_ARRAY.name().into(),
                Type::RECORD_ARRAY.oid(),
                Kind::Array(field_into_pg_type(field)?),
                Type::RECORD_ARRAY.schema().into(),
            ),
            list_type => {
                return Err(PgWireError::UserError(Box::new(ErrorInfo::new(
                    "ERROR".to_owned(),
                    "XX000".to_owned(),
                    format!("Unsupported List Datatype {list_type}"),
                ))));
            }
        },
        DataType::Dictionary(_, value_type) => into_pg_type(value_type.as_ref())?,
        DataType::Struct(fields) => {
            let name: String = fields
                .iter()
                .map(|x| x.name().clone())
                .reduce(|a, b| a + ", " + &b)
                .map(|x| format!("({x})"))
                .unwrap_or("()".to_string());
            let kind = Kind::Composite(
                fields
                    .iter()
                    .map(|x| {
                        field_into_pg_type(x)
                            .map(|_type| postgres_types::Field::new(x.name().clone(), _type))
                    })
                    .collect::<Result<Vec<_>, PgWireError>>()?,
            );
            Type::new(name, Type::RECORD.oid(), kind, Type::RECORD.schema().into())
        }
        _ => {
            return Err(PgWireError::UserError(Box::new(ErrorInfo::new(
                "ERROR".to_owned(),
                "XX000".to_owned(),
                format!("Unsupported Datatype {arrow_type}"),
            ))));
        }
    };

    Ok(datatype)
}

pub fn field_into_pg_type(field: &Arc<Field>) -> PgWireResult<Type> {
    let arrow_type = field.data_type();

    // pg_catalog stores oid/oid-alias values as Arrow Int32 columns, annotated
    // with metadata by datafusion-pg-catalog. PostgreSQL advertises those
    // columns as `oid` on the wire, not plain int4; tokio-postgres relies on
    // that when resolving unknown extension/custom type OIDs via pg_type.
    if matches!(arrow_type, DataType::Int32 | DataType::UInt32)
        && (field.metadata().contains_key(OID_ALIAS_METADATA_KEY)
            || is_oid_column_name(field.name()))
    {
        return Ok(Type::OID);
    }

    if matches!(
        arrow_type,
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View
    ) && field.name().eq_ignore_ascii_case("typtype")
    {
        return Ok(Type::CHAR);
    }

    // PostGIS-compat: explicitly annotated binary fields, followed by legacy
    // conventional names, are advertised with a dedicated type OID (not bytea)
    // so clients like QGIS/GeoServer recognise them as spatial columns. The
    // wire encoding is unchanged: arrow-pg still writes the raw WKB bytes
    // (binary format) or hex-EWKB (text format), both of which are
    // wire-compatible with PostGIS geometry transport.
    match classify_spatial_field(field) {
        Some(SpatialFamily::Geometry) => return Ok(geometry_pg_type()),
        Some(SpatialFamily::Geography) => return Ok(geography_pg_type()),
        None => {}
    }

    match field.extension_type_name() {
        // As of arrow 56, there are additional extension logical type that is
        // defined using field metadata, for instance, json or geo.
        //
        // TODO: there is no fixed Geometry/Geography type id, here we use text
        // for placeholder.
        #[cfg(feature = "postgis")]
        Some(geoarrow_schema::PointType::NAME) => Ok(Type::TEXT),
        #[cfg(feature = "postgis")]
        Some(geoarrow_schema::LineStringType::NAME) => Ok(Type::TEXT),
        #[cfg(feature = "postgis")]
        Some(geoarrow_schema::PolygonType::NAME) => Ok(Type::TEXT),
        #[cfg(feature = "postgis")]
        Some(geoarrow_schema::MultiPointType::NAME) => Ok(Type::TEXT),
        #[cfg(feature = "postgis")]
        Some(geoarrow_schema::MultiLineStringType::NAME) => Ok(Type::TEXT),
        #[cfg(feature = "postgis")]
        Some(geoarrow_schema::MultiPolygonType::NAME) => Ok(Type::TEXT),
        #[cfg(feature = "postgis")]
        Some(geoarrow_schema::GeometryCollectionType::NAME) => Ok(Type::TEXT),
        #[cfg(feature = "postgis")]
        Some(geoarrow_schema::GeometryType::NAME) => Ok(Type::TEXT),
        #[cfg(feature = "postgis")]
        Some(geoarrow_schema::RectType::NAME) => Ok(Type::TEXT),
        #[cfg(feature = "postgis")]
        Some(geoarrow_schema::WktType::NAME) => Ok(Type::TEXT),
        #[cfg(feature = "postgis")]
        Some(geoarrow_schema::WkbType::NAME) => Ok(Type::TEXT),

        _ if field.name() == "properties"
            && matches!(
                arrow_type,
                DataType::Utf8 | DataType::Utf8View | DataType::LargeUtf8
            ) =>
        {
            Ok(Type::JSONB)
        }
        _ => into_pg_type(arrow_type),
    }
}

pub fn arrow_schema_to_pg_fields(
    schema: &Schema,
    format: &Format,
    data_format_options: Option<Arc<FormatOptions>>,
) -> PgWireResult<Vec<FieldInfo>> {
    let _ = data_format_options;
    schema
        .fields()
        .iter()
        .enumerate()
        .map(|(idx, f)| {
            let pg_type = field_into_pg_type(f)?;
            let mut field_info =
                FieldInfo::new(f.name().into(), None, None, pg_type, format.format_for(idx));
            if let Some(data_format_options) = &data_format_options {
                field_info = field_info.with_format_options(data_format_options.clone());
            }

            Ok(field_info)
        })
        .collect::<PgWireResult<Vec<FieldInfo>>>()
}

pub fn encode_recordbatch(
    fields: Arc<Vec<FieldInfo>>,
    record_batch: RecordBatch,
) -> Box<impl Iterator<Item = PgWireResult<DataRow>>> {
    let mut row_stream = RowEncoder::new(record_batch, fields);
    Box::new(std::iter::from_fn(move || row_stream.next_row()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geometry_named_binary_advertises_geometry_oid() {
        let field = Arc::new(Field::new("geom", DataType::Binary, true));
        let ty = field_into_pg_type(&field).expect("geometry field type");
        assert_eq!(ty.oid(), GEOMETRY_OID);
        assert_eq!(ty.name(), "geometry");
    }

    #[test]
    fn geography_named_binary_advertises_geography_oid() {
        let field = Arc::new(Field::new("the_geog", DataType::BinaryView, true));
        let ty = field_into_pg_type(&field).expect("geography field type");
        assert_eq!(ty.oid(), GEOGRAPHY_OID);
        assert_eq!(ty.name(), "geography");
    }

    #[test]
    fn non_geometry_binary_stays_bytea() {
        let field = Arc::new(Field::new("payload", DataType::Binary, true));
        let ty = field_into_pg_type(&field).expect("payload field type");
        assert_eq!(ty.oid(), Type::BYTEA.oid());
    }

    #[test]
    fn explicit_family_wins_over_conventional_name() {
        let field = with_spatial_family_metadata(
            Field::new("geom", DataType::Binary, true),
            Some(SpatialFamily::Geography),
        );
        assert_eq!(
            classify_spatial_field(&field),
            Some(SpatialFamily::Geography)
        );
        let ty = field_into_pg_type(&Arc::new(field)).expect("explicit geography field type");
        assert_eq!(ty.oid(), GEOGRAPHY_OID);
    }

    #[test]
    fn unconventional_explicit_geometry_and_footprint_fallback_classify() {
        let location = with_spatial_family_metadata(
            Field::new("location", DataType::LargeBinary, true),
            Some(SpatialFamily::Geometry),
        );
        assert_eq!(
            classify_spatial_field(&location),
            Some(SpatialFamily::Geometry)
        );
        assert_eq!(
            classify_spatial_field(&Field::new("footprint", DataType::BinaryView, true)),
            Some(SpatialFamily::Geometry)
        );
    }

    #[test]
    fn non_binary_geom_and_invalid_metadata_are_not_explicit_spatial_fields() {
        let text_geom = Field::new("geom", DataType::Utf8, true);
        assert_eq!(classify_spatial_field(&text_geom), None);

        let invalid = Field::new("payload", DataType::Binary, true).with_metadata(
            [(SPATIAL_FAMILY_METADATA_KEY.to_string(), "point".to_string())]
                .into_iter()
                .collect(),
        );
        assert_eq!(explicit_spatial_family(&invalid), None);
        assert_eq!(classify_spatial_field(&invalid), None);
    }

    #[test]
    fn properties_string_variants_advertise_jsonb() {
        for data_type in [DataType::Utf8, DataType::Utf8View, DataType::LargeUtf8] {
            let field = Arc::new(Field::new("properties", data_type, true));
            let ty = field_into_pg_type(&field).expect("properties field type");
            assert_eq!(ty, Type::JSONB);
        }
    }
}
