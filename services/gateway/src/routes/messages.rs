use std::sync::Arc;

use axum::{extract::State, http::HeaderMap, response::Response, Json};

use crate::{
    models::MessagesRequest,
    state::{upstream_for_model, AppState},
    upstream::proxy_anthropic_request,
};

fn messages_upstream<'a>(state: &'a AppState, payload: &mut MessagesRequest) -> &'a str {
    let selected_model = payload
        .model
        .get_or_insert_with(|| state.default_model.clone());
    upstream_for_model(state, selected_model)
}

pub async fn messages(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(mut payload): Json<MessagesRequest>,
) -> Response {
    let upstream = messages_upstream(state.as_ref(), &mut payload);
    proxy_anthropic_request(state.as_ref(), headers, payload, upstream, "messages").await
}

pub async fn messages_count_tokens(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(mut payload): Json<MessagesRequest>,
) -> Response {
    let upstream = messages_upstream(state.as_ref(), &mut payload);
    proxy_anthropic_request(
        state.as_ref(),
        headers,
        payload,
        upstream,
        "messages/count_tokens",
    )
    .await
}
