use std::{
    collections::HashMap,
    fs::{self, File},
    path::PathBuf,
    process::{Command, Stdio},
    thread,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use colored::Colorize;
use directories::ProjectDirs;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use serde_json::{Value, json};

use crate::{
    client::{self, NodeClient},
    project::{self, ProjectType},
};

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Ocel {
    pub config_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub tofu_bin_path: PathBuf,
    pub bun_bin_path: PathBuf,

    pub current_project: Option<project::OcelProject>,
}

pub trait DevClient {
    fn dev(&self) -> Result<()>;
}

impl Ocel {
    pub fn new() -> Self {
        let project_dirs = ProjectDirs::from("com", "ocel", "ocel-cli")
            .expect("Could not determine project directories");
        let config_dir = project_dirs.config_dir();
        let cache_dir = project_dirs.cache_dir();

        if !config_dir.exists() {
            std::fs::create_dir_all(config_dir)
                .expect("Could not create Ocel configuration directory");
        }

        if !cache_dir.exists() {
            std::fs::create_dir_all(cache_dir).expect("Could not create Ocel cache directory");
        }

        let tofu_bin_path = config_dir.join("bin").join("tofu").join(
            cfg!(target_os = "windows")
                .then(|| "tofu.exe")
                .unwrap_or("tofu"),
        );

        let bun_bin_path = config_dir.join("bin").join("bun").join(
            cfg!(target_os = "windows")
                .then(|| "bun.exe")
                .unwrap_or("bun"),
        );

        Ocel {
            config_dir: config_dir.to_path_buf(),
            cache_dir: cache_dir.to_path_buf(),
            tofu_bin_path,
            bun_bin_path,
            current_project: None,
        }
    }

    pub fn init() -> Result<Self> {
        let config = Ocel::new();
        let m = MultiProgress::new();

        let style = ProgressStyle::default_bar()
            .template("{msg} [{wide_bar:.cyan/blue}] {bytes}/{total_bytes}")?
            .progress_chars("#>-");

        let tofu_handle = if !config.tofu_bin_path.exists() {
            let cfg = config.clone();
            let pb_tofu = m.add(ProgressBar::new(0));
            pb_tofu.set_style(style.clone());
            pb_tofu.set_message("Installing OpenTofu...");

            Some(thread::spawn(move || -> Result<()> {
                crate::tofu::install_tofu(&cfg, &pb_tofu)?;
                pb_tofu.finish_with_message("Tofu installed");
                Ok(())
            }))
        } else {
            None
        };

        let bun_handle = if !config.bun_bin_path.exists() {
            let cfg = config.clone();
            let pb_bun = m.add(ProgressBar::new(0));
            pb_bun.set_style(style.clone());
            pb_bun.set_message("Installing Bun...");

            Some(thread::spawn(move || -> Result<()> {
                crate::bun::install_bun(&cfg, &pb_bun)?;
                pb_bun.finish_with_message("Bun installed");
                Ok(())
            }))
        } else {
            None
        };

        if let Some(handle) = tofu_handle {
            handle.join().expect("Tofu thread panicked")?;
        }

        if let Some(handle) = bun_handle {
            handle.join().expect("Bun thread panicked")?;
        }

        // cleanup progress bars
        m.clear()?;

        Ok(config)
    }
}

impl Ocel {
    pub fn set_current_project(&mut self, project: &project::OcelProject) {
        self.current_project = Some(project.clone());
    }

    pub fn init_providers(&self) -> Result<()> {
        let provider_config = json!({
            "terraform": {
                "required_providers": {
                    "aws": {
                        "source": "hashicorp/aws",
                        "version": "~> 6.0"
                    }
                }
                // "backend": { ... } // TODO: Remote s3 backend
            }
        });

        let project = self.current_project.as_ref().unwrap();
        let cwd = &project.current_env_dir;

        if !cwd.exists() {
            fs::create_dir_all(cwd).with_context(|| {
                format!(
                    "Failed to create project environment directory at {:?}",
                    cwd
                )
            })?;
        }

        let provider_config_path = cwd.join("provider.tf.json");
        let provider_config_file = File::create(&provider_config_path).with_context(|| {
            format!("Failed to create config file at {:?}", provider_config_path)
        })?;

        serde_json::to_writer_pretty(provider_config_file, &provider_config)
            .with_context(|| format!("Failed to write config to {:?}", provider_config_path))?;

        self.run_tofu(&["init", "-input=false"], None)?;

        Ok(())
    }

    pub fn run_tofu(
        &self,
        args: &[&str],
        env_vars: Option<&HashMap<String, String>>,
    ) -> Result<()> {
        if self.current_project.is_none() {
            bail!("No current project set. Please initialize or select a project first.");
        }

        let project = self.current_project.as_ref().unwrap();
        let cwd = &project.current_env_dir;

        let stdout_log = cwd.join("tofu.log");
        let stderr_log = cwd.join("tofu_error.log");

        let stdout_file = File::create(&stdout_log)
            .with_context(|| format!("Failed to create log file: {:?}", stdout_log))?;

        let stderr_file = File::create(&stderr_log)
            .with_context(|| format!("Failed to create log file: {:?}", stderr_log))?;

        let pb = ProgressBar::new_spinner();
        pb.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.green} {msg}")?
                .tick_strings(&["-", "\\", "|", "/", "done"]), // simple rotation
        );

        pb.enable_steady_tick(Duration::from_millis(120));
        pb.set_message(format!("Running 'tofu {}'...", args.join(" ")));

        let mut cmd = Command::new(&self.tofu_bin_path);

        cmd.args(args)
            .current_dir(cwd)
            .env("FORCE_COLOR", "0")
            .stdout(Stdio::from(stdout_file))
            .stderr(Stdio::from(stderr_file));

        if let Some(vars) = env_vars {
            cmd.envs(vars);
        }

        let status = cmd.status().context("Failed to execute Tofu")?;

        if status.success() {
            pb.finish_with_message(format!("{} Tofu command completed", "✔".green()));
            Ok(())
        } else {
            pb.finish_with_message(format!("{} Tofu command failed", "✘".red()));
            bail!("Tofu exited with code {:?}", status.code());
        }
    }

    pub fn get_tofu_outputs(&self) -> Result<HashMap<String, String>> {
        let project = self.current_project.as_ref().unwrap();
        let cwd = &project.current_env_dir;

        let output = Command::new(&self.tofu_bin_path)
            .args(&["output", "-json"])
            .current_dir(cwd)
            .output()
            .context("Failed to execute Tofu output command")?;

        if !output.status.success() {
            bail!(
                "Tofu output command failed with code {:?}",
                output.status.code()
            );
        }

        let stdout = String::from_utf8(output.stdout)
            .context("Failed to parse Tofu output command output")?;

        let raw_json: Value =
            serde_json::from_str(&stdout).context("Failed to parse Tofu output JSON")?;

        let mut flattened_outputs = HashMap::new();

        if let Value::Object(map) = raw_json {
            for (key, metadata_obj) in map {
                // We only care about the inner "value" field
                if let Some(inner_value) = metadata_obj.get("value") {
                    flattened_outputs.insert(
                        key,
                        inner_value
                            .as_str()
                            .expect("Expected a string value")
                            .to_string(),
                    );
                }
            }
        }

        Ok(flattened_outputs)
    }

    pub fn run_dev_mode(&self) -> Result<()> {
        // TODO: leader and infra server etc
        let client = self.get_dev_client()?;
        client.dev()?;

        Ok(())
    }

    fn get_dev_client(&self) -> Result<Box<dyn DevClient + '_>> {
        let project = self.current_project.as_ref().unwrap();
        match project.project_type {
            ProjectType::Typescript => {
                let client = Box::new(NodeClient::new(&self));

                return Ok(client);
            }
            _ => {
                bail!(
                    "{}",
                    "Currently, only Typescript projects are supported in dev mode.".red()
                )
            }
        }
    }
}
