use std::sync::Arc;

use axum::{extract::State, http::HeaderMap, response::Response, Json};

use crate::{
    models::ChatCompletionRequest,
    state::{upstream_for_model, AppState},
    upstream::proxy_request,
};

pub async fn chat_completions(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(mut payload): Json<ChatCompletionRequest>,
) -> Response {
    if payload.model.is_none() {
        payload.model = Some(state.default_model.clone());
    }

    let selected_model = payload
        .model
        .as_deref()
        .unwrap_or(state.default_model.as_str())
        .to_string();

    let upstream = upstream_for_model(state.as_ref(), &selected_model);

    proxy_request(
        state.as_ref(),
        headers,
        payload,
        upstream,
        "chat/completions",
    )
    .await
}
