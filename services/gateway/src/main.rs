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
use serde_json::Value;
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
}

#[derive(Clone)]
struct ResponseStore {
    connection: redis::aio::MultiplexedConnection,
    key_prefix: String,
    ttl_seconds: u64,
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
    let responses_api_store_enabled = parse_bool_env("RESPONSES_API_STORE_ENABLED", false);
    let response_store = if responses_api_store_enabled {
        Some(response_store_from_env().await?)
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

async fn response_store_from_env() -> Result<ResponseStore> {
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

    let upstream = upstream_for_model(state.as_ref(), &selected_model).to_string();

    proxy_response_request(state, headers, payload, upstream).await
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
    let upstream = match response_upstream(state.as_ref(), &response_id).await {
        Ok(upstream) => upstream,
        Err(response) => return response,
    };

    proxy_get(
        state.as_ref(),
        headers,
        &upstream,
        &endpoint_with_query(&format!("responses/{response_id}"), &uri),
    )
    .await
}

async fn delete_response(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(response_id): Path<String>,
) -> Response {
    let upstream = match response_upstream(state.as_ref(), &response_id).await {
        Ok(upstream) => upstream,
        Err(response) => return response,
    };

    proxy_delete(
        state.as_ref(),
        headers,
        &upstream,
        &format!("responses/{response_id}"),
    )
    .await
}

async fn cancel_response(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(response_id): Path<String>,
) -> Response {
    let upstream = match response_upstream(state.as_ref(), &response_id).await {
        Ok(upstream) => upstream,
        Err(response) => return response,
    };

    proxy_post_empty(
        state.as_ref(),
        headers,
        &upstream,
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
    let upstream = match response_upstream(state.as_ref(), &response_id).await {
        Ok(upstream) => upstream,
        Err(response) => return response,
    };

    proxy_get(
        state.as_ref(),
        headers,
        &upstream,
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

async fn response_upstream(
    state: &AppState,
    response_id: &str,
) -> std::result::Result<String, Response> {
    if !state.responses_api_store_enabled {
        return Err(response_not_found(response_id));
    }

    let Some(response_store) = &state.response_store else {
        error!("responses API store is enabled but no response store is configured");
        return Err((StatusCode::BAD_GATEWAY, "response id store unavailable").into_response());
    };

    match response_store.load(response_id).await {
        Ok(Some(upstream)) => Ok(upstream),
        Ok(None) => Err(response_not_found(response_id)),
        Err(e) => {
            error!("failed to read response id store for {response_id}: {e}");
            Err((StatusCode::BAD_GATEWAY, "response id store read failed").into_response())
        }
    }
}

async fn proxy_response_request<T: Serialize>(
    state: Arc<AppState>,
    headers: HeaderMap,
    payload: T,
    upstream: String,
) -> Response {
    let url = format!("{upstream}/responses");
    let req = state.client.post(&url).json(&payload);

    proxy_upstream_tracking_response_id(state, headers, req, upstream).await
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

async fn proxy_upstream_tracking_response_id(
    state: Arc<AppState>,
    headers: HeaderMap,
    mut req: reqwest::RequestBuilder,
    upstream: String,
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

            if is_event_stream(&headers) {
                let tracker = Arc::new(ResponseIdTracker::new(state, upstream));
                let stream = resp
                    .bytes_stream()
                    .inspect_ok(move |chunk| tracker.observe(chunk));
                let mut downstream = Response::new(Body::from_stream(stream));
                *downstream.status_mut() = status;
                *downstream.headers_mut() = headers;
                downstream
            } else {
                match resp.bytes().await {
                    Ok(body) => {
                        track_response_id_from_json(&state, &upstream, &body).await;
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

async fn track_response_id_from_json(state: &AppState, upstream: &str, body: &[u8]) {
    let Ok(value) = serde_json::from_slice::<Value>(body) else {
        return;
    };

    if let Some(response_id) = response_id_from_value(&value) {
        store_response_upstream(state, response_id, upstream.to_string()).await;
    }
}

async fn store_response_upstream(state: &AppState, response_id: String, upstream: String) {
    if !state.responses_api_store_enabled {
        return;
    }

    let Some(response_store) = &state.response_store else {
        error!("responses API store is enabled but no response store is configured");
        return;
    };

    if let Err(e) = response_store.store(&response_id, &upstream).await {
        error!("failed to store upstream for response {response_id}: {e}");
    }
}

impl ResponseStore {
    async fn store(&self, response_id: &str, upstream: &str) -> redis::RedisResult<()> {
        let mut connection = self.connection.clone();
        connection
            .set_ex(self.key(response_id), upstream, self.ttl_seconds)
            .await
    }

    async fn load(&self, response_id: &str) -> redis::RedisResult<Option<String>> {
        let mut connection = self.connection.clone();
        connection.get(self.key(response_id)).await
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

struct ResponseIdTracker {
    state: Arc<AppState>,
    upstream: String,
    buffer: Mutex<String>,
    tracked: AtomicBool,
}

impl ResponseIdTracker {
    fn new(state: Arc<AppState>, upstream: String) -> Self {
        Self {
            state,
            upstream,
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

        let Some(response_id) = self.find_response_id(chunk) else {
            return;
        };

        if self
            .tracked
            .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            let state = Arc::clone(&self.state);
            let upstream = self.upstream.clone();
            tokio::spawn(async move {
                store_response_upstream(state.as_ref(), response_id, upstream).await;
            });
        }
    }

    fn find_response_id(&self, chunk: &str) -> Option<String> {
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
                if let Some(response_id) = response_id_from_value(&value) {
                    return Some(response_id);
                }
            }
        }

        serde_json::from_str::<Value>(&buffer)
            .ok()
            .and_then(|value| response_id_from_value(&value))
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn preserves_query_strings_for_response_subresources() {
        let uri: Uri = "/v1/responses/resp_123/input_items?after=item_1&limit=20"
            .parse()
            .expect("valid uri");

        assert_eq!(
            endpoint_with_query("responses/resp_123/input_items", &uri),
            "responses/resp_123/input_items?after=item_1&limit=20"
        );

        let uri: Uri = "/v1/responses/resp_123".parse().expect("valid uri");
        assert_eq!(
            endpoint_with_query("responses/resp_123", &uri),
            "responses/resp_123"
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
    fn extracts_streamed_response_id_across_chunks() {
        let state = Arc::new(AppState {
            upstream_base: "http://default:8000/v1".to_string(),
            model_upstreams: HashMap::new(),
            default_model: "model-default".to_string(),
            upstream_api_key: None,
            client: Client::new(),
            responses_api_store_enabled: false,
            response_store: None,
        });
        let tracker = ResponseIdTracker::new(state, "http://default:8000/v1".to_string());

        assert_eq!(
            tracker.find_response_id("data: {\"type\":\"response.created\","),
            None
        );
        assert_eq!(
            tracker
                .find_response_id(
                    "\"response\":{\"id\":\"resp_streamed\",\"object\":\"response\"}}\n"
                )
                .as_deref(),
            Some("resp_streamed")
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
}
