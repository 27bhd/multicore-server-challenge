//! Request-body and query-string helpers with the 422 behavior SPEC.md requires.

use axum::{
    body::Bytes,
    extract::{FromRequest, Request},
};
use serde_json::{Map, Value};

use crate::error::AppError;

/// A JSON request body. Anything that isn't valid JSON is 422.
pub struct JsonBody(pub Value);

impl<S: Send + Sync> FromRequest<S> for JsonBody {
    type Rejection = AppError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let bytes = Bytes::from_request(req, state)
            .await
            .map_err(|_| AppError::Unprocessable("could not read body"))?;
        serde_json::from_slice(&bytes).map(JsonBody).map_err(|_| AppError::Unprocessable("malformed JSON body"))
    }
}

impl JsonBody {
    /// The object under `key` (e.g. `{"user": {...}}`), or 422.
    pub fn object(&self, key: &str) -> Result<&Map<String, Value>, AppError> {
        self.0.get(key).and_then(Value::as_object).ok_or(AppError::Unprocessable("missing or invalid request object"))
    }
}

/// A required, non-empty string field.
pub fn required_str(obj: &Map<String, Value>, key: &str) -> Result<String, AppError> {
    match obj.get(key) {
        Some(Value::String(s)) if !s.is_empty() => Ok(s.clone()),
        _ => Err(AppError::Unprocessable("required field missing or empty")),
    }
}

/// An optional string field: absent or null is `None`; any other non-string is 422,
/// and so is an empty string when `non_empty` is set.
pub fn optional_str(obj: &Map<String, Value>, key: &str, non_empty: bool) -> Result<Option<String>, AppError> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if !(non_empty && s.is_empty()) => Ok(Some(s.clone())),
        _ => Err(AppError::Unprocessable("field must be a non-empty string")),
    }
}

/// ASCII digits only (no sign, no spaces) that fit in an i64.
pub fn parse_uint(s: &str) -> Option<i64> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// First value of each query parameter (like Go's `url.Values.Get`).
pub struct QueryParams(Vec<(String, String)>);

impl QueryParams {
    pub fn parse(query: Option<&str>) -> Self {
        QueryParams(
            form_urlencoded::parse(query.unwrap_or("").as_bytes()).map(|(k, v)| (k.into_owned(), v.into_owned())).collect(),
        )
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    /// A non-empty filter value.
    pub fn filter(&self, key: &str) -> Option<&str> {
        self.get(key).filter(|v| !v.is_empty())
    }

    /// `limit` (default 20, 1..=100) and `offset` (default 0, >= 0); anything else is 422.
    pub fn page(&self) -> Result<(i64, i64), AppError> {
        let limit = match self.get("limit") {
            None => 20,
            Some(v) => parse_uint(v).filter(|n| (1..=100).contains(n)).ok_or(AppError::Unprocessable("invalid limit"))?,
        };
        let offset = match self.get("offset") {
            None => 0,
            Some(v) => parse_uint(v).ok_or(AppError::Unprocessable("invalid offset"))?,
        };
        Ok((limit, offset))
    }
}
