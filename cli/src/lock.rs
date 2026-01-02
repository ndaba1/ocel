use anyhow::Result;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;
use sysinfo::{Pid, System};
use tokio::sync::mpsc::Sender;
use tokio::sync::{Mutex, broadcast};

use crate::CoordinatorMsg;
use crate::engine::OcelEngine;
use crate::ocel::Ocel;
use crate::server::{IpcMessage, start_server};

#[derive(Serialize, Deserialize, Debug)]
pub struct LeaderInfo {
    pub pid: u32,
    pub port: u16,
    pub addr: String,
}

pub enum Role {
    #[allow(unused)]
    Leader {
        lock_file: File,
        info: LeaderInfo,
        engine: Arc<Mutex<OcelEngine>>,
    },
    Follower(LeaderInfo),
}

pub async fn attempt_election(
    project_dir: &PathBuf,
    ocel: Arc<Ocel>,
    tx: Sender<CoordinatorMsg>,
    tx_broadcast: broadcast::Sender<IpcMessage>,
) -> Result<Role> {
    let lock_path = project_dir.join(".ocel").join("daemon.lock");
    fs::create_dir_all(lock_path.parent().unwrap())?;

    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(&lock_path)?;

    // 1. Try to acquire an exclusive lock
    match file.try_lock_exclusive() {
        Ok(_) => {
            let engine = Arc::new(Mutex::new(OcelEngine::new(ocel.clone(), tx.clone())));
            let (port, addr) = start_server(engine.clone(), tx_broadcast.clone()).await?;

            // We are the LEADER
            let info = LeaderInfo {
                pid: std::process::id(),
                port,
                addr,
            };

            file.set_len(0)?; // Clear file
            let writer = &file;
            serde_json::to_writer(writer, &info)?;

            // Return the file handle so the lock persists until we exit
            Ok(Role::Leader {
                lock_file: file,
                info,
                engine,
            })
        }
        Err(_) => {
            // We are a FOLLOWER.
            let mut content = String::new();
            let mut reader = &file;
            reader.read_to_string(&mut content)?;

            let info: LeaderInfo = serde_json::from_str(&content)?;

            // Double check: Is that process actually alive?
            let mut sys = System::new_all();
            sys.refresh_all();

            if !sys.process(Pid::from(info.pid as usize)).is_some() {
                // Stale lock file (Leader crashed).
                // In a real app, you might try to delete it and retry election.
                anyhow::bail!(
                    "Lock file exists but process {} is dead. Please delete .ocel/daemon.lock",
                    info.pid
                );
            }

            Ok(Role::Follower(info))
        }
    }
}
