#!/usr/bin/env bash
# Runs the server in the foreground. Config: DATABASE_URL, JWT_SECRET, HOST, PORT.
# HikariCP pool of 10 connections per core, as in the video. Default JVM flags: the JVM sizes heap, GC and
# GC threads from the box.
set -euo pipefail
cd "$(dirname "$0")"
export DB_POOL_SIZE="${DB_POOL_SIZE:-$(( 10 * $(nproc 2>/dev/null || sysctl -n hw.ncpu) ))}"
if [ -x /opt/jdk/bin/java ]; then JAVA=/opt/jdk/bin/java; else JAVA="$JAVA_HOME/bin/java"; fi   # install.sh's JDK 27 first
exec "$JAVA" -jar target/conduit.jar
