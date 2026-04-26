use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("No data loaded. Upload a CSV first.")]
    NoData,
    #[error("CSV parse error: {0}")]
    CsvParse(String),
    #[error("Activity not found: {0}")]
    NotFound(u64),
    #[error("Invalid parameter: {0}")]
    BadRequest(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, msg) = match &self {
            AppError::NoData => (StatusCode::BAD_REQUEST, self.to_string()),
            AppError::CsvParse(_) => (StatusCode::UNPROCESSABLE_ENTITY, self.to_string()),
            AppError::NotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            AppError::BadRequest(_) => (StatusCode::BAD_REQUEST, self.to_string()),
        };
        let body = axum::Json(json!({ "error": msg }));
        (status, body).into_response()
    }
}
