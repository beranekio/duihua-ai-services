use axum::response::IntoResponse;
use duihua_common::StoredResponse;
use serde_json::Value;
use tracing::error;

use crate::{queue::unix_seconds_now, state::AppState};

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

    let stored = StoredResponse {
        upstream: upstream.clone(),
        response: queued_response.clone(),
        input,
        pending_upstream_request: Some(upstream_request),
        upstream_authorization,
        enqueued_at: Some(unix_seconds_now()),
    };
    if let Err(e) = response_store.store(&response_id, &stored).await {
        error!("failed to store queued background response {response_id}: {e}");
        return Err((
            axum::http::StatusCode::BAD_GATEWAY,
            "response id store write failed",
        )
            .into_response());
    }

    let Some(background_queue) = &state.background_queue else {
        error!("background responses require queue support");
        let _ = response_store.delete(&response_id).await;
        return Err((
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "background responses require queue support",
        )
            .into_response());
    };

    if let Err(e) = background_queue.enqueue(&response_id).await {
        error!("failed to enqueue background response {response_id}: {e}");
        let _ = response_store.delete(&response_id).await;
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
    stored: &StoredResponse,
) -> Result<(), axum::response::Response> {
    let Some(response_store) = &state.response_store else {
        error!("responses API store is enabled but no response store is configured");
        return Err((
            axum::http::StatusCode::BAD_GATEWAY,
            "response id store unavailable",
        )
            .into_response());
    };

    if duihua_common::is_in_flight_background(stored) {
        let mut tombstone = stored.clone();
        tombstone.response = serde_json::json!({
            "id": response_id,
            "object": "response",
            "status": "deleted",
            "background": true,
            "deleted": true
        });
        tombstone.pending_upstream_request = None;
        tombstone.upstream_authorization = None;
        if let Err(e) = response_store.store(response_id, &tombstone).await {
            error!("failed to tombstone deleted background response {response_id}: {e}");
            return Err((
                axum::http::StatusCode::BAD_GATEWAY,
                "response store write failed",
            )
                .into_response());
        }
        return Ok(());
    }

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

pub use duihua_common::{
    build_cancelled_response, build_queued_response, build_upstream_request, generate_response_id,
    is_in_flight_background, stored_response_status,
};
