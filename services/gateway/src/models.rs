use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use responses_api_store_client::StoredResponse;

#[derive(Serialize)]
pub struct ModelListResponse {
    pub object: &'static str,
    pub data: Vec<ModelItem>,
}

#[derive(Serialize)]
pub struct ModelItem {
    pub id: String,
    pub object: &'static str,
    pub owned_by: &'static str,
}

#[derive(Deserialize, Serialize)]
pub struct ChatCompletionRequest {
    pub model: Option<String>,
    pub messages: Vec<Value>,
    #[serde(flatten)]
    pub extra: Value,
}

#[derive(Deserialize, Serialize)]
pub struct MessagesRequest {
    pub model: Option<String>,
    #[serde(flatten)]
    pub extra: Value,
}

#[derive(Deserialize, Serialize)]
pub struct EmbeddingsRequest {
    pub model: Option<String>,
    pub input: Value,
    #[serde(flatten)]
    pub extra: Value,
}

#[derive(Deserialize, Serialize)]
pub struct ResponsesRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_response_id: Option<String>,
    #[serde(flatten)]
    pub extra: Value,
}

pub fn request_input(request: &ResponsesRequest) -> Option<&Value> {
    request.extra.get("input")
}

pub fn normalized_input(input: Option<&Value>) -> Vec<Value> {
    match input {
        Some(Value::Array(items)) => items.clone(),
        Some(Value::String(text)) => vec![json!({"role": "user", "content": text})],
        Some(input) if !input.is_null() => vec![input.clone()],
        _ => Vec::new(),
    }
}

pub fn continuation_input(previous: &StoredResponse, input: Option<&Value>) -> Vec<Value> {
    let mut messages = previous.input.clone();
    if let Some(output) = previous.response.get("output").and_then(Value::as_array) {
        messages.extend(output.iter().cloned());
    }
    messages.extend(normalized_input(input));
    messages
}

pub fn set_request_input(request: &mut ResponsesRequest, input: Vec<Value>) {
    request.extra["input"] = Value::Array(input);
}

pub fn should_store_response(request: &ResponsesRequest) -> bool {
    request.extra.get("store").and_then(Value::as_bool) != Some(false)
}

pub fn disable_upstream_response_store(request: &mut ResponsesRequest) {
    request.extra["store"] = Value::Bool(false);
}

pub fn should_persist_gateway_response(store_enabled: bool, request: &ResponsesRequest) -> bool {
    store_enabled && should_store_response(request)
}

pub fn response_model(response: &Value) -> Option<String> {
    response
        .get("model")
        .and_then(Value::as_str)
        .map(str::to_string)
}

pub fn is_background_request(request: &ResponsesRequest) -> bool {
    request.extra.get("background").and_then(Value::as_bool) == Some(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn deserializes_previous_response_id_without_model() {
        let request = serde_json::from_value::<ResponsesRequest>(serde_json::json!({
            "previous_response_id": "resp_prior",
            "input": "continue"
        }))
        .expect("valid responses request");

        assert_eq!(request.model, None);
        assert_eq!(request.previous_response_id.as_deref(), Some("resp_prior"));
    }

    #[test]
    fn materializes_continuation_input() {
        let previous = StoredResponse {
            upstream: "http://model-a:8000/v1".to_string(),
            response: json!({
                "id": "resp_previous",
                "model": "model-a",
                "output": [{"role": "assistant", "content": "prior answer"}]
            }),
            input: vec![json!({"role": "user", "content": "prior question"})],
            pending_upstream_request: None,
            upstream_authorization: None,
            enqueued_at: None,
        };

        assert_eq!(
            continuation_input(&previous, Some(&json!("next question"))),
            vec![
                json!({"role": "user", "content": "prior question"}),
                json!({"role": "assistant", "content": "prior answer"}),
                json!({"role": "user", "content": "next question"}),
            ]
        );
    }

    #[test]
    fn honors_explicit_response_store_flag() {
        let default_request = serde_json::from_value::<ResponsesRequest>(json!({
            "input": "persist by default"
        }))
        .expect("valid responses request");
        assert!(should_store_response(&default_request));

        let stored_request = serde_json::from_value::<ResponsesRequest>(json!({
            "input": "persist explicitly",
            "store": true
        }))
        .expect("valid responses request");
        assert!(should_store_response(&stored_request));

        let unpersisted_request = serde_json::from_value::<ResponsesRequest>(json!({
            "input": "do not persist",
            "store": false
        }))
        .expect("valid responses request");
        assert!(!should_store_response(&unpersisted_request));
    }

    #[test]
    fn preserves_gateway_persistence_decision_before_disabling_upstream_store() {
        let mut default_request = serde_json::from_value::<ResponsesRequest>(json!({
            "input": "persist by default"
        }))
        .expect("valid responses request");

        let persist_response = should_persist_gateway_response(true, &default_request);
        disable_upstream_response_store(&mut default_request);

        assert!(persist_response);
        assert!(!should_store_response(&default_request));

        let unpersisted_request = serde_json::from_value::<ResponsesRequest>(json!({
            "input": "do not persist",
            "store": false
        }))
        .expect("valid responses request");
        assert!(!should_persist_gateway_response(true, &unpersisted_request));
    }

    #[test]
    fn serializes_stateless_continuation_request() {
        let mut request = serde_json::from_value::<ResponsesRequest>(json!({
            "previous_response_id": "resp_prior",
            "input": "continue"
        }))
        .expect("valid responses request");
        request.previous_response_id = None;
        set_request_input(
            &mut request,
            vec![json!({"role": "user", "content": "continue"})],
        );
        disable_upstream_response_store(&mut request);

        assert_eq!(
            serde_json::to_value(request).expect("serializable request"),
            json!({
                "input": [{"role": "user", "content": "continue"}],
                "store": false
            })
        );
    }

    #[test]
    fn deserializes_messages_request_without_model() {
        let request = serde_json::from_value::<MessagesRequest>(json!({
            "max_tokens": 256,
            "messages": [{"role": "user", "content": "Hello"}]
        }))
        .expect("valid messages request");

        assert_eq!(request.model, None);
    }
}
