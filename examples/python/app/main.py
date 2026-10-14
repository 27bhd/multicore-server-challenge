"""Conduit (RealWorld) API: FastAPI + asyncpg. Contract: SPEC.md."""

import os
import time
from contextlib import asynccontextmanager
from datetime import datetime, timezone
from typing import Annotated

import asyncpg
import bcrypt
import jwt
from fastapi import Depends, FastAPI, Request
from fastapi.exceptions import RequestValidationError
from fastapi.responses import JSONResponse, Response
from pydantic import BaseModel, Field
from starlette.concurrency import run_in_threadpool
from starlette.exceptions import HTTPException as StarletteHTTPException

from . import sql

DATABASE_URL = os.environ["DATABASE_URL"]
JWT_SECRET = os.environ["JWT_SECRET"]
DB_POOL_SIZE = int(os.environ.get("DB_POOL_SIZE") or 10)

TOKEN_TTL_S = 30 * 24 * 3600
BCRYPT_COST = 10
INT64_MAX = 2**63 - 1


# ---------------------------------------------------------------------------- app, pool, errors


@asynccontextmanager
async def lifespan(app: FastAPI):
    # One pool per Gunicorn worker process. asyncpg's default min_size (10) is above our max_size,
    # so min is clamped to max: every connection opens at boot, as with asyncpg's defaults.
    app.state.pool = await asyncpg.create_pool(
        DATABASE_URL, min_size=DB_POOL_SIZE, max_size=DB_POOL_SIZE, command_timeout=30
    )
    yield
    await app.state.pool.close()


app = FastAPI(lifespan=lifespan, docs_url=None, redoc_url=None, openapi_url=None)


class ApiError(Exception):
    def __init__(self, status_code: int, message: str):
        self.status_code = status_code
        self.message = message


def error_response(status_code: int, message: str) -> JSONResponse:
    return JSONResponse({"errors": {"body": [message]}}, status_code=status_code)


@app.exception_handler(ApiError)
async def api_error_handler(request: Request, exc: ApiError):
    return error_response(exc.status_code, exc.message)


@app.exception_handler(StarletteHTTPException)
async def http_error_handler(request: Request, exc: StarletteHTTPException):
    # Router-level errors: unknown path (404), wrong method (405).
    return error_response(exc.status_code, str(exc.detail).lower())


@app.exception_handler(RequestValidationError)
async def validation_error_handler(request: Request, exc: RequestValidationError):
    err = exc.errors()[0]
    where = ".".join(str(p) for p in err["loc"][1:]) or err["loc"][0]
    return error_response(422, f"{where}: {err['msg']}")


def get_pool(request: Request) -> asyncpg.Pool:
    return request.app.state.pool


Pool = Annotated[asyncpg.Pool, Depends(get_pool)]


# ---------------------------------------------------------------------------- auth


class Auth:
    __slots__ = ("user_id", "token")

    def __init__(self, user_id: int, token: str | None):
        self.user_id = user_id  # 0 = anonymous
        self.token = token


ANONYMOUS = Auth(0, None)


def new_token(user_id: int) -> str:
    now = int(time.time())
    return jwt.encode({"sub": str(user_id), "iat": now, "exp": now + TOKEN_TTL_S}, JWT_SECRET, algorithm="HS256")


def parse_authorization(header: str) -> Auth:
    if not header.startswith("Token "):
        raise ApiError(401, "invalid authorization header")
    token = header[6:]
    try:
        claims = jwt.decode(
            token, JWT_SECRET, algorithms=["HS256"], options={"require": ["exp", "sub"], "verify_iat": False}
        )
    except jwt.PyJWTError:
        raise ApiError(401, "invalid or expired token")
    sub = claims["sub"]
    user_id = parse_uint(sub) if isinstance(sub, str) else None
    if not user_id:  # None (malformed) or 0
        raise ApiError(401, "invalid token subject")
    return Auth(user_id, token)


async def auth_required(request: Request) -> Auth:
    header = request.headers.get("authorization")
    if not header:
        raise ApiError(401, "authorization required")
    return parse_authorization(header)


async def auth_optional(request: Request) -> Auth:
    header = request.headers.get("authorization")
    if not header:  # an empty header counts as absent
        return ANONYMOUS
    return parse_authorization(header)


AuthRequired = Annotated[Auth, Depends(auth_required)]
AuthOptional = Annotated[Auth, Depends(auth_optional)]


def hash_password(password: str) -> str:
    try:
        return bcrypt.hashpw(password.encode(), bcrypt.gensalt(BCRYPT_COST)).decode()
    except ValueError:  # bcrypt 5 rejects passwords over 72 bytes
        raise ApiError(422, "password is too long")


def check_password(password: str, password_hash: str) -> bool:
    try:
        return bcrypt.checkpw(password.encode(), password_hash.encode())
    except ValueError:  # bcrypt 5 rejects passwords over 72 bytes
        return False


# ---------------------------------------------------------------------------- request bodies

NonEmpty = Annotated[str, Field(min_length=1)]


class NewUser(BaseModel):
    username: NonEmpty
    email: NonEmpty
    password: NonEmpty


class NewUserBody(BaseModel):
    user: NewUser


class LoginUser(BaseModel):
    email: NonEmpty
    password: NonEmpty


class LoginBody(BaseModel):
    user: LoginUser


class UpdateUser(BaseModel):
    email: NonEmpty | None = None
    username: NonEmpty | None = None
    password: NonEmpty | None = None
    bio: str | None = None
    image: str | None = None


class UpdateUserBody(BaseModel):
    user: UpdateUser


class NewArticle(BaseModel):
    title: NonEmpty
    description: NonEmpty
    body: NonEmpty
    tagList: list[NonEmpty] | None = None


class NewArticleBody(BaseModel):
    article: NewArticle


class UpdateArticle(BaseModel):
    title: NonEmpty | None = None
    description: NonEmpty | None = None
    body: NonEmpty | None = None


class UpdateArticleBody(BaseModel):
    article: UpdateArticle


class NewComment(BaseModel):
    body: NonEmpty


class NewCommentBody(BaseModel):
    comment: NewComment


def parse_uint(s: str) -> int | None:
    """ASCII digits only (no sign, no spaces, no '_'), fitting in a bigint; else None."""
    if not s or not s.isascii() or not s.isdigit():
        return None
    n = int(s)
    return n if n <= INT64_MAX else None


class Page:
    __slots__ = ("limit", "offset")

    def __init__(self, limit: int, offset: int):
        self.limit = limit
        self.offset = offset


def page_params(request: Request) -> Page:
    """limit (default 20, 1-100) and offset (default 0, >= 0). A parameter that is present, even empty,
    must be valid; with repeats, the first value counts."""
    q = request.query_params
    limit, offset = 20, 0
    if "limit" in q:
        limit = parse_uint(q.getlist("limit")[0])
        if limit is None or not 1 <= limit <= 100:
            raise ApiError(422, "limit must be an integer from 1 to 100")
    if "offset" in q:
        offset = parse_uint(q.getlist("offset")[0])
        if offset is None:
            raise ApiError(422, "offset must be an integer >= 0")
    return Page(limit, offset)


PageParams = Annotated[Page, Depends(page_params)]


def first_param(request: Request, name: str) -> str:
    values = request.query_params.getlist(name)
    return values[0] if values else ""


# ---------------------------------------------------------------------------- response shapes


def iso(ts: datetime) -> str:
    """RFC 3339 UTC, exactly 3 fractional digits (truncated), 'Z'."""
    ts = ts.astimezone(timezone.utc)
    return f"{ts:%Y-%m-%dT%H:%M:%S}.{ts.microsecond // 1000:03d}Z"


def user_json(row, token: str) -> dict:
    return {
        "email": row["email"],
        "token": token,
        "username": row["username"],
        "bio": row["bio"],
        "image": row["image"],
    }


def profile_json(row, following: bool) -> dict:
    return {"username": row["username"], "bio": row["bio"], "image": row["image"], "following": following}


def article_json(row, with_body: bool) -> dict:
    article = {
        "slug": row["slug"],
        "title": row["title"],
        "description": row["description"],
    }
    if with_body:
        article["body"] = row["body"]
    article["tagList"] = row["tag_list"]
    article["createdAt"] = iso(row["created_at"])
    article["updatedAt"] = iso(row["updated_at"])
    article["favorited"] = row["favorited"]
    article["favoritesCount"] = row["favorites_count"]
    article["author"] = profile_json(row, row["following"])
    return article


def comment_json(row, following: bool) -> dict:
    return {
        "id": row["id"],
        "createdAt": iso(row["created_at"]),
        "updatedAt": iso(row["updated_at"]),
        "body": row["body"],
        "author": profile_json(row, following),
    }


# ---------------------------------------------------------------------------- users and auth


@app.post("/api/users", status_code=201)
async def register(body: NewUserBody, pool: Pool):
    u = body.user
    password_hash = await run_in_threadpool(hash_password, u.password)
    try:
        row = await pool.fetchrow(sql.INSERT_USER, u.username, u.email, password_hash)
    except asyncpg.UniqueViolationError:
        raise ApiError(422, "username or email already taken")
    return {"user": user_json(row, new_token(row["id"]))}


@app.post("/api/users/login")
async def login(body: LoginBody, pool: Pool):
    row = await pool.fetchrow(sql.LOGIN_USER, body.user.email)
    if row is None or not await run_in_threadpool(check_password, body.user.password, row["password_hash"]):
        raise ApiError(401, "invalid email or password")
    return {"user": user_json(row, new_token(row["id"]))}


@app.get("/api/user")
async def current_user(auth: AuthRequired, pool: Pool):
    row = await pool.fetchrow(sql.GET_USER, auth.user_id)
    if row is None:
        raise ApiError(401, "user not found")
    return {"user": user_json(row, auth.token)}


@app.put("/api/user")
async def update_user(body: UpdateUserBody, auth: AuthRequired, pool: Pool):
    u = body.user
    password_hash = None
    if u.password is not None:
        password_hash = await run_in_threadpool(hash_password, u.password)
    try:
        row = await pool.fetchrow(sql.UPDATE_USER, auth.user_id, u.email, u.username, password_hash, u.bio, u.image)
    except asyncpg.UniqueViolationError:
        raise ApiError(422, "username or email already taken")
    if row is None:
        raise ApiError(401, "user not found")
    return {"user": user_json(row, auth.token)}


# ---------------------------------------------------------------------------- profiles


@app.get("/api/profiles/{username}")
async def get_profile(username: str, auth: AuthOptional, pool: Pool):
    row = await pool.fetchrow(sql.GET_PROFILE, auth.user_id, username)
    if row is None:
        raise ApiError(404, "profile not found")
    return {"profile": profile_json(row, row["following"])}


async def set_follow(username: str, viewer: int, pool: asyncpg.Pool, follow: bool) -> dict:
    async with pool.acquire() as conn:
        row = await conn.fetchrow(sql.USER_BY_USERNAME, username)
        if row is None:
            raise ApiError(404, "profile not found")
        await conn.execute(sql.FOLLOW if follow else sql.UNFOLLOW, viewer, row["id"])
    return {"profile": profile_json(row, follow)}


@app.post("/api/profiles/{username}/follow")
async def follow(username: str, auth: AuthRequired, pool: Pool):
    return await set_follow(username, auth.user_id, pool, True)


@app.delete("/api/profiles/{username}/follow")
async def unfollow(username: str, auth: AuthRequired, pool: Pool):
    return await set_follow(username, auth.user_id, pool, False)


# ---------------------------------------------------------------------------- articles


async def article_page(pool: asyncpg.Pool, queries: tuple[str, str], page_args: list, count_arg) -> dict:
    page_sql, count_sql = queries
    async with pool.acquire() as conn:
        rows = await conn.fetch(page_sql, *page_args)
        count = await conn.fetchval(count_sql, count_arg)
    return {"articles": [article_json(r, False) for r in rows], "articlesCount": count}


@app.get("/api/articles")
async def list_articles(request: Request, auth: AuthOptional, page: PageParams, pool: Pool):
    # The first non-empty filter in the order tag, author, favorited applies.
    viewer, limit, offset = auth.user_id, page.limit, page.offset
    if tag := first_param(request, "tag"):
        return await article_page(pool, sql.LIST_TAG, [viewer, limit, offset, tag], tag)
    if author := first_param(request, "author"):
        return await article_page(pool, sql.LIST_AUTHOR, [viewer, limit, offset, author], author)
    if favorited := first_param(request, "favorited"):
        return await article_page(pool, sql.LIST_FAVORITED, [viewer, limit, offset, favorited], favorited)
    page_sql, count_sql = sql.LIST_NONE
    async with pool.acquire() as conn:
        rows = await conn.fetch(page_sql, viewer, limit, offset)
        count = await conn.fetchval(count_sql)
    return {"articles": [article_json(r, False) for r in rows], "articlesCount": count}


@app.get("/api/articles/feed")
async def feed(auth: AuthRequired, page: PageParams, pool: Pool):
    viewer = auth.user_id
    return await article_page(pool, sql.FEED, [viewer, page.limit, page.offset], viewer)


@app.get("/api/articles/{slug}")
async def get_article(slug: str, auth: AuthOptional, pool: Pool):
    row = await pool.fetchrow(sql.ARTICLE_BY_SLUG, auth.user_id, slug)
    if row is None:
        raise ApiError(404, "article not found")
    return {"article": article_json(row, True)}


@app.post("/api/articles", status_code=201)
async def create_article(body: NewArticleBody, auth: AuthRequired, pool: Pool):
    a = body.article
    viewer = auth.user_id
    async with pool.acquire() as conn:
        async with conn.transaction():
            article_id = await conn.fetchval(sql.INSERT_ARTICLE, viewer, a.title, a.description, a.body)
            if a.tagList:
                await conn.execute(sql.INSERT_TAGS, a.tagList)
                await conn.execute(sql.INSERT_ARTICLE_TAGS, article_id, a.tagList)
        row = await conn.fetchrow(sql.ARTICLE_BY_ID, viewer, article_id)
    return {"article": article_json(row, True)}


async def owned_article_id(conn: asyncpg.Connection, slug: str, viewer: int) -> int:
    row = await conn.fetchrow(sql.ARTICLE_OWNER, slug)
    if row is None:
        raise ApiError(404, "article not found")
    if row["author_id"] != viewer:
        raise ApiError(403, "not the author of this article")
    return row["id"]


@app.put("/api/articles/{slug}")
async def update_article(slug: str, body: UpdateArticleBody, auth: AuthRequired, pool: Pool):
    a = body.article
    viewer = auth.user_id
    async with pool.acquire() as conn:
        article_id = await owned_article_id(conn, slug, viewer)
        await conn.execute(sql.UPDATE_ARTICLE, article_id, a.title, a.description, a.body)
        row = await conn.fetchrow(sql.ARTICLE_BY_ID, viewer, article_id)
    return {"article": article_json(row, True)}


@app.delete("/api/articles/{slug}", status_code=204)
async def delete_article(slug: str, auth: AuthRequired, pool: Pool):
    async with pool.acquire() as conn:
        article_id = await owned_article_id(conn, slug, auth.user_id)
        await conn.execute(sql.DELETE_ARTICLE, article_id)
    return Response(status_code=204)


# ---------------------------------------------------------------------------- favorites


async def set_favorite(slug: str, viewer: int, pool: asyncpg.Pool, favorite: bool) -> dict:
    async with pool.acquire() as conn:
        article_id = await conn.fetchval(sql.ARTICLE_ID, slug)
        if article_id is None:
            raise ApiError(404, "article not found")
        await conn.execute(sql.FAVORITE if favorite else sql.UNFAVORITE, viewer, article_id)
        row = await conn.fetchrow(sql.ARTICLE_BY_ID, viewer, article_id)
    return {"article": article_json(row, True)}


@app.post("/api/articles/{slug}/favorite")
async def favorite(slug: str, auth: AuthRequired, pool: Pool):
    return await set_favorite(slug, auth.user_id, pool, True)


@app.delete("/api/articles/{slug}/favorite")
async def unfavorite(slug: str, auth: AuthRequired, pool: Pool):
    return await set_favorite(slug, auth.user_id, pool, False)


# ---------------------------------------------------------------------------- comments


@app.get("/api/articles/{slug}/comments")
async def list_comments(slug: str, auth: AuthOptional, pool: Pool):
    async with pool.acquire() as conn:
        article_id = await conn.fetchval(sql.ARTICLE_ID, slug)
        if article_id is None:
            raise ApiError(404, "article not found")
        rows = await conn.fetch(sql.LIST_COMMENTS, auth.user_id, article_id)
    return {"comments": [comment_json(r, r["following"]) for r in rows]}


@app.post("/api/articles/{slug}/comments", status_code=201)
async def add_comment(slug: str, body: NewCommentBody, auth: AuthRequired, pool: Pool):
    async with pool.acquire() as conn:
        article_id = await conn.fetchval(sql.ARTICLE_ID, slug)
        if article_id is None:
            raise ApiError(404, "article not found")
        row = await conn.fetchrow(sql.INSERT_COMMENT, article_id, auth.user_id, body.comment.body)
    return {"comment": comment_json(row, False)}


@app.delete("/api/articles/{slug}/comments/{comment_id}", status_code=204)
async def delete_comment(slug: str, comment_id: str, auth: AuthRequired, pool: Pool):
    cid = parse_uint(comment_id)
    if cid is None:
        raise ApiError(404, "comment not found")
    async with pool.acquire() as conn:
        author_id = await conn.fetchval(sql.COMMENT_OWNER, cid, slug)
        if author_id is None:
            raise ApiError(404, "comment not found")
        if author_id != auth.user_id:
            raise ApiError(403, "not the author of this comment")
        await conn.execute(sql.DELETE_COMMENT, cid)
    return Response(status_code=204)


# ---------------------------------------------------------------------------- tags


@app.get("/api/tags")
async def tags(pool: Pool):
    rows = await pool.fetch(sql.TAGS)
    return {"tags": [r["name"] for r in rows]}
