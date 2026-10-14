"""SQL from SPEC.md, verbatim (asyncpg uses $n natively)."""

ARTICLE_LIST_SELECT = """
SELECT a.id, a.slug, a.title, a.description, a.created_at, a.updated_at,
       ARRAY(SELECT t.name FROM article_tags at JOIN tags t ON t.id = at.tag_id
              WHERE at.article_id = a.id ORDER BY t.name) AS tag_list,
       (SELECT count(*) FROM favorites f WHERE f.article_id = a.id) AS favorites_count,
       EXISTS (SELECT 1 FROM favorites f WHERE f.article_id = a.id AND f.user_id = $1) AS favorited,
       u.username, u.bio, u.image,
       EXISTS (SELECT 1 FROM follows fo WHERE fo.follower_id = $1 AND fo.followee_id = a.author_id) AS following
  FROM articles a
  JOIN users u ON u.id = a.author_id
"""

ARTICLE_FULL_SELECT = """
SELECT a.id, a.slug, a.title, a.description, a.body, a.created_at, a.updated_at,
       ARRAY(SELECT t.name FROM article_tags at JOIN tags t ON t.id = at.tag_id
              WHERE at.article_id = a.id ORDER BY t.name) AS tag_list,
       (SELECT count(*) FROM favorites f WHERE f.article_id = a.id) AS favorites_count,
       EXISTS (SELECT 1 FROM favorites f WHERE f.article_id = a.id AND f.user_id = $1) AS favorited,
       u.username, u.bio, u.image,
       EXISTS (SELECT 1 FROM follows fo WHERE fo.follower_id = $1 AND fo.followee_id = a.author_id) AS following
  FROM articles a
  JOIN users u ON u.id = a.author_id
"""

ORDER_PAGE = " ORDER BY a.id DESC LIMIT $2 OFFSET $3"

# Users and auth
INSERT_USER = """INSERT INTO users (username, email, password_hash) VALUES ($1, $2, $3)
RETURNING id, username, email, bio, image"""
LOGIN_USER = "SELECT id, username, email, bio, image, password_hash FROM users WHERE email = $1"
GET_USER = "SELECT id, username, email, bio, image FROM users WHERE id = $1"
UPDATE_USER = """UPDATE users SET email = COALESCE($2, email), username = COALESCE($3, username),
                 password_hash = COALESCE($4, password_hash), bio = COALESCE($5, bio), image = COALESCE($6, image)
 WHERE id = $1
RETURNING id, username, email, bio, image"""

# Profiles
GET_PROFILE = """SELECT u.id, u.username, u.bio, u.image,
       EXISTS (SELECT 1 FROM follows fo WHERE fo.follower_id = $1 AND fo.followee_id = u.id) AS following
  FROM users u WHERE u.username = $2"""
USER_BY_USERNAME = "SELECT id, username, bio, image FROM users WHERE username = $1"
FOLLOW = "INSERT INTO follows (follower_id, followee_id) VALUES ($1, $2) ON CONFLICT DO NOTHING"
UNFOLLOW = "DELETE FROM follows WHERE follower_id = $1 AND followee_id = $2"

# Article lists: (page, count) per filter
LIST_NONE = (
    ARTICLE_LIST_SELECT + ORDER_PAGE,
    "SELECT count(*) FROM (SELECT 1 FROM articles a LIMIT 1000) s",
)
LIST_TAG = (
    ARTICLE_LIST_SELECT
    + " WHERE a.id IN (SELECT at.article_id FROM article_tags at WHERE at.tag_id = (SELECT t.id FROM tags t WHERE t.name = $4) ORDER BY at.article_id DESC LIMIT $2 OFFSET $3)"
    + " ORDER BY a.id DESC",
    "SELECT count(*) FROM (SELECT 1 FROM article_tags at WHERE at.tag_id = (SELECT t.id FROM tags t WHERE t.name = $1) LIMIT 1000) s",
)
LIST_AUTHOR = (
    ARTICLE_LIST_SELECT + " WHERE a.author_id = (SELECT au.id FROM users au WHERE au.username = $4)" + ORDER_PAGE,
    "SELECT count(*) FROM (SELECT 1 FROM articles a WHERE a.author_id = (SELECT u.id FROM users u WHERE u.username = $1) LIMIT 1000) s",
)
LIST_FAVORITED = (
    ARTICLE_LIST_SELECT
    + " WHERE a.id IN (SELECT f.article_id FROM favorites f WHERE f.user_id = (SELECT fu.id FROM users fu WHERE fu.username = $4) ORDER BY f.article_id DESC LIMIT $2 OFFSET $3)"
    + " ORDER BY a.id DESC",
    "SELECT count(*) FROM (SELECT 1 FROM favorites f WHERE f.user_id = (SELECT fu.id FROM users fu WHERE fu.username = $1) LIMIT 1000) s",
)
FEED = (
    ARTICLE_LIST_SELECT
    + " WHERE a.id IN (SELECT x.id FROM follows fo CROSS JOIN LATERAL (SELECT a2.id FROM articles a2 WHERE a2.author_id = fo.followee_id ORDER BY a2.id DESC LIMIT ($2::int + $3::int)) x WHERE fo.follower_id = $1 ORDER BY x.id DESC LIMIT $2 OFFSET $3)"
    + " ORDER BY a.id DESC",
    "SELECT count(*) FROM (SELECT 1 FROM articles a WHERE a.author_id IN (SELECT fo.followee_id FROM follows fo WHERE fo.follower_id = $1) LIMIT 1000) s",
)

ARTICLE_BY_SLUG = ARTICLE_FULL_SELECT + " WHERE a.slug = $2"
ARTICLE_BY_ID = ARTICLE_FULL_SELECT + " WHERE a.id = $2"

INSERT_ARTICLE = """INSERT INTO articles (id, slug, author_id, title, description, body)
SELECT n.id, trim(both '-' from lower(regexp_replace($2, '[^A-Za-z0-9]+', '-', 'g'))) || '-' || n.id, $1, $2, $3, $4
  FROM (SELECT nextval(pg_get_serial_sequence('articles', 'id')) AS id) n
RETURNING id"""
INSERT_TAGS = "INSERT INTO tags (name) SELECT DISTINCT unnest($1::text[]) ON CONFLICT (name) DO NOTHING"
INSERT_ARTICLE_TAGS = (
    "INSERT INTO article_tags (article_id, tag_id) SELECT $1, t.id FROM tags t WHERE t.name = ANY($2::text[])"
)

ARTICLE_OWNER = "SELECT id, author_id FROM articles WHERE slug = $1"
UPDATE_ARTICLE = """UPDATE articles SET title = COALESCE($2, title), description = COALESCE($3, description), body = COALESCE($4, body),
       slug = CASE WHEN $2::text IS NULL THEN slug ELSE trim(both '-' from lower(regexp_replace($2, '[^A-Za-z0-9]+', '-', 'g'))) || '-' || id END,
       updated_at = now()
 WHERE id = $1"""
DELETE_ARTICLE = "DELETE FROM articles WHERE id = $1"

# Favorites
ARTICLE_ID = "SELECT id FROM articles WHERE slug = $1"
FAVORITE = "INSERT INTO favorites (user_id, article_id) VALUES ($1, $2) ON CONFLICT DO NOTHING"
UNFAVORITE = "DELETE FROM favorites WHERE user_id = $1 AND article_id = $2"

# Comments
LIST_COMMENTS = """SELECT c.id, c.body, c.created_at, c.updated_at, u.username, u.bio, u.image,
       EXISTS (SELECT 1 FROM follows fo WHERE fo.follower_id = $1 AND fo.followee_id = c.author_id) AS following
  FROM comments c JOIN users u ON u.id = c.author_id
 WHERE c.article_id = $2
 ORDER BY c.created_at DESC, c.id DESC LIMIT 100"""
INSERT_COMMENT = """WITH c AS (INSERT INTO comments (article_id, author_id, body) VALUES ($1, $2, $3)
           RETURNING id, body, created_at, updated_at, author_id)
SELECT c.id, c.body, c.created_at, c.updated_at, u.username, u.bio, u.image
  FROM c JOIN users u ON u.id = c.author_id"""
COMMENT_OWNER = """SELECT c.author_id FROM comments c JOIN articles a ON a.id = c.article_id
 WHERE c.id = $1 AND a.slug = $2"""
DELETE_COMMENT = "DELETE FROM comments WHERE id = $1"

# Tags
TAGS = "SELECT name FROM tags ORDER BY name"
