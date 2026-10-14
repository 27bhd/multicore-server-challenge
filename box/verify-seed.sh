#!/usr/bin/env bash
# Fingerprint the seed database with sizes, as root on the benchmark box (seed/make-seed.sh checks the fingerprints too).
#
#   verify-seed.sh [dbname]     # default conduit_seed
#
# Per table: row count and an order-independent content hash: the sum over all rows of the first 60 bits of
# md5(row as text), as numeric. It streams through a plain aggregate (constant memory, fine on the 2 GB box).
# Row text includes every column (ids, timestamps rendered in UTC), so any difference in content or in the
# identity values assigned at load time changes the hash. Also prints sizes (heap, indexes, total) and sequences.
set -euo pipefail
DB="${1:-conduit_seed}"
Q() { runuser -u postgres -- psql -qtAX -F ' ' -d "$DB" -c "SET TimeZone = 'UTC'" -c "$1"; }
echo "# $(hostname) db=$DB $(runuser -u postgres -- psql -tAXc 'show server_version')"
printf '%-13s %10s %26s %8s %8s %8s\n' table rows fingerprint heap indexes total
for t in users follows articles tags article_tags favorites comments; do
  read -r n h <<<"$(Q "SELECT count(*), coalesce(sum(('x' || substr(md5(x::text), 1, 15))::bit(60)::bigint), 0) FROM $t x")"
  read -r heap idx tot <<<"$(Q "SELECT replace(pg_size_pretty(pg_table_size('$t')), ' ', ''), replace(pg_size_pretty(pg_indexes_size('$t')), ' ', ''), replace(pg_size_pretty(pg_total_relation_size('$t')), ' ', '')")"
  printf '%-13s %10s %26s %8s %8s %8s\n' "$t" "$n" "$h" "$heap" "$idx" "$tot"
done
echo "sequences: $(Q "SELECT string_agg(sequencename || '=' || last_value, ' ' ORDER BY sequencename) FROM pg_sequences")"
echo "database size: $(Q "SELECT pg_size_pretty(pg_database_size('$DB'))")"
echo "owner of tables: $(Q "SELECT string_agg(DISTINCT tableowner, ',') FROM pg_tables WHERE schemaname = 'public'")"
