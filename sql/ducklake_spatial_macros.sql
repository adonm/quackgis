-- SPDX-License-Identifier: Apache-2.0
-- Optional convenience macros for DuckLake spatial layouts.
--
-- Usage:
--   LOAD sedonadb;
--   LOAD ducklake;
--   .read sql/ducklake_spatial_macros.sql
--
-- These macros do not create extension state. They are thin SQL wrappers around
-- deterministic functions so they are safe for multi-writer DuckLake use.

-- Layout columns ------------------------------------------------------------

CREATE OR REPLACE MACRO sedona_layout_xmin(g) AS st_xmin(g);
CREATE OR REPLACE MACRO sedona_layout_ymin(g) AS st_ymin(g);
CREATE OR REPLACE MACRO sedona_layout_xmax(g) AS st_xmax(g);
CREATE OR REPLACE MACRO sedona_layout_ymax(g) AS st_ymax(g);

CREATE OR REPLACE MACRO sedona_layout_cell(g, zoom := 6) AS st_quadkey(g, zoom);
CREATE OR REPLACE MACRO sedona_layout_sort(g, bits := 12) AS st_hilbert(g, bits);

-- Query-side covering cells ------------------------------------------------

CREATE OR REPLACE MACRO sedona_covering_cells(g, zoom := 6, max_cells := 1000) AS TABLE
    SELECT quadkey, tile_x, tile_y
    FROM st_covering_quadkeys(g, zoom, max_cells);

CREATE OR REPLACE MACRO sedona_covering_cells_bbox(xmin, ymin, xmax, ymax, zoom := 6, max_cells := 1000) AS TABLE
    SELECT quadkey, tile_x, tile_y
    FROM st_covering_quadkeys(st_makeenvelope(xmin, ymin, xmax, ymax), zoom, max_cells);

-- Common bbox overlap rewrite for PostGIS `&&` ----------------------------

CREATE OR REPLACE MACRO sedona_bbox_overlaps(axmin, aymin, axmax, aymax, bxmin, bymin, bxmax, bymax) AS
    axmax >= bxmin AND axmin <= bxmax AND aymax >= bymin AND aymin <= bymax;
