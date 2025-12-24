use anyhow::{Ok, Result, bail};
use indicatif::ProgressBar;

use crate::{ocel::Ocel, utils::archive};

const BUN_VERSION: &str = "1.3.5";

pub fn install_bun(ocel_cfg: &Ocel, pb: &ProgressBar) -> Result<()> {
    let url = get_download_url()?;
    let (filename, file_ext) = get_bun_filename()?;

    let temp_dir = std::env::temp_dir();
    let archive_dest = temp_dir.join(&filename);
    let bun_bin_path = &ocel_cfg.bun_bin_path;
    let install_dir = bun_bin_path.parent().expect("Cant find bun bin path");
    let binary_name = bun_bin_path
        .file_name()
        .expect("Cant find bun binary name")
        .to_str()
        .expect("Cant convert bun binary name to str");

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
    let (file_name, _) = get_bun_filename()?;
    let url = format!(
        "https://github.com/oven-sh/bun/releases/download/bun-v{}/{}",
        BUN_VERSION, file_name
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
        "x64"
    } else if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        bail!("Unsupported architecture");
    };

    Ok(arch.to_string())
}

/// Get the bun filename and file extension based on OS and architecture
/// Returns (filename, file_extension)
fn get_bun_filename() -> Result<(String, String)> {
    let os_name = get_os_name()?;
    let arch_name = get_arch_name()?;

    let file_ext = "zip".to_string();
    let filename = format!("bun-{}-{}.{}", os_name, arch_name, file_ext);

    Ok((filename, file_ext))
}
