use std::{
    collections::HashMap,
    fs::{self},
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
        method: LambdaTriggerApiMethod,
    },
}

#[derive(Deserialize, Clone, Copy)]
enum LambdaTriggerApiMethod {
    GET,
    PUT,
    PATCH,
    POST,
    OPTIONS,
    HEAD,
    ANY,
}

impl LambdaTriggerApiMethod {
    fn to_http_method(&self) -> &'static str {
        match self {
            LambdaTriggerApiMethod::GET => "GET",
            LambdaTriggerApiMethod::PUT => "PUT",
            LambdaTriggerApiMethod::PATCH => "PATCH",
            LambdaTriggerApiMethod::POST => "POST",
            LambdaTriggerApiMethod::OPTIONS => "OPTIONS",
            LambdaTriggerApiMethod::HEAD => "HEAD",
            LambdaTriggerApiMethod::ANY => "ANY",
        }
    }
}

/// Parse API path into segments. E.g. "/users/{id}" -> ["users", "{id}"]
fn parse_api_path(path: &str) -> Result<Vec<String>> {
    let trimmed = path.trim_matches('/');
    if trimmed.is_empty() {
        return Ok(vec![]);
    }
    let parts: Vec<String> = trimmed.split('/').map(|s| s.to_string()).collect();
    for part in &parts {
        if part.is_empty() {
            bail!("Invalid API path '{}': empty segment", path);
        }
        let valid = part.chars().all(|c| {
            c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' || c == '{' || c == '}'
        });
        if !valid {
            bail!(
                "Invalid API path '{}': segment '{}' contains invalid characters",
                path,
                part
            );
        }
        if part.starts_with('{') && !part.ends_with('}') {
            bail!("Invalid API path '{}': malformed path parameter '{}'", path, part);
        }
    }
    Ok(parts)
}

/// Generate Terraform resource key for a path segment. E.g. ["users", "{id}"] -> "users_id"
fn path_segment_to_key(segment: &str) -> String {
    segment
        .trim_matches(|c| c == '{' || c == '}')
        .replace('-', "_")
        .to_lowercase()
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
                        "depends_on": [
                            format!("aws_s3_object.{}", &keys.artifact_key),
                            format!("aws_iam_role.{}", &keys.role_key)
                        ]
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
                    let permission_key = format!("{}_cron_permission", &self.id);

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

                        let permissions_entry = resource
                            .entry("aws_lambda_permission".to_string())
                            .or_insert_with(|| json!({}));
                        permissions_entry.as_object_mut().unwrap().insert(
                            permission_key,
                            json!({
                                "statement_id": format!("{}-cron-invoke", &self.id),
                                "action": "lambda:InvokeFunction",
                                "function_name": format!("${{aws_lambda_function.{}.function_name}}", &self.id),
                                "principal": "events.amazonaws.com",
                                "source_arn": format!("${{aws_cloudwatch_event_rule.{}.arn}}", &rule_key)
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
                                }],
                                "depends_on": [
                                    format!("aws_lambda_permission.{}", &permission_key)
                                ]
                            }
                        }),
                    );
                    }
                }
                LambdaTrigger::Api {
                    api_id,
                    path,
                    method,
                } => {
                    let api_id = match api_id {
                        None => String::from("default_api"),
                        Some(id) => id.to_owned(),
                    };

                    let path_parts = parse_api_path(path).expect("Invalid API path");
                    let api_key = format!("api_{}", api_id.replace('-', "_"));
                    let project = ocel.current_project.as_ref().unwrap();
                    let api_name = format!(
                        "ocel-{}-{}-{}",
                        project.name,
                        project.current_env_name,
                        api_id.replace('-', "_")
                    );

                    // Data source for region (needed for Lambda integration URI)
                    if let Some(data) = tf_json.get_mut("data") {
                        if let Some(data_obj) = data.as_object_mut() {
                            if !data_obj.contains_key("aws_region") {
                                data_obj.insert(
                                    "aws_region".to_string(),
                                    json!({ "current": {} }),
                                );
                            }
                        }
                    } else {
                        tf_json["data"] = json!({
                            "aws_region": {
                                "current": {}
                            }
                        });
                    }

                    if let Some(resource) =
                        tf_json.get_mut("resource").and_then(|r| r.as_object_mut())
                    {
                        // REST API
                        let rest_api_entry = resource
                            .entry("aws_api_gateway_rest_api".to_string())
                            .or_insert_with(|| json!({}));
                        if let Some(apis) = rest_api_entry.as_object_mut() {
                            apis.entry(api_key.clone()).or_insert_with(|| {
                                json!({
                                    "name": api_name,
                                    "description": "Ocel API Gateway"
                                })
                            });
                        }

                        // Determine target resource for method/integration
                        let method_resource_id = if path_parts.is_empty() {
                            // Path "/" - use root
                            format!("${{aws_api_gateway_rest_api.{}.root_resource_id}}", api_key)
                        } else {
                            // Build resource hierarchy
                            let resources_entry = resource
                                .entry("aws_api_gateway_resource".to_string())
                                .or_insert_with(|| json!({}));
                            let resources = resources_entry.as_object_mut().unwrap();

                            let mut parent_ref = format!(
                                "${{aws_api_gateway_rest_api.{}.root_resource_id}}",
                                api_key
                            );
                            let mut cumulative_key = String::new();

                            for part in path_parts.iter() {
                                cumulative_key = if cumulative_key.is_empty() {
                                    path_segment_to_key(part)
                                } else {
                                    format!("{}_{}", cumulative_key, path_segment_to_key(part))
                                };
                                let res_key = format!("{}_res_{}", api_key, cumulative_key);
                                resources.entry(res_key.clone()).or_insert_with(|| {
                                    json!({
                                        "rest_api_id": format!("${{aws_api_gateway_rest_api.{}.id}}", api_key),
                                        "parent_id": parent_ref,
                                        "path_part": part
                                    })
                                });
                                parent_ref = format!("${{aws_api_gateway_resource.{}.id}}", res_key);
                            }
                            format!("${{aws_api_gateway_resource.{}_res_{}.id}}", api_key, cumulative_key)
                        };

                        let method_http = method.to_http_method();

                        // Method resource
                        let method_key = format!("{}_api_method", &self.id);
                        let methods_entry = resource
                            .entry("aws_api_gateway_method".to_string())
                            .or_insert_with(|| json!({}));
                        methods_entry.as_object_mut().unwrap().insert(
                            method_key.clone(),
                            json!({
                                "rest_api_id": format!("${{aws_api_gateway_rest_api.{}.id}}", api_key),
                                "resource_id": method_resource_id,
                                "http_method": method_http,
                                "authorization": "NONE"
                            }),
                        );

                        // Lambda integration
                        let integration_key = format!("{}_api_integration", &self.id);
                        let integrations_entry = resource
                            .entry("aws_api_gateway_integration".to_string())
                            .or_insert_with(|| json!({}));
                        integrations_entry.as_object_mut().unwrap().insert(
                            integration_key.clone(),
                            json!({
                                "rest_api_id": format!("${{aws_api_gateway_rest_api.{}.id}}", api_key),
                                "resource_id": method_resource_id,
                                "http_method": format!("${{aws_api_gateway_method.{}.http_method}}", method_key),
                                "integration_http_method": "POST",
                                "type": "AWS_PROXY",
                                "uri": format!("arn:aws:apigateway:${{data.aws_region.current.name}}:lambda:path/2015-03-31/functions/${{aws_lambda_function.{}.arn}}/invocations", &self.id)
                            }),
                        );

                        // Lambda permission for API Gateway
                        let permission_key = format!("{}_api_permission", &self.id);
                        let permissions_entry = resource
                            .entry("aws_lambda_permission".to_string())
                            .or_insert_with(|| json!({}));
                        permissions_entry.as_object_mut().unwrap().insert(
                            permission_key.clone(),
                            json!({
                                "statement_id": format!("{}-api-invoke", &self.id),
                                "action": "lambda:InvokeFunction",
                                "function_name": format!("${{aws_lambda_function.{}.function_name}}", &self.id),
                                "principal": "apigateway.amazonaws.com",
                                "source_arn": format!("${{aws_api_gateway_rest_api.{}.execution_arn}}/*/*", api_key)
                            }),
                        );

                        // CORS: OPTIONS method + MOCK integration for the same resource
                        let cors_method_key = format!("{}_api_cors_method", &self.id);
                        let cors_methods_entry = resource
                            .entry("aws_api_gateway_method".to_string())
                            .or_insert_with(|| json!({}));
                        cors_methods_entry.as_object_mut().unwrap().insert(
                            cors_method_key.clone(),
                            json!({
                                "rest_api_id": format!("${{aws_api_gateway_rest_api.{}.id}}", api_key),
                                "resource_id": method_resource_id,
                                "http_method": "OPTIONS",
                                "authorization": "NONE"
                            }),
                        );

                        let cors_integration_key = format!("{}_api_cors_integration", &self.id);
                        let cors_integrations_entry = resource
                            .entry("aws_api_gateway_integration".to_string())
                            .or_insert_with(|| json!({}));
                        cors_integrations_entry.as_object_mut().unwrap().insert(
                            cors_integration_key.clone(),
                            json!({
                                "rest_api_id": format!("${{aws_api_gateway_rest_api.{}.id}}", api_key),
                                "resource_id": method_resource_id,
                                "http_method": "OPTIONS",
                                "type": "MOCK",
                                "request_templates": {
                                    "application/json": "{\"statusCode\": 200}"
                                }
                            }),
                        );

                        let cors_method_response_key = format!("{}_api_cors_method_response", &self.id);
                        let method_responses_entry = resource
                            .entry("aws_api_gateway_method_response".to_string())
                            .or_insert_with(|| json!({}));
                        method_responses_entry.as_object_mut().unwrap().insert(
                            cors_method_response_key.clone(),
                            json!({
                                "rest_api_id": format!("${{aws_api_gateway_rest_api.{}.id}}", api_key),
                                "resource_id": method_resource_id,
                                "http_method": "OPTIONS",
                                "status_code": "200",
                                "response_parameters": {
                                    "method.response.header.Access-Control-Allow-Origin": true,
                                    "method.response.header.Access-Control-Allow-Methods": true,
                                    "method.response.header.Access-Control-Allow-Headers": true
                                }
                            }),
                        );

                        let cors_integration_response_key = format!("{}_api_cors_integration_response", &self.id);
                        let integration_responses_entry = resource
                            .entry("aws_api_gateway_integration_response".to_string())
                            .or_insert_with(|| json!({}));
                        integration_responses_entry.as_object_mut().unwrap().insert(
                            cors_integration_response_key.clone(),
                            json!({
                                "rest_api_id": format!("${{aws_api_gateway_rest_api.{}.id}}", api_key),
                                "resource_id": method_resource_id,
                                "http_method": "OPTIONS",
                                "status_code": "200",
                                "response_parameters": {
                                    "method.response.header.Access-Control-Allow-Origin": "'*'",
                                    "method.response.header.Access-Control-Allow-Headers": "'Content-Type,Authorization,X-Amz-Date,X-Api-Key,X-Amz-Security-Token'",
                                    "method.response.header.Access-Control-Allow-Methods": "'GET,POST,PUT,PATCH,DELETE,OPTIONS,HEAD'"
                                },
                                "depends_on": [format!("aws_api_gateway_integration.{}", cors_integration_key)]
                            }),
                        );

                        // Deployment (depends_on accumulates per api_id via json_deep_merge)
                        let deployment_key = format!("{}_deployment", api_key);
                        let deployment_entry = resource
                            .entry("aws_api_gateway_deployment".to_string())
                            .or_insert_with(|| json!({}));
                        let deployment_obj = deployment_entry.as_object_mut().unwrap();
                        let deployment_resource = deployment_obj.entry(deployment_key.clone()).or_insert_with(|| {
                            json!({
                                "rest_api_id": format!("${{aws_api_gateway_rest_api.{}.id}}", api_key),
                                "depends_on": [format!("aws_api_gateway_integration.{}", integration_key)],
                                "lifecycle": {
                                    "create_before_destroy": true
                                }
                            })
                        });
                        if let Some(dep) = deployment_resource.as_object_mut() {
                            if let Some(depends_on) = dep.get_mut("depends_on") {
                                if let Some(arr) = depends_on.as_array_mut() {
                                    let dep_ref = format!("aws_api_gateway_integration.{}", integration_key);
                                    if !arr.iter().any(|v| v.as_str() == Some(dep_ref.as_str())) {
                                        arr.push(serde_json::Value::String(dep_ref));
                                    }
                                    let cors_dep_ref = format!("aws_api_gateway_integration.{}", cors_integration_key);
                                    if !arr.iter().any(|v| v.as_str() == Some(cors_dep_ref.as_str())) {
                                        arr.push(serde_json::Value::String(cors_dep_ref));
                                    }
                                }
                            }
                        }

                        // Stage
                        let stage_key = format!("{}_stage", api_key);
                        let stages_entry = resource
                            .entry("aws_api_gateway_stage".to_string())
                            .or_insert_with(|| json!({}));
                        stages_entry.as_object_mut().unwrap().entry(stage_key.clone()).or_insert_with(|| {
                            json!({
                                "deployment_id": format!("${{aws_api_gateway_deployment.{}.id}}", deployment_key),
                                "rest_api_id": format!("${{aws_api_gateway_rest_api.{}.id}}", api_key),
                                "stage_name": project.current_env_name
                            })
                        });
                    }

                    // Output: API URL
                    let stage_key = format!("{}_stage", api_key);
                    let output_key = format!("RESOURCE_{}_API_URL", api_id.to_uppercase().replace('-', "_"));
                    if let Some(output) = tf_json.get_mut("output").and_then(|o| o.as_object_mut()) {
                        output.entry(output_key).or_insert_with(|| {
                            json!({
                                "value": format!(
                                    "https://${{aws_api_gateway_rest_api.{}.id}}.execute-api.${{data.aws_region.current.name}}.amazonaws.com/${{aws_api_gateway_stage.{}.stage_name}}",
                                    api_key,
                                    stage_key
                                )
                            })
                        });
                    }
                }
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

    let output = Command::new(bun_path)
        .current_dir(&artifact_dir)
        .arg("run")
        .arg("analyze.mjs")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
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
    let build_output = Command::new(bun_path).args([
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
        .envs(envs)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output()?;

    if !build_output.status.success() {
        let stderr = String::from_utf8_lossy(&build_output.stderr);
        bail!("Failed to build Lambda {}:\n{}", id, stderr);
    }

    let zip_path = artifact_dir.join("dist").join(format!("{}.zip", &id));
    fs::create_dir_all(zip_path.parent().unwrap())?;

    utils::archive::create_archive_from_dir(&out_dir, &zip_path)?;

    let zip_bytes = fs::read(&zip_path)?;
    let hash = Sha256::digest(&zip_bytes);
    let hash_b64 = general_purpose::STANDARD.encode(hash);

    Ok(LambdaBuildOutput { zip_path, hash_b64 })
}
