use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use responses_api_store_client::{response_id_from_value, StoredResponse};
use serde_json::Value;
use tracing::error;

use crate::{background, error::response_not_found, state::AppState};

pub async fn load_stored_response(
    state: &AppState,
    response_id: &str,
) -> std::result::Result<StoredResponse, Response> {
    if !state.responses_api_store_enabled {
        return Err(response_not_found(response_id));
    }

    let Some(response_store) = &state.response_store else {
        error!("responses API store is enabled but no response store is configured");
        return Err((StatusCode::BAD_GATEWAY, "response id store unavailable").into_response());
    };

    match response_store
        .get(response_id, state.background_jobs_enabled)
        .await
    {
        Ok(Some(response)) => {
            if background::stored_response_status(&response) == Some("deleted") {
                return Err(response_not_found(response_id));
            }
            Ok(response)
        }
        Ok(None) => Err(response_not_found(response_id)),
        Err(e) => {
            error!("failed to read response id store for {response_id}: {e}");
            Err((StatusCode::BAD_GATEWAY, "response id store read failed").into_response())
        }
    }
}

pub async fn load_response(
    state: &AppState,
    response_id: &str,
) -> std::result::Result<StoredResponse, Response> {
    load_stored_response(state, response_id).await
}

pub async fn store_response(
    state: &AppState,
    upstream: String,
    response: Value,
    input: Vec<Value>,
) {
    if !state.responses_api_store_enabled {
        return;
    }

    let Some(response_store) = &state.response_store else {
        error!("responses API store is enabled but no response store is configured");
        return;
    };

    let Some(response_id) = response_id_from_value(&response) else {
        return;
    };
    let stored = StoredResponse {
        upstream,
        response,
        input,
        pending_upstream_request: None,
        upstream_authorization: None,
        enqueued_at: None,
    };
    if let Err(e) = response_store.store(&response_id, &stored).await {
        error!("failed to store response {response_id}: {e}");
    }
}
