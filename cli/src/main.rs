use std::collections::HashMap;
use std::fs;
use std::sync::Arc;

use anyhow::Result;
use axum::{Router, routing::post};
use clap::{Parser, Subcommand};
use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};

use crate::rpc::{flush_handler, register_handler};
use crate::{engine::OcelEngine, ocel::Ocel, project::OcelProject};

mod bun;
mod client;
mod cmd;
mod components;
mod engine;
mod ocel;
mod project;
mod rpc;
mod tofu;
mod utils;

#[derive(Parser)]
#[command(author, version, about = "Ocel CLI", long_about = None)]
struct Args {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug, Clone)]
enum Commands {
    /// Create a new Ocel project
    Init,
    /// Run your project in development mode
    Dev,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    match args.command {
        Commands::Init => {
            cmd::init()?;
        }
        Commands::Dev => {
            println!("Running ocel in development mode...");

            // TODO: allow passing custom env name
            let mut ocel = Ocel::init()?;
            let current_env = whoami::username();
            let project = OcelProject::get_current_project(current_env)?;

            ocel.set_current_project(&project);

            let ocel_arc = Arc::new(ocel);
            let engine = Arc::new(tokio::sync::Mutex::new(OcelEngine::new(ocel_arc.clone())));

            let app = Router::new()
                .route("/commit", post(flush_handler))
                .route("/register", post(register_handler))
                .with_state(engine);

            let listener = tokio::net::TcpListener::bind("0.0.0.0:8080")
                .await
                .expect("Failed to bind to address");

            tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            });

            let work_dir = ocel_arc
                .current_project
                .as_ref()
                .unwrap()
                .current_env_dir
                .clone();

            let tf_file = work_dir.join("main.tf.json");
            if !tf_file.exists() {
                fs::write(&tf_file, "{}")?;
            }

            let ocel = ocel_arc.clone();
            tokio::spawn(async move {
                let (tx, mut rx) = tokio::sync::mpsc::channel(1);
                let mut watcher = RecommendedWatcher::new(
                    move |res| {
                        let _ = tx.blocking_send(res);
                    },
                    Config::default(),
                )
                .unwrap();
                watcher
                    .watch(&tf_file, RecursiveMode::NonRecursive)
                    .unwrap();

                while let Some(_) = rx.recv().await {
                    ocel.run_tofu(&["apply", "-auto-approve"], None)
                        .expect("Failed to run tofu apply");
                }
            });

            let client = ocel_arc.get_client()?;
            client.start_dev()?
        }
    }

    Ok(())
}
