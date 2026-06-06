use std::{env, time::Duration};

use anyhow::{Context, Result};
use duihua_common::{
    response_store_from_env, stored_response_status, ResponseStore, StoredResponse,
};
use reqwest::Client as HttpClient;
use serde_json::{json, Value};

struct ClaimedWork {
    upstream: String,
    upstream_request: Value,
    input: Vec<Value>,
    upstream_authorization: Option<String>,
}

pub async fn run() -> Result<()> {
    let response_id =
        env::var("BACKGROUND_RESPONSE_ID").context("BACKGROUND_RESPONSE_ID is required")?;
    let upstream_api_key = env::var("UPSTREAM_API_KEY").ok();
    let response_store = response_store_from_env().await?;

    let Some(work) = claim_for_processing(&response_store, &response_id).await? else {
        return Ok(());
    };

    let http = upstream_http_client()?;
    let url = format!("{}/responses", work.upstream);
    let mut req = http.post(&url).json(&work.upstream_request);
    if let Some(authorization) = work.upstream_authorization.as_deref() {
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
                    mark_failed(
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
                mark_failed(&response_store, &response_id, &message).await?;
                return Ok(());
            }

            let Ok(mut response) = serde_json::from_slice::<Value>(&body) else {
                mark_failed(
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

            store_completion(
                &response_store,
                &response_id,
                StoredResponse {
                    upstream: work.upstream,
                    response,
                    input: work.input,
                    pending_upstream_request: None,
                    upstream_authorization: None,
                    enqueued_at: None,
                },
            )
            .await?;
        }
        Err(e) => {
            mark_failed(&response_store, &response_id, &e.to_string()).await?;
        }
    }

    Ok(())
}

/// Load queued work and atomically transition it to `in_progress`.
///
/// Re-reads Valkey immediately before writing so a concurrent cancel/delete does
/// not get overwritten. Returns `None` when the response is terminal, missing, or
/// already claimed by another worker.
async fn claim_for_processing(
    response_store: &ResponseStore,
    response_id: &str,
) -> Result<Option<ClaimedWork>> {
    let Some(stored) = response_store.load(response_id).await? else {
        return Ok(None);
    };
    if !is_claimable(&stored) {
        return Ok(None);
    }

    let work = ClaimedWork {
        upstream: stored.upstream.clone(),
        upstream_request: stored
            .pending_upstream_request
            .clone()
            .context("background response is missing pending upstream request")?,
        input: stored.input.clone(),
        upstream_authorization: stored.upstream_authorization.clone(),
    };

    // Re-read before claiming so gateway cancel/delete wins the race.
    let Some(mut stored) = response_store.load(response_id).await? else {
        return Ok(None);
    };
    if !is_claimable(&stored) {
        return Ok(None);
    }

    stored.pending_upstream_request = None;
    stored.response = with_response_status(&stored.response, "in_progress");
    stored.upstream_authorization = None;
    response_store.store(response_id, &stored).await?;

    Ok(Some(work))
}

fn is_claimable(stored: &StoredResponse) -> bool {
    should_persist(stored) && stored.pending_upstream_request.is_some()
}

fn upstream_http_client() -> Result<HttpClient> {
    HttpClient::builder()
        .timeout(upstream_timeout_from_env())
        .build()
        .context("failed to build upstream HTTP client")
}

fn upstream_timeout_from_env() -> Duration {
    env::var("BACKGROUND_UPSTREAM_TIMEOUT_SECONDS")
        .ok()
        .and_then(|value| value.parse().ok())
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(600))
}

fn should_persist(stored: &StoredResponse) -> bool {
    !matches!(
        stored_response_status(stored),
        Some("cancelled") | Some("deleted")
    )
}

fn with_response_status(response: &Value, status: &str) -> Value {
    let mut updated = response.clone();
    updated["status"] = Value::String(status.to_string());
    updated
}

fn merge_completion(current: &StoredResponse, mut completion: StoredResponse) -> StoredResponse {
    completion.enqueued_at = current.enqueued_at;
    completion
}

async fn store_completion(
    response_store: &ResponseStore,
    response_id: &str,
    stored: StoredResponse,
) -> Result<()> {
    let Some(current) = response_store.load(response_id).await? else {
        return Ok(());
    };
    if !should_persist(&current) {
        return Ok(());
    }
    let stored = merge_completion(&current, stored);
    response_store
        .store(response_id, &stored)
        .await
        .context("failed to store completed background response")
}

async fn mark_failed(
    response_store: &ResponseStore,
    response_id: &str,
    message: &str,
) -> Result<()> {
    let Some(mut stored) = response_store.load(response_id).await? else {
        return Ok(());
    };
    if !should_persist(&stored) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use duihua_common::StoredResponse;

    #[test]
    fn skips_terminal_statuses() {
        let cancelled = StoredResponse {
            upstream: "http://model".to_string(),
            response: json!({"status": "cancelled", "background": true}),
            input: vec![],
            pending_upstream_request: None,
            upstream_authorization: None,
            enqueued_at: None,
        };
        assert!(!should_persist(&cancelled));

        let deleted = StoredResponse {
            upstream: "http://model".to_string(),
            response: json!({"status": "deleted", "background": true}),
            input: vec![],
            pending_upstream_request: None,
            upstream_authorization: None,
            enqueued_at: None,
        };
        assert!(!should_persist(&deleted));
    }

    #[test]
    fn claimable_requires_pending_upstream_request() {
        let queued = StoredResponse {
            upstream: "http://model".to_string(),
            response: json!({"status": "queued", "background": true}),
            input: vec![],
            pending_upstream_request: Some(json!({"input": "hi"})),
            upstream_authorization: None,
            enqueued_at: None,
        };
        assert!(is_claimable(&queued));

        let in_progress = StoredResponse {
            upstream: "http://model".to_string(),
            response: json!({"status": "in_progress", "background": true}),
            input: vec![],
            pending_upstream_request: None,
            upstream_authorization: None,
            enqueued_at: None,
        };
        assert!(!is_claimable(&in_progress));
    }

    #[test]
    fn merge_completion_preserves_enqueued_at() {
        let current = StoredResponse {
            upstream: "http://model".to_string(),
            response: json!({"status": "in_progress", "background": true}),
            input: vec![json!({"role": "user", "content": "hi"})],
            pending_upstream_request: None,
            upstream_authorization: None,
            enqueued_at: Some(1_746_500_000),
        };
        let completion = StoredResponse {
            upstream: "http://model".to_string(),
            response: json!({"status": "completed", "background": true}),
            input: vec![json!({"role": "user", "content": "hi"})],
            pending_upstream_request: None,
            upstream_authorization: None,
            enqueued_at: None,
        };

        let merged = merge_completion(&current, completion);
        assert_eq!(merged.enqueued_at, Some(1_746_500_000));
        assert_eq!(stored_response_status(&merged), Some("completed"));
    }

    #[test]
    fn upstream_timeout_reads_env_or_defaults() {
        env::remove_var("BACKGROUND_UPSTREAM_TIMEOUT_SECONDS");
        assert_eq!(upstream_timeout_from_env(), Duration::from_secs(600));

        env::set_var("BACKGROUND_UPSTREAM_TIMEOUT_SECONDS", "120");
        assert_eq!(upstream_timeout_from_env(), Duration::from_secs(120));
        env::remove_var("BACKGROUND_UPSTREAM_TIMEOUT_SECONDS");
    }
}
