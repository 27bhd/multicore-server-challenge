// Command conduit is the Go implementation of the RealWorld "Conduit" API for the
// multicore server challenge (see SPEC.md).
//
// Standard library net/http with Go 1.22+ pattern routing, encoding/json, pgx/v5 (pgxpool),
// golang.org/x/crypto/bcrypt and golang-jwt/jwt/v5 (HS256).
package main

import (
	"context"
	"errors"
	"log"
	"net"
	"net/http"
	"os"
	"os/signal"
	"strconv"
	"syscall"
	"time"

	"github.com/jackc/pgx/v5/pgxpool"
)

func envOr(name, fallback string) string {
	if v := os.Getenv(name); v != "" {
		return v
	}
	return fallback
}

func main() {
	dbURL := os.Getenv("DATABASE_URL")
	secret := os.Getenv("JWT_SECRET")
	if dbURL == "" || secret == "" {
		log.Fatal("DATABASE_URL and JWT_SECRET are required")
	}
	poolSize, err := strconv.Atoi(envOr("DB_POOL_SIZE", "10"))
	if err != nil || poolSize <= 0 {
		log.Fatalf("DB_POOL_SIZE must be a positive integer, got %q", os.Getenv("DB_POOL_SIZE"))
	}

	poolCfg, err := pgxpool.ParseConfig(dbURL)
	if err != nil {
		log.Fatalf("parse DATABASE_URL: %v", err)
	}
	poolCfg.MaxConns = int32(poolSize)

	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()

	pool, err := pgxpool.NewWithConfig(ctx, poolCfg)
	if err != nil {
		log.Fatalf("create pool: %v", err)
	}
	defer pool.Close()

	a := &app{pool: pool, secret: []byte(secret)}
	srv := &http.Server{
		Addr:    net.JoinHostPort(envOr("HOST", "127.0.0.1"), envOr("PORT", "3000")),
		Handler: a.routes(),
		// Outlive Nginx's upstream keep-alive (series rule 7).
		IdleTimeout:       75 * time.Second,
		ReadHeaderTimeout: 10 * time.Second,
	}

	done := make(chan struct{})
	go func() {
		defer close(done)
		<-ctx.Done()
		sctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
		defer cancel()
		_ = srv.Shutdown(sctx)
	}()

	log.Printf("conduit (go) listening on http://%s, pool %d", srv.Addr, poolSize)
	if err := srv.ListenAndServe(); err != nil && !errors.Is(err, http.ErrServerClosed) {
		log.Fatal(err)
	}
	<-done
}
