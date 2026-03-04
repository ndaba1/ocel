use std::fs::{self, File};
use std::path::Path;

use anyhow::{Context, Result, bail};
use colored::Colorize;
use inquire::Text;

use crate::{
    engine::EnvTarget,
    ocel::Ocel,
    project::{expand_infra_sources, OcelProject, OcelProjectJson, ProjectType},
};

const SCAFFOLD_TEMPLATE: &str = r#"import { Hono } from "hono";
import { lambda } from "ocel/lambda/hono";
import { bucket, uploader } from "ocel/blob";
import { createRouteHandler } from "ocel/blob/hono";

const uploaders = {
  avatars: uploader(
    { middleware: async () => {} },
    { accept: ["image/*"], path: { prefix: "avatars/", randomSuffix: true } },
  ),
};

const storageBucket = bucket("myBucket", { uploaders });

const app = new Hono();

app.get("/", (c) => c.text("Hello, World!!!"));
app.post("/upload", createRouteHandler(storageBucket));

export default lambda("myHonoApp", app, { link: [storageBucket] });
"#;

fn scaffold_example_if_empty(project_root: &Path, infra: &str) -> Result<bool> {
    let infra_path = project_root.join(infra);

    let is_empty = if infra_path.exists() {
        !fs::read_dir(&infra_path)
            .with_context(|| format!("Failed to read infra dir {:?}", infra_path))?
            .any(|e| e.is_ok())
    } else {
        true
    };

    if !is_empty {
        return Ok(false);
    }

    fs::create_dir_all(&infra_path)
        .with_context(|| format!("Failed to create infra dir {:?}", infra_path))?;

    let index_path = infra_path.join("index.ts");
    fs::write(&index_path, SCAFFOLD_TEMPLATE)
        .with_context(|| format!("Failed to write {:?}", index_path))?;

    Ok(true)
}

pub async fn init() -> Result<()> {
    println!("Initializing a new Ocel project...");

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

    let infra_input = Text::new("Where will your infra code live? (directory path)")
        .with_default("./infra")
        .prompt()
        .context("Failed to get infra directory")?;

    let infra = infra_input.trim_start_matches("./").to_string();
    let infra_sources = expand_infra_sources(&[infra.clone()], project_type);

    // TODO: allow passing custom env name
    let current_env = whoami::username();
    let cwd = std::env::current_dir().context("Failed to get current working directory")?;
    let project = OcelProject {
        name: project_name,
        project_type,
        infra_sources,
        project_root: cwd.clone(),
        current_env_name: current_env.clone(),
        current_env_dir: cwd.join(".ocel").join("tofu").join(&current_env),
        // TODO: init apps ?
        apps: vec![],
    };

    // doesn't matter if we pass None here as we are not loading an existing project
    let ocel = Ocel::init(Some(project.clone()), EnvTarget::Dev).await?;

    ocel.init_providers().await?;

    let cfg_path = cwd.join("ocel.json");
    let cfg_file = File::create(&cfg_path)
        .with_context(|| format!("Failed to create config file at {:?}", cfg_path))?;
    let cfg_file_contents = OcelProjectJson::from(&project);

    serde_json::to_writer_pretty(cfg_file, &cfg_file_contents)
        .with_context(|| format!("Failed to write config to {:?}", cfg_path))?;

    let scaffolded = scaffold_example_if_empty(&cwd, &infra)?;

    println!("{}", "Project initialized successfully!".green());
    if scaffolded {
        println!(
            "\nWe've scaffolded a minimal Lambda + Blob example in {}/index.ts.",
            infra
        );
        println!("\nNext steps:");
        println!("  1. Install dependencies: pnpm add ocel hono");
        println!("  2. Explore the file in {}/", infra);
        println!("  3. Run 'ocel dev' when ready");
        println!("  4. Once deployed, a Lambda URL will be output for you to use");
    } else {
        println!("\nNext steps:");
        println!("  1. Run 'ocel dev' when ready");
    }

    Ok(())
}
