use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use crate::application::auth::AuthError;
use crate::application::ports::RepositoryError;
use crate::domain::error::DomainError;

/// Uniform error envelope: `{"error": {"code": "...", "message": "..."}}`.
/// Internal details (raw DB errors) are logged, never leaked to clients.
pub struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl ApiError {
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self { status: StatusCode::BAD_REQUEST, code: "bad_request", message: message.into() }
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self { status: StatusCode::UNAUTHORIZED, code: "unauthorized", message: message.into() }
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self { status: StatusCode::FORBIDDEN, code: "forbidden", message: message.into() }
    }

    pub fn too_many_requests(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::TOO_MANY_REQUESTS,
            code: "rate_limited",
            message: message.into(),
        }
    }

    pub fn not_found(what: &str) -> Self {
        Self { status: StatusCode::NOT_FOUND, code: "not_found", message: format!("{what} not found") }
    }

    pub fn service_unavailable(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            code: "service_unavailable",
            message: message.into(),
        }
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self { status: StatusCode::CONFLICT, code: "conflict", message: message.into() }
    }

    pub fn internal(logged: impl std::fmt::Display) -> Self {
        tracing::error!("Internal error: {logged}");
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "internal_error",
            message: "internal error".into(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = json!({ "error": { "code": self.code, "message": self.message } });
        (self.status, Json(body)).into_response()
    }
}

impl From<RepositoryError> for ApiError {
    fn from(e: RepositoryError) -> Self {
        match e {
            RepositoryError::NotFound => ApiError::not_found("resource"),
            RepositoryError::Conflict(msg) => ApiError::conflict(msg),
            RepositoryError::InvalidData(msg) => ApiError::bad_request(msg),
            other => ApiError::internal(other),
        }
    }
}

impl From<AuthError> for ApiError {
    fn from(e: AuthError) -> Self {
        match e {
            AuthError::InvalidCredentials => ApiError::unauthorized("invalid credentials"),
            AuthError::EmailExists => ApiError::conflict("an account with this email already exists"),
            AuthError::InvalidInput(msg) => ApiError::bad_request(msg),
            AuthError::TokenInvalid => ApiError::unauthorized("invalid or expired session"),
            other => ApiError::internal(other),
        }
    }
}

impl From<DomainError> for ApiError {
    fn from(e: DomainError) -> Self {
        ApiError::bad_request(e.to_string())
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        ApiError::internal(e)
    }
}
