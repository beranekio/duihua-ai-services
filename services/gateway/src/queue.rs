use std::env;

use anyhow::{Context, Result};
use duihua_common::{is_in_flight_background, parse_bool_env, ResponseStore, StoredResponse};
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
        if !is_in_flight_background(stored) {
            return Ok(stored.clone());
        }

        let now = unix_seconds_now();
        if !is_stale_enqueued(stored.enqueued_at, now, self.stale_seconds) {
            return Ok(stored.clone());
        }

        mark_background_stale(response_store, response_id).await?;
        Ok(response_store
            .load(response_id)
            .await?
            .unwrap_or_else(|| stored.clone()))
    }
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

async fn mark_background_stale(response_store: &ResponseStore, response_id: &str) -> Result<()> {
    let Some(mut stored) = response_store.load(response_id).await? else {
        return Ok(());
    };
    if !is_in_flight_background(&stored) {
        return Ok(());
    }
    stored.response = serde_json::json!({
        "id": response_id,
        "object": "response",
        "status": "failed",
        "background": true,
        "error": {
            "message": "background response stale",
            "type": "server_error"
        }
    });
    stored.pending_upstream_request = None;
    stored.upstream_authorization = None;
    response_store.store(response_id, &stored).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_requires_enqueued_at() {
        assert!(!is_stale_enqueued(None, 2_000, 60));
    }

    #[test]
    fn detects_stale_enqueued_responses() {
        assert!(!is_stale_enqueued(Some(1_000), 1_050, 60));
        assert!(is_stale_enqueued(Some(1_000), 1_060, 60));
    }
}
