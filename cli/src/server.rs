use std::sync::Arc;

use anyhow::Result;
use axum::{
    Json, Router,
    extract::{
        State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    response::IntoResponse,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::{Mutex, broadcast, mpsc::Sender};

use crate::{
    CoordinatorMsg,
    engine::{OcelEngine, ResourceType},
    ocel::Ocel,
};

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

#[derive(Clone, Deserialize, Serialize, Debug)]
#[serde(tag = "type", content = "data")]
pub enum IpcMessage {
    /// Initial handshake or update
    EnvVars(Vec<(String, String)>),
}

pub struct ServerState {
    pub engine: Arc<Mutex<OcelEngine>>,
    pub tx_broadcast: broadcast::Sender<IpcMessage>,
}

pub async fn start_server(
    engine: Arc<Mutex<OcelEngine>>,
    tx_broadcast: broadcast::Sender<IpcMessage>,
) -> Result<(u16, String)> {
    let state = ServerState {
        engine: engine.clone(),
        tx_broadcast,
    };
    let state = Arc::new(Mutex::new(state));

    let app = Router::new()
        .route("/ws", get(ws_handler))
        .route("/commit", post(flush_handler))
        .route("/register", post(register_handler))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind to address");

    let h = listener.local_addr()?;

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let full_addr = format!("{}:{}", h.ip(), h.port());

    Ok((h.port(), full_addr))
}

pub async fn register_handler(
    State(state): State<Arc<Mutex<ServerState>>>,
    Json(payload): Json<RegisterRequest>,
) -> impl IntoResponse {
    let state = state.lock().await;
    let mut engine = state.engine.lock().await;

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

pub async fn flush_handler(State(state): State<Arc<Mutex<ServerState>>>) -> impl IntoResponse {
    let state = state.lock().await;
    let engine = state.engine.lock().await;

    engine.flush().await.expect("Failed to flush state");

    Json(json!({"status": "flushed"}))
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<Mutex<ServerState>>>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(mut socket: WebSocket, state: Arc<Mutex<ServerState>>) {
    // 1. Setup Phase: Get the Broadcast Receiver AND the Ocel instance
    // We scope this block to ensure locks are dropped immediately
    let (mut rx, ocel) = {
        let state_guard = state.lock().await;
        let engine_guard = state_guard.engine.lock().await;
        (
            state_guard.tx_broadcast.subscribe(),
            engine_guard.get_ocel(), // Ensure this method exists in OcelEngine!
        )
    };

    // 2. Initial Handshake: Fetch current state asynchronously
    // We do this OUTSIDE the lock to keep the server responsive
    // If tofu hasn't run yet, this returns empty, which is fine (app starts with no envs)
    let initial_outputs = ocel.get_tofu_outputs().await.unwrap_or_default();

    // 3. Send Initial State IMMEDIATELY
    let init_msg = IpcMessage::EnvVars(initial_outputs.into_iter().collect());
    let init_json = serde_json::to_string(&init_msg).unwrap();

    if let Err(e) = socket.send(Message::Text(init_json.into())).await {
        tracing::debug!("Client disconnected during handshake: {}", e);
        return;
    }

    // 4. Main Loop: Wait for FUTURE updates
    while let Ok(msg) = rx.recv().await {
        let json = serde_json::to_string(&msg).unwrap();
        if socket.send(Message::Text(json.into())).await.is_err() {
            break;
        }
    }
}
