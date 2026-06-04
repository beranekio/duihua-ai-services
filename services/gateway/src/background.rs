use std::env;

use anyhow::{Context, Result};
use axum::response::IntoResponse;
use k8s_openapi::{
    api::{
        batch::v1::{Job, JobSpec},
        core::v1::{
            Container, EnvVar, EnvVarSource, ObjectFieldSelector, PodSpec, PodTemplateSpec,
            ResourceRequirements,
        },
    },
    apimachinery::pkg::apis::meta::v1::ObjectMeta,
};
use kube::{
    api::{DeleteParams, PostParams, PropagationPolicy},
    Api, Client, Error,
};
use reqwest::Client as HttpClient;
use serde_json::{json, Value};
use tracing::error;

use crate::{response_store_from_env, AppState, ResponseStore, StoredResponse};

const WORKER_SUBCOMMAND: &str = "background-worker";

pub struct BackgroundJobs {
    client: Client,
    namespace: String,
    image: String,
    image_pull_policy: String,
    service_account_name: String,
    ttl_seconds_after_finished: i32,
    resources: Option<ResourceRequirements>,
}

pub fn is_background_worker_invocation() -> bool {
    env::args().nth(1).as_deref() == Some(WORKER_SUBCOMMAND)
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

    let Some(mut stored) = response_store.load(&response_id).await? else {
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

pub async fn finalize_background_deletion(
    state: &AppState,
    response_id: &str,
    stored: &StoredResponse,
) -> Result<(), axum::response::Response> {
    let Some(response_store) = &state.response_store else {
        error!("responses API store is enabled but no response store is configured");
        return Err((
            axum::http::StatusCode::BAD_GATEWAY,
            "response id store unavailable",
        )
            .into_response());
    };

    if is_in_flight_background(stored) {
        if let Some(background_jobs) = &state.background_jobs {
            if let Err(e) = background_jobs.cancel(response_id).await {
                error!("failed to cancel background job for {response_id}: {e}");
                return Err((
                    axum::http::StatusCode::BAD_GATEWAY,
                    "failed to cancel background response job",
                )
                    .into_response());
            }
        }

        let mut tombstone = stored.clone();
        tombstone.response = json!({
            "id": response_id,
            "object": "response",
            "status": "deleted",
            "background": true,
            "deleted": true
        });
        tombstone.pending_upstream_request = None;
        tombstone.upstream_authorization = None;
        if let Err(e) = response_store.store(response_id, &tombstone).await {
            error!("failed to tombstone deleted background response {response_id}: {e}");
            return Err((
                axum::http::StatusCode::BAD_GATEWAY,
                "response id store write failed",
            )
                .into_response());
        }
        return Ok(());
    }

    if let Err(e) = response_store.delete(response_id).await {
        error!("failed to delete response {response_id}: {e}");
        return Err((
            axum::http::StatusCode::BAD_GATEWAY,
            "response store delete failed",
        )
            .into_response());
    }

    Ok(())
}

pub async fn background_jobs_from_env() -> Result<Option<BackgroundJobs>> {
    if !parse_bool_env("RESPONSES_BACKGROUND_JOBS_ENABLED", false) {
        return Ok(None);
    }

    let namespace = env::var("BACKGROUND_JOB_NAMESPACE")
        .or_else(|_| env::var("POD_NAMESPACE"))
        .unwrap_or_else(|_| "default".to_string());
    let image = env::var("BACKGROUND_JOB_IMAGE").context("BACKGROUND_JOB_IMAGE is required")?;
    let image_pull_policy =
        env::var("BACKGROUND_JOB_IMAGE_PULL_POLICY").unwrap_or_else(|_| "IfNotPresent".to_string());
    let service_account_name =
        env::var("BACKGROUND_JOB_SERVICE_ACCOUNT_NAME").unwrap_or_else(|_| "default".to_string());
    let ttl_seconds_after_finished = env::var("BACKGROUND_JOB_TTL_SECONDS_AFTER_FINISHED")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(600);

    let client = Client::try_default()
        .await
        .context("failed to create Kubernetes client for background jobs")?;

    Ok(Some(BackgroundJobs {
        client,
        namespace,
        image,
        image_pull_policy,
        service_account_name,
        ttl_seconds_after_finished,
        resources: background_job_resources_from_env(),
    }))
}

impl BackgroundJobs {
    pub async fn enqueue(&self, response_id: &str, store_env: &[(&str, &str)]) -> Result<()> {
        let jobs: Api<Job> = Api::namespaced(self.client.clone(), &self.namespace);
        let job = background_job(
            response_id,
            &self.image,
            &self.image_pull_policy,
            &self.service_account_name,
            self.ttl_seconds_after_finished,
            self.resources.clone(),
            store_env,
        )?;
        jobs.create(&PostParams::default(), &job)
            .await
            .with_context(|| format!("failed to create background job for {response_id}"))?;
        Ok(())
    }

    pub async fn cancel(&self, response_id: &str) -> Result<()> {
        let jobs: Api<Job> = Api::namespaced(self.client.clone(), &self.namespace);
        let name = background_job_name(response_id);
        let delete_params = DeleteParams {
            propagation_policy: Some(PropagationPolicy::Background),
            ..Default::default()
        };
        match jobs.delete(&name, &delete_params).await {
            Ok(_) => Ok(()),
            Err(Error::Api(err)) if err.code == 404 => Ok(()),
            Err(err) => Err(err).context(format!("failed to delete background job {name}")),
        }
    }

    pub async fn reconcile_failed_response(
        &self,
        response_store: &ResponseStore,
        response_id: &str,
        stored: &StoredResponse,
    ) -> Result<StoredResponse> {
        if !is_in_flight_background(stored) {
            return Ok(stored.clone());
        }

        let jobs: Api<Job> = Api::namespaced(self.client.clone(), &self.namespace);
        let name = background_job_name(response_id);
        let job = match jobs.get(&name).await {
            Ok(job) => job,
            Err(Error::Api(err)) if err.code == 404 => return Ok(stored.clone()),
            Err(err) => {
                error!("failed to read background job {name} for reconciliation: {err}");
                return Ok(stored.clone());
            }
        };

        if !job_has_failed(&job) {
            return Ok(stored.clone());
        }

        let message = job_failure_message(&job);
        mark_background_failed(response_store, response_id, &message).await?;
        Ok(response_store
            .load(response_id)
            .await?
            .unwrap_or_else(|| stored.clone()))
    }
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

fn background_job_resources_from_env() -> Option<ResourceRequirements> {
    let json = env::var("BACKGROUND_JOB_RESOURCES_JSON").ok()?;
    serde_json::from_str(&json).ok()
}

fn job_has_failed(job: &Job) -> bool {
    let Some(status) = &job.status else {
        return false;
    };
    status.failed.unwrap_or(0) > 0
        || status.conditions.as_ref().is_some_and(|conditions| {
            conditions
                .iter()
                .any(|condition| condition.type_ == "Failed" && condition.status == "True")
        })
}

fn job_failure_message(job: &Job) -> String {
    job.status
        .as_ref()
        .and_then(|status| status.conditions.as_ref())
        .and_then(|conditions| {
            conditions
                .iter()
                .find(|condition| condition.type_ == "Failed" && condition.status == "True")
                .and_then(|condition| condition.message.clone())
        })
        .filter(|message| !message.is_empty())
        .unwrap_or_else(|| "background job failed".to_string())
}

fn background_job(
    response_id: &str,
    image: &str,
    image_pull_policy: &str,
    service_account_name: &str,
    ttl_seconds_after_finished: i32,
    resources: Option<ResourceRequirements>,
    store_env: &[(&str, &str)],
) -> Result<Job> {
    let name = background_job_name(response_id);
    let mut env = vec![
        env_var("BACKGROUND_RESPONSE_ID", response_id),
        env_var_from_field("BACKGROUND_JOB_NAMESPACE", "metadata.namespace"),
    ];
    for (key, value) in store_env {
        env.push(env_var(key, value));
    }

    Ok(Job {
        metadata: ObjectMeta {
            name: Some(name),
            labels: Some(
                [
                    (
                        "app.kubernetes.io/component".to_string(),
                        "background-response".to_string(),
                    ),
                    (
                        "duihua.ai/response-id".to_string(),
                        sanitize_label_value(response_id),
                    ),
                ]
                .into(),
            ),
            ..Default::default()
        },
        spec: Some(JobSpec {
            ttl_seconds_after_finished: Some(ttl_seconds_after_finished),
            backoff_limit: Some(0),
            template: PodTemplateSpec {
                spec: Some(PodSpec {
                    restart_policy: Some("Never".to_string()),
                    service_account_name: Some(service_account_name.to_string()),
                    containers: vec![Container {
                        name: "worker".to_string(),
                        image: Some(image.to_string()),
                        image_pull_policy: Some(image_pull_policy.to_string()),
                        command: Some(vec![
                            "/usr/local/bin/duihua-gateway".to_string(),
                            WORKER_SUBCOMMAND.to_string(),
                        ]),
                        env: Some(env),
                        resources,
                        ..Default::default()
                    }],
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        }),
        ..Default::default()
    })
}

fn env_var(name: &str, value: &str) -> EnvVar {
    EnvVar {
        name: name.to_string(),
        value: Some(value.to_string()),
        ..Default::default()
    }
}

fn env_var_from_field(name: &str, field_path: &str) -> EnvVar {
    EnvVar {
        name: name.to_string(),
        value_from: Some(EnvVarSource {
            field_ref: Some(ObjectFieldSelector {
                field_path: field_path.to_string(),
                ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    }
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

fn sanitize_label_value(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .take(63)
        .collect()
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

pub async fn enqueue_background_response(
    state: &AppState,
    response_id: String,
    upstream: String,
    input: Vec<Value>,
    upstream_request: Value,
    queued_response: Value,
    upstream_authorization: Option<String>,
) -> Result<(), axum::response::Response> {
    let Some(response_store) = &state.response_store else {
        error!("responses API store is enabled but no response store is configured");
        return Err((
            axum::http::StatusCode::BAD_GATEWAY,
            "response id store unavailable",
        )
            .into_response());
    };

    let stored = StoredResponse {
        upstream: upstream.clone(),
        response: queued_response.clone(),
        input,
        pending_upstream_request: Some(upstream_request),
        upstream_authorization,
    };
    if let Err(e) = response_store.store(&response_id, &stored).await {
        error!("failed to store queued background response {response_id}: {e}");
        return Err((
            axum::http::StatusCode::BAD_GATEWAY,
            "response id store write failed",
        )
            .into_response());
    }

    let Some(background_jobs) = &state.background_jobs else {
        error!("background responses require Kubernetes job support");
        let _ = response_store.delete(&response_id).await;
        return Err((
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "background responses require Kubernetes job support",
        )
            .into_response());
    };

    let store_env = background_worker_store_env();
    let store_env_refs: Vec<(&str, &str)> = store_env
        .iter()
        .map(|(key, value)| (*key, value.as_str()))
        .collect();
    if let Err(e) = background_jobs.enqueue(&response_id, &store_env_refs).await {
        error!("failed to enqueue background job for {response_id}: {e}");
        let _ = response_store.delete(&response_id).await;
        return Err((
            axum::http::StatusCode::BAD_GATEWAY,
            "failed to enqueue background response job",
        )
            .into_response());
    }

    Ok(())
}

fn background_worker_store_env() -> Vec<(&'static str, String)> {
    let mut env = Vec::new();
    for name in [
        "RESPONSE_ID_STORE_URL",
        "RESPONSE_ID_STORE_KEY_PREFIX",
        "RESPONSE_ID_STORE_TTL_SECONDS",
        "UPSTREAM_API_KEY",
    ] {
        if let Ok(value) = env::var(name) {
            env.push((name, value));
        }
    }
    env
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
}
