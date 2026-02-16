use std::{fmt::Display, path::PathBuf};

use anyhow::{Context, Result, bail};
use colored::Colorize;
use inquire_derive::Selectable;
use serde::{Deserialize, Serialize};

use crate::utils;

#[derive(Debug, Copy, Clone, Selectable, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProjectType {
    Typescript,
    Python,
}

impl Display for ProjectType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProjectType::Typescript => write!(f, "Typescript"),
            ProjectType::Python => {
                write!(f, "Python {}", "(coming soon)".dimmed())
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OcelProject {
    pub name: String,
    pub project_type: ProjectType,
    pub infra_sources: Vec<String>,
    pub apps: Vec<OcelProjectApp>,

    pub project_root: PathBuf,
    pub current_env_name: String,
    pub current_env_dir: PathBuf,
}

#[derive(Serialize, Deserialize)]
pub struct OcelProjectJson {
    pub name: String,
    pub project_type: ProjectType,
    pub infra_sources: Vec<String>,
    pub apps: Vec<OcelProjectApp>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OcelProjectApp {
    pub name: String,
    pub source_dir: PathBuf,
    pub app_type: OcelProjectAppType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OcelProjectAppType {
    Container,
    Serverless,
}

impl From<&OcelProject> for OcelProjectJson {
    fn from(project: &OcelProject) -> Self {
        OcelProjectJson {
            name: project.name.clone(),
            project_type: project.project_type,
            infra_sources: project.infra_sources.clone(),
            apps: project.apps.clone(),
        }
    }
}

impl OcelProject {
    fn add_app(&mut self) -> Result<()> {
        Ok(())
    }

    fn get_project_root() -> Result<PathBuf> {
        if let Some(proj_dir) = utils::find_up("ocel.json", &std::env::current_dir()?) {
            Ok(proj_dir.parent().unwrap().to_path_buf())
        } else {
            bail!(
                "Could not find project root. Make sure you are inside an Ocel project directory."
            )
        }
    }

    pub fn get_current_project(current_env_name: String) -> Result<Self> {
        let cwd = OcelProject::get_project_root()?;
        let json_path = cwd.join("ocel.json");
        let json_content = std::fs::read_to_string(&json_path)
            .with_context(|| format!("Could not read ocel.json file at {:?}", json_path))?;

        let project_json: OcelProjectJson =
            serde_json::from_str(&json_content).with_context(|| {
                format!(
                    "Could not parse ocel.json file at {:?} - is it valid JSON?",
                    json_path
                )
            })?;

        Ok(OcelProject {
            name: project_json.name,
            project_type: project_json.project_type,
            infra_sources: project_json.infra_sources,
            project_root: cwd.clone(),
            current_env_name: current_env_name.clone(),
            current_env_dir: cwd.join(".ocel").join("tofu").join(&current_env_name),
            apps: project_json.apps.clone(),
        })
    }
}
