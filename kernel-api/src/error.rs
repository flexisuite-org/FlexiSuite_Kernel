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

pub fn build_json_error_response_with_details(
    status: StatusCode,
    message: impl Into<String>,
    details: serde_json::Value,
) -> Response {
    let body = build_json_error_with_details(message, Some(details));
    (status, Json(body)).into_response()
}

pub fn build_json_error(message: impl Into<String>) -> JsonError {
    let body = JsonError {
        error: message.into(),
        details: None,
    };
    body
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
