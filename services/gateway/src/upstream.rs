use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

use axum::{
    body::Body,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use futures_util::TryStreamExt;
use serde::Serialize;
use serde_json::Value;
use tracing::error;

use crate::{state::AppState, store::store_response};

pub fn apply_openai_upstream_headers(
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

pub fn apply_anthropic_upstream_headers(
    mut req: reqwest::RequestBuilder,
    headers: &HeaderMap,
    state: &AppState,
) -> reqwest::RequestBuilder {
    let mut has_version = false;

    for name in ["anthropic-version", "anthropic-beta"] {
        if let Some(value) = headers.get(name) {
            req = req.header(name, value);
            if name == "anthropic-version" {
                has_version = true;
            }
        }
    }
    if !has_version {
        req = req.header("anthropic-version", "2023-06-01");
    }

    if let Some(api_key) = &state.upstream_api_key {
        return req.bearer_auth(api_key);
    }

    for name in ["x-api-key", "authorization"] {
        if let Some(value) = headers.get(name) {
            req = req.header(name, value);
        }
    }
    req
}

pub async fn proxy_response_request<T: Serialize>(
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

pub async fn proxy_request<T: Serialize>(
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

pub async fn proxy_anthropic_request<T: Serialize>(
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

pub async fn proxy_upstream_tracking_response(
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

pub fn is_event_stream(headers: &HeaderMap) -> bool {
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

struct ResponseTracker {
    state: Arc<AppState>,
    upstream: String,
    input: Vec<Value>,
    buffer: Mutex<Vec<u8>>,
    tracked: AtomicBool,
}

impl ResponseTracker {
    fn new(state: Arc<AppState>, upstream: String, input: Vec<Value>) -> Self {
        Self {
            state,
            upstream,
            input,
            buffer: Mutex::new(Vec::new()),
            tracked: AtomicBool::new(false),
        }
    }

    fn observe(&self, chunk: &[u8]) {
        if self.tracked.load(Ordering::Relaxed) {
            return;
        }

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

    fn find_response(&self, chunk: &[u8]) -> Option<Value> {
        let mut buffer = self.buffer.lock().expect("response id buffer poisoned");
        buffer.extend_from_slice(chunk);

        let mut consumed = 0;
        let mut found_response = None;

        while let Some(newline_idx) = buffer[consumed..].iter().position(|&b| b == b'\n') {
            let absolute_newline_idx = consumed + newline_idx;
            let line_bytes = &buffer[consumed..absolute_newline_idx];
            if let Ok(line_str) = std::str::from_utf8(line_bytes) {
                let trimmed = line_str.trim_start();
                if let Some(data) = trimmed.strip_prefix("data:") {
                    let data = data.trim();
                    if data != "[DONE]" {
                        if let Ok(value) = serde_json::from_str::<Value>(data) {
                            if value.get("type").and_then(Value::as_str)
                                == Some("response.completed")
                            {
                                found_response = value.get("response").cloned();
                                consumed = absolute_newline_idx + 1;
                                break;
                            }
                        }
                    }
                }
            }
            consumed = absolute_newline_idx + 1;
        }

        if consumed > 0 {
            buffer.drain(..consumed);
        }

        if found_response.is_some() {
            return found_response;
        }

        if buffer.len() > 1_048_576 {
            buffer.clear();
        }

        None
    }

    #[cfg(test)]
    fn buffer_len(&self) -> usize {
        self.buffer
            .lock()
            .expect("response id buffer poisoned")
            .len()
    }
}

pub async fn proxy_upstream_request(req: reqwest::RequestBuilder) -> Response {
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
    use std::{collections::HashMap, sync::Arc};

    use axum::http::HeaderMap;
    use reqwest::Client;
    use serde_json::json;

    use crate::state::AppState;

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
            background_jobs_enabled: false,
        });
        let tracker = ResponseTracker::new(state, "http://default:8000/v1".to_string(), Vec::new());

        assert_eq!(
            tracker.find_response(b"data: {\"type\":\"response.completed\","),
            None
        );
        assert_eq!(
            tracker.find_response(
                b"\"response\":{\"id\":\"resp_streamed\",\"object\":\"response\"}}\n"
            ),
            Some(json!({"id": "resp_streamed", "object": "response"}))
        );
    }

    #[test]
    fn extracts_completed_streamed_response_when_utf8_splits_across_chunks() {
        let state = Arc::new(AppState {
            upstream_base: "http://default:8000/v1".to_string(),
            model_upstreams: HashMap::new(),
            default_model: "model-default".to_string(),
            upstream_api_key: None,
            client: Client::new(),
            responses_api_store_enabled: false,
            response_store: None,
            background_jobs_enabled: false,
        });
        let tracker = ResponseTracker::new(state, "http://default:8000/v1".to_string(), Vec::new());

        let prefix = b"data: {\"type\":\"response.completed\",\"emoji\":\"";
        let emoji = "😀".as_bytes();
        let suffix = b"\",\"response\":{\"id\":\"resp_emoji\"}}\n";
        let mut full = prefix.to_vec();
        full.extend_from_slice(emoji);
        full.extend_from_slice(suffix);
        let split_at = prefix.len() + 2;
        let (first, second) = full.split_at(split_at);

        assert_eq!(tracker.find_response(first), None);
        assert_eq!(
            tracker.find_response(second),
            Some(json!({"id": "resp_emoji"}))
        );
    }

    #[test]
    fn drains_complete_sse_lines_from_tracker_buffer() {
        let state = Arc::new(AppState {
            upstream_base: "http://default:8000/v1".to_string(),
            model_upstreams: HashMap::new(),
            default_model: "model-default".to_string(),
            upstream_api_key: None,
            client: Client::new(),
            responses_api_store_enabled: false,
            response_store: None,
            background_jobs_enabled: false,
        });
        let tracker = ResponseTracker::new(state, "http://default:8000/v1".to_string(), Vec::new());

        for i in 0..500 {
            let line = format!("data: {{\"type\":\"other\",\"i\":{i}}}\n");
            assert_eq!(tracker.find_response(line.as_bytes()), None);
        }
        assert_eq!(tracker.buffer_len(), 0);

        assert_eq!(
            tracker.find_response(b"data: {\"type\":\"response.completed\","),
            None
        );
        assert!(
            tracker.buffer_len() < 64,
            "buffer should only retain the incomplete line"
        );
        assert_eq!(
            tracker.find_response(
                b"\"response\":{\"id\":\"resp_drained\",\"object\":\"response\"}}\n"
            ),
            Some(json!({"id": "resp_drained", "object": "response"}))
        );
        assert_eq!(tracker.buffer_len(), 0);
    }

    #[test]
    fn drains_many_sse_lines_delivered_in_one_chunk() {
        let state = Arc::new(AppState {
            upstream_base: "http://default:8000/v1".to_string(),
            model_upstreams: HashMap::new(),
            default_model: "model-default".to_string(),
            upstream_api_key: None,
            client: Client::new(),
            responses_api_store_enabled: false,
            response_store: None,
            background_jobs_enabled: false,
        });
        let tracker = ResponseTracker::new(state, "http://default:8000/v1".to_string(), Vec::new());

        let mut chunk = String::new();
        for i in 0..500 {
            chunk.push_str(&format!("data: {{\"type\":\"other\",\"i\":{i}}}\n"));
        }
        chunk.push_str(
            "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_batched\"}}\n",
        );

        assert_eq!(
            tracker.find_response(chunk.as_bytes()),
            Some(json!({"id": "resp_batched"}))
        );
        assert_eq!(tracker.buffer_len(), 0);
    }

    #[test]
    fn applies_default_anthropic_version_and_upstream_bearer_auth() {
        let state = AppState {
            upstream_base: "http://default:8000/v1".to_string(),
            model_upstreams: HashMap::new(),
            default_model: "model-default".to_string(),
            upstream_api_key: Some("upstream-secret".to_string()),
            client: Client::new(),
            responses_api_store_enabled: false,
            response_store: None,
            background_jobs_enabled: false,
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
            built.headers().get("x-api-key"),
            None,
            "configured upstream key should not be sent as x-api-key"
        );
        assert_eq!(
            built
                .headers()
                .get("authorization")
                .and_then(|v| v.to_str().ok()),
            Some("Bearer upstream-secret")
        );
    }

    #[test]
    fn uses_upstream_bearer_instead_of_client_api_key() {
        let state = AppState {
            upstream_base: "http://default:8000/v1".to_string(),
            model_upstreams: HashMap::new(),
            default_model: "model-default".to_string(),
            upstream_api_key: Some("upstream-secret".to_string()),
            client: Client::new(),
            responses_api_store_enabled: false,
            response_store: None,
            background_jobs_enabled: false,
        };
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", "dummy".parse().unwrap());
        headers.insert("anthropic-version", "2024-01-01".parse().unwrap());
        headers.insert("anthropic-beta", "messages-2024-10-22".parse().unwrap());

        let req =
            apply_anthropic_upstream_headers(Client::new().post("http://test"), &headers, &state);
        let built = req.build().expect("request should build");
        assert_eq!(
            built.headers().get("x-api-key"),
            None,
            "client x-api-key must not be forwarded when UPSTREAM_API_KEY is configured"
        );
        assert_eq!(
            built
                .headers()
                .get("authorization")
                .and_then(|v| v.to_str().ok()),
            Some("Bearer upstream-secret")
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

    #[test]
    fn forwards_client_auth_when_upstream_api_key_unset() {
        let state = AppState {
            upstream_base: "http://default:8000/v1".to_string(),
            model_upstreams: HashMap::new(),
            default_model: "model-default".to_string(),
            upstream_api_key: None,
            client: Client::new(),
            responses_api_store_enabled: false,
            response_store: None,
            background_jobs_enabled: false,
        };
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", "client-secret".parse().unwrap());
        headers.insert("authorization", "Bearer client-bearer".parse().unwrap());

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
                .get("authorization")
                .and_then(|v| v.to_str().ok()),
            Some("Bearer client-bearer")
        );
    }
}
