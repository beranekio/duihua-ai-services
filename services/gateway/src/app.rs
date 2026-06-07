use std::{env, sync::Arc, time::Duration};

use crate::config::parse_bool_env;
use crate::responses_store::{connect_from_env, StoreHandle};
use anyhow::{Context, Result};
use reqwest::Client;
use tracing::info;

use crate::{
    config::{init_rustls_provider, parse_model_upstreams},
    routes,
    state::AppState,
};

const ENSURE_CONSUMER_GROUP_ATTEMPTS: usize = 30;
const ENSURE_CONSUMER_GROUP_RETRY_DELAY: Duration = Duration::from_secs(2);

async fn ensure_background_consumer_group(
    response_store: &StoreHandle,
    consumer_group: &str,
) -> Result<()> {
    let mut last_err = None;
    for attempt in 1..=ENSURE_CONSUMER_GROUP_ATTEMPTS {
        match response_store
            .ensure_consumer_group(consumer_group, "0")
            .await
        {
            Ok(_) => return Ok(()),
            Err(err) => {
                last_err = Some(err);
                if attempt < ENSURE_CONSUMER_GROUP_ATTEMPTS {
                    tokio::time::sleep(ENSURE_CONSUMER_GROUP_RETRY_DELAY).await;
                }
            }
        }
    }

    Err(last_err.expect("ensure_consumer_group error after retries"))
        .context("failed to ensure background queue consumer group")
}

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
    let background_jobs_enabled =
        responses_api_store_enabled && parse_bool_env("RESPONSES_BACKGROUND_ENABLED", false);
    let response_store = if responses_api_store_enabled {
        Some(connect_from_env().await?)
    } else {
        None
    };

    if background_jobs_enabled {
        if let Some(response_store) = &response_store {
            let consumer_group = env::var("BACKGROUND_QUEUE_CONSUMER_GROUP")
                .unwrap_or_else(|_| "duihua-background".to_string());
            ensure_background_consumer_group(response_store, &consumer_group).await?;
        }
    }

    let state = Arc::new(AppState {
        upstream_base,
        model_upstreams,
        default_model,
        upstream_api_key,
        client: Client::new(),
        responses_api_store_enabled,
        background_jobs_enabled,
        response_store,
    });

    let app = routes::router(state);

    info!("starting duihua gateway on {bind_addr}");
    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .with_context(|| format!("failed to bind {bind_addr}"))?;

    axum::serve(listener, app).await.context("server failure")?;
    Ok(())
}
