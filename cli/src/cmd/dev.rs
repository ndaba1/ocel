use std::sync::Arc;

use anyhow::Result;
use tokio::sync::{broadcast, mpsc, oneshot};

use crate::coordinator::{CoordinatorMsg, start_coordinator};
use crate::engine::EnvTarget;
use crate::lock::{self, Role};
use crate::server::IpcMessage;
use crate::{follower, utils};
use crate::{ocel::Ocel, project::OcelProject};
use clap::Parser;
use tracing::info;

#[derive(Parser, Debug, Clone)]
pub struct DevOpts {
    #[arg(last = true)]
    dev_cmd: Option<Vec<String>>,
}

pub async fn dev(options: DevOpts) -> Result<()> {
    let DevOpts { dev_cmd: cmd_rest } = options;

    info!("Starting Ocel dev server...");

    let (tx, rx) = mpsc::channel::<CoordinatorMsg>(100);
    let (tx_broadcast, rx_broadcast) = broadcast::channel(16);

    let current_env = whoami::username();
    let project = OcelProject::get_current_project(current_env)?;
    let ocel = Ocel::init(Some(project.clone()), EnvTarget::Dev).await?;
    let ocel = Arc::new(ocel);

    let role = lock::attempt_election(
        &project.project_root,
        ocel.clone(),
        tx.clone(),
        tx_broadcast.clone(),
    )
    .await?;

    match role {
        Role::Follower(leader_info) => {
            info!("Ocel already running. Connecting as follower...");
            if let Some(cmd) = cmd_rest {
                follower::run_follower(leader_info.port, cmd).await?;
            } else {
                info!(
                    "Ocel is running (PID {}). Pass a command to run: ocel dev -- <cmd>",
                    leader_info.pid
                );
            }
        }
        Role::Leader {
            info,
            engine,
            lock_file: _,
        } => {
            info!("Running as leader (watching for changes)");
            info!(
                "Watching infra in {}",
                project.infra_sources.first().unwrap_or(&"".to_string())
            );

            // watch for file changes on .tf.json file
            utils::watcher::start_watcher(
                vec![ocel.get_tf_file_path()],
                tx.clone(),
                CoordinatorMsg::TofuFileChanged,
                |event_kind, _| {
                    matches!(
                        event_kind,
                        notify::EventKind::Modify(_)
                            | notify::EventKind::Create(_)
                            | notify::EventKind::Remove(_)
                    )
                },
            )?;

            // watch for source code changes
            utils::watcher::start_glob_watcher(
                project.project_root.clone(),
                project.infra_sources.clone(),
                tx.clone(),
                |paths| CoordinatorMsg::SourceFileChanged(paths),
            )?;

            // client - discovery infra from source files
            ocel.get_client()?.discover(&info.addr).await?;

            // broadcast initial env vars so the child process starts immediately,
            // even if no infrastructure changes are pending
            let initial_outputs = ocel.get_tofu_outputs().await.unwrap_or_default();
            let initial_msg = IpcMessage::EnvVars(initial_outputs.into_iter().collect());
            let _ = tx_broadcast.send(initial_msg);

            // we are also our own follower for ipc messages
            let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
            let follower_handle = if let Some(cmd) = cmd_rest {
                Some(tokio::spawn(async move {
                    follower::run_internal_follower(rx_broadcast, cmd, shutdown_rx).await;
                }))
            } else {
                None
            };

            start_coordinator(rx, tx, tx_broadcast, ocel.clone(), &info, engine).await?;

            let _ = shutdown_tx.send(());
            if let Some(handle) = follower_handle {
                let _ = tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    handle,
                )
                .await;
            }
        }
    };

    Ok(())
}
