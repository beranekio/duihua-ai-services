use std::{env, sync::Arc};

use anyhow::{Context, Result};
use duihua_common::{parse_bool_env, response_store_from_env};
use reqwest::Client;
use tracing::info;

use crate::{
    background,
    config::{init_rustls_provider, parse_model_upstreams},
    routes,
    state::AppState,
};

pub async fn run() -> Result<()> {
    init_rustls_provider();

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

    let app = routes::router(state);

    info!("starting duihua gateway on {bind_addr}");
    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .with_context(|| format!("failed to bind {bind_addr}"))?;

    axum::serve(listener, app).await.context("server failure")?;
    Ok(())
}
