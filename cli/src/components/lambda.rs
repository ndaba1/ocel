use std::{
    collections::HashMap,
    fs::{self},
    os::unix::fs::MetadataExt,
    path::PathBuf,
    process::Command,
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use aws_config::{BehaviorVersion, meta::region::RegionProviderChain};
use aws_sdk_lambda::Client;
use aws_smithy_types::Blob;
use base64::{Engine, engine::general_purpose};
use rand::rand_core::le;
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use tracing::{debug, info};

use crate::{
    components::asset_bucket_ref,
    engine::{Component, ResourceType},
    ocel::Ocel,
    utils,
};

pub struct LambdaComponent {
    id: String,
    config: LambdaConfig,
    source_file: String,
    build_output: Mutex<Option<LambdaBuildOutput>>,
    default_env_vars: HashMap<String, String>,
}

#[derive(Deserialize)]
struct LambdaConfig {
    /// List of resource IDs this Lambda links to
    links: Option<Vec<String>>,

    /// Trigger configuration
    trigger: Option<LambdaTrigger>,
}

pub struct LambdaBuildOutput {
    zip_path: PathBuf,
    hash_b64: String,
}

pub struct LambdaResourceKeys {
    pub role_key: String,
    pub function_key: String,
    pub artifact_key: String,
    pub execution_policy_key: String,
    pub inline_policy_key: String,
}

#[derive(Deserialize)]
#[serde(tag = "type", content = "config", rename_all = "lowercase")]
enum LambdaTrigger {
    Url {
        #[serde(default)]
        streaming: bool,
    },
    Cron {
        schedule: String,
    },
    S3 {
        bucket: String,
        events: Vec<String>,
    },
    Api {
        #[serde(rename = "apiId")]
        api_id: Option<String>,
        path: String,
        method: String,
    },
}

impl LambdaComponent {
    pub fn new(id: String, config: serde_json::Value, source_file: String) -> Self {
        let lambda_config: LambdaConfig =
            serde_json::from_value(config).expect("Invalid Lambda configuration");

        LambdaComponent {
            id,
            config: lambda_config,
            source_file,
            build_output: Mutex::new(None),
            default_env_vars: HashMap::new(),
        }
    }

    fn get_fn_name(&self, ocel: &Ocel) -> String {
        let project = ocel
            .current_project
            .as_ref()
            .expect("Unable to resolve current project");

        let current_env = &project.current_env_name;

        format!("{}-{}-{}", &self.id, &project.name, current_env)
    }

    pub fn get_resource_keys(&self) -> LambdaResourceKeys {
        LambdaResourceKeys {
            role_key: format!("{}_role", &self.id),
            function_key: format!("{}_function", &self.id),
            artifact_key: format!("{}_artifact", &self.id),
            execution_policy_key: format!("{}_policy", &self.id),
            inline_policy_key: format!("{}_inline_policy", &self.id),
        }
    }

    pub fn set_default_env_vars(&mut self, vars: HashMap<String, String>) {
        self.default_env_vars.extend(vars);
    }
}

const ANALYZER_SCRIPT: &str = r#"
import { writeFile } from 'fs/promises';

async function resolve() {
    try {
        const mod = await import('__OCEL_TARGET_FILE__');
        const exportKeys = Object.keys(mod);

        // Find the export with the specific Symbol
        const handlerName = exportKeys.find(key => mod[key]?.[Symbol.for('ocel:lambda:id')] === '__OCEL_LAMBDA_ID__');
        
        if (!handlerName) {
            console.error(`❌ Ocel Error: No export found with ID '__OCEL_LAMBDA_ID__' in ${'__OCEL_TARGET_FILE__'}`);
            process.exit(1);
        }

        await writeFile('analysis.json', JSON.stringify({ handlerName }));
    } catch (err) {
        console.error('\n❌ Ocel Discovery Failed:');
        console.error('Your code crashed during the build phase.');
        console.error('If you have side effects (DB connections), wrap them in ocel.effect()');
        console.error('\nOriginal Error:');
        console.error(err);
        process.exit(1);
    }
}

resolve();
"#;

#[async_trait]
impl Component for LambdaComponent {
    fn id(&self) -> &str {
        &self.id
    }

    fn resource_type(&self) -> ResourceType {
        ResourceType::Lambda
    }

    fn source_file(&self) -> Option<&String> {
        Some(&self.source_file)
    }

    /// update the lambda function code in place during dev mode
    async fn dev(&self, ocel: Arc<Ocel>) -> anyhow::Result<()> {
        info!("Updating Lambda function code for {}", &self.id);

        self.build(ocel.clone()).await?;

        let file_path = self
            .build_output
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .zip_path
            .clone();
        let file_meta = file_path.metadata()?;
        let file_size = file_meta.len();
        let file_bytes = fs::read(&file_path)?;

        let region_provider = RegionProviderChain::default_provider().or_else("us-east-1");
        let config = aws_config::defaults(BehaviorVersion::latest())
            .region(region_provider)
            .load()
            .await;
        let client = Client::new(&config);

        // if file is larger than 50MB, use s3 instead
        if file_size < 50 * 1024 * 1024 {
            let zip_blob = Blob::new(file_bytes);

            let response = client
                .update_function_code()
                .function_name(self.get_fn_name(ocel.as_ref()))
                .zip_file(zip_blob)
                .send()
                .await?;

            debug!(
                "Lambda function code updated: {:?}",
                response.last_modified.unwrap_or_default()
            );
        } else {
            bail!(
                "Lambda code update via direct upload not supported for files larger than 50MB. Please redeploy the infrastructure."
            );
        }

        Ok(())
    }

    async fn build(&self, ocel: Arc<Ocel>) -> anyhow::Result<()> {
        let project = ocel
            .current_project
            .as_ref()
            .context("Unable to resolve current project")?;

        match project.project_type {
            crate::project::ProjectType::Typescript => (),
            _ => {
                return Ok(());
            }
        };

        let env_dir = project.current_env_dir.clone();
        let source_file = self.source_file.clone();
        let lambda_id = self.id.clone();
        let bun_path = ocel.bun_bin_path.clone();

        // TODO: compare hashes to determine if rebuild is needed

        let build_result = tokio::task::spawn_blocking(move || -> Result<LambdaBuildOutput> {
            build_lambda_artifact(&lambda_id, &source_file, &env_dir, &bun_path)
        })
        .await??;

        let mut output_lock = self.build_output.lock().unwrap();
        *output_lock = Some(build_result);

        Ok(())
    }

    fn to_terraform(
        &self,
        engine: &crate::engine::OcelEngine,
        _outputs: HashMap<String, String>,
    ) -> serde_json::Value {
        let mut env_vars = self.default_env_vars.clone();
        let mut permissions: Vec<serde_json::Value> = Vec::new();

        let output_lock = self.build_output.lock().unwrap();
        let build_data = output_lock.as_ref().expect("Build output missing!");

        let source_hash = &build_data.hash_b64;
        let zip_path = &build_data.zip_path;

        if let Some(links) = &self.config.links {
            for link_id in links {
                if let Some(linkable) = engine.get_linkable(link_id) {
                    // Merge environment variables
                    for (key, value) in linkable.get_env_vars() {
                        env_vars.insert(key, value);
                    }

                    // Merge permissions
                    permissions.extend(linkable.get_permissions());
                }
            }
        }

        let ocel = engine.get_ocel();
        let keys = self.get_resource_keys();
        let function_name = self.get_fn_name(ocel.as_ref());
        let role_name = format!("{}-role", function_name);
        let policy_name = format!("{}-policy", function_name);

        let assume_role_policy = json!({
            "Version": "2012-10-17",
            "Statement": [{
                "Action": "sts:AssumeRole",
                "Effect": "Allow",
                "Principal": { "Service": "lambda.amazonaws.com" }
            }]
        });

        let s3_bucket = asset_bucket_ref();
        let s3_key = format!(
            "{}/{}",
            &self.id,
            zip_path.file_name().unwrap().to_str().unwrap()
        );

        let mut inline_policy_map = serde_json::Map::new();
        if !permissions.is_empty() {
            inline_policy_map.insert(
                keys.inline_policy_key.clone(),
                json!({
                    "name": policy_name,
                    "role": format!("${{aws_iam_role.{}.name}}", &keys.role_key),
                    "policy": json!({
                        "Version": "2012-10-17",
                        "Statement": permissions
                    }).to_string()
                }),
            );
        }

        // TODO: outputs for Lambda URL etc.
        let mut tf_json = json!({
            "resource": {
                "aws_iam_role": {
                    &keys.role_key: {
                        "name": &role_name,
                        "assume_role_policy": assume_role_policy.to_string()
                    }
                },
                "aws_iam_role_policy_attachment": {
                    &keys.execution_policy_key: {
                        "role": format!("${{aws_iam_role.{}.name}}", &keys.role_key),
                        "policy_arn": "arn:aws:iam::aws:policy/service-role/AWSLambdaBasicExecutionRole"
                    }
                },
                "aws_lambda_function": {
                    &self.id: {
                        "timeout": 30,
                        "memory_size": 1024,
                        "function_name": function_name,
                        "handler": "index.handler",
                        "runtime": "nodejs22.x",
                        "role": format!("${{aws_iam_role.{}.arn}}", &keys.role_key),
                        "environment": {
                            "variables": env_vars
                        },
                        "s3_bucket": s3_bucket,
                        "s3_key": s3_key,
                        "source_code_hash": source_hash,
                    }
                },
                "aws_s3_object": {
                    &keys.artifact_key: {
                        "bucket": s3_bucket,
                        "key": s3_key,
                        "source": zip_path.to_str().unwrap(),
                        "source_hash": source_hash,
                    }
                }
            },
            "output": {}
        });

        if !inline_policy_map.is_empty() {
            if let Some(resource) = tf_json.get_mut("resource").and_then(|r| r.as_object_mut()) {
                resource.insert(
                    "aws_iam_role_policy".to_string(),
                    serde_json::Value::Object(inline_policy_map),
                );
            }
        }

        if let Some(trigger) = &self.config.trigger {
            match trigger {
                LambdaTrigger::Url { streaming } => {
                    let lambda_url_key = format!("{}_url", &self.id);
                    let lambda_url_output_key = format!("RESOURCE_{}_LAMBDA_URL", &self.id);

                    let url_config = json!({
                        "function_name": format!("${{aws_lambda_function.{}.function_name}}", &self.id),
                        "authorization_type": "NONE",
                        "invoke_mode": if *streaming { "RESPONSE_STREAM" } else { "BUFFERED" },
                    });

                    if let Some(resource) =
                        tf_json.get_mut("resource").and_then(|r| r.as_object_mut())
                    {
                        resource.insert(
                            "aws_lambda_function_url".to_string(),
                            json!({
                                &lambda_url_key: url_config
                            }),
                        );
                    }

                    if let Some(output) = tf_json.get_mut("output").and_then(|o| o.as_object_mut())
                    {
                        output.insert(
                        lambda_url_output_key,
                        json!({
                            "value": format!("${{aws_lambda_function_url.{}.function_url}}", lambda_url_key)
                        }),
                    );
                    }
                }
                LambdaTrigger::Cron { schedule } => {
                    let rule_key = format!("{}_cron_rule", &self.id);
                    let target_key = format!("{}_cron_target", &self.id);

                    if let Some(resource) =
                        tf_json.get_mut("resource").and_then(|r| r.as_object_mut())
                    {
                        resource.insert(
                            "aws_cloudwatch_event_rule".to_string(),
                            json!({
                                &rule_key: {
                                    "name": format!("{}-cron-rule", &self.id),
                                    "schedule_expression": schedule,
                                }
                            }),
                        );

                        resource.insert(
                        "aws_cloudwatch_event_target".to_string(),
                        json!({
                            &target_key: {
                                "rule": format!("${{aws_cloudwatch_event_rule.{}.name}}", &rule_key),
                                "target_id": format!("{}-lambda-target", &self.id),
                                "arn": format!("${{aws_lambda_function.{}.arn}}", &self.id),
                            }
                        }),
                    );
                    }
                }
                LambdaTrigger::S3 { bucket, events } => {
                    let permission_key = format!("{}_s3_permission", &self.id);
                    let bucket_notification_key = format!("{}_s3_notification", &self.id);

                    if let Some(resource) =
                        tf_json.get_mut("resource").and_then(|r| r.as_object_mut())
                    {
                        resource.insert(
                        "aws_lambda_permission".to_string(),
                        json!({
                            &permission_key: {
                                "statement_id": format!("{}-s3-invoke", &self.id),
                                "action": "lambda:InvokeFunction",
                                "function_name": format!("${{aws_lambda_function.{}.function_name}}", &self.id),
                                "principal": "s3.amazonaws.com",
                                "source_arn": format!("arn:aws:s3:::{}", bucket),
                            }
                        }),
                    );

                        resource.insert(
                        "aws_s3_bucket_notification".to_string(),
                        json!({
                            &bucket_notification_key: {
                                "bucket": bucket,
                                "lambda_function": [{
                                    "lambda_function_arn": format!("${{aws_lambda_function.{}.arn}}", &self.id),
                                    "events": events,
                                }]
                            }
                        }),
                    );
                    }
                }
                _ => {}
            }
        }
        tf_json
    }
}

fn build_lambda_artifact(
    id: &str,
    source_file: &str,
    env_dir: &PathBuf,
    bun_path: &PathBuf,
) -> Result<LambdaBuildOutput> {
    let artifact_dir = env_dir.join("artifacts").join(id);

    // cleanup previous build
    if artifact_dir.exists() {
        fs::remove_dir_all(&artifact_dir)?;
    }

    fs::create_dir_all(&artifact_dir).context("Failed to create Lambda artifact directory")?;

    let analyzer_code = ANALYZER_SCRIPT
        .replace("__OCEL_TARGET_FILE__", &source_file)
        .replace("__OCEL_LAMBDA_ID__", &id);

    let analyzer_path = artifact_dir.join("analyze.mjs");
    fs::write(&analyzer_path, analyzer_code)?;

    let mut cmd_analyze = Command::new(bun_path);
    cmd_analyze.args(["run", analyzer_path.to_str().unwrap()]);

    let output = Command::new(bun_path)
        .current_dir(&artifact_dir)
        .arg("run")
        .arg("analyze.mjs")
        .output()
        .context("Failed to execute analyzer")?;

    if !output.status.success() {
        // Forward the stderr from the analyzer
        bail!("{}", String::from_utf8_lossy(&output.stderr));
    }

    // Read the result from the file, NOT stdout
    let analysis_file_path = artifact_dir.join("analysis.json");
    let analysis_content = fs::read_to_string(&analysis_file_path)
        .context("Analyzer did not produce analysis.json")?;

    let analysis_json: serde_json::Value = serde_json::from_str(&analysis_content)?;
    let handler_name = analysis_json["handlerName"]
        .as_str()
        .context("handlerName missing")?;

    let shim_path = artifact_dir.join("shim.ts");

    let shim_content = format!(
        r#"
import {{ {handler} as lambdaFn }} from "{source}";

// actual handler aws lambda looks for
export const handler = lambdaFn.__handler;
"#,
        handler = handler_name,
        source = source_file
    )
    .trim_start()
    .trim_end()
    .to_string();

    fs::write(&shim_path, shim_content)?;

    let envs = HashMap::from([("OCEL_PHASE", "build")]);

    let out_dir = artifact_dir.join("build");
    let out_file = out_dir.join("index.mjs");

    // TODO: sourcemaps ?
    let status = Command::new(bun_path).args([
            "build",
            shim_path.to_str().unwrap(),
            "--outfile",
            out_file.to_str().unwrap(),
            "--target",
            "node",
            "--format",
            "esm",
            "--define",
            "process.env.OCEL_PHASE='\"0\"'",
            "--define",
            "IS_OCEL_DISCOVERY=false",
            "--minify",
            "--banner",
            "import { createRequire } from 'module'; const require = createRequire(import.meta.url);",
        ])
        .envs(envs).status()?;

    if !status.success() {
        bail!("Failed to build Lambda {}", id);
    }

    let zip_path = artifact_dir.join("dist").join(format!("{}.zip", &id));
    fs::create_dir_all(zip_path.parent().unwrap())?;

    utils::archive::create_archive_from_dir(&out_dir, &zip_path)?;

    let zip_bytes = fs::read(&zip_path)?;
    let hash = Sha256::digest(&zip_bytes);
    let hash_b64 = general_purpose::STANDARD.encode(hash);

    Ok(LambdaBuildOutput { zip_path, hash_b64 })
}
