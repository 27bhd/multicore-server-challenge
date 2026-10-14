//! GET/POST /api/articles/:slug/comments, DELETE /api/articles/:slug/comments/:id.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};

use crate::{
    SharedState,
    auth::{AuthUser, MaybeAuth},
    error::{AppError, AppResult},
    model::{Comment, CommentEnvelope, CommentList, Profile},
    util::{JsonBody, parse_uint, required_str},
};

#[derive(sqlx::FromRow)]
struct CommentRow {
    id: i64,
    body: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    username: String,
    bio: Option<String>,
    image: Option<String>,
    #[sqlx(default)]
    following: bool,
}

impl From<CommentRow> for Comment {
    fn from(r: CommentRow) -> Self {
        Comment {
            id: r.id,
            created_at: r.created_at,
            updated_at: r.updated_at,
            body: r.body,
            author: Profile { username: r.username, bio: r.bio, image: r.image, following: r.following },
        }
    }
}

async fn article_id(state: &SharedState, slug: String) -> AppResult<i64> {
    sqlx::query_scalar("SELECT id FROM articles WHERE slug = $1")
        .bind(slug)
        .fetch_optional(&state.pool)
        .await?
        .ok_or(AppError::NotFound)
}

pub async fn list(State(state): State<SharedState>, auth: MaybeAuth, Path(slug): Path<String>) -> AppResult<Json<CommentList>> {
    let id = article_id(&state, slug).await?;
    let rows = sqlx::query_as::<_, CommentRow>(
        "SELECT c.id, c.body, c.created_at, c.updated_at, u.username, u.bio, u.image,
       EXISTS (SELECT 1 FROM follows fo WHERE fo.follower_id = $1 AND fo.followee_id = c.author_id) AS following
  FROM comments c JOIN users u ON u.id = c.author_id
 WHERE c.article_id = $2
 ORDER BY c.created_at DESC, c.id DESC LIMIT 100",
    )
    .bind(auth.viewer())
    .bind(id)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(CommentList { comments: rows.into_iter().map(Comment::from).collect() }))
}

pub async fn create(
    State(state): State<SharedState>,
    auth: AuthUser,
    Path(slug): Path<String>,
    body: JsonBody,
) -> AppResult<(StatusCode, Json<CommentEnvelope>)> {
    let text = required_str(body.object("comment")?, "body")?;
    let id = article_id(&state, slug).await?;
    let row = sqlx::query_as::<_, CommentRow>(
        "WITH c AS (INSERT INTO comments (article_id, author_id, body) VALUES ($1, $2, $3)
           RETURNING id, body, created_at, updated_at, author_id)
SELECT c.id, c.body, c.created_at, c.updated_at, u.username, u.bio, u.image
  FROM c JOIN users u ON u.id = c.author_id",
    )
    .bind(id)
    .bind(auth.id)
    .bind(text)
    .fetch_one(&state.pool)
    .await?;
    Ok((StatusCode::CREATED, Json(CommentEnvelope { comment: row.into() })))
}

pub async fn delete(
    State(state): State<SharedState>,
    auth: AuthUser,
    Path((slug, id)): Path<(String, String)>,
) -> AppResult<StatusCode> {
    let id = parse_uint(&id).ok_or(AppError::NotFound)?;
    let author: i64 = sqlx::query_scalar(
        "SELECT c.author_id FROM comments c JOIN articles a ON a.id = c.article_id
 WHERE c.id = $1 AND a.slug = $2",
    )
    .bind(id)
    .bind(slug)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;
    if author != auth.id {
        return Err(AppError::Forbidden);
    }
    sqlx::query("DELETE FROM comments WHERE id = $1").bind(id).execute(&state.pool).await?;
    Ok(StatusCode::NO_CONTENT)
}
