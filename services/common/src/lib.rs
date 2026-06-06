use std::env;

use anyhow::{Context, Result};
use redis::AsyncCommands;
use reqwest::Client as HttpClient;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Clone, Deserialize, Serialize)]
pub struct StoredResponse {
    pub upstream: String,
    pub response: Value,
    pub input: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_upstream_request: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream_authorization: Option<String>,
}

#[derive(Clone)]
pub struct ResponseStore {
    connection: redis::aio::MultiplexedConnection,
    key_prefix: String,
    ttl_seconds: u64,
}

pub async fn response_store_from_env() -> Result<ResponseStore> {
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

pub fn parse_bool_env(name: &str, default: bool) -> bool {
    env::var(name)
        .ok()
        .and_then(|value| match value.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "0" | "false" | "no" | "off" => Some(false),
            _ => None,
        })
        .unwrap_or(default)
}

impl ResponseStore {
    pub async fn store(
        &self,
        response_id: &str,
        response: &StoredResponse,
    ) -> redis::RedisResult<()> {
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

    pub async fn load(&self, response_id: &str) -> redis::RedisResult<Option<StoredResponse>> {
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

    pub async fn delete(&self, response_id: &str) -> redis::RedisResult<()> {
        let mut connection = self.connection.clone();
        connection.del(self.key(response_id)).await
    }

    fn key(&self, response_id: &str) -> String {
        response_store_key(&self.key_prefix, response_id)
    }
}

pub fn response_store_key(prefix: &str, response_id: &str) -> String {
    format!("{prefix}:{response_id}")
}

pub fn response_id_from_value(value: &Value) -> Option<String> {
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

pub fn stored_response_status(stored: &StoredResponse) -> Option<&str> {
    stored.response.get("status").and_then(Value::as_str)
}

pub fn is_in_flight_background(stored: &StoredResponse) -> bool {
    stored.pending_upstream_request.is_some()
        || matches!(
            stored_response_status(stored),
            Some("queued") | Some("in_progress")
        )
}

fn should_worker_persist(stored: &StoredResponse) -> bool {
    !matches!(
        stored_response_status(stored),
        Some("cancelled") | Some("deleted")
    )
}

pub async fn run_background_worker() -> Result<()> {
    let response_id =
        env::var("BACKGROUND_RESPONSE_ID").context("BACKGROUND_RESPONSE_ID is required")?;
    let upstream_api_key = env::var("UPSTREAM_API_KEY").ok();
    let response_store = response_store_from_env().await?;

    let Some(stored) = response_store.load(&response_id).await? else {
        return Ok(());
    };
    if !should_worker_persist(&stored) {
        return Ok(());
    }

    let upstream_authorization = stored.upstream_authorization.clone();
    let upstream_request = stored
        .pending_upstream_request
        .clone()
        .context("background response is missing pending upstream request")?;
    let upstream = stored.upstream.clone();
    let input = stored.input.clone();

    let Some(mut stored) = response_store.load(&response_id).await? else {
        return Ok(());
    };
    if !should_worker_persist(&stored) {
        return Ok(());
    }
    if stored.pending_upstream_request.is_none() {
        return Ok(());
    }

    stored.pending_upstream_request = None;
    stored.response = with_response_status(&stored.response, "in_progress");
    stored.upstream_authorization = None;
    response_store.store(&response_id, &stored).await?;

    let http = HttpClient::new();
    let url = format!("{upstream}/responses");
    let mut req = http.post(&url).json(&upstream_request);
    if let Some(authorization) = upstream_authorization.as_deref() {
        req = req.header("authorization", authorization);
    } else if let Some(api_key) = upstream_api_key {
        req = req.bearer_auth(api_key);
    }

    match req.send().await {
        Ok(resp) => {
            let status = resp.status();
            let body = match resp.bytes().await {
                Ok(body) => body,
                Err(e) => {
                    mark_background_failed(
                        &response_store,
                        &response_id,
                        &format!("failed to read upstream background response body: {e}"),
                    )
                    .await?;
                    return Ok(());
                }
            };
            if !status.is_success() {
                let message = String::from_utf8_lossy(&body);
                mark_background_failed(&response_store, &response_id, &message).await?;
                return Ok(());
            }

            let Ok(mut response) = serde_json::from_slice::<Value>(&body) else {
                mark_background_failed(
                    &response_store,
                    &response_id,
                    "upstream returned invalid JSON",
                )
                .await?;
                return Ok(());
            };
            response["id"] = Value::String(response_id.clone());
            response["background"] = Value::Bool(true);
            if response.get("status").is_none() {
                response["status"] = Value::String("completed".to_string());
            }

            store_background_completion(
                &response_store,
                &response_id,
                StoredResponse {
                    upstream,
                    response,
                    input,
                    pending_upstream_request: None,
                    upstream_authorization: None,
                },
            )
            .await?;
        }
        Err(e) => {
            mark_background_failed(&response_store, &response_id, &e.to_string()).await?;
        }
    }

    Ok(())
}

async fn store_background_completion(
    response_store: &ResponseStore,
    response_id: &str,
    stored: StoredResponse,
) -> Result<()> {
    let Some(current) = response_store.load(response_id).await? else {
        return Ok(());
    };
    if !should_worker_persist(&current) {
        return Ok(());
    }
    response_store
        .store(response_id, &stored)
        .await
        .context("failed to store completed background response")
}

async fn mark_background_failed(
    response_store: &ResponseStore,
    response_id: &str,
    message: &str,
) -> Result<()> {
    let Some(mut stored) = response_store.load(response_id).await? else {
        return Ok(());
    };
    if !should_worker_persist(&stored) {
        return Ok(());
    }
    stored.response = json!({
        "id": response_id,
        "object": "response",
        "status": "failed",
        "background": true,
        "error": {
            "message": message,
            "type": "server_error"
        }
    });
    stored.pending_upstream_request = None;
    stored.upstream_authorization = None;
    response_store.store(response_id, &stored).await?;
    Ok(())
}

pub fn generate_response_id() -> String {
    format!("resp_{}", uuid::Uuid::new_v4().simple())
}

pub fn build_queued_response(response_id: &str, model: &str, request: &Value) -> Value {
    let mut response = json!({
        "id": response_id,
        "object": "response",
        "status": "queued",
        "model": model,
        "background": true,
        "output": []
    });
    if let Some(input) = request.get("input") {
        response["input"] = input.clone();
    }
    response
}

pub fn build_upstream_request(request: &Value) -> Value {
    let mut upstream = request.clone();
    if let Some(obj) = upstream.as_object_mut() {
        obj.remove("background");
        obj.remove("previous_response_id");
        obj.insert("store".to_string(), Value::Bool(false));
    }
    upstream
}

pub fn with_response_status(response: &Value, status: &str) -> Value {
    let mut updated = response.clone();
    updated["status"] = Value::String(status.to_string());
    updated
}

pub fn build_cancelled_response(stored: &StoredResponse, response_id: &str) -> Value {
    let mut response = stored.response.clone();
    response["id"] = Value::String(response_id.to_string());
    response["status"] = Value::String("cancelled".to_string());
    response["background"] = Value::Bool(true);
    response
}

pub fn background_job_name(response_id: &str) -> String {
    let suffix = response_id
        .trim_start_matches("resp_")
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    let name = format!("duihua-bg-{suffix}");
    name.chars().take(63).collect()
}

pub fn init_rustls_provider() {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        rustls::crypto::aws_lc_rs::default_provider()
            .install_default()
            .expect("failed to install rustls crypto provider");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_stable_background_job_names() {
        assert_eq!(
            background_job_name("resp_abcDEF-123"),
            "duihua-bg-abcdef-123"
        );
    }

    #[test]
    fn strips_background_from_upstream_request() {
        let request = json!({
            "model": "demo",
            "input": "hello",
            "background": true,
            "previous_response_id": "resp_old",
            "store": true
        });
        assert_eq!(
            build_upstream_request(&request),
            json!({
                "model": "demo",
                "input": "hello",
                "store": false
            })
        );
    }

    #[test]
    fn detects_in_flight_background_responses() {
        let queued = StoredResponse {
            upstream: "http://model".to_string(),
            response: json!({"status": "queued", "background": true}),
            input: vec![],
            pending_upstream_request: Some(json!({"input": "hi"})),
            upstream_authorization: None,
        };
        assert!(is_in_flight_background(&queued));

        let completed = StoredResponse {
            upstream: "http://model".to_string(),
            response: json!({"status": "completed", "background": true}),
            input: vec![],
            pending_upstream_request: None,
            upstream_authorization: None,
        };
        assert!(!is_in_flight_background(&completed));
    }

    #[test]
    fn worker_skips_terminal_statuses() {
        let cancelled = StoredResponse {
            upstream: "http://model".to_string(),
            response: json!({"status": "cancelled", "background": true}),
            input: vec![],
            pending_upstream_request: None,
            upstream_authorization: None,
        };
        assert!(!should_worker_persist(&cancelled));

        let deleted = StoredResponse {
            upstream: "http://model".to_string(),
            response: json!({"status": "deleted", "background": true}),
            input: vec![],
            pending_upstream_request: None,
            upstream_authorization: None,
        };
        assert!(!should_worker_persist(&deleted));
    }

    #[test]
    fn extracts_response_id_from_response_objects() {
        let response = json!({ "id": "resp_123", "object": "response" });
        assert_eq!(
            response_id_from_value(&response).as_deref(),
            Some("resp_123")
        );

        let stream_event = json!({
            "type": "response.created",
            "response": { "id": "resp_456", "object": "response" }
        });
        assert_eq!(
            response_id_from_value(&stream_event).as_deref(),
            Some("resp_456")
        );
    }

    #[test]
    fn builds_response_store_keys_with_prefix() {
        assert_eq!(
            response_store_key("duihua:responses", "resp_model_a"),
            "duihua:responses:resp_model_a"
        );
    }
}
