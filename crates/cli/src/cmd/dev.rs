use anyhow::{Ok, Result};

use crate::{ocel::Ocel, project::OcelProject};

pub fn dev() -> Result<()> {
    println!("Running ocel in development mode...");

    // TODO: allow passing custom env name
    let mut ocel = Ocel::init()?;
    let current_env = whoami::username();
    let project = OcelProject::get_current_project(current_env)?;

    ocel.set_current_project(&project);
    ocel.run_dev_mode()?;

    Ok(())
}
