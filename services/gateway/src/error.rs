use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;

#[derive(Serialize)]
pub struct ErrorResponse {
    pub error: ErrorBody,
}

#[derive(Serialize)]
pub struct ErrorBody {
    pub message: String,
    #[serde(rename = "type")]
    pub error_type: &'static str,
    pub param: &'static str,
    pub code: u16,
}

pub fn response_not_found(response_id: &str) -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(ErrorResponse {
            error: ErrorBody {
                message: format!("Response with id '{response_id}' not found."),
                error_type: "invalid_request_error",
                param: "response_id",
                code: 404,
            },
        }),
    )
        .into_response()
}

pub fn previous_response_not_ready() -> Response {
    (
        StatusCode::CONFLICT,
        Json(ErrorResponse {
            error: ErrorBody {
                message: "Previous response is not ready.".to_string(),
                error_type: "invalid_request_error",
                param: "previous_response_id",
                code: 409,
            },
        }),
    )
        .into_response()
}
