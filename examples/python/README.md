# Conduit API: Python (FastAPI + Uvicorn)

**Front: nginx.** This is the Python server from the video, unchanged except for `install.sh`, a `build.sh` that runs as a normal
user, and the pool/worker defaults in `start.sh`. It follows the video's rules: the language's most popular framework, the reference SQL verbatim, no
tuning. **Video score on 8 vCPU: 10,000 users** (DigitalOcean Premium AMD, so not directly comparable with the
challenge box). Copy it into `submissions/` as a starting point if you like.

The rest of this README is the video-era write-up: "the deploy" means the benchmark box, and "series rules" are
the video's rules, which are stricter than this challenge's.

Implements [`SPEC.md`](../../SPEC.md) with FastAPI on Uvicorn's built-in multiprocess supervisor,
asyncpg and CPython 3.15 (standard GIL build).

## Versions (exact pins in `requirements.txt`, generated from `requirements.in`)

| Piece | Version |
|---|---|
| CPython | 3.15.0 (uv-managed, `UV_PYTHON_INSTALL_DIR=/opt/uv/python` on the targets) |
| FastAPI | 0.143.0 (Starlette 1.7.0, Pydantic 2.14.1 / pydantic-core 2.50.1) |
| Uvicorn | 0.54.0 with `[standard]` extras: uvloop 0.23.0, httptools 0.9.0 |
| asyncpg | 0.32.0 |
| bcrypt (pyca) | 5.0.0, cost 10 |
| PyJWT | 2.15.1, HS256 |

All compiled dependencies ship cp315 wheels (asyncpg, uvloop, httptools, pydantic-core, websockets) or
abi3 wheels (bcrypt, watchfiles). The one exception is **PyYAML 6.0.3** (pulled in by `uvicorn[standard]`, unused
here): it has no cp315 wheel yet, so uv builds it from the sdist during `build.sh` (pure-Python build, no
libyaml; this works cleanly). Nothing forced a fallback to 3.14.

Regenerate pins with
`uv pip compile requirements.in --universal --python-version 3.15 --no-header -o requirements.txt`.

## How it uses multiple cores

Processes. `uvicorn --workers $WORKERS` starts one supervisor and `WORKERS` worker processes that share
the listening socket. Each worker runs its own uvloop event loop and its own asyncpg pool. The deploy sets
`WORKERS` and `DB_POOL_SIZE` (total connections ≈ `WORKERS × DB_POOL_SIZE`). The worker count (cores vs
2×cores+1) is decided by a measurement, so `start.sh` hard-codes neither; it defaults to `nproc` only when
`WORKERS` is unset. bcrypt runs via `starlette.concurrency.run_in_threadpool` (AnyIO's worker threads;
pyca/bcrypt releases the GIL while hashing), so a hash doesn't stall the worker's event loop.

## Build and run

```bash
./build.sh   # creates .venv with CPython 3.15 (from install.sh, else uv on PATH), `uv pip sync` the pins
DATABASE_URL=... JWT_SECRET=... DB_POOL_SIZE=4 WORKERS=4 ./start.sh
```

`start.sh` runs:

```bash
exec .venv/bin/uvicorn app.main:app --host "${HOST:-127.0.0.1}" --port "${PORT:-3000}" \
  --workers "${WORKERS:-$(nproc)}" --timeout-keep-alive 65 --loop uvloop --http httptools --no-access-log
```

Local dev: `PORT=3101`, database `conduit_py` (`dropdb --if-exists conduit_py; createdb -T conduit_dev conduit_py`).

## Layout

- `app/main.py`: app, lifespan (pool), auth, request models, handlers
- `app/sql.py`: every statement, verbatim from SPEC.md (`SLUGIFY` written out)

## Decisions and deviations from framework defaults

- **No Gunicorn**: Uvicorn's own `--workers` supervisor, as FastAPI's current deployment docs recommend.
- **`--timeout-keep-alive 65`** (default 5 s): series rule 7. **`--no-access-log`**: series rule 6.
  `--loop uvloop --http httptools` just pin what `auto` would pick with `uvicorn[standard]` installed.
- **asyncpg pool `min_size = max_size = DB_POOL_SIZE`**: asyncpg's default `min_size` (10) is above the pool
  sizes the deploy uses, so it's clamped to `max_size`; connections are opened at boot as with asyncpg's
  defaults. `command_timeout=30`.
- **Error shape**: handlers for `RequestValidationError` (→ 422), Starlette `HTTPException` (router 404/405) and
  the app's own `ApiError` all return `{"errors": {"body": [...]}}`. `/docs`, `/redoc` and `/openapi.json` are off.
- **Pydantic models** for request bodies (non-empty strings via `min_length=1`; on PUT, `null`/absent means
  unchanged but an empty `email`/`username`/`password`/`title`/`description`/`body` is 422, as in the Go reference).
- **`limit`/`offset`** are parsed by a small dependency rather than `Query(int)`, so the accepted syntax matches
  the Go reference exactly (ASCII digits only; a present-but-empty value is 422; the first of repeated values wins).
  The list filters likewise use the first non-empty value of `tag`, `author`, `favorited`.
- **Responses** are plain dicts (FastAPI's normal JSON path); timestamps are formatted to
  `YYYY-MM-DDTHH:MM:SS.mmmZ` (truncated) by hand.
- **JWT**: `exp` and `sub` are required; `iat` isn't checked (golang-jwt v5's default). An empty
  `Authorization` header counts as absent.
- **bcrypt 5** refuses passwords over 72 bytes: register/update give 422, login gives 401 (same as Go's
  `x/crypto/bcrypt`).
- FastAPI's default `strict_content_type=True` is kept: a JSON body must be sent as `application/json`.

## Free-threaded CPython (possible later variant, not built)

`3.15t` looks viable: asyncpg, uvloop, httptools, pydantic-core and websockets publish cp315t wheels. bcrypt
5.0.0, watchfiles 1.3.0 and PyYAML 6.0.3 have only up to cp314t wheels, so they would build from source
(bcrypt needs a Rust toolchain). It wouldn't change this stack's model by itself, though: Uvicorn workers are
still single-threaded event loops, so free threading would only help the bcrypt thread pool unless the
server moved to threads instead of processes.
