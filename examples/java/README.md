# Conduit: Java (Spring Boot 4, virtual threads)

**Front: nginx.** This is the Java server from the video, unchanged except for `install.sh`, a `build.sh` that runs as a normal
user, and the pool/worker defaults in `start.sh`. It follows the video's rules: the language's most popular framework, the reference SQL verbatim, no
tuning. **Video score on 8 vCPU: 21,000 users** (DigitalOcean Premium AMD, so not directly comparable with the
challenge box). Copy it into `submissions/` as a starting point if you like.

The rest of this README is the video-era write-up: "the deploy" means the benchmark box, and "series rules" are
the video's rules, which are stricter than this challenge's.

Spring MVC on embedded Tomcat, JdbcTemplate (`NamedParameterJdbcTemplate`) over HikariCP, raw SQL verbatim
from `SPEC.md`. One JVM; Tomcat runs **every request on its own virtual thread**
(`spring.threads.virtual.enabled=true`).

## Versions (from the built jar)

| Piece | Version |
|---|---|
| JDK | 27 (servers: `JAVA_HOME=/opt/jdk`; Mac: Homebrew `openjdk@27`), bytecode `--release 27` |
| Spring Boot | 4.1.1 (`spring-boot-starter-webmvc`, `spring-boot-starter-jdbc`) |
| Spring Framework / MVC / JDBC | 7.0.9 |
| Tomcat (embedded) | 11.0.24 |
| Jackson | 3.1.5 (Boot default; java-jwt pulls in Jackson 2.21.5 for its own parsing) |
| HikariCP | 7.0.2 |
| PostgreSQL JDBC (pgjdbc) | 42.7.14 (latest; Boot 4.1.1 manages 42.7.13) |
| spring-security-crypto | 7.1.1, `BCryptPasswordEncoder(10)` only (no Spring Security filter chain) |
| JWT | auth0 `java-jwt` 4.6.1, HS256 |
| Maven | 3.9.16 via the checked-in wrapper (`./mvnw`, wrapper 3.3.4) |

## Build and run

```bash
./build.sh   # JAVA_HOME=${JAVA_HOME:-/opt/jdk} ./mvnw -q -B -DskipTests package -> target/conduit.jar
./start.sh   # exec "${JAVA_HOME:-/opt/jdk}/bin/java" -jar target/conduit.jar
```

Environment: `DATABASE_URL` (`postgres://user:pass@host:port/db`, translated to a JDBC URL at startup),
`JWT_SECRET`, `DB_POOL_SIZE` (Hikari `maximumPoolSize`, default 10), `HOST` (default 127.0.0.1), `PORT` (default 3000).

Local: `dropdb --if-exists conduit_java; createdb -T conduit_dev conduit_java`, then
`DATABASE_URL=postgres://$USER@localhost:5432/conduit_java JWT_SECRET=conduit-dev-secret PORT=3102 JAVA_HOME=/opt/homebrew/opt/openjdk@27 ./start.sh`.

## How it uses multiple cores

- **Virtual threads.** With `spring.threads.virtual.enabled=true` Tomcat's executor creates one virtual thread per
  request (no 200-thread platform pool). Virtual threads are scheduled on a ForkJoinPool of carrier threads whose
  size is the number of cores, so 1 carrier on the 1-core box and 8 on the 8-core box. A request blocked on JDBC
  unmounts from its carrier. Checked at runtime with a temporary endpoint (since removed):
  `virtual=true VirtualThread[#60,tomcat-handler-1]/runnable@ForkJoinPool-1-worker-1`.
- **HikariCP** `maximumPoolSize = DB_POOL_SIZE` (10 × cores), `connectionTimeout` 5 s. Hikari keeps the pool full
  (`minimumIdle` defaults to the maximum). Waiting for a connection parks the virtual thread.
- **JIT and GC** threads are sized by the JVM from the core count. No JVM flags at all (no `-Xmx`, no GC choice,
  no AppCDS, no native image).

### GC ergonomics (JDK 27)

The rule "SerialGC on < 2 CPUs or < 1792 MB" applied to older JDKs (it is what JDK 21 did on the 1-vCPU box). **JDK 27 picks G1 everywhere** (G1 became the
default in all environments), including the 1-vCPU / 2 GB box:

| Cores (`-XX:ActiveProcessorCount`) | GC | ParallelGCThreads | ConcGCThreads |
|---|---|---|---|
| 1 | G1 (ergonomic) | 1 | 1 |
| 2 | G1 | 2 | 1 |
| 4 | G1 | 4 | 1 |
| 8 | G1 | 8 | 2 |

Measured on the Mac with JDK 27 (`java -XX:ActiveProcessorCount=N -XX:+PrintFlagsFinal -version`). JDK 27 removed
`-XX:MaxRAM`, so the RAM side can't be emulated on the Mac; the earlier series saw G1 on the real 1-vCPU droplet with
JDK 27 too (`servers/java-boot4-j27`). Max heap is the default 25% of RAM: about 512 MB / 1 / 2 / 4 GB on the
2 / 4 / 8 / 16 GB boxes. Compact object headers are on by default in 27. Confirm on a box with
`/opt/jdk/bin/java -XX:+PrintFlagsFinal -version | grep -E 'Use(G1|Serial)GC |MaxHeapSize'`.

## Settings that differ from framework defaults (and why)

- `server.address=${HOST}`, `server.port=${PORT}`: the series' env names, not Spring's.
- `spring.threads.virtual.enabled=true`: the multicore model for Java in this experiment (SPEC rule 2).
- `server.tomcat.keep-alive-timeout=65s`: SPEC rule 7 (Tomcat's default is its 20 s connection timeout).
- `server.tomcat.max-connections=20000`, `accept-count=1024`, `max-keep-alive-requests=-1`: Tomcat's defaults
  (8192 connections, a 100-deep accept queue, close each keep-alive connection after 100 requests) are caps the Go
  server doesn't have; the server itself should be what limits.
- `spring.datasource.hikari.maximum-pool-size=${DB_POOL_SIZE}`, `connection-timeout=5000`.
- A `HikariDataSource` bean built from `DATABASE_URL` (libpq-style URL; JDBC can't take it directly).
- `spring.web.resources.add-mappings=false`, `spring.mvc.formcontent.filter.enabled=false`, and the Accept header
  is ignored (always JSON): no static-resource lookup on unknown paths, request bodies are JSON whatever the
  Content-Type, no 406.
- Errors: a `@RestControllerAdvice` plus a replacement `/error` controller render every error, including Spring's
  own and 500s, as `{"errors":{"body":[...]}}`. Like the Go reference's catch-all route, an unknown path **or a
  known path with another method** is a 404 (Spring would give 405), and OPTIONS is a 404 too.
- No per-request logging (Boot logs nothing per request by default). Pool-timeout 500s aren't logged either;
  other unexpected 500s are.

## Implementation notes / deviations

- **JWT library: auth0 `java-jwt`, not jjwt/nimbus.** jjwt and nimbus-jose-jwt both refuse HS256 secrets shorter
  than 256 bits (RFC 7518 §3.2), and the dev secret `conduit-dev-secret` is 18 bytes. java-jwt accepts any secret,
  like golang-jwt. Verification: signature, `alg` = HS256, `exp` required and not past (`nbf` if present), `iat` not
  checked (as golang-jwt's defaults), `sub` must be ASCII digits > 0.
- Bodies are parsed as a Jackson tree (not bound to DTOs) so `{"title": 123}` is a 422 rather than coerced to
  `"123"`; malformed JSON, a missing/non-object wrapper and trailing garbage are 422, matching the Go reference.
- Every SQL parameter is bound with an explicit type (viewer/ids `BIGINT`, strings `VARCHAR`, tag lists `text[]`),
  so a `NULL` in `COALESCE($n, col)` never triggers a parameter-describe round trip. SPEC's `$n` placeholders are
  named parameters; a placeholder used twice (`$1` viewer, `$2` title) is bound twice by JDBC.
- `POST /api/articles` runs its three statements in one `TransactionTemplate` transaction.
- Timestamps: `OffsetDateTime` formatted with `uuuu-MM-dd'T'HH:mm:ss.SSS'Z'` in UTC (truncates microseconds).
- Passwords over 72 bytes are a 422 on register/update (bcrypt's limit; Spring's encoder and Go's both refuse them).
