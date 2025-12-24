use std::path::{Path, PathBuf};

use anyhow::Result;
use globwalk::GlobWalkerBuilder;

pub fn globby(patterns: &[&str], base_dir: &Path) -> Result<Vec<PathBuf>> {
    let cleaned_patterns: Vec<String> = patterns
        .iter()
        .map(|p| {
            let path = Path::new(p);
            // If the path starts with "./", strip it
            if let Ok(stripped) = path.strip_prefix("./") {
                stripped.to_string_lossy().to_string()
            } else {
                p.to_string()
            }
        })
        .collect();

    let builder = GlobWalkerBuilder::from_patterns(base_dir, &cleaned_patterns).follow_links(true);
    let walker = builder.build()?;

    let mut paths = Vec::new();
    for entry in walker {
        match entry {
            Ok(dir_entry) => {
                paths.push(dir_entry.path().to_path_buf());
            }
            Err(e) => {
                eprintln!("Error reading entry: {}", e);
            }
        }
    }

    Ok(paths)
}
