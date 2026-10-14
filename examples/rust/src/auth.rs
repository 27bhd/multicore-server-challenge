//! `Authorization: Token <jwt>` (RealWorld's scheme). HS256, claims {sub, iat, exp}; verified on every request.

use axum::{
    extract::FromRequestParts,
    http::{header::AUTHORIZATION, request::Parts},
};
use jsonwebtoken::{Header, decode, encode};
use serde::{Deserialize, Serialize};

use crate::{SharedState, error::AppError, util::parse_uint};

const TOKEN_TTL_SECS: u64 = 30 * 24 * 60 * 60;

#[derive(Serialize)]
struct Claims {
    sub: String,
    iat: u64,
    exp: u64,
}

/// What verification reads: `exp` (and `nbf`, if present) are checked by `Validation`; a missing `sub` is "".
#[derive(Deserialize)]
struct VerifiedClaims {
    #[serde(default)]
    sub: String,
}

pub fn issue_token(state: &SharedState, user_id: i64) -> Result<String, AppError> {
    let now = jsonwebtoken::get_current_timestamp();
    let claims = Claims { sub: user_id.to_string(), iat: now, exp: now + TOKEN_TTL_SECS };
    encode(&Header::default(), &claims, &state.enc_key).map_err(|e| AppError::Internal(e.to_string()))
}

/// An authenticated request: the user id from `sub` and the raw token it presented.
pub struct AuthUser {
    pub id: i64,
    pub token: String,
}

/// Auth-optional endpoints: `None` when there is no Authorization header; an invalid header is still 401.
pub struct MaybeAuth(pub Option<AuthUser>);

impl MaybeAuth {
    pub fn viewer(&self) -> i64 {
        self.0.as_ref().map_or(0, |u| u.id)
    }
}

fn verify(parts: &Parts, state: &SharedState) -> Result<Option<AuthUser>, AppError> {
    // A missing or empty header means anonymous.
    let Some(value) = parts.headers.get(AUTHORIZATION).filter(|v| !v.is_empty()) else { return Ok(None) };
    let token = value.to_str().ok().and_then(|v| v.strip_prefix("Token ")).ok_or(AppError::Unauthorized)?;
    let claims = decode::<VerifiedClaims>(token, &state.dec_key, &state.validation)
        .map_err(|_| AppError::Unauthorized)?
        .claims;
    let id = parse_uint(&claims.sub).filter(|id| *id > 0).ok_or(AppError::Unauthorized)?;
    Ok(Some(AuthUser { id, token: token.to_string() }))
}

impl FromRequestParts<SharedState> for AuthUser {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &SharedState) -> Result<Self, Self::Rejection> {
        verify(parts, state)?.ok_or(AppError::Unauthorized)
    }
}

impl FromRequestParts<SharedState> for MaybeAuth {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &SharedState) -> Result<Self, Self::Rejection> {
        verify(parts, state).map(MaybeAuth)
    }
}
