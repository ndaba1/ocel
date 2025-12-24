use std::path::{Path, PathBuf};

pub fn find_up(filename: &str, start_dir: &Path) -> Option<PathBuf> {
    for dir in start_dir.ancestors() {
        let file_path = dir.join(filename);
        if file_path.exists() {
            return Some(file_path);
        }
    }
    None
}
