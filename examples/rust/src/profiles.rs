//! GET /api/profiles/:username, POST/DELETE /api/profiles/:username/follow.

use axum::{
    Json,
    extract::{Path, State},
};
use sqlx::Row;

use crate::{
    SharedState,
    auth::{AuthUser, MaybeAuth},
    error::{AppError, AppResult},
    model::{Profile, ProfileEnvelope},
};

pub async fn get_profile(
    State(state): State<SharedState>,
    auth: MaybeAuth,
    Path(username): Path<String>,
) -> AppResult<Json<ProfileEnvelope>> {
    let row = sqlx::query(
        "SELECT u.id, u.username, u.bio, u.image,
       EXISTS (SELECT 1 FROM follows fo WHERE fo.follower_id = $1 AND fo.followee_id = u.id) AS following
  FROM users u WHERE u.username = $2",
    )
    .bind(auth.viewer())
    .bind(username)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;
    Ok(Json(ProfileEnvelope {
        profile: Profile {
            username: row.try_get("username")?,
            bio: row.try_get("bio")?,
            image: row.try_get("image")?,
            following: row.try_get("following")?,
        },
    }))
}

async fn set_follow(state: SharedState, viewer: i64, username: String, follow: bool) -> AppResult<Json<ProfileEnvelope>> {
    let row = sqlx::query("SELECT id, username, bio, image FROM users WHERE username = $1")
        .bind(username)
        .fetch_optional(&state.pool)
        .await?
        .ok_or(AppError::NotFound)?;
    let followee: i64 = row.try_get("id")?;
    let sql = if follow {
        "INSERT INTO follows (follower_id, followee_id) VALUES ($1, $2) ON CONFLICT DO NOTHING"
    } else {
        "DELETE FROM follows WHERE follower_id = $1 AND followee_id = $2"
    };
    sqlx::query(sql).bind(viewer).bind(followee).execute(&state.pool).await?;
    Ok(Json(ProfileEnvelope {
        profile: Profile {
            username: row.try_get("username")?,
            bio: row.try_get("bio")?,
            image: row.try_get("image")?,
            following: follow,
        },
    }))
}

pub async fn follow(State(state): State<SharedState>, auth: AuthUser, Path(username): Path<String>) -> AppResult<Json<ProfileEnvelope>> {
    set_follow(state, auth.id, username, true).await
}

pub async fn unfollow(State(state): State<SharedState>, auth: AuthUser, Path(username): Path<String>) -> AppResult<Json<ProfileEnvelope>> {
    set_follow(state, auth.id, username, false).await
}
