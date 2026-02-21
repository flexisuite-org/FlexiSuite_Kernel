use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};

#[derive(serde::Serialize)]
pub struct JsonError {
    pub error: JsonErrorBody,
}

#[derive(serde::Serialize)]
pub struct JsonErrorBody {
    pub code: u16,
    pub message: String,
    pub timestamp: String,
}

pub fn build_json_error_response(message: impl Into<String>, status: StatusCode) -> Response {
    let now = chrono::Utc::now().to_rfc3339();
    let body = JsonError {
        error: JsonErrorBody {
            code: status.as_u16(),
            message: message.into(),
            timestamp: now,
        },
    };
    (status, Json(body)).into_response()
}
