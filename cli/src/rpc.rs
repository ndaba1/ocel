use std::sync::Arc;

use axum::{Json, Router, extract::State, response::IntoResponse, routing::post};
use serde::Deserialize;
use serde_json::json;

use crate::{engine::OcelEngine, ocel::Ocel};

pub struct OcelServer {}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResourceType {
    Bucket,
    Lambda,
}

#[derive(Deserialize)]
pub struct RegisterRequest {
    pub id: String,

    #[serde(rename = "type")]
    pub rtype: ResourceType,

    #[serde(rename = "source")]
    pub source_file: String,

    /// Component specific configuration
    pub config: serde_json::Value,
}

pub async fn register_handler(
    State(engine): State<Arc<tokio::sync::Mutex<OcelEngine>>>,
    Json(payload): Json<RegisterRequest>,
) -> impl IntoResponse {
    println!("Registering resource: {}", payload.id);

    let mut engine = engine.lock().await;

    match engine.register_resource(payload) {
        Ok(_) => (
            axum::http::StatusCode::OK,
            "Resource registered successfully",
        )
            .into_response(),
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to register resource: {}", e),
        )
            .into_response(),
    }
}

pub async fn flush_handler(
    State(engine): State<Arc<tokio::sync::Mutex<OcelEngine>>>,
) -> impl IntoResponse {
    println!("Flushing infrastructure...");

    let engine = engine.lock().await;

    match engine.flush().await {
        Ok(_) => Json(json!({"status": "flushed"})),
        Err(e) => Json(json!({"status": "error", "message": e.to_string()})),
    }
}
