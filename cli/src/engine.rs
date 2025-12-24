use std::{
    collections::HashMap,
    fs::{self, File},
    path::Path,
    sync::Arc,
};

use anyhow::Result;
use serde_json::{Value, json};

use crate::{
    components::{bucket::BucketComponent, get_default_components, lambda::LambdaComponent},
    ocel::Ocel,
    rpc::{RegisterRequest, ResourceType},
    utils::json_deep_merge,
};

pub struct OcelEngine {
    ocel: Arc<Ocel>,
    resources: HashMap<String, Arc<dyn Component + Send + Sync>>,
    linkables: HashMap<String, Arc<dyn Linkable + Send + Sync>>,

    existing_state: Value,
}

pub trait Component {
    /// Run side effects required to build this component (e.g., esbuild assets)
    fn build(&self, _engine: &OcelEngine) -> Result<()> {
        Ok(())
    }

    /// Generate Terraform JSON for this component
    fn to_terraform(&self, engine: &OcelEngine) -> Value;
}

pub trait Linkable {
    /// Returns environment variables this resource exposes (e.g., BUCKET_NAME).
    /// values should be Terraform interpolation strings: "${aws_s3_bucket.main.id}"
    fn get_env_vars(&self) -> HashMap<String, String> {
        HashMap::new()
    }

    /// Returns IAM policy statements required to access this resource.
    fn get_permissions(&self) -> Vec<Value> {
        Vec::new()
    }
}

impl OcelEngine {
    pub fn new(ocel: Arc<Ocel>) -> Self {
        let project = ocel.current_project.as_ref().unwrap();
        let cwd = &project.current_env_dir;

        let existing_state = if cwd.join("main.tf.json").exists() {
            let content = fs::read_to_string(cwd.join("main.tf.json")).unwrap_or_default();
            serde_json::from_str(&content).unwrap_or(json!({}))
        } else {
            json!({})
        };

        OcelEngine {
            ocel,
            resources: HashMap::new(),
            linkables: HashMap::new(),
            existing_state,
        }
    }

    pub fn get_ocel(&self) -> Arc<Ocel> {
        self.ocel.clone()
    }

    pub fn get_linkable(&self, id: &str) -> Option<Arc<dyn Linkable + Send + Sync>> {
        self.linkables.get(id).cloned()
    }

    pub fn register_resource(&mut self, payload: RegisterRequest) -> Result<()> {
        let id = payload.id.clone();
        let config = payload.config;

        match payload.rtype {
            ResourceType::Bucket => {
                let bucket = Arc::new(BucketComponent::new(id.clone(), config));

                self.resources.insert(id.clone(), bucket.clone());
                self.linkables.insert(id, bucket);
            }

            ResourceType::Lambda => {
                let lambda = Arc::new(LambdaComponent::new(
                    id.clone(),
                    config,
                    payload.source_file,
                ));
                self.resources.insert(id, lambda);
            }
        }

        println!("Resource {} registered.", payload.id);

        Ok(())
    }

    pub async fn flush(&self) -> Result<()> {
        let mut root_tf = json!({ "resource": {}, "output": {} });

        // TODO: multithreaded
        for (_id, comp) in &self.resources {
            comp.build(&self)?;
        }

        // default stuff
        let default = get_default_components(self.ocel.as_ref())?;
        json_deep_merge(&mut root_tf, default);

        for (_id, comp) in &self.resources {
            let tf_json = comp.to_terraform(&self);
            json_deep_merge(&mut root_tf, tf_json);
        }

        if self.existing_state == root_tf {
            println!("No changes detected. Skipping flush.");
            return Ok(());
        }

        let ocel = self.get_ocel();
        let project = ocel.current_project.as_ref().unwrap();
        let cwd = &project.current_env_dir;

        let output_path = cwd.join("main.tf.json");
        let new_state = serde_json::to_string_pretty(&root_tf)?;

        fs::write(output_path, new_state)?;

        Ok(())
    }
}
