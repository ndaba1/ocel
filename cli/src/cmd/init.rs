use std::fs::File;

use anyhow::{Context, Result, bail};
use colored::Colorize;
use inquire::Text;

use crate::{
    engine::EnvTarget,
    ocel::Ocel,
    project::{OcelProject, OcelProjectJson, ProjectType},
};

pub async fn init() -> Result<()> {
    println!("Initializing a new Ocel project...");

    // doesn't matter if we pass None here as we are not loading an existing project
    let ocel = Ocel::init(None, EnvTarget::Dev).await?;

    let project_name = Text::new("What should we name your project ?")
        .with_default("ocel-example")
        .prompt()
        .context("Failed to get project name")?;

    let project_type = ProjectType::select("Select the type of project you want to create:")
        .prompt()
        .context("Failed to select project type")?;

    match project_type {
        ProjectType::Typescript => {}
        _ => {
            bail!(
                "{}",
                "Currently, only Typescript projects are supported.".red()
            )
        }
    }

    let infra_source = Text::new("Where will your infra code live ?")
        .with_default("./infra/**/*.ts")
        .prompt()
        .context("Failed to get infra source")?;

    // TODO: allow passing custom env name
    let current_env = whoami::username();
    let cwd = std::env::current_dir().context("Failed to get current working directory")?;
    let project = OcelProject {
        name: project_name,
        project_type,
        infra_sources: vec![infra_source],
        project_root: cwd.clone(),
        current_env_name: current_env.clone(),
        current_env_dir: cwd.join(".ocel").join("tofu").join(&current_env),
    };

    ocel.init_providers().await?;

    let cfg_path = cwd.join("ocel.json");
    let cfg_file = File::create(&cfg_path)
        .with_context(|| format!("Failed to create config file at {:?}", cfg_path))?;
    let cfg_file_contents = OcelProjectJson::from(&project);

    serde_json::to_writer_pretty(cfg_file, &cfg_file_contents)
        .with_context(|| format!("Failed to write config to {:?}", cfg_path))?;

    println!("{}", "Project initialized successfully!".green());

    Ok(())
}
