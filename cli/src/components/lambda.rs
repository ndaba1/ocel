use std::collections::HashMap;

use serde::Deserialize;
use serde_json::json;

use crate::engine::Component;

pub struct LambdaComponent {
    id: String,
    config: LambdaConfig,
}

#[derive(Deserialize)]
struct LambdaConfig {
    /// List of resource IDs this Lambda links to
    links: Vec<String>,

    /// Trigger configuration
    trigger: LambdaTrigger,
}

#[derive(Deserialize)]
#[serde(tag = "type", content = "config", rename_all = "lowercase")]
enum LambdaTrigger {
    Url { auth_type: String, streaming: bool },
    Cron { schedule: String },
}

impl LambdaComponent {
    pub fn new(id: String, config: serde_json::Value) -> Self {
        let lambda_config: LambdaConfig =
            serde_json::from_value(config).expect("Invalid Lambda configuration");

        LambdaComponent {
            id,
            config: lambda_config,
        }
    }
}

impl Component for LambdaComponent {
    fn to_terraform(&self, engine: &crate::engine::OcelEngine) -> serde_json::Value {
        let mut env_vars = HashMap::new();
        let mut permissions: Vec<serde_json::Value> = Vec::new();

        for link_id in &self.config.links {
            if let Some(linkable) = engine.get_linkable(link_id) {
                // Merge environment variables
                for (key, value) in linkable.get_env_vars() {
                    env_vars.insert(key, value);
                }

                // Merge permissions
                permissions.extend(linkable.get_permissions());
            }
        }

        let role_name = format!("{}_role", self.id);

        let assume_role_policy = json!({
            "Version": "2012-10-17",
            "Statement": [{
                "Action": "sts:AssumeRole",
                "Effect": "Allow",
                "Principal": { "Service": "lambda.amazonaws.com" }
            }]
        });

        json!({
            "resource": {
                "aws_iam_role": {
                    &role_name: {
                        "name": &role_name,
                        "assume_role_policy": assume_role_policy.to_string()
                    }
                },
                "aws_lambda_function": {
                    &self.id: {
                        "function_name": &self.id,
                        "handler": "index.handler",
                        "runtime": "nodejs14.x",
                        "role": "arn:aws:iam::123456789012:role/lambda-role",
                        "filename": format!("{}.zip", &self.id),
                        "environment": {
                            "variables": env_vars
                        }
                    }
                }
            }
        })
    }
}
