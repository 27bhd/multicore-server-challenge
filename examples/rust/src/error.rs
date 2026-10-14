use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;

#[derive(Serialize)]
struct ErrorBody {
    errors: ErrorList,
}

#[derive(Serialize)]
struct ErrorList {
    body: [&'static str; 1],
}

/// Every error the API returns, rendered as `{"errors":{"body":["<message>"]}}`.
#[derive(Debug)]
pub enum AppError {
    Unauthorized,
    Forbidden,
    NotFound,
    Unprocessable(&'static str),
    Internal(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            AppError::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
            AppError::Forbidden => (StatusCode::FORBIDDEN, "forbidden"),
            AppError::NotFound => (StatusCode::NOT_FOUND, "not found"),
            AppError::Unprocessable(msg) => (StatusCode::UNPROCESSABLE_ENTITY, msg),
            AppError::Internal(err) => {
                eprintln!("[error] {err}");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal server error")
            }
        };
        (status, Json(ErrorBody { errors: ErrorList { body: [message] } })).into_response()
    }
}

impl From<sqlx::Error> for AppError {
    fn from(err: sqlx::Error) -> Self {
        AppError::Internal(err.to_string())
    }
}

/// Maps a unique violation (SQLSTATE 23505) to 422, anything else to 500.
pub fn unique_to_422(err: sqlx::Error, msg: &'static str) -> AppError {
    match err.as_database_error().and_then(|e| e.code()) {
        Some(code) if code == "23505" => AppError::Unprocessable(msg),
        _ => AppError::from(err),
    }
}

pub type AppResult<T> = Result<T, AppError>;
