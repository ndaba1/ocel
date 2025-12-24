use anyhow::{Ok, Result, bail};
use indicatif::ProgressBar;

use crate::{ocel::Ocel, utils::archive};

const TOFU_VERSION: &str = "1.10.7";

pub fn install_tofu(ocel_cfg: &Ocel, pb: &ProgressBar) -> Result<()> {
    let url = get_download_url()?;
    let (filename, file_ext) = get_tofu_filename()?;

    let temp_dir = std::env::temp_dir();
    let archive_dest = temp_dir.join(&filename);
    let tofu_bin_path = &ocel_cfg.tofu_bin_path;
    let install_dir = tofu_bin_path.parent().expect("Cant find tofu bin path");
    let binary_name = tofu_bin_path
        .file_name()
        .expect("Cant find tofu binary name")
        .to_str()
        .expect("Cant convert tofu binary name to str");

    if !install_dir.exists() {
        std::fs::create_dir_all(&install_dir)?;
    }

    archive::download_archive(&url, &archive_dest, pb)?;
    archive::extract_archive(&archive_dest, &install_dir, &file_ext, &binary_name)?;

    // cleanup
    std::fs::remove_file(archive_dest).ok();

    Ok(())
}

fn get_download_url() -> Result<String> {
    let (file_name, _) = get_tofu_filename()?;
    let url = format!(
        "https://github.com/opentofu/opentofu/releases/download/v{}/{}",
        TOFU_VERSION, file_name
    );
    Ok(url)
}

fn get_os_name() -> Result<String> {
    let name = if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        bail!("Unsupported operating system");
    };

    Ok(name.to_string())
}

fn get_arch_name() -> Result<String> {
    let arch = if cfg!(target_arch = "x86_64") {
        "amd64"
    } else if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        bail!("Unsupported architecture");
    };

    Ok(arch.to_string())
}

/// Get the tofu filename and file extension based on OS and architecture
/// Returns (filename, file_extension)
fn get_tofu_filename() -> Result<(String, String)> {
    let os_name = get_os_name()?;
    let arch_name = get_arch_name()?;

    let file_ext = if os_name == "windows" {
        "zip"
    } else {
        "tar.gz"
    }
    .to_string();

    let filename = format!(
        "tofu_{}_{}_{}.{}",
        TOFU_VERSION, os_name, arch_name, file_ext
    );

    Ok((filename, file_ext))
}
