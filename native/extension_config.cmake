# SPDX-License-Identifier: Apache-2.0
# Loaded by the central DuckDB build after the preparer sets exact source paths.

if(NOT DEFINED QUACKGIS_DUCKLAKE_SOURCE OR NOT DEFINED QUACKGIS_SPATIAL_SOURCE)
    message(FATAL_ERROR "prepared DuckLake and Spatial source paths are required")
endif()

duckdb_extension_load(ducklake
    SOURCE_DIR ${QUACKGIS_DUCKLAKE_SOURCE}
    DONT_LINK
    LOAD_TESTS
)
duckdb_extension_load(spatial
    SOURCE_DIR ${QUACKGIS_SPATIAL_SOURCE}
    INCLUDE_DIR ${QUACKGIS_SPATIAL_SOURCE}/src/spatial
    DONT_LINK
    LOAD_TESTS
)
