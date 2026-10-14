//! POST /api/users, POST /api/users/login, GET /api/user, PUT /api/user.

use axum::{Json, extract::State, http::StatusCode};
use sqlx::Row;

use crate::{
    SharedState,
    auth::{AuthUser, issue_token},
    error::{AppError, AppResult, unique_to_422},
    model::{User, UserEnvelope},
    util::{JsonBody, optional_str, required_str},
};

const BCRYPT_COST: u32 = 10;

/// bcrypt only uses the first 72 bytes; like Go's `GenerateFromPassword`, longer passwords are 422.
async fn hash_password(password: String) -> AppResult<String> {
    if password.len() > 72 {
        return Err(AppError::Unprocessable("password is too long"));
    }
    tokio::task::spawn_blocking(move || bcrypt::hash(password, BCRYPT_COST))
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
        .map_err(|e| AppError::Internal(e.to_string()))
}

async fn verify_password(password: String, hash: String) -> AppResult<bool> {
    tokio::task::spawn_blocking(move || bcrypt::verify(password, &hash).unwrap_or(false))
        .await
        .map_err(|e| AppError::Internal(e.to_string()))
}

fn user_from_row(row: &sqlx::postgres::PgRow, token: String) -> AppResult<User> {
    Ok(User {
        email: row.try_get("email")?,
        token,
        username: row.try_get("username")?,
        bio: row.try_get("bio")?,
        image: row.try_get("image")?,
    })
}

pub async fn register(State(state): State<SharedState>, body: JsonBody) -> AppResult<(StatusCode, Json<UserEnvelope>)> {
    let obj = body.object("user")?;
    let username = required_str(obj, "username")?;
    let email = required_str(obj, "email")?;
    let password = required_str(obj, "password")?;
    let hash = hash_password(password).await?;

    let row = sqlx::query(
        "INSERT INTO users (username, email, password_hash) VALUES ($1, $2, $3)
RETURNING id, username, email, bio, image",
    )
    .bind(username)
    .bind(email)
    .bind(hash)
    .fetch_one(&state.pool)
    .await
    .map_err(|e| unique_to_422(e, "username or email already taken"))?;

    let id: i64 = row.try_get("id")?;
    let token = issue_token(&state, id)?;
    Ok((StatusCode::CREATED, Json(UserEnvelope { user: user_from_row(&row, token)? })))
}

pub async fn login(State(state): State<SharedState>, body: JsonBody) -> AppResult<Json<UserEnvelope>> {
    let obj = body.object("user")?;
    let email = required_str(obj, "email")?;
    let password = required_str(obj, "password")?;

    let row = sqlx::query("SELECT id, username, email, bio, image, password_hash FROM users WHERE email = $1")
        .bind(email)
        .fetch_optional(&state.pool)
        .await?
        .ok_or(AppError::Unauthorized)?;

    let hash: String = row.try_get("password_hash")?;
    if !verify_password(password, hash).await? {
        return Err(AppError::Unauthorized);
    }
    let id: i64 = row.try_get("id")?;
    let token = issue_token(&state, id)?;
    Ok(Json(UserEnvelope { user: user_from_row(&row, token)? }))
}

pub async fn current(State(state): State<SharedState>, auth: AuthUser) -> AppResult<Json<UserEnvelope>> {
    let row = sqlx::query("SELECT id, username, email, bio, image FROM users WHERE id = $1")
        .bind(auth.id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or(AppError::Unauthorized)?;
    Ok(Json(UserEnvelope { user: user_from_row(&row, auth.token)? }))
}

pub async fn update(State(state): State<SharedState>, auth: AuthUser, body: JsonBody) -> AppResult<Json<UserEnvelope>> {
    let obj = body.object("user")?;
    let email = optional_str(obj, "email", true)?;
    let username = optional_str(obj, "username", true)?;
    let password = optional_str(obj, "password", true)?;
    let bio = optional_str(obj, "bio", false)?;
    let image = optional_str(obj, "image", false)?;
    let hash = match password {
        Some(p) => Some(hash_password(p).await?),
        None => None,
    };

    let row = sqlx::query(
        "UPDATE users SET email = COALESCE($2, email), username = COALESCE($3, username),
                 password_hash = COALESCE($4, password_hash), bio = COALESCE($5, bio), image = COALESCE($6, image)
 WHERE id = $1
RETURNING id, username, email, bio, image",
    )
    .bind(auth.id)
    .bind(email)
    .bind(username)
    .bind(hash)
    .bind(bio)
    .bind(image)
    .fetch_optional(&state.pool)
    .await
    .map_err(|e| unique_to_422(e, "username or email already taken"))?
    .ok_or(AppError::Unauthorized)?;
    Ok(Json(UserEnvelope { user: user_from_row(&row, auth.token)? }))
}
