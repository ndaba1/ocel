use crate::server::IpcMessage;
use anyhow::{Context, Result};
use futures::StreamExt;
use std::process::Stdio;
use tokio::{
    process::{Child, Command},
    signal,
    sync::broadcast,
};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use tracing::{debug, error, info};
use url::Url;

pub async fn run_internal_follower(mut rx: broadcast::Receiver<IpcMessage>, user_cmd: Vec<String>) {
    let mut current_child: Option<Child> = None;
    info!("🔌 Internal follower attached.");

    while let Ok(msg) = rx.recv().await {
        match msg {
            IpcMessage::EnvVars(envs) => {
                restart_process(&mut current_child, &user_cmd, envs).await;
            }
        }
    }
}

// --- 2. Shared Process Logic (The Zombie Killer) ---

async fn restart_process(
    current_child: &mut Option<Child>,
    user_cmd: &[String],
    envs: Vec<(String, String)>,
) {
    // A. Kill existing process group
    if let Some(child) = current_child.take() {
        info!("♻️  Restarting process...");
        kill_process_group(child).await;
    } else {
        info!("🚀 Starting process...");
    }

    // B. Start new process
    if !user_cmd.is_empty() {
        let prog = &user_cmd[0];
        let args = &user_cmd[1..];

        // On Unix, we set a process group so we can kill the whole tree later
        #[cfg(unix)]
        let mut cmd = Command::new(prog);
        #[cfg(unix)]
        cmd.process_group(0); // Create new PGID

        #[cfg(windows)]
        let mut cmd = Command::new(prog);

        let mut env_vars: Vec<(&str, &str)> =
            envs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();

        env_vars.push(("OCEL_DEV", "1"));

        let child_res = cmd
            .args(args)
            .envs(env_vars)
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn();

        match child_res {
            Ok(c) => *current_child = Some(c),
            Err(e) => error!("❌ Failed to start command: {}", e),
        }
    }
}

/// Properly kills a process and its children using Process Groups
async fn kill_process_group(mut child: Child) {
    #[cfg(unix)]
    {
        use nix::sys::signal::{self, Signal};
        use nix::unistd::Pid;

        if let Some(id) = child.id() {
            // Send SIGTERM to the whole process group (negative PID)
            let _ = signal::kill(Pid::from_raw(-(id as i32)), Signal::SIGTERM);
        }
    }

    #[cfg(windows)]
    {
        let _ = child.kill().await;
    }

    let _ = child.wait().await;
}

pub async fn run_follower(leader_port: u16, user_cmd: Vec<String>) -> Result<()> {
    debug!("🔗 Connected to Ocel Leader on port {}", leader_port);

    let url = Url::parse(&format!("ws://127.0.0.1:{}/ws", leader_port))?;
    let (ws_stream, _) = connect_async(url.to_string())
        .await
        .context("Failed to connect to leader")?;

    let (_, mut read) = ws_stream.split();
    let mut current_child: Option<Child> = None;

    let mut last_envs: Option<Vec<(String, String)>> = None;

    // Wait for messages from the Leader
    loop {
        tokio::select! {
            Some(msg) = read.next() => {
                let msg = msg?;
                if let Message::Text(text) = msg {
                    let ipc_msg: IpcMessage = serde_json::from_str(&text)?;

                    match ipc_msg {
                        IpcMessage::EnvVars(mut new_envs) => {
                            // Sort to ensure consistent comparison
                            new_envs.sort();

                            // CHECK: Did anything actually change?
                            if let Some(last) = &last_envs {
                                if last == &new_envs {
                                    debug!(
                                        "Example: Tofu ran, but outputs are identical. Skipping restart."
                                    );
                                    continue;
                                }
                            }

                            // Update state
                            last_envs = Some(new_envs.clone());

                            // Restart Logic (same as before)
                            restart_process(&mut current_child, &user_cmd, new_envs).await;
                        }
                    }
                }
            }

            _ = signal::ctrl_c() => {
                info!("🛑 Received Ctrl+C, shutting down child process...");
                if let Some(child) = current_child.take() {
                    kill_process_group(child).await;
                }
                break;
            }
        }
    }

    Ok(())
}
