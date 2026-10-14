#!/usr/bin/env bash
# Runs the release binary in the foreground. Config: DATABASE_URL, JWT_SECRET, HOST, PORT.
# One sqlx PgPool of 10 connections per core, as in the video.
set -euo pipefail
export DB_POOL_SIZE="${DB_POOL_SIZE:-$(( 10 * $(nproc 2>/dev/null || sysctl -n hw.ncpu) ))}"
exec "$(cd "$(dirname "$0")" && pwd)/target/release/conduit"
