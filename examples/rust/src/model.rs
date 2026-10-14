//! Response objects and the SQL shared between endpoints.

use chrono::{DateTime, Utc};
use serde::{Serialize, Serializer};

/// RFC 3339 UTC with exactly 3 fractional digits, truncated: "2025-04-01T12:00:00.000Z".
pub fn ser_ts<S: Serializer>(ts: &DateTime<Utc>, s: S) -> Result<S::Ok, S::Error> {
    s.collect_str(&ts.format("%Y-%m-%dT%H:%M:%S%.3fZ"))
}

#[derive(Serialize)]
pub struct User {
    pub email: String,
    pub token: String,
    pub username: String,
    pub bio: Option<String>,
    pub image: Option<String>,
}

#[derive(Serialize)]
pub struct UserEnvelope {
    pub user: User,
}

#[derive(Serialize)]
pub struct Profile {
    pub username: String,
    pub bio: Option<String>,
    pub image: Option<String>,
    pub following: bool,
}

#[derive(Serialize)]
pub struct ProfileEnvelope {
    pub profile: Profile,
}

/// One row of ARTICLE_LIST_SELECT / ARTICLE_FULL_SELECT (`body` only in the latter).
#[derive(sqlx::FromRow)]
pub struct ArticleRow {
    #[allow(dead_code)]
    pub id: i64,
    pub slug: String,
    pub title: String,
    pub description: String,
    #[sqlx(default)]
    pub body: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub tag_list: Vec<String>,
    pub favorites_count: i64,
    pub favorited: bool,
    pub username: String,
    pub bio: Option<String>,
    pub image: Option<String>,
    pub following: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Article {
    pub slug: String,
    pub title: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    pub tag_list: Vec<String>,
    #[serde(serialize_with = "ser_ts")]
    pub created_at: DateTime<Utc>,
    #[serde(serialize_with = "ser_ts")]
    pub updated_at: DateTime<Utc>,
    pub favorited: bool,
    pub favorites_count: i64,
    pub author: Profile,
}

impl From<ArticleRow> for Article {
    fn from(r: ArticleRow) -> Self {
        Article {
            slug: r.slug,
            title: r.title,
            description: r.description,
            body: r.body,
            tag_list: r.tag_list,
            created_at: r.created_at,
            updated_at: r.updated_at,
            favorited: r.favorited,
            favorites_count: r.favorites_count,
            author: Profile { username: r.username, bio: r.bio, image: r.image, following: r.following },
        }
    }
}

#[derive(Serialize)]
pub struct ArticleEnvelope {
    pub article: Article,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArticleList {
    pub articles: Vec<Article>,
    pub articles_count: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Comment {
    pub id: i64,
    #[serde(serialize_with = "ser_ts")]
    pub created_at: DateTime<Utc>,
    #[serde(serialize_with = "ser_ts")]
    pub updated_at: DateTime<Utc>,
    pub body: String,
    pub author: Profile,
}

#[derive(Serialize)]
pub struct CommentEnvelope {
    pub comment: Comment,
}

#[derive(Serialize)]
pub struct CommentList {
    pub comments: Vec<Comment>,
}

#[derive(Serialize)]
pub struct TagList {
    pub tags: Vec<String>,
}

macro_rules! article_select {
    ($body:literal) => {
        concat!(
            "SELECT a.id, a.slug, a.title, a.description, ",
            $body,
            "a.created_at, a.updated_at,
       ARRAY(SELECT t.name FROM article_tags at JOIN tags t ON t.id = at.tag_id
              WHERE at.article_id = a.id ORDER BY t.name) AS tag_list,
       (SELECT count(*) FROM favorites f WHERE f.article_id = a.id) AS favorites_count,
       EXISTS (SELECT 1 FROM favorites f WHERE f.article_id = a.id AND f.user_id = $1) AS favorited,
       u.username, u.bio, u.image,
       EXISTS (SELECT 1 FROM follows fo WHERE fo.follower_id = $1 AND fo.followee_id = a.author_id) AS following
  FROM articles a
  JOIN users u ON u.id = a.author_id"
        )
    };
}

macro_rules! article_list_select {
    () => {
        crate::model::article_select!("")
    };
}

macro_rules! article_full_select {
    () => {
        crate::model::article_select!("a.body, ")
    };
}

macro_rules! order_page {
    () => {
        " ORDER BY a.id DESC LIMIT $2 OFFSET $3"
    };
}

pub(crate) use {article_full_select, article_list_select, article_select, order_page};

pub const ARTICLE_BY_ID: &str = concat!(article_full_select!(), " WHERE a.id = $2");
