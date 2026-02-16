use std::sync::Mutex;
use std::time::Instant;

use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing_subscriber::fmt::time::FormatTime;

use crate::coordinator::CoordinatorMsg;
use colored::Colorize;

mod bun;
mod client;
mod cmd;
mod components;
mod coordinator;
mod engine;
mod follower;
mod lock;
mod ocel;
mod project;
mod server;
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
    /// Add components to your project (e.g, services, domains, etc)
    Add(cmd::AddOpts),
    /// Run your project in development mode
    Dev(cmd::DevOpts),
    /// Deploy your Ocel project
    Deploy,
    /// Bootstrap your Ocel project infrastructure
    Bootstrap(cmd::BootstrapOpts),
}

struct DeltaTimer {
    // Stores the time of the previous log
    last_time: Mutex<Instant>,
}

impl DeltaTimer {
    fn new() -> Self {
        Self {
            last_time: Mutex::new(Instant::now()),
        }
    }
}

impl FormatTime for DeltaTimer {
    fn format_time(&self, w: &mut tracing_subscriber::fmt::format::Writer<'_>) -> std::fmt::Result {
        let mut last = self.last_time.lock().unwrap();
        let now = Instant::now();
        let delta = now.duration_since(*last);
        *last = now;

        let millis = delta.as_millis();

        // Color logic: Green/Dim for fast, Red for slow
        let output = if millis < 200 {
            format!("+{:>3}ms", millis).green()
        } else if millis < 1000 {
            format!("+{:>3}ms", millis).yellow()
        } else {
            // Show seconds if it's really slow
            format!("+{:>3.1}s ", delta.as_secs_f64()).red().bold()
        };

        write!(w, "{}", output)
    }
}
#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_timer(DeltaTimer::new())
        .init();

    let args = Args::parse();

    match args.command {
        Commands::Init => {
            cmd::init().await?;
        }
        Commands::Dev(opts) => {
            cmd::dev(opts).await?;
        }
        Commands::Bootstrap(opts) => {
            cmd::bootstrap(opts).await?;
        }
        Commands::Add(add_opts) => {
            cmd::add(&add_opts).await?;
        }

        /*
         * deploy is basically dev without the file watcher
         * also, we need a way to tell the engine we are in "production" mode, so it can behave accordingly
         * however, deploy still need to deploy "apps"/"services" defined in the ocel project
         */
        Commands::Deploy => {
            println!("Deploying ocel project...");
        }
    }

    Ok(())
}
