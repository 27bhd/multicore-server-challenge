# Go: `net/http` + pgx

**Front: nginx.** This is the Go server from the video, unchanged except for `install.sh`, a `build.sh` that runs as a normal
user, and the pool/worker defaults in `start.sh`. It follows the video's rules: the language's most popular framework, the reference SQL verbatim, no
tuning. **Video score on 8 vCPU: 22,000 users** (DigitalOcean Premium AMD, so not directly comparable with the
challenge box). Copy it into `submissions/` as a starting point if you like.

The rest of this README is the video-era write-up: "the deploy" means the benchmark box, and "series rules" are
the video's rules, which are stricter than this challenge's.

Implements [`SPEC.md`](../../SPEC.md). Passes `bench/cmd/conformance` (197/197 on the dev seed);
its transcript is the reference `bench/cmd/conformance/golden-dev.jsonl`.

| Piece | Version |
|---|---|
| Go | 1.27.x (`go.mod`: `go 1.27`) |
| HTTP | stdlib `net/http`, Go 1.22+ method/pattern routing |
| JSON | stdlib `encoding/json` |
| Postgres | `github.com/jackc/pgx/v5` v5.11.0, `pgxpool`, `MaxConns = DB_POOL_SIZE` |
| JWT | `github.com/golang-jwt/jwt/v5` v5.3.1 (HS256 only, `exp` required) |
| bcrypt | `golang.org/x/crypto/bcrypt`, cost 10 |

## Multicore model

One process. `GOMAXPROCS` is the runtime default (= cores), the GC is the default, and `net/http` runs one
goroutine per connection. bcrypt runs on the request goroutine (the Go scheduler spreads it over all cores).
One `pgxpool` of `DB_POOL_SIZE` connections (the deploy sets `10 × cores`). pgx's default extended protocol
with its per-connection prepared-statement cache.

## Build and run

```bash
bash examples/go/build.sh      # -> bin/conduit (/usr/local/go/bin/go, else go on PATH)
DATABASE_URL=postgres://conduit:conduit@127.0.0.1:5432/conduit_go JWT_SECRET=conduit-dev-secret \
  DB_POOL_SIZE=10 PORT=3103 bash examples/go/start.sh
```

Environment: `DATABASE_URL`, `JWT_SECRET` (required), `DB_POOL_SIZE` (10), `HOST` (127.0.0.1), `PORT` (3000).

## Settings beyond the defaults

- `http.Server.IdleTimeout = 75s` (outlives Nginx's upstream keep-alive, rule 7); `ReadHeaderTimeout = 10s`.
- Unmatched routes (unknown path, or known path with another method) return a JSON 404 instead of the mux's
  plain-text 404/405.
- Request bodies are decoded into `map[string]json.RawMessage` so key matching is case-sensitive
  (`encoding/json` struct decoding would accept `"User"` for `"user"`).
- No request-body size limit, no per-request logging.
