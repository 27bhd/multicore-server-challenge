//! Conduit (RealWorld) API for the multicore server challenge (see SPEC.md):
//! axum 0.8 on tokio's multi-thread runtime, raw SQL through a sqlx PgPool, HS256 JWT auth.

mod articles;
mod auth;
mod comments;
mod error;
mod model;
mod profiles;
mod users;
mod util;

use std::{env, sync::Arc, time::Duration};

use axum::{
    Router,
    routing::{get, post},
};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Validation};
use sqlx::{PgPool, postgres::PgPoolOptions};
use tokio::net::TcpListener;

use crate::error::AppError;

pub struct AppState {
    pub pool: PgPool,
    pub enc_key: EncodingKey,
    pub dec_key: DecodingKey,
    pub validation: Validation,
}

pub type SharedState = Arc<AppState>;

fn env_or(key: &str, default: &str) -> String {
    env::var(key).ok().filter(|v| !v.is_empty()).unwrap_or_else(|| default.to_string())
}

#[tokio::main]
async fn main() {
    let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let jwt_secret = env::var("JWT_SECRET").expect("JWT_SECRET must be set");
    let pool_size: u32 = env_or("DB_POOL_SIZE", "10").parse().expect("DB_POOL_SIZE must be an integer");
    let host = env_or("HOST", "127.0.0.1");
    let port: u16 = env_or("PORT", "3000").parse().expect("PORT must be an integer");

    let pool = PgPoolOptions::new()
        .max_connections(pool_size)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&database_url)
        .await
        .expect("could not connect to Postgres");

    // HS256 with `exp` required and checked, `nbf` checked when present, no clock leeway,
    // and no audience check (the same rules as golang-jwt v5's defaults in the Go reference).
    let mut validation = Validation::new(Algorithm::HS256);
    validation.leeway = 0;
    validation.validate_nbf = true;
    validation.validate_aud = false;

    let state = Arc::new(AppState {
        pool,
        enc_key: EncodingKey::from_secret(jwt_secret.as_bytes()),
        dec_key: DecodingKey::from_secret(jwt_secret.as_bytes()),
        validation,
    });

    let app = Router::new()
        .route("/api/users", post(users::register))
        .route("/api/users/login", post(users::login))
        .route("/api/user", get(users::current).put(users::update))
        .route("/api/profiles/{username}", get(profiles::get_profile))
        .route("/api/profiles/{username}/follow", post(profiles::follow).delete(profiles::unfollow))
        .route("/api/articles", get(articles::list).post(articles::create))
        .route("/api/articles/feed", get(articles::feed))
        .route("/api/articles/{slug}", get(articles::get_article).put(articles::update).delete(articles::delete))
        .route("/api/articles/{slug}/favorite", post(articles::favorite).delete(articles::unfavorite))
        .route("/api/articles/{slug}/comments", get(comments::list).post(comments::create))
        .route("/api/articles/{slug}/comments/{id}", axum::routing::delete(comments::delete))
        .route("/api/tags", get(articles::tags))
        // Anything else (unknown path, or a known path with another method) is a JSON 404, as in Go.
        .fallback(|| async { AppError::NotFound })
        .method_not_allowed_fallback(|| async { AppError::NotFound })
        .with_state(state);

    let listener = TcpListener::bind((host.as_str(), port)).await.expect("could not bind");
    println!("[server] listening on http://{host}:{port} (pid {}, pool {pool_size})", std::process::id());
    axum::serve(listener, app).with_graceful_shutdown(shutdown_signal()).await.expect("server error");
}

async fn shutdown_signal() {
    let ctrl_c = async { tokio::signal::ctrl_c().await.expect("failed to install Ctrl+C handler") };
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };
    tokio::select! { _ = ctrl_c => {}, _ = terminate => {} }
}
