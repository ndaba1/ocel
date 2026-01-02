use anyhow::Result;
use serde_json::{Value, json};

use crate::{ocel::Ocel, utils::get_nanoid};

pub const ASSET_BUCKET_NAME: &str = "ocel_asset_bucket";
pub const ASSET_BUCKET_OUTPUT_NAME: &str = "OCEL_ASSET_BUCKET_NAME";

pub fn asset_bucket_ref() -> String {
    format!("${{aws_s3_bucket.{}.id}}", ASSET_BUCKET_NAME)
}

/// Returns default components to be included in every Ocel project
pub async fn get_default_components(ocel: &Ocel) -> Result<Value> {
    let outputs = ocel.get_tofu_outputs().await?;
    let project = ocel.current_project.as_ref().unwrap();
    let current_env = project.current_env_name.clone();

    let default_asset_bucket_id = format!(
        "ocel-assets-{}-{}-{}",
        project.name,
        current_env,
        get_nanoid(7)
    )
    .to_lowercase();

    let asset_bucket_id = outputs
        .get(ASSET_BUCKET_OUTPUT_NAME)
        .unwrap_or(&default_asset_bucket_id);

    let asset_bucket = json!({
        "bucket": asset_bucket_id.to_string(),
        "force_destroy": true
    });

    Ok(json!({
        "resource": {
            "aws_s3_bucket": {
                ASSET_BUCKET_NAME: asset_bucket
            }
        },
        "output": {
            ASSET_BUCKET_OUTPUT_NAME: {
                "value": format!("${{aws_s3_bucket.{}.id}}", ASSET_BUCKET_NAME)
            }
        }
    }))
}
