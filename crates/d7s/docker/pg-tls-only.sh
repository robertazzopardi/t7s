#!/bin/sh
# Runs inside the postgres-tls container during initdb: reject non-TLS TCP clients.
sed -i 's/^host /hostssl /' "$PGDATA/pg_hba.conf"
