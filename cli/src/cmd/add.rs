use clap::{Args, Parser, Subcommand};

use crate::{engine::EnvTarget, ocel::Ocel, project::OcelProject};

#[derive(Parser, Debug, Clone)]
pub struct AddOpts {
    #[command(subcommand)]
    subcommand: AddSubcommand,
}

#[derive(Subcommand, Debug, Clone)]
pub enum AddSubcommand {
    /// Add an app to your project (e.g, Next.js, Service, etc)
    App { name: String },
    /// Add a new domain to the Ocel project
    Domain { name: String },
}

pub async fn add(add_opts: &AddOpts) -> anyhow::Result<()> {
    let current_env = whoami::username();
    let project = OcelProject::get_current_project(current_env)?;

    match &add_opts.subcommand {
        AddSubcommand::App { name } => {
            println!("Adding app: {}", name);
            // Implement app addition logic here
        }
        AddSubcommand::Domain { name } => {
            println!("Adding domain: {}", name);
            // Implement domain addition logic here
        }
    }
    Ok(())
}

fn handle_add_app(project: OcelProject) {}
