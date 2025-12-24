use std::sync::Arc;

use axum::{
    Json, Router,
    extract::State,
    response::IntoResponse,
    routing::{get, post},
};
use serde::Deserialize;

use crate::{
    engine::{self, OcelEngine},
    ocel::Ocel,
};

pub struct OcelServer {
    engine: Arc<OcelEngine>,
}

#[derive(Deserialize)]
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

impl OcelServer {
    pub fn new(engine: Arc<OcelEngine>) -> Self {
        OcelServer { engine }
    }

    pub async fn start(self) {
        let engine = self.engine.clone();
        let app = Router::new()
            .route("/register", post(register_handler))
            .with_state(engine);

        let listener = tokio::net::TcpListener::bind("0.0.0.0:8080")
            .await
            .expect("Failed to bind to address");

        axum::serve(listener, app).await.expect("Server failed");
    }
}

async fn register_handler(
    State(engine): State<Arc<OcelEngine>>,
    Json(payload): Json<RegisterRequest>,
) -> impl IntoResponse {
    // let mut engine = engine.lo
    // match engine.register_resource(payload.id, payload) {
    //     Ok(_) => (
    //         axum::http::StatusCode::OK,
    //         "Resource registered successfully",
    //     )
    //         .into_response(),
    //     Err(e) => (
    //         axum::http::StatusCode::INTERNAL_SERVER_ERROR,
    //         format!("Failed to register resource: {}", e),
    //     )
    //         .into_response(),
    // }

    (
        axum::http::StatusCode::OK,
        "Resource registration endpoint is under construction",
    )
        .into_response()
}
