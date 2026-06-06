use std::env;

use anyhow::{Context, Result};
use duihua_common::{
    is_in_flight_background, parse_bool_env, stored_response_status, ResponseStore, StoredResponse,
};
use redis::AsyncCommands;

pub struct BackgroundQueue {
    connection: redis::aio::MultiplexedConnection,
    stream_key: String,
    stale_seconds: i64,
}

pub async fn background_queue_from_env() -> Result<Option<BackgroundQueue>> {
    if !parse_bool_env("RESPONSES_BACKGROUND_ENABLED", false) {
        return Ok(None);
    }

    let url =
        env::var("RESPONSE_ID_STORE_URL").unwrap_or_else(|_| "redis://valkey:6379".to_string());
    let stream_key = env::var("BACKGROUND_QUEUE_STREAM_KEY")
        .unwrap_or_else(|_| "duihua:responses:background".to_string());
    let stale_seconds = env::var("BACKGROUND_RESPONSE_STALE_SECONDS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(3600);

    let client = redis::Client::open(url.as_str())
        .with_context(|| format!("invalid RESPONSE_ID_STORE_URL {url}"))?;
    let connection = client
        .get_multiplexed_async_connection()
        .await
        .with_context(|| format!("failed to connect to background queue at {url}"))?;

    Ok(Some(BackgroundQueue {
        connection,
        stream_key,
        stale_seconds,
    }))
}

impl BackgroundQueue {
    pub async fn enqueue(&self, response_id: &str) -> redis::RedisResult<()> {
        let mut connection = self.connection.clone();
        connection
            .xadd(&self.stream_key, "*", &[("response_id", response_id)])
            .await
    }

    pub async fn reconcile_stale_response(
        &self,
        response_store: &ResponseStore,
        response_id: &str,
        stored: &StoredResponse,
    ) -> Result<StoredResponse> {
        if !should_reconcile_stale(stored) {
            return Ok(stored.clone());
        }

        let now = unix_seconds_now();
        if !is_stale_enqueued(stored.enqueued_at, now, self.stale_seconds) {
            return Ok(stored.clone());
        }

        let mut updated = stored.clone();
        updated.response = serde_json::json!({
            "id": response_id,
            "object": "response",
            "status": "failed",
            "background": true,
            "error": {
                "message": "background response stale",
                "type": "server_error"
            }
        });
        updated.pending_upstream_request = None;
        updated.upstream_authorization = None;

        response_store.store(response_id, &updated).await?;
        Ok(updated)
    }
}

pub fn should_reconcile_stale(stored: &StoredResponse) -> bool {
    is_in_flight_background(stored) && stored_response_status(stored) == Some("queued")
}

pub fn unix_seconds_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

pub fn is_stale_enqueued(enqueued_at: Option<i64>, now: i64, stale_seconds: i64) -> bool {
    let Some(enqueued_at) = enqueued_at else {
        return false;
    };
    now.saturating_sub(enqueued_at) >= stale_seconds
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn stale_requires_enqueued_at() {
        assert!(!is_stale_enqueued(None, 2_000, 60));
    }

    #[test]
    fn detects_stale_enqueued_responses() {
        assert!(!is_stale_enqueued(Some(1_000), 1_050, 60));
        assert!(is_stale_enqueued(Some(1_000), 1_060, 60));
    }

    #[test]
    fn reconciles_only_queued_in_flight_responses() {
        let queued = StoredResponse {
            upstream: "http://model".to_string(),
            response: json!({"status": "queued", "background": true}),
            input: vec![],
            pending_upstream_request: Some(json!({"input": "hi"})),
            upstream_authorization: None,
            enqueued_at: Some(1_000),
        };
        assert!(should_reconcile_stale(&queued));

        let in_progress = StoredResponse {
            upstream: "http://model".to_string(),
            response: json!({"status": "in_progress", "background": true}),
            input: vec![],
            pending_upstream_request: None,
            upstream_authorization: None,
            enqueued_at: Some(1_000),
        };
        assert!(!should_reconcile_stale(&in_progress));
    }
}
