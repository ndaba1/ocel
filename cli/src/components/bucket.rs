use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    process::Command,
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use base64::{engine::general_purpose, Engine};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::{
    components::asset_bucket_ref,
    engine::{Component, EnvTarget, Linkable, OcelEngine, ResourceType},
    ocel::Ocel,
    utils::{self, get_nanoid},
};

pub struct BucketComponent {
    id: String,
    config: BucketConfig,
    build_output: Mutex<Option<BucketListenerBuildOutput>>,
}

struct BucketListenerBuildOutput {
    zip_path: PathBuf,
    hash_b64: String,
}

#[derive(serde::Deserialize)]
struct BucketConfig {
    versioning: Option<bool>,
}

impl BucketComponent {
    pub fn new(id: String, config: Value) -> Self {
        let config = serde_json::from_value(config).expect("Invalid bucket config");

        BucketComponent {
            id,
            config,
            build_output: Mutex::new(None),
        }
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

#[async_trait]
impl Component for BucketComponent {
    fn id(&self) -> &str {
        &self.id
    }

    fn resource_type(&self) -> ResourceType {
        ResourceType::Bucket
    }

    async fn build(&self, ocel: Arc<Ocel>) -> Result<()> {
        let project = ocel
            .current_project
            .as_ref()
            .context("Unable to resolve current project")?;

        let env_dir = project.current_env_dir.clone();
        let bucket_id = self.id.clone();
        let bun_path = ocel.bun_bin_path.clone();
        let project_root = project.project_root.clone();

        let build_result = tokio::task::spawn_blocking(move || -> Result<BucketListenerBuildOutput> {
            build_blob_listener_artifact(&bucket_id, &env_dir, &project_root, &bun_path)
        })
        .await??;

        let mut output_lock = self.build_output.lock().unwrap();
        *output_lock = Some(build_result);

        Ok(())
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

        let mut tf_json = serde_json::json!({
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
        });

        // S3 listener Lambda for blob upload completion
        if let Some(build_data) = self.build_output.lock().unwrap().as_ref() {
            let listener_role_key = format!("{}_listener_role", self.id);
            let listener_policy_key = format!("{}_listener_policy", self.id);
            let listener_exec_key = format!("{}_listener_exec", self.id);
            let listener_fn_key = format!("{}_listener", self.id);
            let listener_artifact_key = format!("{}_listener_artifact", self.id);
            let listener_permission_key = format!("{}_listener_s3_permission", self.id);
            let listener_notification_key = format!("{}_listener_s3_notification", self.id);

            let project = ocel.current_project.as_ref().unwrap();
            let function_name = format!(
                "{}-{}-{}-listener",
                self.id,
                project.name,
                project.current_env_name
            );
            let s3_bucket = asset_bucket_ref();
            let s3_key = format!(
                "{}/blob-listener/{}",
                self.id,
                build_data.zip_path.file_name().unwrap().to_str().unwrap()
            );

            let listener_resources = json!({
                "resource": {
                    "aws_iam_role": {
                        &listener_role_key: {
                            "name": format!("{}-role", function_name),
                            "assume_role_policy": json!({
                                "Version": "2012-10-17",
                                "Statement": [{
                                    "Action": "sts:AssumeRole",
                                    "Effect": "Allow",
                                    "Principal": { "Service": "lambda.amazonaws.com" }
                                }]
                            }).to_string()
                        }
                    },
                    "aws_iam_role_policy_attachment": {
                        &listener_exec_key: {
                            "role": format!("${{aws_iam_role.{}.name}}", listener_role_key),
                            "policy_arn": "arn:aws:iam::aws:policy/service-role/AWSLambdaBasicExecutionRole"
                        }
                    },
                    "aws_iam_role_policy": {
                        &listener_policy_key: {
                            "name": format!("{}-policy", function_name),
                            "role": format!("${{aws_iam_role.{}.name}}", listener_role_key),
                            "policy": json!({
                                "Version": "2012-10-17",
                                "Statement": [
                                    {
                                        "Effect": "Allow",
                                        "Action": [
                                            "dynamodb:UpdateItem"
                                        ],
                                        "Resource": "arn:aws:dynamodb:*:*:table/OcelTable"
                                    },
                                    {
                                        "Effect": "Allow",
                                        "Action": [
                                            "s3:GetObject",
                                            "s3:HeadObject"
                                        ],
                                        "Resource": format!("${{aws_s3_bucket.{}.arn}}/*", self.id)
                                    }
                                ]
                            }).to_string()
                        }
                    },
                    "aws_lambda_function": {
                        &listener_fn_key: {
                            "timeout": 30,
                            "memory_size": 256,
                            "function_name": function_name,
                            "handler": "index.handler",
                            "runtime": "nodejs22.x",
                            "role": format!("${{aws_iam_role.{}.arn}}", listener_role_key),
                            "environment": {
                                "variables": {
                                    "OCEL_TABLE_NAME": "OcelTable"
                                }
                            },
                            "s3_bucket": s3_bucket,
                            "s3_key": s3_key,
                            "source_code_hash": build_data.hash_b64,
                            "depends_on": [
                                format!("aws_iam_role_policy.{}", listener_policy_key),
                                format!("aws_s3_object.{}", listener_artifact_key)
                            ]
                        }
                    },
                    "aws_s3_object": {
                        &listener_artifact_key: {
                            "bucket": s3_bucket,
                            "key": s3_key,
                            "source": build_data.zip_path.to_str().unwrap(),
                            "source_hash": build_data.hash_b64
                        }
                    },
                    "aws_lambda_permission": {
                        &listener_permission_key: {
                            "statement_id": format!("{}-s3-invoke", self.id),
                            "action": "lambda:InvokeFunction",
                            "function_name": format!("${{aws_lambda_function.{}.function_name}}", listener_fn_key),
                            "principal": "s3.amazonaws.com",
                            "source_arn": format!("${{aws_s3_bucket.{}.arn}}", self.id)
                        }
                    },
                    "aws_s3_bucket_notification": {
                        &listener_notification_key: {
                            "bucket": format!("${{aws_s3_bucket.{}.id}}", self.id),
                            "lambda_function": [{
                                "lambda_function_arn": format!("${{aws_lambda_function.{}.arn}}", listener_fn_key),
                                "events": ["s3:ObjectCreated:*"]
                            }],
                            "depends_on": [format!("aws_lambda_permission.{}", listener_permission_key)]
                        }
                    }
                }
            });

            utils::json_deep_merge(&mut tf_json, listener_resources);
        }

        tf_json
    }
}

fn build_blob_listener_artifact(
    bucket_id: &str,
    env_dir: &PathBuf,
    project_root: &PathBuf,
    bun_path: &PathBuf,
) -> Result<BucketListenerBuildOutput> {
    let ocel_pkg = project_root
        .join("node_modules")
        .join("ocel");
    let ocel_pkg_alt = project_root
        .parent()
        .map(|p| p.join("packages").join("ocel"));

    let ocel_root = if ocel_pkg.join("src").join("blob").join("s3-listener.ts").exists() {
        ocel_pkg
    } else if let Some(ref alt) = ocel_pkg_alt {
        if alt.join("src").join("blob").join("s3-listener.ts").exists() {
            alt.clone()
        } else {
            bail!(
                "ocel package not found. Install with: pnpm add ocel. \
                 Expected src/blob/s3-listener.ts at {} or {}",
                ocel_pkg.display(),
                alt.display()
            );
        }
    } else {
        bail!(
            "ocel package not found. Install with: pnpm add ocel. \
             Expected at {}",
            ocel_pkg.display()
        );
    };

    let listener_src = ocel_root.join("src").join("blob").join("s3-listener.ts");
    let artifact_dir = env_dir.join("artifacts").join(bucket_id).join("blob-listener");

    if artifact_dir.exists() {
        fs::remove_dir_all(&artifact_dir)?;
    }
    fs::create_dir_all(&artifact_dir).context("Failed to create blob-listener artifact directory")?;

    let out_file = artifact_dir.join("index.js");
    let status = Command::new(bun_path)
        .current_dir(&ocel_root)
        .args([
            "build",
            listener_src.to_str().unwrap(),
            "--outfile",
            out_file.to_str().unwrap(),
            "--target",
            "node",
            "--format",
            "cjs",
            "--minify",
        ])
        .status()
        .context("Failed to run bun build for blob listener")?;

    if !status.success() {
        bail!("Failed to build blob listener Lambda");
    }

    let zip_path = artifact_dir.parent().unwrap().join("blob-listener.zip");
    utils::archive::create_archive_from_dir(&artifact_dir, &zip_path)?;

    let zip_bytes = fs::read(&zip_path)?;
    let hash = Sha256::digest(&zip_bytes);
    let hash_b64 = general_purpose::STANDARD.encode(hash);

    Ok(BucketListenerBuildOutput { zip_path, hash_b64 })
}

impl Linkable for BucketComponent {
    fn get_env_vars(&self) -> HashMap<String, String> {
        let mut vars = HashMap::new();
        let env_key = format!("RESOURCE_{}_BUCKET_NAME", self.id);
        vars.insert(
            env_key,
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
