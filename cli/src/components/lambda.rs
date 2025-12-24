use std::{
    collections::HashMap,
    fs::{self},
    path::{Path, PathBuf},
    process::Command,
    sync::Mutex,
};

use anyhow::{Context, anyhow, bail};
use base64::{Engine, engine::general_purpose};
use reqwest::Url;
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::{components::asset_bucket_ref, engine::Component, utils};

pub struct LambdaComponent {
    id: String,
    config: LambdaConfig,
    source_file: String,
    build_output: Mutex<Option<LambdaBuildOutput>>,
}

#[derive(Deserialize)]
struct LambdaConfig {
    /// List of resource IDs this Lambda links to
    links: Option<Vec<String>>,

    /// Trigger configuration
    trigger: Option<LambdaTrigger>,
}

struct LambdaBuildOutput {
    zip_path: PathBuf,
    hash_b64: String,
}

struct LambdaResourceKeys {
    role_key: String,
    function_key: String,
    artifact_key: String,
    policy_key: String,
}

#[derive(Deserialize)]
#[serde(tag = "type", content = "config", rename_all = "lowercase")]
enum LambdaTrigger {
    Url { auth_type: String, streaming: bool },
    Cron { schedule: String },
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
        }
    }

    fn get_resource_keys(&self) -> LambdaResourceKeys {
        LambdaResourceKeys {
            role_key: format!("{}_role", &self.id),
            function_key: format!("{}_function", &self.id),
            artifact_key: format!("{}_artifact", &self.id),
            policy_key: format!("{}_policy", &self.id),
        }
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

impl Component for LambdaComponent {
    fn build(&self, engine: &crate::engine::OcelEngine) -> anyhow::Result<()> {
        let ocel = engine.get_ocel();
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

        // TODO: compare hashes to determine if rebuild is needed

        let artifact_dir = project.current_env_dir.join("artifacts").join(&self.id);

        // cleanup previous build
        if artifact_dir.exists() {
            fs::remove_dir_all(&artifact_dir)?;
        }

        fs::create_dir_all(&artifact_dir).context("Failed to create Lambda artifact directory")?;

        println!("Building Lambda {}...", self.source_file);

        let source_file = self.source_file.clone();
        let analyzer_code = ANALYZER_SCRIPT
            .replace("__OCEL_TARGET_FILE__", &source_file)
            .replace("__OCEL_LAMBDA_ID__", &self.id);

        let analyzer_path = artifact_dir.join("analyze.mjs");
        fs::write(&analyzer_path, analyzer_code)?;

        let mut cmd_analyze = Command::new(&ocel.bun_bin_path);
        cmd_analyze.args(["run", analyzer_path.to_str().unwrap()]);

        let output = Command::new(&ocel.bun_bin_path)
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

        let bun_bin_path = &ocel.bun_bin_path;
        let envs = HashMap::from([("OCEL_PHASE", "build")]);

        let out_dir = artifact_dir.join("build");
        let out_file = out_dir.join("index.js");

        // TODO: sourcemaps ?
        let status = Command::new(bun_bin_path).args([
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
            bail!("Failed to build Lambda {}", self.id);
        }

        let zip_path = artifact_dir.join("dist").join(format!("{}.zip", &self.id));
        fs::create_dir_all(zip_path.parent().unwrap())?;

        utils::archive::create_archive_from_dir(&out_dir, &zip_path)?;

        let zip_bytes = fs::read(&zip_path)?;
        let hash = Sha256::digest(&zip_bytes);
        let hash_b64 = general_purpose::STANDARD.encode(hash);

        let mut output_lock = self.build_output.lock().unwrap();
        *output_lock = Some(LambdaBuildOutput { zip_path, hash_b64 });

        Ok(())
    }

    fn to_terraform(&self, engine: &crate::engine::OcelEngine) -> serde_json::Value {
        let mut env_vars = HashMap::new();
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
        let project = ocel.current_project.as_ref().unwrap();
        let current_env = project.current_env_name.clone();

        let keys = self.get_resource_keys();
        let function_name = format!("{}-{}-{}", &self.id, project.name, current_env);
        let role_name = format!("{}-role", function_name);

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

        // TODO: outputs for Lambda URL etc.
        json!({
            "resource": {
                "aws_iam_role": {
                    &keys.role_key: {
                        "name": &role_name,
                        "assume_role_policy": assume_role_policy.to_string()
                    }
                },
                "aws_iam_role_policy_attachment": {
                    &keys.policy_key: {
                        "role": format!("${{aws_iam_role.{}.name}}", &keys.role_key),
                        "policy_arn": "arn:aws:iam::aws:policy/service-role/AWSLambdaBasicExecutionRole"
                    }
                },
                "aws_lambda_function": {
                    &self.id: {
                        "timeout": 30,
                        "function_name": function_name,
                        "handler": "index.handler",
                        "runtime": "nodejs22.x",
                        "role": format!("${{aws_iam_role.{}.arn}}", &keys.role_key),
                        "environment": {
                            "variables": env_vars
                        },
                        "s3_bucket": s3_bucket,
                        "s3_key": s3_key,
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
            }
        })
    }
}
