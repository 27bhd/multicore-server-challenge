# Rust: axum + tokio + sqlx

**Front: nginx.** This is the Rust server from the video, unchanged except for `install.sh`, a `build.sh` that runs as a normal
user, and the pool/worker defaults in `start.sh`. It follows the video's rules: the language's most popular framework, the reference SQL verbatim, no
tuning. **Video score on 8 vCPU: 19,000 users** (DigitalOcean Premium AMD, so not directly comparable with the
challenge box). Copy it into `submissions/` as a starting point if you like.

The rest of this README is the video-era write-up: "the deploy" means the benchmark box, and "series rules" are
the video's rules, which are stricter than this challenge's.

The Conduit API from [`SPEC.md`](../../SPEC.md), written the way axum's own examples do it:
a `Router` of async handlers, an `AppError` enum implementing `IntoResponse`, custom extractors
for `Authorization: Token <jwt>`, and raw SQL (verbatim from SPEC) through a sqlx `PgPool`.

## Stack (exact versions, from `Cargo.lock`)

| Piece | Version |
|---|---|
| Rust toolchain | stable 1.99 on the servers (built locally with 1.98.1; sqlx 0.9 needs >= 1.94) |
| axum | 0.8.9 (hyper 1.12.0) |
| tokio | 1.53.2 (`features = ["full"]`) |
| sqlx | 0.9.0 (`postgres`, `runtime-tokio`, `chrono`, `derive`; no TLS) |
| bcrypt | 0.19.3 (cost 10) |
| jsonwebtoken | 11.1.0 (`rust_crypto` backend) |
| serde / serde_json, chrono | 1.x / 1.x, 0.4.x |

Release profile is cargo's default (opt-level 3). No LTO, PGO, `target-cpu`, or custom allocator.

## How it uses multiple cores

One process. `#[tokio::main]` starts tokio's default multi-thread runtime: one worker thread per
core (`CORES` is informational only; tokio reads the CPU count itself). bcrypt hashes and verifies
run in `tokio::task::spawn_blocking`, so they go to tokio's blocking pool (up to 512 threads,
created on demand) instead of stalling a worker. One `PgPool` with `max_connections =
DB_POOL_SIZE` (10 × cores, set by the deploy) and a 5 s acquire timeout; everything else is
sqlx's default (lazy connections, per-connection prepared-statement cache).

## Build and run

```bash
bash build.sh    # cargo build --release --locked (Rust 1.99.0 from install.sh, else cargo on PATH)
DATABASE_URL=postgres://... JWT_SECRET=... DB_POOL_SIZE=10 HOST=127.0.0.1 PORT=3000 bash start.sh
```

`start.sh` execs `target/release/conduit` in the foreground and reads only the environment.

## Keep-alive

`axum::serve` (hyper 1 via hyper-util) has no idle keep-alive timeout: a connection stays open
until the client closes it, which outlives Nginx's upstream keep-alive (series rule 7).

## Decisions and deviations

- **Runtime queries (`sqlx::query` / `query_as` + `.bind`)**, not the `query!` macros, so the
  build doesn't need a live database. SQL is `&'static str`, assembled from SPEC's shared
  fragments with `concat!`, and prepared/cached by sqlx.
- **Request bodies** are read as bytes and parsed with serde_json regardless of `Content-Type`.
  Invalid JSON, a missing/non-object wrapper (`user`, `article`, `comment`), a missing or empty
  required string, or a non-string value for an optional field is 422.
- **Auth runs before body validation** (extractor order), so a bad body with no token is 401.
- **JWT**: `Validation::new(HS256)` with `leeway = 0` (the crate's default is 60 s), `exp`
  required. `sub` must parse as an integer, otherwise 401.
- **Query strings**: the first occurrence of a parameter wins; an empty `tag`/`author`/`favorited`
  counts as absent.
- **Unknown paths and wrong methods** both get a JSON 404 in the `errors.body` shape (matching Go).
- **Validation matches Go:** `limit`/`offset`/comment id/`sub` are ASCII digits only; present-but-empty
  `limit`/`offset` give 422; on PUT, sent `email`/`username`/`password`/`title`/`description`/`body` must be non-empty;
  passwords over 72 bytes give 422 (the bcrypt crate would otherwise truncate silently).
- Response structs (not `json!`) keep SPEC's field order.
