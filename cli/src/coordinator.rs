use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use clap::{Parser, Subcommand};
use tokio::sync::{Mutex, broadcast, mpsc};
use tracing_subscriber::fmt::time::FormatTime;

use crate::components::lambda::LambdaComponent;
use crate::engine::{self, OcelEngine};
use crate::lock::{LeaderInfo, Role};
use crate::server::IpcMessage;
use crate::{ocel::Ocel, project::OcelProject};
use colored::Colorize;
use tokio::signal;
use tracing::{debug, error, info};

#[derive(Debug, Clone)]
pub enum CoordinatorMsg {
    /// A component wants to write new infrastructure state
    CommitState(serde_json::Value),
    /// The file watcher noticed the main.tf.json changed on disk
    TofuFileChanged,
    // changes to js/ts files
    SourceFileChanged(Vec<PathBuf>),
    /// A request to trigger a full reconciliation (e.g., after docker start)
    Reconcile,
    ReconcileDone,
}

pub async fn start_coordinator(
    mut rx: mpsc::Receiver<CoordinatorMsg>,
    tx: mpsc::Sender<CoordinatorMsg>,
    tx_broadcast: broadcast::Sender<IpcMessage>,
    ocel: Arc<Ocel>,
    info: &LeaderInfo,
    engine: Arc<Mutex<OcelEngine>>,
) -> Result<()> {
    let mut is_reconciling = false;
    let mut is_discovering = false;
    let mut pending_reconcile = false;

    loop {
        tokio::select! {
            Some(msg) = rx.recv() => {
                match msg {
                    // TODO: if only lambda changed, skip discovery
                    CoordinatorMsg::SourceFileChanged(paths) => {

                        debug!("Following source files changed: {:?}", paths);

                        if !is_discovering {
                            is_discovering = true;
                            debug!("📝 Source change detected. Re-running discovery...");

                            let engine = engine.lock().await;
                            engine.process_changes(paths, info).await?;

                            is_discovering = false;
                        }
                    }


                    CoordinatorMsg::CommitState(new_state) => {
                        debug!("💾 State change detected, saving...");

                        // writing state triggers file change, which triggers reconcile
                        if let Err(e) = ocel.write_state(new_state).await {
                            error!("Failed to write state: {}", e);
                            continue;
                        }

                    }
                    CoordinatorMsg::TofuFileChanged => {
                        if is_reconciling {
                            // If busy, mark as dirty so we run again immediately after finishing
                            pending_reconcile = true;
                        } else {
                            // Triggers the logic below
                            tx.send(CoordinatorMsg::Reconcile).await?;
                        }
                    }
                    CoordinatorMsg::Reconcile => {
                        if is_reconciling {
                            pending_reconcile = true;
                        } else {
                            is_reconciling = true;
                            let ocel_ref = ocel.clone();
                            let tx_ref = tx.clone();
                            let tx_broadcast = tx_broadcast.clone();

                            // Spawn the heavy I/O task so we don't block the loop
                            // But track it so we don't spawn 10 at once
                            tokio::spawn(async move {
                                debug!("🔄 Syncing Infrastructure...");
                                let envs = ocel_ref.get_tofu_outputs().await.unwrap_or_default();

                                match ocel_ref.run_tofu(&["apply", "-refresh=false", "-auto-approve"], Some(&envs)).await {
                                    Ok(_) => debug!("✅ Infrastructure Synced."),
                                    Err(e) => error!("❌ Sync Failed: {}", e),
                                }

                                let _ = tx_ref.send(CoordinatorMsg::ReconcileDone).await;

                                // broadcast new env vars to followers
                                let new_outputs = ocel_ref.get_tofu_outputs().await.unwrap_or_default();
                                let msg = IpcMessage::EnvVars(new_outputs.into_iter().collect());

                                let _ = tx_broadcast.send(msg);
                            });

                        }
                    }
                    CoordinatorMsg::ReconcileDone => {
                        is_reconciling = false;

                        // file changed again while we were reconciling?
                        if pending_reconcile {
                            tx.send(CoordinatorMsg::Reconcile).await?;
                        }

                        debug!("✅ Reconciliation complete.");
                    }
                }
            }
            _ = signal::ctrl_c() => {
                debug!("🛑 Shutdown signal received.");
                break;
            }
        };
    }

    Ok(())
}
