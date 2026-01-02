use std::{collections::HashMap, sync::Arc};

use anyhow::Context;
use async_trait::async_trait;
use aws_sdk_ecs::types::{
    AssignPublicIp, AwsVpcConfiguration, ContainerOverride, KeyValuePair, LaunchType,
    NetworkConfiguration, TaskOverride,
};
use indicatif::{ProgressBar, ProgressStyle};
use serde_json::json;
use tracing::debug;

use crate::{
    cmd::{CORE_STACK_NAME, get_stack},
    engine::{Component, Linkable, OcelEngine, ResourceType},
    ocel::Ocel,
};

pub struct PostgresComponent {
    id: String,
    config: Option<PostgresConfig>,
}

#[derive(serde::Deserialize)]
struct PostgresConfig {
    version: Option<String>,
    extensions: Option<Vec<String>>,
}
impl PostgresComponent {
    pub fn new(id: String, config: serde_json::Value) -> Self {
        let config = serde_json::from_value(config).expect("Invalid postgres config");

        PostgresComponent { id, config }
    }
}

fn create_spinner(msg: &str) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::default_spinner()
            .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏ ")
            .template("{spinner:.blue} {msg}")
            .unwrap(),
    );
    pb.set_message(msg.to_string());
    pb.enable_steady_tick(std::time::Duration::from_millis(100));
    pb
}

#[async_trait]
impl Component for PostgresComponent {
    fn id(&self) -> &str {
        &self.id
    }

    fn resource_type(&self) -> ResourceType {
        ResourceType::Postgres
    }

    async fn build(&self, ocel: Arc<Ocel>) -> anyhow::Result<()> {
        debug!("Building Postgres component: {}", self.id);
        let config = ocel.get_aws_config().await;

        let cfn_client = aws_sdk_cloudformation::Client::new(&config);
        let ecs_client = aws_sdk_ecs::Client::new(&config);
        let ec2_client = aws_sdk_ec2::Client::new(&config);

        let core_stack = get_stack(&cfn_client, CORE_STACK_NAME).await?;
        if core_stack.is_none() {
            anyhow::bail!("Core stack not found. Please run `ocel bootstrap` first.");
        }

        let core_stack = core_stack.unwrap();

        let outputs = core_stack
            .outputs()
            .iter()
            .map(|o| {
                (
                    o.output_key().unwrap_or_default().to_string(),
                    o.output_value().unwrap_or_default().to_string(),
                )
            })
            .collect::<HashMap<String, String>>();

        debug!("Core stack outputs: {:?}", outputs);

        let response = ecs_client
            .run_task()
            .cluster(
                outputs
                    .get("OcelClusterName")
                    .context("Ocel cluster not found")?,
            )
            .task_definition(
                outputs
                    .get("OcelPostgresTaskDefArn")
                    .context("Ocel Postgres task definition not found")?,
            )
            .launch_type(LaunchType::Fargate)
            .network_configuration(
                NetworkConfiguration::builder()
                    .awsvpc_configuration(
                        AwsVpcConfiguration::builder()
                            .subnets(
                                outputs
                                    .get("OcelPublicSubnetId")
                                    .context("Ocel private subnet IDs not found")?,
                            )
                            .security_groups(
                                outputs
                                    .get("OcelPostgresSecurityGroupId")
                                    .context("Ocel Postgres security group ID not found")?,
                            )
                            .assign_public_ip(AssignPublicIp::Enabled) // CRITICAL for Public Subnet
                            .build()?,
                    )
                    .build(),
            )
            .overrides(
                TaskOverride::builder()
                    .container_overrides(
                        ContainerOverride::builder()
                            .name("postgres-container")
                            // Override Environment Variable for Password
                            .environment(
                                KeyValuePair::builder()
                                    .name("POSTGRES_PASSWORD")
                                    .value("some-password") // TODO: securely generate/store password
                                    .build(),
                            )
                            // Optional: Override Command (e.g., config flags)
                            .command("postgres")
                            .command("-c")
                            .command("max_connections=50")
                            .build(),
                    )
                    .build(),
            )
            .send()
            .await;

        debug!("ECS Run Task response: {:?}", response);

        let response = response.context("Failed to run Postgres task")?;

        let tasks = response.tasks.context("No tasks found")?;
        let task_arn = tasks
            .first()
            .context("Task list empty")?
            .task_arn
            .clone()
            .context("No ARN")?;

        match wait_for_public_ip(
            &ecs_client,
            &ec2_client,
            &task_arn,
            outputs
                .get("OcelClusterName")
                .context("Ocel cluster not found")?,
        )
        .await
        {
            Ok(ip) => {
                println!("\n🎉 SUCCESS! Postgres is up.");
                println!("---------------------------------------------------");
                println!("Host: {}", ip);
                println!("Port: 5432");
                println!("User: postgres");
                println!("Pass: my-secret-password");
                println!(
                    "Conn: postgres://postgres:my-secret-password@{}:5432/postgres",
                    ip
                );
                println!("---------------------------------------------------");
            }
            Err(e) => eprintln!("\n❌ Error waiting for IP: {}", e),
        }

        Ok(())
    }

    fn to_terraform(
        &self,
        engine: &OcelEngine,
        outputs: HashMap<String, String>,
    ) -> serde_json::Value {
        json!({}) // No TF generation yet
    }
}

impl Linkable for PostgresComponent {
    fn get_env_vars(&self) -> HashMap<String, String> {
        let mut vars = HashMap::new();
        vars.insert(
            "BUCKET_NAME".to_string(),
            format!("${{aws_s3_bucket.{}.id}}", self.id),
        );
        vars
    }

    fn get_permissions(&self) -> Vec<serde_json::Value> {
        vec![serde_json::json!({
            "Effect": "Allow",
            "Action": [
                "s3:GetObject",
                "s3:PutObject",
                "s3:DeleteObject"
            ],
            "Resource": format!("${{aws_s3_bucket.{}.arn}}", self.id)
        })]
    }
}

async fn wait_for_public_ip(
    ecs_client: &aws_sdk_ecs::Client,
    ec2_client: &aws_sdk_ec2::Client,
    task_arn: &str,
    cluster_name: &str,
) -> Result<String, String> {
    let mut attempts = 0;
    let max_attempts = 40; // Wait up to ~80 seconds

    loop {
        attempts += 1;
        if attempts > max_attempts {
            return Err("Timed out waiting for Public IP".to_string());
        }

        // A. Describe the ECS Task to get the ENI ID
        let resp = ecs_client
            .describe_tasks()
            .cluster(cluster_name)
            .tasks(task_arn)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        // Extract the task object
        if let Some(task) = resp.tasks.and_then(|t| t.into_iter().next()) {
            let status = task.last_status.as_deref().unwrap_or("UNKNOWN");

            // Fail fast if task stopped unexpectedly
            if status == "STOPPED" {
                let reason = task
                    .stopped_reason
                    .unwrap_or_else(|| "Unknown reason".into());
                return Err(format!("Task stopped unexpectedly: {}", reason));
            }

            // Check if status is RUNNING (or at least if attachments are present)
            if status == "RUNNING" {
                // B. Find the Elastic Network Interface (ENI) ID
                // ECS returns this buried in: task -> attachments -> details -> name="networkInterfaceId"
                let eni_id_option = task
                    .attachments
                    .unwrap_or_default()
                    .iter()
                    .filter(|a| a.r#type.as_deref() == Some("ElasticNetworkInterface"))
                    .find_map(|a| {
                        a.details.as_ref()?.iter().find_map(|d| {
                            if d.name.as_deref() == Some("networkInterfaceId") {
                                d.value.clone()
                            } else {
                                None
                            }
                        })
                    });

                if let Some(eni_id) = eni_id_option {
                    // C. Call EC2 to get the Public IP from the ENI ID
                    let eni_resp = ec2_client
                        .describe_network_interfaces()
                        .network_interface_ids(&eni_id)
                        .send()
                        .await
                        .map_err(|e| format!("Failed to describe ENI: {}", e))?;

                    if let Some(interface) = eni_resp
                        .network_interfaces
                        .and_then(|n| n.into_iter().next())
                    {
                        if let Some(association) = interface.association {
                            if let Some(public_ip) = association.public_ip {
                                return Ok(public_ip);
                            }
                        }
                    }
                }
            }
        }

        // Wait 2 seconds before retry
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}
