#!/usr/bin/env bash
# Runs Uvicorn in the foreground with its built-in multiprocess supervisor (FastAPI's deployment docs).
# Config: DATABASE_URL, JWT_SECRET, HOST, PORT. One worker per core (beat 2 x cores + 1 in the video) and
# 10 Postgres connections per core in total, split across the workers' asyncpg pools.
set -euo pipefail
cd "$(dirname "$0")"
WORKERS="${WORKERS:-$(nproc 2>/dev/null || sysctl -n hw.ncpu)}"
export DB_POOL_SIZE="${DB_POOL_SIZE:-$(( (10 * $(nproc 2>/dev/null || sysctl -n hw.ncpu) + WORKERS - 1) / WORKERS ))}"
exec .venv/bin/uvicorn app.main:app \
  --host "${HOST:-127.0.0.1}" \
  --port "${PORT:-3000}" \
  --workers "$WORKERS" \
  --timeout-keep-alive 65 \
  --loop uvloop \
  --http httptools \
  --no-access-log
