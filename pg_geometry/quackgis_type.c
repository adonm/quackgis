// SPDX-License-Identifier: Apache-2.0
//
// PostgreSQL geometry type for QuackGIS.
// Stores WKB bytes; WKT I/O via GEOS.

#include "postgres.h"

#include "fmgr.h"
#include "libpq/pqformat.h"
#include "utils/array.h"
#include "catalog/pg_type_d.h"  /* INT4OID */
#include "utils/builtins.h"

#include <geos_c.h>

PG_MODULE_MAGIC;

/* ── GEOS lifecycle ──────────────────────────────────────────────────── */

static GEOSContextHandle_t
get_geos_ctx(void) {
    static GEOSContextHandle_t ctx = NULL;
    if (!ctx) {
        ctx = GEOS_init_r();
    }
    return ctx;
}

static void
geos_notice(const char *msg, void *userdata) { }

static void
geos_error(const char *msg, void *userdata) { }

/* ── geometry_in: WKT text → WKB bytes ───────────────────────────────── */

PG_FUNCTION_INFO_V1(geometry_in);
Datum
geometry_in(PG_FUNCTION_ARGS) {
    char *input = PG_GETARG_CSTRING(0);
    GEOSContextHandle_t ctx = get_geos_ctx();

    GEOSWKTReader *reader = GEOSWKTReader_create_r(ctx);
    if (!reader)
        ereport(ERROR, (errmsg("geometry_in: could not create WKT reader")));

    GEOSGeometry *geom = GEOSWKTReader_read_r(ctx, reader, input);
    GEOSWKTReader_destroy_r(ctx, reader);

    if (!geom)
        ereport(ERROR,
                (errcode(ERRCODE_INVALID_TEXT_REPRESENTATION),
                 errmsg("could not parse geometry from WKT: %s", input)));

    GEOSWKBWriter *writer = GEOSWKBWriter_create_r(ctx);
    if (!writer) {
        GEOSGeom_destroy_r(ctx, geom);
        ereport(ERROR, (errmsg("geometry_in: could not create WKB writer")));
    }

    size_t wkb_len;
    unsigned char *wkb = GEOSWKBWriter_write_r(ctx, writer, geom, &wkb_len);
    GEOSWKBWriter_destroy_r(ctx, writer);
    GEOSGeom_destroy_r(ctx, geom);

    if (!wkb || wkb_len == 0)
        ereport(ERROR, (errmsg("geometry_in: WKB conversion failed")));

    bytea *result = palloc(wkb_len + VARHDRSZ);
    SET_VARSIZE(result, wkb_len + VARHDRSZ);
    memcpy(VARDATA(result), wkb, wkb_len);
    GEOSFree_r(ctx, wkb);

    PG_RETURN_BYTEA_P(result);
}

/* ── geometry_out: WKB bytes → WKT text ──────────────────────────────── */

PG_FUNCTION_INFO_V1(geometry_out);
Datum
geometry_out(PG_FUNCTION_ARGS) {
    bytea *wkb_data = PG_GETARG_BYTEA_P(0);
    size_t wkb_len = VARSIZE_ANY_EXHDR(wkb_data);

    if (wkb_len == 0)
        PG_RETURN_CSTRING(pstrdup(""));

    GEOSContextHandle_t ctx = get_geos_ctx();
    GEOSWKBReader *reader = GEOSWKBReader_create_r(ctx);
    GEOSGeometry *geom = GEOSWKBReader_read_r(ctx, reader,
        (unsigned char *)VARDATA_ANY(wkb_data), wkb_len);
    GEOSWKBReader_destroy_r(ctx, reader);

    if (!geom)
        PG_RETURN_CSTRING(pstrdup("GEOMETRYCOLLECTION EMPTY"));

    GEOSWKTWriter *writer = GEOSWKTWriter_create_r(ctx);
    GEOSWKTWriter_setTrim_r(ctx, writer, 1);
    GEOSWKTWriter_setOutputDimension_r(ctx, writer, 3);

    char *wkt = GEOSWKTWriter_write_r(ctx, writer, geom);
    GEOSWKTWriter_destroy_r(ctx, writer);
    GEOSGeom_destroy_r(ctx, geom);

    char *result = pstrdup(wkt);
    GEOSFree_r(ctx, wkt);

    PG_RETURN_CSTRING(result);
}

/* ── geometry_recv: accept raw WKB bytes ─────────────────────────────── */

PG_FUNCTION_INFO_V1(geometry_recv);
Datum
geometry_recv(PG_FUNCTION_ARGS) {
    StringInfo buf = (StringInfo) PG_GETARG_POINTER(0);
    bytea *result = palloc(buf->len + VARHDRSZ);
    SET_VARSIZE(result, buf->len + VARHDRSZ);
    memcpy(VARDATA(result), buf->data, buf->len);
    PG_RETURN_BYTEA_P(result);
}

/* ── geometry_send: send raw WKB bytes ───────────────────────────────── */

PG_FUNCTION_INFO_V1(geometry_send);
Datum
geometry_send(PG_FUNCTION_ARGS) {
    bytea *wkb_data = PG_GETARG_BYTEA_P(0);
    StringInfoData buf;
    pq_begintypsend(&buf);
    pq_copymsgbytes(&buf, VARDATA_ANY(wkb_data), VARSIZE_ANY_EXHDR(wkb_data));
    PG_RETURN_BYTEA_P(pq_endtypsend(&buf));
}

/* ── geometry_typmod_in ──────────────────────────────────────────────── */

PG_FUNCTION_INFO_V1(geometry_typmod_in);
Datum
geometry_typmod_in(PG_FUNCTION_ARGS) {
    ArrayType *arr = PG_GETARG_ARRAYTYPE_P(0);
    Datum *arr_data;
    int n;
    int32 typmod = 0;

    deconstruct_array(arr, INT4OID, 4, true, 'i', &arr_data, NULL, &n);

    if (n >= 1)
        typmod |= (DatumGetInt32(arr_data[0]) & 0xFF);
    if (n >= 2)
        typmod |= ((DatumGetInt32(arr_data[1]) & 0xFFFFFF) << 8);

    PG_RETURN_INT32(typmod);
}

/* ── geometry_typmod_out ─────────────────────────────────────────────── */

PG_FUNCTION_INFO_V1(geometry_typmod_out);
Datum
geometry_typmod_out(PG_FUNCTION_ARGS) {
    int32 typmod = PG_GETARG_INT32(0);
    char buf[64];

    static const char *type_names[] = {
        "GEOMETRY", "POINT", "LINESTRING", "POLYGON",
        "MULTIPOINT", "MULTILINESTRING", "MULTIPOLYGON",
        "GEOMETRYCOLLECTION"
    };

    int geom_type = typmod & 0xFF;
    int srid = (typmod >> 8) & 0xFFFFFF;

    if (geom_type == 0 && srid == 0)
        snprintf(buf, sizeof(buf), "");
    else if (srid == 0)
        snprintf(buf, sizeof(buf), "(%s)", type_names[geom_type % 8]);
    else if (geom_type == 0)
        snprintf(buf, sizeof(buf), "(,%d)", srid);
    else
        snprintf(buf, sizeof(buf), "(%s,%d)", type_names[geom_type % 8], srid);

    PG_RETURN_CSTRING(pstrdup(buf));
}

/* ── Module init ─────────────────────────────────────────────────────── */

/* ── geometry_to_text: WKB bytes → WKT text (for text cast) ──────────── */

PG_FUNCTION_INFO_V1(geometry_to_text);
Datum
geometry_to_text(PG_FUNCTION_ARGS) {
    bytea *wkb_data = PG_GETARG_BYTEA_P(0);
    size_t wkb_len = VARSIZE_ANY_EXHDR(wkb_data);

    if (wkb_len == 0) {
        PG_RETURN_TEXT_P(cstring_to_text(""));
    }

    GEOSContextHandle_t ctx = get_geos_ctx();
    GEOSWKBReader *reader = GEOSWKBReader_create_r(ctx);
    GEOSGeometry *geom = GEOSWKBReader_read_r(ctx, reader,
        (unsigned char *)VARDATA_ANY(wkb_data), wkb_len);
    GEOSWKBReader_destroy_r(ctx, reader);

    if (!geom) {
        PG_RETURN_TEXT_P(cstring_to_text("GEOMETRYCOLLECTION EMPTY"));
    }

    GEOSWKTWriter *writer = GEOSWKTWriter_create_r(ctx);
    GEOSWKTWriter_setTrim_r(ctx, writer, 1);
    GEOSWKTWriter_setOutputDimension_r(ctx, writer, 3);

    char *wkt = GEOSWKTWriter_write_r(ctx, writer, geom);
    GEOSWKTWriter_destroy_r(ctx, writer);
    GEOSGeom_destroy_r(ctx, geom);

    text *result = cstring_to_text(wkt);
    GEOSFree_r(ctx, wkt);

    PG_RETURN_TEXT_P(result);
}

/* Identity cast functions: geometry and bytea share the same varlena layout. */

PG_FUNCTION_INFO_V1(geometry_to_bytea);
Datum
geometry_to_bytea(PG_FUNCTION_ARGS) {
    PG_RETURN_BYTEA_P(PG_GETARG_BYTEA_P_COPY(0));
}

PG_FUNCTION_INFO_V1(bytea_to_geometry);
Datum
bytea_to_geometry(PG_FUNCTION_ARGS) {
    PG_RETURN_BYTEA_P(PG_GETARG_BYTEA_P_COPY(0));
}

/* ── Module init ─────────────────────────────────────────────────────── */

void
_PG_init(void) {
    GEOSContextHandle_t ctx = GEOS_init_r();
    GEOSContext_setNoticeMessageHandler_r(ctx, geos_notice, NULL);
    GEOSContext_setErrorMessageHandler_r(ctx, geos_error, NULL);
}
