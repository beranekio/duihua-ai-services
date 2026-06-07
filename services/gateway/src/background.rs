use axum::response::IntoResponse;
use responses_api_store_client::StoredResponse;
use serde_json::Value;
use tracing::error;

use crate::state::AppState;

pub async fn enqueue_background_response(
    state: &AppState,
    response_id: String,
    upstream: String,
    input: Vec<Value>,
    upstream_request: Value,
    queued_response: Value,
    upstream_authorization: Option<String>,
) -> Result<(), axum::response::Response> {
    let Some(response_store) = &state.response_store else {
        error!("responses API store is enabled but no response store is configured");
        return Err((
            axum::http::StatusCode::BAD_GATEWAY,
            "response id store unavailable",
        )
            .into_response());
    };

    if !state.background_jobs_enabled {
        error!("background responses require queue support");
        return Err((
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "background responses require queue support",
        )
            .into_response());
    }

    let stored = StoredResponse {
        upstream,
        response: queued_response,
        input,
        pending_upstream_request: Some(upstream_request),
        upstream_authorization,
        enqueued_at: None,
    };
    if let Err(e) = response_store
        .enqueue_background_job(&response_id, &stored)
        .await
    {
        error!("failed to enqueue background response {response_id}: {e}");
        return Err((
            axum::http::StatusCode::BAD_GATEWAY,
            "failed to enqueue background response",
        )
            .into_response());
    }

    Ok(())
}

pub async fn finalize_background_deletion(
    state: &AppState,
    response_id: &str,
    _stored: &StoredResponse,
) -> Result<(), axum::response::Response> {
    let Some(response_store) = &state.response_store else {
        error!("responses API store is enabled but no response store is configured");
        return Err((
            axum::http::StatusCode::BAD_GATEWAY,
            "response id store unavailable",
        )
            .into_response());
    };

    if let Err(e) = response_store.delete(response_id).await {
        error!("failed to delete response {response_id}: {e}");
        return Err((
            axum::http::StatusCode::BAD_GATEWAY,
            "response store delete failed",
        )
            .into_response());
    }

    Ok(())
}

pub use responses_api_store_client::{
    build_cancelled_response, build_queued_response, build_upstream_request, generate_response_id,
    is_in_flight_background, stored_response_status,
};
