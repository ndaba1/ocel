use std::{collections::HashMap, fs::File, io::Write, process::Stdio};

use anyhow::{Result, bail};
use async_trait::async_trait;
use tokio::process::Command;
use tracing::info;

use crate::{
    ocel::{DiscoveryClient, Ocel},
    utils,
};

pub struct NodeClient<'a> {
    ocel: &'a Ocel,
}

#[async_trait]
impl<'a> DiscoveryClient for NodeClient<'a> {
    async fn discover(&self, server_addr: &str) -> Result<()> {
        info!("Discovering infrastructure...");

        // initial discovery run
        let project = self.ocel.current_project.as_ref().unwrap();
        let sources = &project
            .infra_sources
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<&str>>();

        let infra_files = utils::globby(sources, &project.project_root)?;

        let import_statements = infra_files
            .iter()
            .map(|file_path| {
                let abs_path = file_path.to_str().unwrap().replace("\\", "/");
                format!("import '{}';", abs_path)
            })
            .collect::<Vec<String>>()
            .join("\n");

        let discovery_script = format!(
            r#"
{}

const __promises: Promise<Response>[] = globalThis.__ocelRegister;
await Promise.all(__promises);
await fetch(`${{process.env.OCEL_SERVER}}/commit`, {{ method: "POST" }});
        "#,
            import_statements,
        )
        .trim_start()
        .trim_end()
        .to_string();

        let discovery_path = project
            .project_root
            .join(".ocel")
            .join("discovery-entry.ts");

        File::create(&discovery_path)?.write_all(discovery_script.as_bytes())?;

        let mut envs = HashMap::new();

        envs.insert("OCEL_SERVER".to_string(), format!("http://{}", server_addr));
        envs.insert("OCEL_PHASE".to_string(), "discovery".to_string());

        // sdks may depend on some outputs
        let outputs = self.ocel.get_tofu_outputs().await?;
        for (key, value) in outputs {
            envs.insert(key, value);
        }

        let mut cmd = Command::new(&self.ocel.bun_bin_path);

        cmd.args([discovery_path.to_str().unwrap()])
            .envs(envs)
            .stdout(Stdio::null())
            .stderr(Stdio::piped());

        let output = cmd.output().await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("Node.js discovery process failed:\n{}", stderr);
        }

        info!("Discovery complete");

        Ok(())
    }
}

impl<'a> NodeClient<'a> {
    pub fn new(_ocel: &'a Ocel) -> Self {
        NodeClient { ocel: _ocel }
    }
}
