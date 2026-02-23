use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};

#[derive(serde::Serialize)]
pub struct JsonError {
    // Backward-compatible field: keep `error` as a string.
    pub error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<JsonErrorBody>,
}

#[derive(serde::Serialize)]
pub struct JsonErrorBody {
    pub code: u16,
    pub message: String,
    pub timestamp: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

pub fn build_json_error_response(
    message: impl Into<String>,
    status: StatusCode,
    request_id: Option<String>,
) -> Response {
    let message = message.into();
    let now = chrono::Utc::now().to_rfc3339();
    let body = JsonError {
        error: message.clone(),
        details: Some(JsonErrorBody {
            code: status.as_u16(),
            message,
            timestamp: now,
            request_id,
        }),
    };
    (status, Json(body)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt;

    #[tokio::test]
    async fn test_json_error_keeps_legacy_error_field() {
        let response = build_json_error_response(
            "Action not found",
            StatusCode::NOT_FOUND,
            Some("req-123".to_string()),
        );

        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(json["error"], "Action not found");
        assert_eq!(json["details"]["code"], 404);
        assert_eq!(json["details"]["message"], "Action not found");
        assert_eq!(json["details"]["request_id"], "req-123");
        assert!(json["details"]["timestamp"].is_string());
    }
}
