use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;

#[derive(Serialize)]
pub struct JsonError {
    pub error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

pub fn build_json_error_response(status: StatusCode, message: impl Into<String>) -> Response {
    let body = build_json_error(message);
    (status, Json(body)).into_response()
}

pub fn build_json_error(message: impl Into<String>) -> JsonError {
    JsonError {
        error: message.into(),
        details: None,
    }
}

pub fn build_json_error_with_details(
    message: impl Into<String>,
    details: Option<serde_json::Value>,
) -> JsonError {
    JsonError {
        error: message.into(),
        details,
    }
}
