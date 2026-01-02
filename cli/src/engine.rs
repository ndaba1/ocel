use std::{collections::HashMap, path::PathBuf, sync::Arc};

use anyhow::{Context, Result};
use async_trait::async_trait;
use futures::{StreamExt, TryStreamExt, stream};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::mpsc::Sender;
use tracing::{debug, error};

use crate::{
    CoordinatorMsg,
    components::{
        bucket::BucketComponent, get_default_components, lambda::LambdaComponent,
        postgres::PostgresComponent, workflow::WorkflowComponent,
    },
    lock::LeaderInfo,
    ocel::Ocel,
    server::RegisterRequest,
    utils::json_deep_merge,
};

#[derive(Deserialize, Clone, Copy, Debug)]
#[serde(rename_all = "lowercase")]
pub enum ResourceType {
    Bucket,
    Lambda,
    Workflow,
    Postgres,
}

#[derive(Clone, Debug)]
pub enum EnvTarget {
    Dev,
    Preview { env_name: String },
    Prod,
}

pub struct OcelEngine {
    ocel: Arc<Ocel>,
    resources: HashMap<String, Arc<dyn Component + Send + Sync>>,
    linkables: HashMap<String, Arc<dyn Linkable + Send + Sync>>,
    tx: Sender<CoordinatorMsg>,
}

#[async_trait]
pub trait Component {
    fn id(&self) -> &str;

    fn resource_type(&self) -> ResourceType;

    /// Components can implement this skip infra changes in dev mode
    /// if a code change is detected that corresponds to this component,
    /// the coordinator will call this method instead of triggering a full reconcile.
    async fn dev(&self, _ocel: Arc<Ocel>) -> Result<()> {
        Ok(())
    }

    /// Run side effects required to build this component (e.g., esbuild assets)
    async fn build(&self, _ocel: Arc<Ocel>) -> Result<()> {
        Ok(())
    }

    fn source_file(&self) -> Option<&String> {
        None
    }

    /// Generate Terraform JSON for this component
    fn to_terraform(&self, engine: &OcelEngine, outputs: HashMap<String, String>) -> Value;
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
    pub fn new(ocel: Arc<Ocel>, tx: Sender<CoordinatorMsg>) -> Self {
        OcelEngine {
            ocel,
            tx,
            resources: HashMap::new(),
            linkables: HashMap::new(),
        }
    }

    pub fn get_ocel(&self) -> Arc<Ocel> {
        self.ocel.clone()
    }

    pub fn get_linkable(&self, id: &str) -> Option<Arc<dyn Linkable + Send + Sync>> {
        self.linkables.get(id).cloned()
    }

    pub fn get_resource_by_source(
        &self,
        source_file: &str,
    ) -> Option<Arc<dyn Component + Send + Sync>> {
        for comp in self.resources.values() {
            if let Some(comp_source) = comp.source_file() {
                if comp_source == source_file {
                    return Some(comp.clone());
                }
            }
        }

        None
    }

    fn add_linkable<T>(&mut self, component: T)
    where
        T: Component + Linkable + Send + Sync + 'static,
    {
        let id = component.id().to_string();
        let arc = Arc::new(component);

        self.resources
            .insert(id.clone(), arc.clone() as Arc<dyn Component + Send + Sync>);

        self.linkables
            .insert(id, arc as Arc<dyn Linkable + Send + Sync>);
    }

    fn add_component<T>(&mut self, component: T)
    where
        T: Component + Send + Sync + 'static,
    {
        let id = component.id().to_string();
        self.resources.insert(id, Arc::new(component));
    }

    pub fn register_resource(&mut self, payload: RegisterRequest) -> Result<()> {
        let id = payload.id.clone();
        let config = payload.config;

        match payload.rtype {
            ResourceType::Bucket => {
                let bucket = BucketComponent::new(id, config);
                self.add_linkable(bucket);
            }
            ResourceType::Lambda => {
                let lambda = LambdaComponent::new(id, config, payload.source_file);
                self.add_component(lambda);
            }
            ResourceType::Workflow => {
                let workflow = WorkflowComponent::new(id, config, payload.source_file);
                self.add_component(workflow);
            }
            ResourceType::Postgres => {
                let pg = PostgresComponent::new(id, config);
                self.add_linkable(pg);
            }
        };

        Ok(())
    }

    pub async fn flush(&self) -> Result<()> {
        let mut root_tf = json!({ "resource": {}, "output": {} });
        let ocel = self.ocel.clone();

        let build_tasks: Vec<_> = self
            .resources
            .values()
            .cloned()
            .map(|comp| {
                let ocel_ref = ocel.clone();
                let id = comp.id().to_string();

                async move {
                    comp.build(ocel_ref)
                        .await
                        .with_context(|| format!("Failed to build component '{}'", id))
                }
            })
            .collect();

        stream::iter(build_tasks)
            .buffer_unordered(10)
            .try_collect::<Vec<()>>()
            .await?;

        // default stuff
        let default = get_default_components(self.ocel.as_ref()).await?;
        json_deep_merge(&mut root_tf, default);

        let existing_outputs = self.ocel.get_tofu_outputs().await?;

        for (_id, comp) in &self.resources {
            let tf_json = comp.to_terraform(&self, existing_outputs.clone());
            json_deep_merge(&mut root_tf, tf_json);
        }

        self.tx.send(CoordinatorMsg::CommitState(root_tf)).await?;

        Ok(())
    }

    pub async fn process_changes(&self, paths: Vec<PathBuf>, info: &LeaderInfo) -> Result<()> {
        let mut affected_components: Vec<Arc<dyn Component + Send + Sync>> = Vec::new();

        for path in paths {
            if let Some(component) = self.get_resource_by_source(
                path.to_str()
                    .ok_or_else(|| anyhow::anyhow!("Invalid path"))?,
            ) {
                affected_components.push(component);
            }
        }

        // all affected components are only lambda/workflow, we can skip full reconcile
        let skip_reconcile = affected_components.iter().all(|comp| {
            matches!(
                comp.resource_type(),
                ResourceType::Lambda | ResourceType::Workflow
            )
        });

        if !skip_reconcile {
            debug!("🔄 Changes affect infra, triggering full reconcile.");
            let ocel_ref = self.ocel.clone();

            if let Ok(client) = ocel_ref.get_client() {
                if let Err(e) = client.discover(&info.addr).await {
                    error!("❌ Discovery failed: {}", e);
                } else {
                    debug!("✅ Discovery complete.");
                }
            }
        }

        debug!(
            "⚡️ Running dev updates for affected components... {:?}",
            affected_components
                .iter()
                .map(|c| c.id())
                .collect::<Vec<_>>()
        );

        let dev_tasks: Vec<_> = affected_components
            .into_iter()
            .map(|comp| {
                let ocel_ref = self.ocel.clone();

                async move { comp.dev(ocel_ref).await }
            })
            .collect();

        stream::iter(dev_tasks)
            .buffer_unordered(10)
            .try_collect::<Vec<()>>()
            .await?;

        Ok(())
    }
}
