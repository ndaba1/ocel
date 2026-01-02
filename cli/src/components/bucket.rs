use std::collections::HashMap;

use serde_json::Value;

use crate::{
    engine::{Component, EnvTarget, Linkable, OcelEngine, ResourceType},
    ocel::Ocel,
    utils::get_nanoid,
};

pub struct BucketComponent {
    id: String,
    config: BucketConfig,
}

#[derive(serde::Deserialize)]
struct BucketConfig {
    versioning: Option<bool>,
}

impl BucketComponent {
    pub fn new(id: String, config: Value) -> Self {
        let config = serde_json::from_value(config).expect("Invalid bucket config");

        BucketComponent { id, config }
    }

    fn generate_bucket_name(&self, ocel: &Ocel) -> String {
        let project = ocel
            .current_project
            .clone()
            .expect("No current project set");
        let env = project.current_env_name;

        format!(
            "ocel-{}-{}-{}-{}",
            project.name,
            env,
            self.id,
            get_nanoid(6)
        )
    }
}

impl Component for BucketComponent {
    fn id(&self) -> &str {
        &self.id
    }

    fn resource_type(&self) -> ResourceType {
        ResourceType::Bucket
    }

    fn to_terraform(
        &self,
        engine: &OcelEngine,
        outputs: HashMap<String, String>,
    ) -> serde_json::Value {
        let ocel = &engine.get_ocel();
        let mut bucket_name = self.generate_bucket_name(ocel);

        let bucket_name_output_key = format!("RESOURCE_{}_BUCKET_NAME", self.id);
        let bucket_cors_key = format!("{}_cors", self.id);
        let bucket_versioning_key = format!("{}_versioning", self.id);

        if let Some(existing_name) = outputs.get(&bucket_name_output_key) {
            bucket_name = existing_name.to_string();
        }

        let protected = matches!(ocel.env_target, EnvTarget::Prod);
        let versioning = self.config.versioning.unwrap_or(false);

        // TODO: refine cors origins ?
        // TODO: in prod target, deploy lambda subscriber for onUploadComplete notifications
        serde_json::json!({
            "resource": {
                "aws_s3_bucket": {
                    &self.id: {
                        "bucket": bucket_name.to_lowercase(),
                        "force_destroy": !protected,
                    }
                },
                "aws_s3_bucket_cors_configuration": {
                    &bucket_cors_key: {
                        "bucket": format!("${{aws_s3_bucket.{}.id}}", &self.id),
                        "cors_rule": [{
                            "allowed_headers": ["*"],
                            "allowed_methods": ["GET", "PUT", "POST", "DELETE", "HEAD"],
                            "allowed_origins": ["*"],
                            "expose_headers": ["ETag"],
                            "max_age_seconds": 3000
                        }]
                    }
                },
                "aws_s3_bucket_versioning": {
                    &bucket_versioning_key: {
                        "bucket": format!("${{aws_s3_bucket.{}.id}}", &self.id),
                        "versioning_configuration": {
                            "status": if versioning { "Enabled" } else { "Suspended" }
                        }
                    }
                }
            },
            "output": {
                bucket_name_output_key: {
                    "value": format!("${{aws_s3_bucket.{}.id}}", &self.id)
                }
            }
        })
    }
}

impl Linkable for BucketComponent {
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
