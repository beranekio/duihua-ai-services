use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::error;

use crate::{background, error::response_not_found, state::AppState};

#[derive(Clone)]
pub struct ResponseStore {
    pub(crate) connection: redis::aio::MultiplexedConnection,
    pub(crate) key_prefix: String,
    pub(crate) ttl_seconds: u64,
}

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
    };
    if let Err(e) = response_store.store(&response_id, &stored).await {
        error!("failed to store response {response_id}: {e}");
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
}
