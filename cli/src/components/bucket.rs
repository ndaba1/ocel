use std::collections::HashMap;

use serde_json::Value;

use crate::engine::{Component, Linkable, OcelEngine};

pub struct BucketComponent {
    name: String,
    config: BucketConfig,
}

#[derive(serde::Deserialize)]
struct BucketConfig {
    versioning: bool,
}

impl BucketComponent {
    pub fn new(name: String, config: Value) -> Self {
        let config = serde_json::from_value(config).expect("Invalid bucket config");

        BucketComponent { name, config }
    }
}

impl Component for BucketComponent {
    fn to_terraform(&self, engine: &OcelEngine) -> serde_json::Value {
        serde_json::json!({
            "resource": {
                "aws_s3_bucket": {
                    &self.name: {
                        "bucket": &self.name,
                        "force_destroy": true
                    }
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
            format!("${{aws_s3_bucket.{}.id}}", self.name),
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
            "Resource": format!("arn:aws:s3:::{}/*", self.name)
        })]
    }
}
