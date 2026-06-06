use std::env;

use anyhow::{Context, Result};
use redis::AsyncCommands;
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
