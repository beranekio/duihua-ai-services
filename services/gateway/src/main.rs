use std::{collections::HashMap, env, sync::Arc};

use anyhow::{Context, Result};
use axum::{
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing::{error, info};

#[derive(Clone)]
struct AppState {
    upstream_base: String,
    model_upstreams: HashMap<String, String>,
    default_model: String,
    upstream_api_key: Option<String>,
    client: Client,
}

#[derive(Serialize)]
struct ModelListResponse {
    object: &'static str,
    data: Vec<ModelItem>,
}

#[derive(Serialize)]
struct ModelItem {
    id: String,
    object: &'static str,
    owned_by: &'static str,
}

#[derive(Deserialize, Serialize)]
struct ChatCompletionRequest {
    model: Option<String>,
    messages: Vec<Value>,
    #[serde(flatten)]
    extra: Value,
}

#[derive(Deserialize, Serialize)]
struct EmbeddingsRequest {
    model: Option<String>,
    input: Value,
    #[serde(flatten)]
    extra: Value,
}

#[derive(Deserialize, Serialize)]
struct ResponsesRequest {
    model: Option<String>,
    #[serde(flatten)]
    extra: Value,
}

#[tokio::main]
async fn main() -> Result<()> {
    let env_filter =
        env::var("RUST_LOG").unwrap_or_else(|_| "info,duihua_gateway=debug".to_string());
    tracing_subscriber::fmt().with_env_filter(env_filter).init();

    let bind_addr = env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_string());
    let upstream_base = env::var("UPSTREAM_BASE_URL")
        .unwrap_or_else(|_| "http://vllm:8000/v1".to_string())
        .trim_end_matches('/')
        .to_string();
    let default_model =
        env::var("DEFAULT_MODEL").unwrap_or_else(|_| "google/gemma-4-31B-it".to_string());
    let upstream_api_key = env::var("UPSTREAM_API_KEY").ok();
    let model_upstreams = parse_model_upstreams(env::var("MODEL_UPSTREAMS").ok());

    let state = Arc::new(AppState {
        upstream_base,
        model_upstreams,
        default_model,
        upstream_api_key,
        client: Client::new(),
    });

    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/v1/models", get(list_models))
        .route("/v1/chat/completions", post(chat_completions))
        .route("/v1/responses", post(responses))
        .route("/v1/responses/input_tokens", post(response_input_tokens))
        .route("/v1/embeddings", post(embeddings))
        .route(
            "/v1/responses/{response_id}",
            get(get_response).delete(delete_response),
        )
        .route("/v1/responses/{response_id}/cancel", post(cancel_response))
        .route(
            "/v1/responses/{response_id}/input_items",
            get(list_response_input_items),
        )
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    info!("starting duihua gateway on {bind_addr}");
    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .with_context(|| format!("failed to bind {bind_addr}"))?;

    axum::serve(listener, app).await.context("server failure")?;
    Ok(())
}

fn parse_model_upstreams(value: Option<String>) -> HashMap<String, String> {
    value
        .unwrap_or_default()
        .split(',')
        .filter_map(|pair| {
            let (model, upstream) = pair.split_once('=')?;
            Some((
                model.trim().to_string(),
                upstream.trim().trim_end_matches('/').to_string(),
            ))
        })
        .filter(|(model, upstream)| !model.is_empty() && !upstream.is_empty())
        .collect()
}

async fn healthz() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

async fn list_models(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let mut models: Vec<String> = state.model_upstreams.keys().cloned().collect();
    if !models.iter().any(|m| m == &state.default_model) {
        models.push(state.default_model.clone());
    }
    models.sort();

    let body = ModelListResponse {
        object: "list",
        data: models
            .into_iter()
            .map(|id| ModelItem {
                id,
                object: "model",
                owned_by: "duihua",
            })
            .collect(),
    };

    (StatusCode::OK, Json(body))
}

async fn chat_completions(
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

async fn responses(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(mut payload): Json<ResponsesRequest>,
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

    proxy_request(state.as_ref(), headers, payload, upstream, "responses").await
}

async fn response_input_tokens(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(mut payload): Json<ResponsesRequest>,
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
        "responses/input_tokens",
    )
    .await
}

async fn get_response(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    uri: Uri,
    Path(response_id): Path<String>,
) -> Response {
    proxy_get(
        state.as_ref(),
        headers,
        default_upstream(state.as_ref()),
        &endpoint_with_query(&format!("responses/{response_id}"), &uri),
    )
    .await
}

async fn delete_response(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(response_id): Path<String>,
) -> Response {
    proxy_delete(
        state.as_ref(),
        headers,
        default_upstream(state.as_ref()),
        &format!("responses/{response_id}"),
    )
    .await
}

async fn cancel_response(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(response_id): Path<String>,
) -> Response {
    proxy_post_empty(
        state.as_ref(),
        headers,
        default_upstream(state.as_ref()),
        &format!("responses/{response_id}/cancel"),
    )
    .await
}

async fn list_response_input_items(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    uri: Uri,
    Path(response_id): Path<String>,
) -> Response {
    proxy_get(
        state.as_ref(),
        headers,
        default_upstream(state.as_ref()),
        &endpoint_with_query(&format!("responses/{response_id}/input_items"), &uri),
    )
    .await
}

async fn embeddings(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(mut payload): Json<EmbeddingsRequest>,
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

    proxy_request(state.as_ref(), headers, payload, upstream, "embeddings").await
}

fn upstream_for_model<'a>(state: &'a AppState, model: &str) -> &'a str {
    state
        .model_upstreams
        .get(model)
        .map(String::as_str)
        .unwrap_or(state.upstream_base.as_str())
}

fn default_upstream(state: &AppState) -> &str {
    upstream_for_model(state, &state.default_model)
}

async fn proxy_request<T: Serialize>(
    state: &AppState,
    headers: HeaderMap,
    payload: T,
    upstream: &str,
    endpoint: &str,
) -> Response {
    let url = format!("{}/{}", upstream, endpoint);
    let req = state.client.post(&url).json(&payload);

    proxy_upstream(state, headers, req).await
}

async fn proxy_get(
    state: &AppState,
    headers: HeaderMap,
    upstream: &str,
    endpoint: &str,
) -> Response {
    let url = format!("{}/{}", upstream, endpoint);
    let req = state.client.get(&url);

    proxy_upstream(state, headers, req).await
}

async fn proxy_delete(
    state: &AppState,
    headers: HeaderMap,
    upstream: &str,
    endpoint: &str,
) -> Response {
    let url = format!("{}/{}", upstream, endpoint);
    let req = state.client.delete(&url);

    proxy_upstream(state, headers, req).await
}

async fn proxy_post_empty(
    state: &AppState,
    headers: HeaderMap,
    upstream: &str,
    endpoint: &str,
) -> Response {
    let url = format!("{}/{}", upstream, endpoint);
    let req = state.client.post(&url);

    proxy_upstream(state, headers, req).await
}

fn endpoint_with_query(endpoint: &str, uri: &Uri) -> String {
    match uri.query() {
        Some(query) => format!("{endpoint}?{query}"),
        None => endpoint.to_string(),
    }
}

async fn proxy_upstream(
    state: &AppState,
    headers: HeaderMap,
    mut req: reqwest::RequestBuilder,
) -> Response {
    if let Some(auth_header) = headers.get("authorization") {
        req = req.header("authorization", auth_header);
    } else if let Some(api_key) = &state.upstream_api_key {
        req = req.bearer_auth(api_key);
    }

    match req.send().await {
        Ok(resp) => {
            let status = resp.status();
            let headers = resp.headers().clone();
            let stream = resp.bytes_stream();
            let mut downstream = Response::new(Body::from_stream(stream));
            *downstream.status_mut() = status;
            *downstream.headers_mut() = headers;
            downstream
        }
        Err(e) => {
            error!("upstream request failed: {e}");
            (StatusCode::BAD_GATEWAY, "upstream request failed").into_response()
        }
    }
}
