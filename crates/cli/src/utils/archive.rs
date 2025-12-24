use std::{
    fs::{self, File},
    io,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow, bail};
use flate2::read::GzDecoder;
use indicatif::ProgressBar;
use tar::Archive;

pub fn download_archive(url: &str, dest_path: &Path, pb: &ProgressBar) -> Result<()> {
    let response = reqwest::blocking::get(url)
        .with_context(|| format!("Failed to download archive from {}", url))?;

    if !response.status().is_success() {
        return Err(anyhow!(
            "Failed to download archive: HTTP {}",
            response.status()
        ));
    }

    if let Some(len) = response.content_length() {
        pb.set_length(len);
    }

    let mut logged_response = pb.wrap_read(response);
    let mut dest_file = fs::File::create(dest_path)
        .with_context(|| format!("Failed to create file at {:?}", dest_path))?;

    io::copy(&mut logged_response, &mut dest_file)
        .with_context(|| format!("Failed to write to file at {:?}", dest_path))?;

    Ok(())
}

pub fn extract_archive(
    archive_path: &Path,
    dest_dir: &Path,
    file_ext: &str,
    binary_name: &str,
) -> Result<PathBuf> {
    if file_ext == "zip" {
        extract_zip(archive_path, dest_dir, binary_name)?;
    } else if file_ext == "tar.gz" {
        extract_tar_gz(archive_path, dest_dir, binary_name)?;
    } else {
        bail!("Unsupported file extension: {}", file_ext);
    }

    let binary_path = dest_dir.join(binary_name);

    if !binary_path.exists() {
        bail!(
            "Extraction finished, but binary not found at {:?}",
            binary_path
        );
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&binary_path)?.permissions();
        perms.set_mode(0o755); // rwxr-xr-x
        std::fs::set_permissions(&binary_path, perms)?;
    }

    Ok(binary_path)
}

fn extract_tar_gz(archive_path: &Path, dest_dir: &Path, binary_name: &str) -> Result<()> {
    let file = File::open(archive_path).context("Failed to open tar.gz file")?;
    let tar = GzDecoder::new(file);
    let mut archive = Archive::new(tar);

    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?;

        if let Some(name) = path.file_name() {
            if name == binary_name {
                entry.unpack(dest_dir.join(binary_name))?;
                return Ok(()); // Stop searching once found
            }
        }
    }

    bail!("Binary '{}' not found in archive", binary_name);
}

fn extract_zip(archive_path: &Path, dest_dir: &Path, binary_name: &str) -> Result<()> {
    let file = File::open(archive_path).context("Failed to open zip file")?;
    let mut archive = zip::ZipArchive::new(file).context("Failed to read zip archive")?;

    // for i in 0..archive.len() {
    //     let mut file = archive.by_index(i)?;

    //     // Sanitize the filename to prevent "Zip Slip" vulnerability
    //     let outpath = match file.enclosed_name() {
    //         Some(path) => dest_dir.join(path),
    //         None => continue,
    //     };

    //     if file.name().ends_with('/') {
    //         std::fs::create_dir_all(&outpath)?;
    //     } else {
    //         if let Some(p) = outpath.parent() {
    //             if !p.exists() {
    //                 std::fs::create_dir_all(p)?;
    //             }
    //         }
    //         let mut outfile = File::create(&outpath)?;
    //         std::io::copy(&mut file, &mut outfile)?;
    //     }
    // }

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;

        let entry_path = Path::new(file.name());

        if let Some(name) = entry_path.file_name() {
            // 2. Check if this is the binary we want (e.g., "bun" or "bun.exe")
            if name == binary_name {
                // 3. FLATTEN: Join dest_dir with strictly the binary name, NOT the full zip path
                let final_path = dest_dir.join(binary_name);

                let mut outfile = File::create(&final_path)
                    .context(format!("Failed to create output file at {:?}", final_path))?;

                std::io::copy(&mut file, &mut outfile)?;

                return Ok(()); // Found it, extracted it, we are done.
            }
        }
    }

    Ok(())
}
