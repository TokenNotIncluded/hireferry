use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;

pub type AppResult<T> = Result<T, AppError>;

#[derive(Debug)]
pub struct AppError {
    pub status: StatusCode,
    pub message: String,
}
impl AppError {
    pub fn bad_request(m: impl ToString) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: m.to_string(),
        }
    }
    pub fn unauthorized(m: impl ToString) -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            message: m.to_string(),
        }
    }
    pub fn forbidden(m: impl ToString) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            message: m.to_string(),
        }
    }
    pub fn not_found(m: impl ToString) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: m.to_string(),
        }
    }
    pub fn conflict(m: impl ToString) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: m.to_string(),
        }
    }
    pub fn unavailable(m: impl ToString) -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            message: m.to_string(),
        }
    }
    pub fn internal(e: impl std::fmt::Display) -> Self {
        tracing::error!(error = %e, "request failed");
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: "Internal error. Please try again.".into(),
        }
    }
}
impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for AppError {}
impl From<rusqlite::Error> for AppError {
    fn from(e: rusqlite::Error) -> Self {
        Self::internal(e)
    }
}
impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        Self::bad_request(e)
    }
}
impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({"error": self.message}))).into_response()
    }
}
