use std::time::Duration;

use anyhow::{Context, Result};
use aws_config::{BehaviorVersion, Region, meta::region::RegionProviderChain};
use aws_sdk_cloudformation::{
    Client,
    types::{
        Capability, Change, ChangeAction, ChangeSetStatus, ChangeSetType, Output, Parameter,
        Replacement, Stack, StackStatus,
    },
};
use clap::Parser;
use colored::{ColoredString, Colorize};
use indicatif::{ProgressBar, ProgressStyle};
use inquire::Confirm;
use serde_json::json;
use tokio::{signal, time::sleep};
use tracing::{debug, info};

use crate::{ocel::Ocel, utils::get_nanoid};

pub const CORE_STACK_NAME: &str = "ocel-core-stack";
pub const CONTAINER_STACK_NAME: &str = "ocel-container-stack";

const CHANGE_SET_PREFIX: &str = "ocel-deploy-";

#[derive(Debug, Clone)]
pub enum StackKind {
    /// Always required (for all ocel functionality)
    Core,
    /// Required for containers only
    Container,
}

pub struct OcelStack {
    pub name: String,
    pub kind: StackKind,
    pub template: String,
}

#[derive(Parser, Debug, Clone)]
pub struct BootstrapOpts {
    /// Skip confirmation and auto-apply changes
    #[arg(long, short = 'y')]
    pub yes: bool,

    /// Bootstrap container stack as well
    #[arg(long, short = 'c')]
    pub container: bool,
}

fn get_stacks() -> Vec<OcelStack> {
    vec![
        OcelStack {
            name: CORE_STACK_NAME.to_string(),
            kind: StackKind::Core,
            template: include_str!("../stacks/core.json").to_string(),
        },
        OcelStack {
            name: CONTAINER_STACK_NAME.to_string(),
            kind: StackKind::Container,
            template: String::new(), // filled in later
        },
    ]
}

pub async fn check_stacks(kinds: Vec<StackKind>) -> Result<bool> {
    let region_provider = RegionProviderChain::first_try(Region::new("us-east-1"));
    let config = aws_config::defaults(BehaviorVersion::latest())
        .region(region_provider)
        .load()
        .await;
    let client = Client::new(&config);

    let all_stacks = get_stacks();
    let targets: Vec<&OcelStack> = all_stacks
        .iter()
        .filter(|s| {
            kinds
                .iter()
                .any(|k| std::mem::discriminant(k) == std::mem::discriminant(&s.kind))
        })
        .collect();

    for stack_def in targets {
        let stack_opt = get_stack(&client, &stack_def.name).await?;

        match stack_opt {
            None => {
                debug!("Stack {} is missing.", stack_def.name);
                return Ok(false);
            }
            Some(stack) => {
                //  is it healthy?
                if let Some(status) = stack.stack_status() {
                    if !is_stack_healthy(status) {
                        debug!(
                            "Stack {} exists but is unhealthy: {:?}",
                            stack_def.name, status
                        );
                        return Ok(false);
                    }
                } else {
                    return Ok(false);
                }
            }
        }
    }

    Ok(true)
}

// TODO: get outputs to avoid infinite loops
// TODO: dynamic stack looping
pub async fn bootstrap(options: BootstrapOpts) -> Result<()> {
    debug!("Bootstrapping Ocel stacks...");

    // this stack must be created in us-east-1
    let region_provider = RegionProviderChain::first_try(Region::new("us-east-1"));
    let config = aws_config::defaults(BehaviorVersion::latest())
        .region(region_provider)
        .load()
        .await;
    let client = Client::new(&config);

    let core_stack = get_stacks()
        .into_iter()
        .find(|s| std::mem::discriminant(&s.kind) == std::mem::discriminant(&StackKind::Core))
        .context("Core stack template not found")?;

    let spinner = create_spinner("Checking infrastructure state...");
    let stack = get_stack(&client, &core_stack.name).await?;
    let stack_status = stack.as_ref().and_then(|s| s.stack_status().cloned());

    spinner.finish_and_clear();

    info!("Current stack status: {:?}", stack_status);

    let change_set_type = match stack_status {
        Some(status) if is_stack_healthy(&status) => {
            println!("✅  Found existing stack. Calculating update diff...");
            ChangeSetType::Update
        }
        Some(status) if is_stack_broken(&status) => {
            anyhow::bail!(
                "❌ Stack is in a broken state: {:?}. Please delete it manually first.",
                status
            );
        }
        Some(status) if status == StackStatus::ReviewInProgress => {
            // stack hangup from a failed create - delete and recreate
            ChangeSetType::Create
        }
        None => {
            println!("✨  No stack found. Preparing initial creation...");
            ChangeSetType::Create
        }
        _ => ChangeSetType::Update, // Default fallback
    };

    let change_set_name = format!("{}{}", CHANGE_SET_PREFIX, chrono::Utc::now().timestamp());
    let spinner = create_spinner("Creating Cloudformation ChangeSet...");

    let mut assets_bucket_name = format!("ocel-assets-{}", get_nanoid(12)).to_lowercase();

    // try to reuse the existing bucket name to avoid replacement
    if let Some(existing_stack) = &stack {
        let params = existing_stack.parameters();
        info!("Existing stack parameters: {:?}", params);

        if let Some(p) = params
            .iter()
            .find(|p| p.parameter_key() == Some("AssetsBucketName"))
        {
            if let Some(val) = p.parameter_value() {
                debug!("Found existing bucket name parameter: {}", val);
                assets_bucket_name = val.to_string();
            }
        }
    }

    let bucket_param = Parameter::builder()
        .parameter_key("AssetsBucketName")
        .parameter_value(assets_bucket_name)
        .build();

    let create_resp = client
        .create_change_set()
        .stack_name(&core_stack.name)
        .change_set_name(&change_set_name)
        .template_body(core_stack.template.to_string())
        .change_set_type(change_set_type)
        .parameters(bucket_param)
        .capabilities(Capability::CapabilityNamedIam) // Essential for creating Roles
        .send()
        .await
        .context("Failed to create ChangeSet")?;

    let change_set_id = create_resp.id().context("No ChangeSet ID returned")?;

    wait_for_change_set(&client, &change_set_id).await?;
    spinner.finish_and_clear();

    let changes = get_change_set_details(&client, &change_set_id).await?;

    if changes.is_empty() {
        println!("{}", "🎉 Infrastructure is already up to date.".green());

        // Clean up the empty change set
        let _ = client
            .delete_change_set()
            .change_set_name(change_set_id)
            .send()
            .await;
        return Ok(());
    }

    print_changes(&changes);

    let has_replacements = changes.iter().any(|c| {
        c.resource_change()
            .map(|rc| rc.replacement() == Some(&Replacement::True))
            .unwrap_or(false)
    });

    if has_replacements {
        println!(
            "\n{}\n",
            "⚠️  WARNING: Some resources will be REPLACED (Potential Data Loss)"
                .on_red()
                .white()
                .bold()
        );
    }

    if !options.yes {
        let confirmed = Confirm::new("Do you want to apply these changes?")
            .with_default(false)
            .prompt()?;

        if !confirmed {
            println!("❌  Bootstrap cancelled.");
            // Clean up
            let _ = client
                .delete_change_set()
                .change_set_name(change_set_id)
                .send()
                .await;

            return Ok(());
        }
    }

    let spinner = create_spinner("Applying changes to AWS (this may take a minute)...");

    client
        .execute_change_set()
        .change_set_name(change_set_id)
        .stack_name(CORE_STACK_NAME)
        .send()
        .await
        .context("Failed to execute ChangeSet")?;

    loop {
        tokio::select! {
            _ = wait_for_stack_completion(&client, CORE_STACK_NAME) => {
                spinner.finish_with_message("Done");
                println!(
                    "\n{}",
                    "✅  Bootstrap completed successfully!".bold().green()
                );

                break;
            }
             _ = signal::ctrl_c() => {
                debug!("🛑 Shutdown signal received.");
                break;
            }
        }
    }

    Ok(())
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
    pb.enable_steady_tick(Duration::from_millis(100));
    pb
}

pub async fn get_stack(client: &Client, stack_name: &str) -> Result<Option<Stack>> {
    let resp = client.describe_stacks().stack_name(stack_name).send().await;

    match resp {
        Ok(output) => {
            let stack = output
                .stacks()
                .first()
                .context("No stacks found in response")?;

            Ok(Some(stack.clone()))
        }
        Err(e) => {
            if let Some(service_err) = e.as_service_error() {
                if service_err
                    .meta()
                    .message()
                    .unwrap_or("")
                    .contains("does not exist")
                {
                    return Ok(None);
                }
            }

            Err(anyhow::anyhow!("AWS Error Details: {:#?}", e))
        }
    }
}

fn is_stack_healthy(status: &StackStatus) -> bool {
    matches!(
        status,
        StackStatus::CreateComplete | StackStatus::UpdateComplete | StackStatus::RollbackComplete
    )
}

fn is_stack_broken(status: &StackStatus) -> bool {
    matches!(
        status,
        StackStatus::CreateFailed | StackStatus::DeleteFailed | StackStatus::RollbackFailed
    )
}

async fn wait_for_change_set(client: &Client, change_set_id: &str) -> Result<()> {
    loop {
        let resp = client
            .describe_change_set()
            .change_set_name(change_set_id)
            .send()
            .await?;

        let status = resp.status().unwrap_or(&ChangeSetStatus::CreateInProgress);

        match status {
            ChangeSetStatus::CreateComplete => return Ok(()),
            ChangeSetStatus::Failed => {
                let reason = resp.status_reason().unwrap_or("Unknown");
                // "No updates" is a failure in AWS terms, but logic handles it later by checking empty changes
                if reason.contains("didn't contain changes") {
                    return Ok(());
                }
                anyhow::bail!("ChangeSet Failed: {}", reason);
            }
            _ => sleep(Duration::from_secs(1)).await,
        }
    }
}

async fn get_change_set_details(client: &Client, change_set_id: &str) -> Result<Vec<Change>> {
    let resp = client
        .describe_change_set()
        .change_set_name(change_set_id)
        .send()
        .await?;

    Ok(resp.changes().to_vec())
}

fn print_changes(changes: &[Change]) {
    println!(
        "\n{}",
        "Proposed Infrastructure Changes:".bold().underline()
    );

    for change in changes {
        if let Some(rc) = change.resource_change() {
            let action = rc.action().unwrap();
            let logical_id = rc.logical_resource_id().unwrap_or("Unknown");
            let type_name = rc.resource_type().unwrap_or("Unknown");
            let replacement = rc.replacement().unwrap_or(&Replacement::False);

            let (symbol, color_func): (&str, fn(&str) -> ColoredString) = match action {
                ChangeAction::Add => ("+", |s| s.green()),
                ChangeAction::Remove => ("-", |s| s.red()),
                ChangeAction::Modify => ("~", |s| s.yellow()),
                _ => ("?", |s| s.normal()),
            };

            let replace_msg = if replacement == &Replacement::True {
                " [REPLACEMENT]".red().bold().to_string()
            } else {
                "".to_string()
            };

            println!(
                "{} {} ({}){}",
                color_func(symbol),
                logical_id.bold(),
                type_name.dimmed(),
                replace_msg
            );
        }
    }
    println!();
}

async fn wait_for_stack_completion(client: &Client, stack_name: &str) -> Result<()> {
    loop {
        let stack = get_stack(client, stack_name).await?.unwrap();
        let status = stack.stack_status().unwrap();

        if matches!(
            status,
            StackStatus::CreateComplete | StackStatus::UpdateComplete
        ) {
            return Ok(());
        }

        if status.as_str().ends_with("_FAILED") || status.as_str().ends_with("_ROLLBACK_COMPLETE") {
            anyhow::bail!("Stack update failed. Status: {:?}", status);
        }

        sleep(Duration::from_secs(2)).await;
    }
}
