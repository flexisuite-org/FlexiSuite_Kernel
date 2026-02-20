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
    let body = JsonError {
        error: message.into(),
        details: None,
    };
    (status, Json(body)).into_response()
}
