//! Article list/feed/CRUD, favorites and tags.

use axum::{
    Json,
    extract::{Path, State},
    http::{StatusCode, Uri},
};
use serde_json::Value;
use sqlx::Row;

use crate::{
    SharedState,
    auth::{AuthUser, MaybeAuth},
    error::{AppError, AppResult},
    model::{
        ARTICLE_BY_ID, Article, ArticleEnvelope, ArticleList, ArticleRow, TagList, article_full_select,
        article_list_select, order_page,
    },
    util::{JsonBody, QueryParams, optional_str, required_str},
};

const LIST_ALL: &str = concat!(article_list_select!(), order_page!());
const LIST_TAG: &str = concat!(
    article_list_select!(),
    " WHERE a.id IN (SELECT at.article_id FROM article_tags at WHERE at.tag_id = (SELECT t.id FROM tags t WHERE t.name = $4) ORDER BY at.article_id DESC LIMIT $2 OFFSET $3)",
    " ORDER BY a.id DESC"
);
const LIST_AUTHOR: &str = concat!(article_list_select!(), " WHERE a.author_id = (SELECT au.id FROM users au WHERE au.username = $4)", order_page!());
const LIST_FAVORITED: &str = concat!(
    article_list_select!(),
    " WHERE a.id IN (SELECT f.article_id FROM favorites f WHERE f.user_id = (SELECT fu.id FROM users fu WHERE fu.username = $4) ORDER BY f.article_id DESC LIMIT $2 OFFSET $3)",
    " ORDER BY a.id DESC"
);
const LIST_FEED: &str = concat!(
    article_list_select!(),
    " WHERE a.id IN (SELECT x.id FROM follows fo CROSS JOIN LATERAL (SELECT a2.id FROM articles a2 WHERE a2.author_id = fo.followee_id ORDER BY a2.id DESC LIMIT ($2::int + $3::int)) x WHERE fo.follower_id = $1 ORDER BY x.id DESC LIMIT $2 OFFSET $3)",
    " ORDER BY a.id DESC"
);
const COUNT_ALL: &str = "SELECT count(*) FROM (SELECT 1 FROM articles a LIMIT 1000) s";
const COUNT_TAG: &str = "SELECT count(*) FROM (SELECT 1 FROM article_tags at WHERE at.tag_id = (SELECT t.id FROM tags t WHERE t.name = $1) LIMIT 1000) s";
const COUNT_AUTHOR: &str = "SELECT count(*) FROM (SELECT 1 FROM articles a WHERE a.author_id = (SELECT u.id FROM users u WHERE u.username = $1) LIMIT 1000) s";
const COUNT_FAVORITED: &str = "SELECT count(*) FROM (SELECT 1 FROM favorites f WHERE f.user_id = (SELECT fu.id FROM users fu WHERE fu.username = $1) LIMIT 1000) s";
const COUNT_FEED: &str = "SELECT count(*) FROM (SELECT 1 FROM articles a WHERE a.author_id IN (SELECT fo.followee_id FROM follows fo WHERE fo.follower_id = $1) LIMIT 1000) s";
const ARTICLE_BY_SLUG: &str = concat!(article_full_select!(), " WHERE a.slug = $2");

pub async fn list(State(state): State<SharedState>, auth: MaybeAuth, uri: Uri) -> AppResult<Json<ArticleList>> {
    let q = QueryParams::parse(uri.query());
    let (limit, offset) = q.page()?;
    let viewer = auth.viewer();

    let (page_sql, count_sql, filter) = if let Some(tag) = q.filter("tag") {
        (LIST_TAG, COUNT_TAG, Some(tag))
    } else if let Some(author) = q.filter("author") {
        (LIST_AUTHOR, COUNT_AUTHOR, Some(author))
    } else if let Some(fav) = q.filter("favorited") {
        (LIST_FAVORITED, COUNT_FAVORITED, Some(fav))
    } else {
        (LIST_ALL, COUNT_ALL, None)
    };

    let mut page = sqlx::query_as::<_, ArticleRow>(page_sql).bind(viewer).bind(limit).bind(offset);
    let mut count = sqlx::query_scalar::<_, i64>(count_sql);
    if let Some(f) = filter {
        page = page.bind(f);
        count = count.bind(f);
    }
    let rows = page.fetch_all(&state.pool).await?;
    let articles_count = count.fetch_one(&state.pool).await?;
    Ok(Json(ArticleList { articles: rows.into_iter().map(Article::from).collect(), articles_count }))
}

pub async fn feed(State(state): State<SharedState>, auth: AuthUser, uri: Uri) -> AppResult<Json<ArticleList>> {
    let q = QueryParams::parse(uri.query());
    let (limit, offset) = q.page()?;
    let rows = sqlx::query_as::<_, ArticleRow>(LIST_FEED)
        .bind(auth.id)
        .bind(limit)
        .bind(offset)
        .fetch_all(&state.pool)
        .await?;
    let articles_count = sqlx::query_scalar::<_, i64>(COUNT_FEED).bind(auth.id).fetch_one(&state.pool).await?;
    Ok(Json(ArticleList { articles: rows.into_iter().map(Article::from).collect(), articles_count }))
}

async fn article_by_id(state: &SharedState, viewer: i64, id: i64) -> AppResult<Article> {
    let row = sqlx::query_as::<_, ArticleRow>(ARTICLE_BY_ID)
        .bind(viewer)
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or(AppError::NotFound)?;
    Ok(row.into())
}

pub async fn get_article(
    State(state): State<SharedState>,
    auth: MaybeAuth,
    Path(slug): Path<String>,
) -> AppResult<Json<ArticleEnvelope>> {
    let row = sqlx::query_as::<_, ArticleRow>(ARTICLE_BY_SLUG)
        .bind(auth.viewer())
        .bind(slug)
        .fetch_optional(&state.pool)
        .await?
        .ok_or(AppError::NotFound)?;
    Ok(Json(ArticleEnvelope { article: row.into() }))
}

pub async fn create(
    State(state): State<SharedState>,
    auth: AuthUser,
    body: JsonBody,
) -> AppResult<(StatusCode, Json<ArticleEnvelope>)> {
    let obj = body.object("article")?;
    let title = required_str(obj, "title")?;
    let description = required_str(obj, "description")?;
    let text = required_str(obj, "body")?;
    let tags: Vec<String> = match obj.get("tagList") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|t| match t {
                Value::String(s) if !s.is_empty() => Ok(s.clone()),
                _ => Err(AppError::Unprocessable("tagList must be an array of non-empty strings")),
            })
            .collect::<Result<_, _>>()?,
        Some(_) => return Err(AppError::Unprocessable("tagList must be an array of non-empty strings")),
    };

    let mut tx = state.pool.begin().await?;
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO articles (id, slug, author_id, title, description, body)
SELECT n.id, trim(both '-' from lower(regexp_replace($2, '[^A-Za-z0-9]+', '-', 'g'))) || '-' || n.id, $1, $2, $3, $4
  FROM (SELECT nextval(pg_get_serial_sequence('articles', 'id')) AS id) n
RETURNING id",
    )
    .bind(auth.id)
    .bind(title)
    .bind(description)
    .bind(text)
    .fetch_one(&mut *tx)
    .await?;
    if !tags.is_empty() {
        sqlx::query("INSERT INTO tags (name) SELECT DISTINCT unnest($1::text[]) ON CONFLICT (name) DO NOTHING")
            .bind(&tags)
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO article_tags (article_id, tag_id) SELECT $1, t.id FROM tags t WHERE t.name = ANY($2::text[])")
            .bind(id)
            .bind(&tags)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;

    let article = article_by_id(&state, auth.id, id).await?;
    Ok((StatusCode::CREATED, Json(ArticleEnvelope { article })))
}

/// `SELECT id, author_id FROM articles WHERE slug = $1` → 404; author ≠ viewer → 403.
async fn own_article(state: &SharedState, viewer: i64, slug: String) -> AppResult<i64> {
    let row = sqlx::query("SELECT id, author_id FROM articles WHERE slug = $1")
        .bind(slug)
        .fetch_optional(&state.pool)
        .await?
        .ok_or(AppError::NotFound)?;
    let author: i64 = row.try_get("author_id")?;
    if author != viewer {
        return Err(AppError::Forbidden);
    }
    Ok(row.try_get("id")?)
}

pub async fn update(
    State(state): State<SharedState>,
    auth: AuthUser,
    Path(slug): Path<String>,
    body: JsonBody,
) -> AppResult<Json<ArticleEnvelope>> {
    let obj = body.object("article")?;
    let title = optional_str(obj, "title", true)?;
    let description = optional_str(obj, "description", true)?;
    let text = optional_str(obj, "body", true)?;

    let id = own_article(&state, auth.id, slug).await?;
    sqlx::query(
        "UPDATE articles SET title = COALESCE($2, title), description = COALESCE($3, description), body = COALESCE($4, body),
       slug = CASE WHEN $2::text IS NULL THEN slug ELSE trim(both '-' from lower(regexp_replace($2, '[^A-Za-z0-9]+', '-', 'g'))) || '-' || id END,
       updated_at = now()
 WHERE id = $1",
    )
    .bind(id)
    .bind(title)
    .bind(description)
    .bind(text)
    .execute(&state.pool)
    .await?;
    let article = article_by_id(&state, auth.id, id).await?;
    Ok(Json(ArticleEnvelope { article }))
}

pub async fn delete(State(state): State<SharedState>, auth: AuthUser, Path(slug): Path<String>) -> AppResult<StatusCode> {
    let id = own_article(&state, auth.id, slug).await?;
    sqlx::query("DELETE FROM articles WHERE id = $1").bind(id).execute(&state.pool).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn set_favorite(state: SharedState, viewer: i64, slug: String, favorite: bool) -> AppResult<Json<ArticleEnvelope>> {
    let id: i64 = sqlx::query_scalar("SELECT id FROM articles WHERE slug = $1")
        .bind(slug)
        .fetch_optional(&state.pool)
        .await?
        .ok_or(AppError::NotFound)?;
    let sql = if favorite {
        "INSERT INTO favorites (user_id, article_id) VALUES ($1, $2) ON CONFLICT DO NOTHING"
    } else {
        "DELETE FROM favorites WHERE user_id = $1 AND article_id = $2"
    };
    sqlx::query(sql).bind(viewer).bind(id).execute(&state.pool).await?;
    let article = article_by_id(&state, viewer, id).await?;
    Ok(Json(ArticleEnvelope { article }))
}

pub async fn favorite(State(state): State<SharedState>, auth: AuthUser, Path(slug): Path<String>) -> AppResult<Json<ArticleEnvelope>> {
    set_favorite(state, auth.id, slug, true).await
}

pub async fn unfavorite(State(state): State<SharedState>, auth: AuthUser, Path(slug): Path<String>) -> AppResult<Json<ArticleEnvelope>> {
    set_favorite(state, auth.id, slug, false).await
}

pub async fn tags(State(state): State<SharedState>) -> AppResult<Json<TagList>> {
    let tags = sqlx::query_scalar("SELECT name FROM tags ORDER BY name").fetch_all(&state.pool).await?;
    Ok(Json(TagList { tags }))
}
