use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use serde_json::json;
use tokio::sync::Mutex;
use tracing::info;

use crate::{
    components::lambda::{self, LambdaBuildOutput, LambdaComponent},
    engine::{Component, ResourceType},
    ocel::Ocel,
};

pub struct WorkflowComponent {
    id: String,
    source_file: String,
    lambda_fn: LambdaComponent,
}

impl WorkflowComponent {
    pub fn new(id: String, config: serde_json::Value, source_file: String) -> Self {
        let mut lambda_fn = LambdaComponent::new(id.clone(), config, source_file.clone());
        let default_vars = HashMap::from([
            (
                "AWS_LAMBDA_EXEC_WRAPPER".to_string(),
                "/opt/otel-instrument".to_string(),
            ),
            (
                "OTEL_NODE_DISABLED_INSTRUMENTATIONS".to_string(),
                "none".to_string(),
            ),
            // TODO: revisit this later
            (
                "OTEL_AWS_APPLICATION_SIGNALS_ENABLED".to_string(),
                "false".to_string(),
            ),
            ("OTEL_TRACES_SAMPLER".to_string(), "always_on".to_string()),
            ("OTEL_LOGS_EXPORTER".to_string(), "console".to_string()),
        ]);

        lambda_fn.set_default_env_vars(default_vars);

        WorkflowComponent {
            id,
            source_file,
            lambda_fn,
        }
    }
}

#[async_trait]
impl Component for WorkflowComponent {
    fn id(&self) -> &str {
        &self.id
    }

    fn resource_type(&self) -> ResourceType {
        ResourceType::Workflow
    }

    fn source_file(&self) -> Option<&String> {
        Some(&self.source_file)
    }

    async fn dev(&self, ocel: Arc<Ocel>) -> anyhow::Result<()> {
        info!("Starting development mode for workflow: {}", self.id);

        self.lambda_fn.dev(ocel).await
    }

    async fn build(&self, ocel: Arc<Ocel>) -> anyhow::Result<()> {
        self.lambda_fn.build(ocel).await
    }

    /**
     * requires the following components:
     * - SQS Queue
     * - Dispatcher lambda (Go/Rust), read from sqs and invoke the workflow lambda
     * - Durable Lambda function
     * - Auto-instrumentation logic
     * - Realtime logic ??? - appsync events api
     */
    fn to_terraform(
        &self,
        engine: &crate::engine::OcelEngine,
        outputs: std::collections::HashMap<String, String>,
    ) -> serde_json::Value {
        let mut tf_json = self.lambda_fn.to_terraform(engine, outputs);
        let lambda_keys = self.lambda_fn.get_resource_keys();

        if let Some(resource) = tf_json.get_mut("resource").and_then(|r| r.as_object_mut()) {
            if let Some(lambda_block) = resource.get_mut("aws_lambda_function") {
                if let Some(my_function) = lambda_block
                    .get_mut(&self.id)
                    .and_then(|f| f.as_object_mut())
                {
                    // durable lambda config
                    my_function.insert(
                        "durable_config".to_string(),
                        json!({ "execution_timeout": 300 }),
                    );

                    my_function.insert("tracing_config".to_string(), json!({ "mode": "Active" }));

                    // otel layer
                    my_function.insert(
                        "layers".to_string(),
                        json!(["arn:aws:lambda:us-east-1:615299751070:layer:AWSOpenTelemetryDistroJs:10"]),
                    );

                    // amp
                    if let Some(policy_attachment_block) =
                        resource.get_mut("aws_iam_role_policy_attachment")
                    {
                        policy_attachment_block.as_object_mut().unwrap().insert(
                            format!("{}_amp_policy_attachment", self.id),
                            json!({
                                "role": format!("${{aws_iam_role.{}.name}}", lambda_keys.role_key),
                                "policy_arn": "arn:aws:iam::aws:policy/CloudWatchLambdaApplicationSignalsExecutionRolePolicy"
                            }),
                        );
                    }

                    // durable config permissions
                    let statements = json!([
                        {
                            "Effect": "Allow",
                            "Action": [
                                "lambda:CheckpointDurableExecution",
                                "lambda:GetDurableExecution*",
                                "lambda:ListDurableExecutions*",
                                "lambda:StopDurableExecution",
                                "xray:PutTraceSegments",
                                "xray:PutTelemetryRecords",
                                "xray:GetSamplingRules",
                                "xray:GetSamplingTargets",
                                "xray:GetSamplingStatisticSummaries"
                            ],
                            "Resource": format!("${{aws_lambda_function.{}.arn}}:*", self.id)
                        }
                    ]);
                    let policy = json!({
                        "name": format!("{}_durable_lambda_policy", self.id),
                        "role": format!("${{aws_iam_role.{}.name}}", lambda_keys.role_key),
                        "policy": json!({
                            "Version": "2012-10-17",
                            "Statement": statements
                        }).to_string()
                    });

                    if let Some(iam_policy_block) = resource.get_mut("aws_iam_role_policy") {
                        iam_policy_block
                            .as_object_mut()
                            .unwrap()
                            .insert(format!("{}_durable_lambda_policy", self.id), policy);
                    } else {
                        resource.insert(
                            "aws_iam_role_policy".to_string(),
                            json!({
                                format!("{}_durable_lambda_policy", self.id): policy
                            }),
                        );
                    }
                }
            }
        }

        tf_json
    }
}
