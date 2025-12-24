use std::{collections::HashMap, fs::File, path::Path, sync::Arc};

use anyhow::Result;
use serde_json::{Value, json};

use crate::{
    components::{
        bucket::{self, BucketComponent},
        lambda::LambdaComponent,
    },
    ocel::Ocel,
    server::{RegisterRequest, ResourceType},
    utils::json_deep_merge,
};

pub struct OcelEngine {
    ocel: Arc<Ocel>,
    resources: HashMap<String, Arc<dyn Component + Send + Sync>>,
    linkables: HashMap<String, Arc<dyn Linkable + Send + Sync>>,
}

pub trait Component {
    /// Run side effects required to build this component (e.g., esbuild assets)
    fn build(&self, engine: &OcelEngine) -> Result<()> {
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
        OcelEngine {
            ocel,
            resources: HashMap::new(),
            linkables: HashMap::new(),
        }
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
                let lambda = Arc::new(LambdaComponent::new(id.clone(), config));
                self.resources.insert(id, lambda);
            }
        }

        Ok(())
    }

    pub async fn flush(&self) -> Result<()> {
        let mut root_tf = json!({ "resource": {}, "output": {} });

        for (_id, comp) in &self.resources {
            let tf_json = comp.to_terraform(&self);
            json_deep_merge(&mut root_tf, tf_json);
        }

        // 4. WRITE
        let output_path = Path::new("main.tf.json");
        let file = File::create(&output_path)?;
        serde_json::to_writer_pretty(file, &root_tf)?;

        Ok(())
    }
}
