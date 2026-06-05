use std::{collections::HashMap, env};

use anyhow::{Context, Result};

use crate::store::ResponseStore;

pub fn init_rustls_provider() {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        rustls::crypto::aws_lc_rs::default_provider()
            .install_default()
            .expect("failed to install rustls crypto provider");
    }
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

pub fn parse_model_upstreams(value: Option<String>) -> HashMap<String, String> {
    value
        .unwrap_or_default()
        .split(',')
        .filter_map(|pair| {
            let (model, upstream) = pair.split_once('=')?;
            Some((
                model.trim().to_string(),
                upstream.trim().trim_end_matches('/').to_string(),
            ))
        })
        .filter(|(model, upstream)| !model.is_empty() && !upstream.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn parses_model_upstreams() {
        let upstreams = parse_model_upstreams(Some(
            " model-a = http://model-a:8000/v1/,invalid,=missing-model,missing-upstream= ,\
             model-b=https://model-b.example/v1/ "
                .to_string(),
        ));

        assert_eq!(upstreams.len(), 2);
        assert_eq!(
            upstreams.get("model-a").map(String::as_str),
            Some("http://model-a:8000/v1")
        );
        assert_eq!(
            upstreams.get("model-b").map(String::as_str),
            Some("https://model-b.example/v1")
        );
    }

    #[test]
    fn parses_bool_env_values() {
        env::set_var("DUIHUA_TEST_BOOL", "true");
        assert!(parse_bool_env("DUIHUA_TEST_BOOL", false));
        env::set_var("DUIHUA_TEST_BOOL", "0");
        assert!(!parse_bool_env("DUIHUA_TEST_BOOL", true));
        env::remove_var("DUIHUA_TEST_BOOL");
        assert!(parse_bool_env("DUIHUA_TEST_BOOL", true));
    }
}
