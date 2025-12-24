use std::{collections::HashMap, fs::File, io::Write, process::Command};

use anyhow::{Result, bail};

use crate::{
    ocel::{DevClient, Ocel},
    utils,
};

pub struct NodeClient<'a> {
    ocel: &'a Ocel,
}

impl<'a> DevClient for NodeClient<'a> {
    fn start_dev(&self) -> Result<()> {
        println!("Starting Node.js development server...");

        // initial discovery run
        self.run_discovery()?;

        Ok(())
    }
}

impl<'a> NodeClient<'a> {
    pub fn new(_ocel: &'a Ocel) -> Self {
        NodeClient { ocel: _ocel }
    }

    fn run_discovery(&self) -> Result<()> {
        println!("Running Node.js discovery...");

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
const __promises = [];
globalThis.__ocelRegister = (p) => __promises.push(p);

{}

await Promise.all(__promises);
await fetch("${{process.env.OCEL_SERVER}}/commit", {{ method: "POST" }});
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

        envs.insert("OCEL_SERVER".to_string(), "TODO".to_string());
        envs.insert("OCEL_PHASE".to_string(), "discovery".to_string());

        // sdks may depend on some outputs
        let outputs = self.ocel.get_tofu_outputs()?;
        for (key, value) in outputs {
            envs.insert(key, value);
        }

        let mut cmd = Command::new(&self.ocel.bun_bin_path);

        cmd.args([discovery_path.to_str().unwrap()]).envs(envs);

        let status = cmd.status()?;

        if !status.success() {
            bail!("Node.js discovery process failed.");
        }

        Ok(())
    }
}
