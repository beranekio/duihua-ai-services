use std::collections::HashMap;

use crate::responses_store::StoreHandle;
use reqwest::Client;

pub struct AppState {
    pub upstream_base: String,
    pub model_upstreams: HashMap<String, String>,
    pub default_model: String,
    pub upstream_api_key: Option<String>,
    pub client: Client,
    pub responses_api_store_enabled: bool,
    pub background_jobs_enabled: bool,
    pub response_store: Option<StoreHandle>,
}

pub fn upstream_for_model<'a>(state: &'a AppState, model: &str) -> &'a str {
    state
        .model_upstreams
        .get(model)
        .map(String::as_str)
        .unwrap_or(state.upstream_base.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    use reqwest::Client;

    #[test]
    fn selects_model_specific_upstream_or_default() {
        let state = AppState {
            upstream_base: "http://default:8000/v1".to_string(),
            model_upstreams: HashMap::from([(
                "model-a".to_string(),
                "http://model-a:8000/v1".to_string(),
            )]),
            default_model: "model-default".to_string(),
            upstream_api_key: None,
            client: Client::new(),
            responses_api_store_enabled: false,
            background_jobs_enabled: false,
            response_store: None,
        };

        assert_eq!(
            upstream_for_model(&state, "model-a"),
            "http://model-a:8000/v1"
        );
        assert_eq!(
            upstream_for_model(&state, "model-b"),
            "http://default:8000/v1"
        );
    }
}
