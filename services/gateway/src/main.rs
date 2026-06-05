mod background;

use std::{
    collections::HashMap,
    env,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

use anyhow::{Context, Result};
use axum::{
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use futures_util::TryStreamExt;
use redis::AsyncCommands;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing::{error, info};

struct AppState {
    upstream_base: String,
    model_upstreams: HashMap<String, String>,
    default_model: String,
    upstream_api_key: Option<String>,
    client: Client,
    responses_api_store_enabled: bool,
    response_store: Option<ResponseStore>,
    background_jobs: Option<background::BackgroundJobs>,
}

#[derive(Clone)]
struct ResponseStore {
    connection: redis::aio::MultiplexedConnection,
    key_prefix: String,
    ttl_seconds: u64,
}

#[derive(Clone, Deserialize, Serialize)]
struct StoredResponse {
    upstream: String,
    response: Value,
    input: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pending_upstream_request: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    upstream_authorization: Option<String>,
}

#[derive(Serialize)]
struct ErrorResponse {
    error: ErrorBody,
}

#[derive(Serialize)]
struct ErrorBody {
    message: String,
    #[serde(rename = "type")]
    error_type: &'static str,
    param: &'static str,
    code: u16,
}

fn response_not_found(response_id: &str) -> Response {
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
struct MessagesRequest {
    model: Option<String>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    previous_response_id: Option<String>,
    #[serde(flatten)]
    extra: Value,
}

fn request_input(request: &ResponsesRequest) -> Option<&Value> {
    request.extra.get("input")
}

fn normalized_input(input: Option<&Value>) -> Vec<Value> {
    match input {
        Some(Value::Array(items)) => items.clone(),
        Some(Value::String(text)) => vec![json!({"role": "user", "content": text})],
        Some(input) if !input.is_null() => vec![input.clone()],
        _ => Vec::new(),
    }
}

fn continuation_input(previous: &StoredResponse, input: Option<&Value>) -> Vec<Value> {
    let mut messages = previous.input.clone();
    if let Some(output) = previous.response.get("output").and_then(Value::as_array) {
        messages.extend(output.iter().cloned());
    }
    messages.extend(normalized_input(input));
    messages
}

fn set_request_input(request: &mut ResponsesRequest, input: Vec<Value>) {
    request.extra["input"] = Value::Array(input);
}

fn should_store_response(request: &ResponsesRequest) -> bool {
    request.extra.get("store").and_then(Value::as_bool) != Some(false)
}

fn disable_upstream_response_store(request: &mut ResponsesRequest) {
    request.extra["store"] = Value::Bool(false);
}

fn should_persist_gateway_response(store_enabled: bool, request: &ResponsesRequest) -> bool {
    store_enabled && should_store_response(request)
}

fn previous_response_not_ready() -> Response {
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

fn response_model(response: &Value) -> Option<String> {
    response
        .get("model")
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn init_rustls_provider() {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        rustls::crypto::aws_lc_rs::default_provider()
            .install_default()
            .expect("failed to install rustls crypto provider");
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    init_rustls_provider();

    let env_filter =
        env::var("RUST_LOG").unwrap_or_else(|_| "info,duihua_gateway=debug".to_string());
    tracing_subscriber::fmt().with_env_filter(env_filter).init();

    if background::is_background_worker_invocation() {
        return background::run_background_worker().await;
    }

    let bind_addr = env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_string());
    let upstream_base = env::var("UPSTREAM_BASE_URL")
        .unwrap_or_else(|_| "http://vllm:8000/v1".to_string())
        .trim_end_matches('/')
        .to_string();
    let default_model =
        env::var("DEFAULT_MODEL").unwrap_or_else(|_| "google/gemma-4-31B-it".to_string());
    let upstream_api_key = env::var("UPSTREAM_API_KEY").ok();
    let model_upstreams = parse_model_upstreams(env::var("MODEL_UPSTREAMS").ok());
    let responses_api_store_enabled = parse_bool_env("RESPONSES_API_STORE_ENABLED", false);
    let response_store = if responses_api_store_enabled {
        Some(response_store_from_env().await?)
    } else {
        None
    };
    let background_jobs = if responses_api_store_enabled {
        background::background_jobs_from_env().await?
    } else {
        None
    };

    let state = Arc::new(AppState {
        upstream_base,
        model_upstreams,
        default_model,
        upstream_api_key,
        client: Client::new(),
        responses_api_store_enabled,
        response_store,
        background_jobs,
    });

    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/v1/models", get(list_models))
        .route("/v1/chat/completions", post(chat_completions))
        .route("/v1/messages", post(messages))
        .route("/v1/messages/count_tokens", post(messages_count_tokens))
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

pub(crate) async fn response_store_from_env() -> Result<ResponseStore> {
    let url =
        env::var("RESPONSE_ID_STORE_URL").unwrap_or_else(|_| "redis://valkey:6379".to_string());
    let key_prefix =
        env::var("RESPONSE_ID_STORE_KEY_PREFIX").unwrap_or_else(|_| "duihua:responses".to_string());
    let ttl_seconds = env::var("RESPONSE_ID_STORE_TTL_SECONDS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(86_400);

    let client = redis::Client::open(url.as_str())
        .with_context(|| format!("invalid RESPONSE_ID_STORE_URL {url}"))?;
    let connection = client
        .get_multiplexed_async_connection()
        .await
        .with_context(|| format!("failed to connect to response id store at {url}"))?;

    Ok(ResponseStore {
        connection,
        key_prefix,
        ttl_seconds,
    })
}

fn parse_bool_env(name: &str, default: bool) -> bool {
    env::var(name)
        .ok()
        .and_then(|value| match value.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "0" | "false" | "no" | "off" => Some(false),
            _ => None,
        })
        .unwrap_or(default)
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

fn messages_upstream<'a>(state: &'a AppState, payload: &mut MessagesRequest) -> &'a str {
    let selected_model = payload
        .model
        .get_or_insert_with(|| state.default_model.clone());
    upstream_for_model(state, selected_model)
}

async fn messages(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(mut payload): Json<MessagesRequest>,
) -> Response {
    let upstream = messages_upstream(state.as_ref(), &mut payload);
    proxy_anthropic_request(state.as_ref(), headers, payload, upstream, "messages").await
}

async fn messages_count_tokens(
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

fn is_background_request(request: &ResponsesRequest) -> bool {
    request.extra.get("background").and_then(Value::as_bool) == Some(true)
}

async fn responses(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(mut payload): Json<ResponsesRequest>,
) -> Response {
    let background = is_background_request(&payload);
    if background {
        if !state.responses_api_store_enabled {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ErrorResponse {
                    error: ErrorBody {
                        message: "Background responses require the gateway-owned response store."
                            .to_string(),
                        error_type: "invalid_request_error",
                        param: "background",
                        code: 503,
                    },
                }),
            )
                .into_response();
        }
        if !should_store_response(&payload) {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: ErrorBody {
                        message: "Background responses require store=true.".to_string(),
                        error_type: "invalid_request_error",
                        param: "store",
                        code: 400,
                    },
                }),
            )
                .into_response();
        }
        if payload.extra.get("stream").and_then(Value::as_bool) == Some(true) {
            return (
                StatusCode::NOT_IMPLEMENTED,
                "streaming background responses are not supported",
            )
                .into_response();
        }
    }

    let (upstream, input) = if let Some(previous_response_id) = payload.previous_response_id.take()
    {
        let previous = match load_response(state.as_ref(), &previous_response_id).await {
            Ok(previous) => previous,
            Err(response) => return response,
        };
        if background::is_in_flight_background(&previous) {
            return previous_response_not_ready();
        }
        if payload.model.is_none() {
            payload.model = response_model(&previous.response);
        }
        let input = continuation_input(&previous, request_input(&payload));
        set_request_input(&mut payload, input.clone());
        (previous.upstream, input)
    } else {
        if payload.model.is_none() {
            payload.model = Some(state.default_model.clone());
        }
        let selected_model = payload
            .model
            .as_deref()
            .unwrap_or(state.default_model.as_str())
            .to_string();
        (
            upstream_for_model(state.as_ref(), &selected_model).to_string(),
            normalized_input(request_input(&payload)),
        )
    };

    let persist_response =
        should_persist_gateway_response(state.responses_api_store_enabled, &payload);

    if state.responses_api_store_enabled {
        disable_upstream_response_store(&mut payload);
    }

    if background {
        return create_background_response(state, headers, payload, upstream, input).await;
    }

    proxy_response_request(state, headers, payload, upstream, input, persist_response).await
}

async fn create_background_response(
    state: Arc<AppState>,
    headers: HeaderMap,
    payload: ResponsesRequest,
    upstream: String,
    input: Vec<Value>,
) -> Response {
    let upstream_authorization = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let model = payload
        .model
        .clone()
        .unwrap_or_else(|| state.default_model.clone());
    let response_id = background::generate_response_id();
    let request_value = serde_json::to_value(&payload).unwrap_or(Value::Null);
    let upstream_request = background::build_upstream_request(&request_value);
    let queued_response = background::build_queued_response(&response_id, &model, &request_value);

    match background::enqueue_background_response(
        state.as_ref(),
        response_id,
        upstream,
        input,
        upstream_request,
        queued_response.clone(),
        upstream_authorization,
    )
    .await
    {
        Ok(()) => (StatusCode::OK, Json(queued_response)).into_response(),
        Err(response) => response,
    }
}

async fn response_input_tokens(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(mut payload): Json<ResponsesRequest>,
) -> Response {
    let upstream = if let Some(previous_response_id) = payload.previous_response_id.take() {
        let previous = match load_response(state.as_ref(), &previous_response_id).await {
            Ok(previous) => previous,
            Err(response) => return response,
        };
        if background::is_in_flight_background(&previous) {
            return previous_response_not_ready();
        }
        if payload.model.is_none() {
            payload.model = response_model(&previous.response);
        }
        let input = continuation_input(&previous, request_input(&payload));
        set_request_input(&mut payload, input);
        previous.upstream
    } else {
        if payload.model.is_none() {
            payload.model = Some(state.default_model.clone());
        }
        let selected_model = payload
            .model
            .as_deref()
            .unwrap_or(state.default_model.as_str())
            .to_string();
        upstream_for_model(state.as_ref(), &selected_model).to_string()
    };

    proxy_request(
        state.as_ref(),
        headers,
        payload,
        &upstream,
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
    let _ = (headers, uri);
    match load_response(state.as_ref(), &response_id).await {
        Ok(stored) => Json(stored.response).into_response(),
        Err(response) => response,
    }
}

async fn delete_response(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(response_id): Path<String>,
) -> Response {
    let _ = headers;
    let stored = match load_stored_response(state.as_ref(), &response_id).await {
        Ok(stored) => stored,
        Err(response) => return response,
    };
    if let Err(response) =
        background::finalize_background_deletion(state.as_ref(), &response_id, &stored).await
    {
        return response;
    }
    Json(json!({"id": response_id, "object": "response.deleted", "deleted": true})).into_response()
}

async fn cancel_response(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(response_id): Path<String>,
) -> Response {
    let _ = headers;
    let mut stored = match load_stored_response(state.as_ref(), &response_id).await {
        Ok(stored) => stored,
        Err(response) => return response,
    };
    let status = background::stored_response_status(&stored);
    if matches!(status, Some("completed" | "failed" | "cancelled")) {
        let message = if status == Some("completed")
            && stored.response.get("background").and_then(Value::as_bool) != Some(true)
        {
            "Cannot cancel a synchronous response.".to_string()
        } else {
            format!(
                "Cannot cancel a response that is already {}.",
                status.unwrap_or("unknown")
            )
        };
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": {"message": message, "type": "invalid_request_error", "param": "response_id", "code": 400}})),
        )
            .into_response();
    }

    if let Some(background_jobs) = &state.background_jobs {
        if let Err(e) = background_jobs.cancel(&response_id).await {
            error!("failed to cancel background job for {response_id}: {e}");
            return (
                StatusCode::BAD_GATEWAY,
                "failed to cancel background response job",
            )
                .into_response();
        }
    }

    let cancelled = background::build_cancelled_response(&stored, &response_id);
    stored.response = cancelled.clone();
    stored.pending_upstream_request = None;
    stored.upstream_authorization = None;
    if let Some(response_store) = &state.response_store {
        if let Err(e) = response_store.store(&response_id, &stored).await {
            error!("failed to persist cancelled background response {response_id}: {e}");
            return (StatusCode::BAD_GATEWAY, "response id store write failed").into_response();
        }
    }

    Json(cancelled).into_response()
}

async fn list_response_input_items(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    uri: Uri,
    Path(response_id): Path<String>,
) -> Response {
    let _ = (headers, uri);
    match load_response(state.as_ref(), &response_id).await {
        Ok(stored) => {
            Json(json!({"object": "list", "data": stored.input, "has_more": false})).into_response()
        }
        Err(response) => response,
    }
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

async fn load_stored_response(
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

    match response_store.load(response_id).await {
        Ok(Some(response)) => {
            if background::stored_response_status(&response) == Some("deleted") {
                return Err(response_not_found(response_id));
            }
            let response = if let Some(background_jobs) = &state.background_jobs {
                match background_jobs
                    .reconcile_failed_response(response_store, response_id, &response)
                    .await
                {
                    Ok(response) => response,
                    Err(e) => {
                        error!("failed to reconcile background job status for {response_id}: {e}");
                        response
                    }
                }
            } else {
                response
            };
            Ok(response)
        }
        Ok(None) => Err(response_not_found(response_id)),
        Err(e) => {
            error!("failed to read response id store for {response_id}: {e}");
            Err((StatusCode::BAD_GATEWAY, "response id store read failed").into_response())
        }
    }
}

async fn load_response(
    state: &AppState,
    response_id: &str,
) -> std::result::Result<StoredResponse, Response> {
    load_stored_response(state, response_id).await
}

async fn proxy_response_request<T: Serialize>(
    state: Arc<AppState>,
    headers: HeaderMap,
    payload: T,
    upstream: String,
    input: Vec<Value>,
    persist_response: bool,
) -> Response {
    let url = format!("{upstream}/responses");
    let req = state.client.post(&url).json(&payload);

    proxy_upstream_tracking_response(state, headers, req, upstream, input, persist_response).await
}

async fn proxy_request<T: Serialize>(
    state: &AppState,
    headers: HeaderMap,
    payload: T,
    upstream: &str,
    endpoint: &str,
) -> Response {
    let url = format!("{}/{}", upstream, endpoint);
    let req =
        apply_openai_upstream_headers(state.client.post(&url).json(&payload), &headers, state);

    proxy_upstream_request(req).await
}

async fn proxy_anthropic_request<T: Serialize>(
    state: &AppState,
    headers: HeaderMap,
    payload: T,
    upstream: &str,
    endpoint: &str,
) -> Response {
    let url = format!("{}/{}", upstream, endpoint);
    let req =
        apply_anthropic_upstream_headers(state.client.post(&url).json(&payload), &headers, state);

    proxy_upstream_request(req).await
}

fn apply_openai_upstream_headers(
    mut req: reqwest::RequestBuilder,
    headers: &HeaderMap,
    state: &AppState,
) -> reqwest::RequestBuilder {
    if let Some(auth_header) = headers.get("authorization") {
        req = req.header("authorization", auth_header);
    } else if let Some(api_key) = &state.upstream_api_key {
        req = req.bearer_auth(api_key);
    }
    req
}

fn apply_anthropic_upstream_headers(
    mut req: reqwest::RequestBuilder,
    headers: &HeaderMap,
    state: &AppState,
) -> reqwest::RequestBuilder {
    let mut has_version = false;
    let mut has_client_auth = false;

    for name in [
        "anthropic-version",
        "anthropic-beta",
        "x-api-key",
        "authorization",
    ] {
        if let Some(value) = headers.get(name) {
            req = req.header(name, value);
            if name == "anthropic-version" {
                has_version = true;
            } else if name == "x-api-key" || name == "authorization" {
                has_client_auth = true;
            }
        }
    }
    if !has_version {
        req = req.header("anthropic-version", "2023-06-01");
    }
    if !has_client_auth {
        if let Some(api_key) = &state.upstream_api_key {
            req = req.header("x-api-key", api_key);
        }
    }
    req
}

async fn proxy_upstream_tracking_response(
    state: Arc<AppState>,
    headers: HeaderMap,
    mut req: reqwest::RequestBuilder,
    upstream: String,
    input: Vec<Value>,
    persist_response: bool,
) -> Response {
    req = apply_openai_upstream_headers(req, &headers, state.as_ref());

    match req.send().await {
        Ok(resp) => {
            let status = resp.status();
            let headers = resp.headers().clone();

            if is_event_stream(&headers) {
                if persist_response {
                    let tracker = Arc::new(ResponseTracker::new(state.clone(), upstream, input));
                    let stream = resp
                        .bytes_stream()
                        .inspect_ok(move |chunk| tracker.observe(chunk));
                    let mut downstream = Response::new(Body::from_stream(stream));
                    *downstream.status_mut() = status;
                    *downstream.headers_mut() = headers;
                    downstream
                } else {
                    let mut downstream = Response::new(Body::from_stream(resp.bytes_stream()));
                    *downstream.status_mut() = status;
                    *downstream.headers_mut() = headers;
                    downstream
                }
            } else {
                match resp.bytes().await {
                    Ok(body) => {
                        if persist_response {
                            track_response_from_json(&state, &upstream, &input, &body).await;
                        }
                        let mut downstream = Response::new(Body::from(body));
                        *downstream.status_mut() = status;
                        *downstream.headers_mut() = headers;
                        downstream
                    }
                    Err(e) => {
                        error!("upstream response body read failed: {e}");
                        (
                            StatusCode::BAD_GATEWAY,
                            "upstream response body read failed",
                        )
                            .into_response()
                    }
                }
            }
        }
        Err(e) => {
            error!("upstream request failed: {e}");
            (StatusCode::BAD_GATEWAY, "upstream request failed").into_response()
        }
    }
}

fn is_event_stream(headers: &HeaderMap) -> bool {
    headers
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("text/event-stream"))
}

async fn track_response_from_json(state: &AppState, upstream: &str, input: &[Value], body: &[u8]) {
    let Ok(response) = serde_json::from_slice::<Value>(body) else {
        return;
    };
    store_response(state, upstream.to_string(), response, input.to_vec()).await;
}

async fn store_response(state: &AppState, upstream: String, response: Value, input: Vec<Value>) {
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
    };
    if let Err(e) = response_store.store(&response_id, &stored).await {
        error!("failed to store response {response_id}: {e}");
    }
}

impl ResponseStore {
    async fn store(&self, response_id: &str, response: &StoredResponse) -> redis::RedisResult<()> {
        let mut connection = self.connection.clone();
        let response = serde_json::to_string(response).map_err(|e| {
            redis::RedisError::from((
                redis::ErrorKind::Client,
                "failed to serialize response",
                e.to_string(),
            ))
        })?;
        connection
            .set_ex(self.key(response_id), response, self.ttl_seconds)
            .await
    }

    async fn load(&self, response_id: &str) -> redis::RedisResult<Option<StoredResponse>> {
        let mut connection = self.connection.clone();
        let response: Option<String> = connection.get(self.key(response_id)).await?;
        response
            .map(|response| {
                serde_json::from_str(&response).map_err(|e| {
                    redis::RedisError::from((
                        redis::ErrorKind::Client,
                        "failed to deserialize response",
                        e.to_string(),
                    ))
                })
            })
            .transpose()
    }

    async fn delete(&self, response_id: &str) -> redis::RedisResult<()> {
        let mut connection = self.connection.clone();
        connection.del(self.key(response_id)).await
    }

    fn key(&self, response_id: &str) -> String {
        response_store_key(&self.key_prefix, response_id)
    }
}

fn response_store_key(prefix: &str, response_id: &str) -> String {
    format!("{prefix}:{response_id}")
}

fn response_id_from_value(value: &Value) -> Option<String> {
    value
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| id.starts_with("resp_"))
        .or_else(|| {
            value
                .get("response")
                .and_then(|response| response.get("id"))
                .and_then(Value::as_str)
                .filter(|id| id.starts_with("resp_"))
        })
        .map(ToString::to_string)
}

struct ResponseTracker {
    state: Arc<AppState>,
    upstream: String,
    input: Vec<Value>,
    buffer: Mutex<String>,
    tracked: AtomicBool,
}

impl ResponseTracker {
    fn new(state: Arc<AppState>, upstream: String, input: Vec<Value>) -> Self {
        Self {
            state,
            upstream,
            input,
            buffer: Mutex::new(String::new()),
            tracked: AtomicBool::new(false),
        }
    }

    fn observe(&self, chunk: &[u8]) {
        if self.tracked.load(Ordering::Relaxed) {
            return;
        }

        let Ok(chunk) = std::str::from_utf8(chunk) else {
            return;
        };

        let Some(response) = self.find_response(chunk) else {
            return;
        };

        if self
            .tracked
            .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            let state = Arc::clone(&self.state);
            let upstream = self.upstream.clone();
            let input = self.input.clone();
            tokio::spawn(async move {
                store_response(state.as_ref(), upstream, response, input).await;
            });
        }
    }

    fn find_response(&self, chunk: &str) -> Option<Value> {
        let mut buffer = self.buffer.lock().expect("response id buffer poisoned");
        buffer.push_str(chunk);

        if buffer.len() > 1_048_576 {
            let keep_from = buffer.len() - 1_048_576;
            buffer.drain(..keep_from);
        }

        for line in buffer.lines() {
            let Some(data) = line.trim_start().strip_prefix("data:") else {
                continue;
            };
            let data = data.trim();
            if data == "[DONE]" {
                continue;
            }
            if let Ok(value) = serde_json::from_str::<Value>(data) {
                if value.get("type").and_then(Value::as_str) == Some("response.completed") {
                    return value.get("response").cloned();
                }
            }
        }

        None
    }
}

async fn proxy_upstream_request(req: reqwest::RequestBuilder) -> Response {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserializes_previous_response_id_without_model() {
        let request = serde_json::from_value::<ResponsesRequest>(serde_json::json!({
            "previous_response_id": "resp_prior",
            "input": "continue"
        }))
        .expect("valid responses request");

        assert_eq!(request.model, None);
        assert_eq!(request.previous_response_id.as_deref(), Some("resp_prior"));
    }

    #[test]
    fn extracts_response_id_from_response_objects() {
        let response = serde_json::json!({ "id": "resp_123", "object": "response" });
        assert_eq!(
            response_id_from_value(&response).as_deref(),
            Some("resp_123")
        );

        let stream_event = serde_json::json!({
            "type": "response.created",
            "response": { "id": "resp_456", "object": "response" }
        });
        assert_eq!(
            response_id_from_value(&stream_event).as_deref(),
            Some("resp_456")
        );
    }

    #[test]
    fn ignores_non_response_ids() {
        let chat_completion = serde_json::json!({ "id": "chatcmpl_123" });
        assert_eq!(response_id_from_value(&chat_completion), None);

        let stream_event = serde_json::json!({
            "type": "response.created",
            "response": { "id": "not-a-response-id" }
        });
        assert_eq!(response_id_from_value(&stream_event), None);
    }

    #[test]
    fn builds_response_store_keys_with_prefix() {
        assert_eq!(
            response_store_key("duihua:responses", "resp_model_a"),
            "duihua:responses:resp_model_a"
        );
    }

    #[test]
    fn parses_model_upstreams() {
        let upstreams = parse_model_upstreams(Some(
            " model-a = http://model-a:8000/v1/,invalid,=missing-model,missing-upstream= ,\
             model-b=https://model-b.example/v1/ "
                .to_string(),
        ));

        assert_eq!(upstreams.len(), 2);
        assert_eq!(
            upstreams.get("model-a").map(String::as_str),
            Some("http://model-a:8000/v1")
        );
        assert_eq!(
            upstreams.get("model-b").map(String::as_str),
            Some("https://model-b.example/v1")
        );
    }

    #[test]
    fn selects_model_specific_upstream_or_default() {
        let state = AppState {
            upstream_base: "http://default:8000/v1".to_string(),
            model_upstreams: HashMap::from([(
                "model-a".to_string(),
                "http://model-a:8000/v1".to_string(),
            )]),
            default_model: "model-default".to_string(),
            upstream_api_key: None,
            client: Client::new(),
            responses_api_store_enabled: false,
            response_store: None,
            background_jobs: None,
        };

        assert_eq!(
            upstream_for_model(&state, "model-a"),
            "http://model-a:8000/v1"
        );
        assert_eq!(
            upstream_for_model(&state, "model-b"),
            "http://default:8000/v1"
        );
    }

    #[test]
    fn detects_event_stream_content_type() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "content-type",
            "text/event-stream; charset=utf-8".parse().unwrap(),
        );
        assert!(is_event_stream(&headers));

        headers.insert("content-type", "application/json".parse().unwrap());
        assert!(!is_event_stream(&headers));
    }

    #[test]
    fn extracts_completed_streamed_response_across_chunks() {
        let state = Arc::new(AppState {
            upstream_base: "http://default:8000/v1".to_string(),
            model_upstreams: HashMap::new(),
            default_model: "model-default".to_string(),
            upstream_api_key: None,
            client: Client::new(),
            responses_api_store_enabled: false,
            response_store: None,
            background_jobs: None,
        });
        let tracker = ResponseTracker::new(state, "http://default:8000/v1".to_string(), Vec::new());

        assert_eq!(
            tracker.find_response("data: {\"type\":\"response.completed\","),
            None
        );
        assert_eq!(
            tracker.find_response(
                "\"response\":{\"id\":\"resp_streamed\",\"object\":\"response\"}}\n"
            ),
            Some(json!({"id": "resp_streamed", "object": "response"}))
        );
    }

    #[test]
    fn materializes_continuation_input() {
        let previous = StoredResponse {
            upstream: "http://model-a:8000/v1".to_string(),
            response: json!({
                "id": "resp_previous",
                "model": "model-a",
                "output": [{"role": "assistant", "content": "prior answer"}]
            }),
            input: vec![json!({"role": "user", "content": "prior question"})],
            pending_upstream_request: None,
            upstream_authorization: None,
        };

        assert_eq!(
            continuation_input(&previous, Some(&json!("next question"))),
            vec![
                json!({"role": "user", "content": "prior question"}),
                json!({"role": "assistant", "content": "prior answer"}),
                json!({"role": "user", "content": "next question"}),
            ]
        );
    }

    #[test]
    fn honors_explicit_response_store_flag() {
        let default_request = serde_json::from_value::<ResponsesRequest>(json!({
            "input": "persist by default"
        }))
        .expect("valid responses request");
        assert!(should_store_response(&default_request));

        let stored_request = serde_json::from_value::<ResponsesRequest>(json!({
            "input": "persist explicitly",
            "store": true
        }))
        .expect("valid responses request");
        assert!(should_store_response(&stored_request));

        let unpersisted_request = serde_json::from_value::<ResponsesRequest>(json!({
            "input": "do not persist",
            "store": false
        }))
        .expect("valid responses request");
        assert!(!should_store_response(&unpersisted_request));
    }

    #[test]
    fn preserves_gateway_persistence_decision_before_disabling_upstream_store() {
        let mut default_request = serde_json::from_value::<ResponsesRequest>(json!({
            "input": "persist by default"
        }))
        .expect("valid responses request");

        let persist_response = should_persist_gateway_response(true, &default_request);
        disable_upstream_response_store(&mut default_request);

        assert!(persist_response);
        assert!(!should_store_response(&default_request));

        let unpersisted_request = serde_json::from_value::<ResponsesRequest>(json!({
            "input": "do not persist",
            "store": false
        }))
        .expect("valid responses request");
        assert!(!should_persist_gateway_response(true, &unpersisted_request));
    }

    #[test]
    fn serializes_stateless_continuation_request() {
        let mut request = serde_json::from_value::<ResponsesRequest>(json!({
            "previous_response_id": "resp_prior",
            "input": "continue"
        }))
        .expect("valid responses request");
        request.previous_response_id = None;
        set_request_input(
            &mut request,
            vec![json!({"role": "user", "content": "continue"})],
        );
        disable_upstream_response_store(&mut request);

        assert_eq!(
            serde_json::to_value(request).expect("serializable request"),
            json!({
                "input": [{"role": "user", "content": "continue"}],
                "store": false
            })
        );
    }

    #[test]
    fn parses_bool_env_values() {
        env::set_var("DUIHUA_TEST_BOOL", "true");
        assert!(parse_bool_env("DUIHUA_TEST_BOOL", false));
        env::set_var("DUIHUA_TEST_BOOL", "0");
        assert!(!parse_bool_env("DUIHUA_TEST_BOOL", true));
        env::remove_var("DUIHUA_TEST_BOOL");
        assert!(parse_bool_env("DUIHUA_TEST_BOOL", true));
    }

    #[test]
    fn deserializes_messages_request_without_model() {
        let request = serde_json::from_value::<MessagesRequest>(json!({
            "max_tokens": 256,
            "messages": [{"role": "user", "content": "Hello"}]
        }))
        .expect("valid messages request");

        assert_eq!(request.model, None);
    }

    #[test]
    fn applies_default_anthropic_version_and_upstream_api_key() {
        let state = AppState {
            upstream_base: "http://default:8000/v1".to_string(),
            model_upstreams: HashMap::new(),
            default_model: "model-default".to_string(),
            upstream_api_key: Some("upstream-secret".to_string()),
            client: Client::new(),
            responses_api_store_enabled: false,
            response_store: None,
            background_jobs: None,
        };
        let headers = HeaderMap::new();
        let req =
            apply_anthropic_upstream_headers(Client::new().post("http://test"), &headers, &state);
        let built = req.build().expect("request should build");
        assert_eq!(
            built
                .headers()
                .get("anthropic-version")
                .and_then(|v| v.to_str().ok()),
            Some("2023-06-01")
        );
        assert_eq!(
            built
                .headers()
                .get("x-api-key")
                .and_then(|v| v.to_str().ok()),
            Some("upstream-secret")
        );
    }

    #[test]
    fn preserves_client_anthropic_headers_over_upstream_api_key() {
        let state = AppState {
            upstream_base: "http://default:8000/v1".to_string(),
            model_upstreams: HashMap::new(),
            default_model: "model-default".to_string(),
            upstream_api_key: Some("upstream-secret".to_string()),
            client: Client::new(),
            responses_api_store_enabled: false,
            response_store: None,
            background_jobs: None,
        };
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", "client-secret".parse().unwrap());
        headers.insert("anthropic-version", "2024-01-01".parse().unwrap());
        headers.insert("anthropic-beta", "messages-2024-10-22".parse().unwrap());

        let req =
            apply_anthropic_upstream_headers(Client::new().post("http://test"), &headers, &state);
        let built = req.build().expect("request should build");
        assert_eq!(
            built
                .headers()
                .get("x-api-key")
                .and_then(|v| v.to_str().ok()),
            Some("client-secret")
        );
        assert_eq!(
            built
                .headers()
                .get("anthropic-version")
                .and_then(|v| v.to_str().ok()),
            Some("2024-01-01")
        );
        assert_eq!(
            built
                .headers()
                .get("anthropic-beta")
                .and_then(|v| v.to_str().ok()),
            Some("messages-2024-10-22")
        );
    }
}
