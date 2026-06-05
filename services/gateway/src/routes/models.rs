use std::sync::Arc;

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};

use crate::{
    models::{ModelItem, ModelListResponse},
    state::AppState,
};

pub async fn list_models(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let mut models: Vec<String> = state.model_upstreams.keys().cloned().collect();
    if !models.iter().any(|m| m == &state.default_model) {
        models.push(state.default_model.clone());
    }
    models.sort();

    let body = ModelListResponse {
        object: "list",
        data: models
            .into_iter()
            .map(|id| ModelItem {
                id,
                object: "model",
                owned_by: "duihua",
            })
            .collect(),
    };

    (StatusCode::OK, Json(body))
}
