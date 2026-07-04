# SPDX-License-Identifier: Apache-2.0
#
# PostgreSQL geometry type extension for QuackGIS.
# Plain Makefile (no PGXS) to avoid visibility issues.

PG_CONFIG ?= pg_config
INCLUDEDIR_SERVER = $(shell $(PG_CONFIG) --includedir-server)
PKGLIBDIR = $(shell $(PG_CONFIG) --pkglibdir)
SHAREDIR = $(shell $(PG_CONFIG) --sharedir)

EXTENSION = quackgis_type
MODULE = quackgis_type

CFLAGS += -fPIC -Wall -shared
CFLAGS += -I$(INCLUDEDIR_SERVER)

all: $(MODULE).so

$(MODULE).so: $(MODULE).c
	gcc $(CFLAGS) -o $@ $< -lgeos_c

install: $(MODULE).so
	mkdir -p $(DESTDIR)$(PKGLIBDIR)
	mkdir -p $(DESTDIR)$(SHAREDIR)/extension
	cp $(MODULE).so $(DESTDIR)$(PKGLIBDIR)/
	cp $(EXTENSION).control $(EXTENSION)--1.0.sql $(DESTDIR)$(SHAREDIR)/extension/

clean:
	rm -f $(MODULE).so

.PHONY: all install clean
