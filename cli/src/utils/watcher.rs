use crate::CoordinatorMsg;
use anyhow::{Context, Result};
use globset::{Glob, GlobSetBuilder};
use notify::EventKind;
use notify_debouncer_full::{new_debouncer, notify::RecursiveMode};
use std::path::{Path, PathBuf};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use tokio::sync::mpsc::Sender;
use tracing::{debug, error};

pub fn start_watcher<F>(
    paths: Vec<PathBuf>,
    tx: Sender<CoordinatorMsg>,
    msg_to_send: CoordinatorMsg,
    should_trigger: F,
) -> Result<JoinHandle<()>>
where
    F: Fn(&EventKind, &Vec<PathBuf>) -> bool + Send + 'static,
    CoordinatorMsg: Clone + Send + 'static,
{
    let builder = thread::Builder::new().name("file-watcher".into());

    let handle = builder.spawn(move || {
        let (d_tx, d_rx) = std::sync::mpsc::channel();
        // Debounce for 500ms to avoid double-triggers on file save
        let mut debouncer = new_debouncer(Duration::from_millis(500), None, d_tx)
            .expect("Failed to create watcher");

        for path in &paths {
            if path.exists() {
                debouncer
                    .watch(path, RecursiveMode::NonRecursive)
                    .expect("Failed to watch file");
            }
        }

        for result in d_rx {
            match result {
                Ok(events) => {
                    let is_valid_evt = events
                        .iter()
                        .any(|event| should_trigger(&event.kind, &event.paths));

                    if is_valid_evt {
                        let _ = tx.blocking_send(msg_to_send.clone());
                    }
                }
                Err(errors) => {
                    eprintln!("Watcher error: {:?}", errors);
                }
            }
        }
    })?;

    Ok(handle)
}

/// Watch a list of globs (e.g., ["src/**/*.infra.ts"])
pub fn start_glob_watcher(
    project_root: PathBuf,
    patterns: Vec<String>,
    tx: Sender<CoordinatorMsg>,
    msg_sender: impl Fn(Vec<PathBuf>) -> CoordinatorMsg + Send + 'static,
) -> Result<JoinHandle<()>>
where
    CoordinatorMsg: Clone + Send + 'static,
{
    // 1. Build the GlobSet (Efficiency: mimics micromatch)
    let mut builder = GlobSetBuilder::new();
    for pattern in &patterns {
        let clean_pattern = pattern.trim_start_matches("./");

        // globset requires specific syntax usually, but works well with standard globs
        builder.add(Glob::new(clean_pattern).context("Failed to parse glob pattern")?);
    }
    let glob_set = builder.build().context("Failed to build glob set")?;

    // 2. Calculate "Parent" directories to watch
    //    We iterate the globs and stop at the first special char (*, ?, {)
    let mut watch_paths = Vec::new();
    for pattern in &patterns {
        let clean_pattern = pattern.strip_prefix("./").unwrap_or(pattern);
        let parent = get_glob_parent(clean_pattern);
        let abs_path = project_root.join(parent);
        watch_paths.push(abs_path);
    }

    // Dedup paths so we don't watch 'src/' twice
    watch_paths.sort();
    watch_paths.dedup();

    debug!("👀 Glob watcher will watch paths: {:?}", watch_paths);

    let thread_builder = thread::Builder::new().name("glob-watcher".into());

    let handle = thread_builder.spawn(move || {
        let (d_tx, d_rx) = std::sync::mpsc::channel();

        // Use a shorter debounce for source files (200ms vs 500ms)
        let mut debouncer = new_debouncer(Duration::from_millis(200), None, d_tx)
            .expect("Failed to create glob watcher");

        // 3. Register watches on the parent directories
        for path in watch_paths {
            if path.exists() {
                debug!("👀 Watching source directory: {:?}", path);
                // We use Recursive because globs usually imply deep matching (src/**/*.ts)
                if let Err(e) = debouncer.watch(&path, RecursiveMode::Recursive) {
                    error!("Failed to watch path {:?}: {}", path, e);
                }
            }
        }

        // 4. Event Loop
        for result in d_rx {
            match result {
                Ok(events) => {
                    let mut triggered = false;
                    for event in events.clone() {
                        // ignore access events (file reads)
                        if matches!(event.kind, notify::EventKind::Access(_)) == true {
                            continue;
                        }

                        debug!(
                            "🔔 Glob watcher event: {:?} on {:?}",
                            event.kind, event.paths
                        );

                        // Filter out node_modules, .git, etc.
                        // (notify-debouncer might group events, so we check paths)
                        for path in &event.paths {
                            if is_ignored(path) {
                                continue;
                            }

                            // 5. Check if the changed path matches our globs
                            // We need path relative to project root for glob matching
                            if let Ok(rel_path) = path.strip_prefix(&project_root) {
                                debug!(
                                    "➡️ Checking path against globs: {:?}, matches: {}",
                                    rel_path,
                                    glob_set.is_match(rel_path)
                                );

                                if glob_set.is_match(rel_path) {
                                    debug!("✅ Match: {:?}", rel_path);
                                    triggered = true;
                                    break;
                                }
                            }
                        }
                        if triggered {
                            break;
                        }
                    }

                    if triggered {
                        let mut sources = events
                            .iter()
                            .flat_map(|e| e.paths.clone())
                            .collect::<Vec<PathBuf>>();

                        sources.sort();
                        sources.dedup();

                        let _ = tx.blocking_send(msg_sender(sources));
                    }
                }
                Err(e) => error!("Glob watcher error: {:?}", e),
            }
        }
    })?;

    Ok(handle)
}

/// Simple helper to find the static part of a glob path
/// e.g. "src/modules/*.ts" -> "src/modules"
fn get_glob_parent(pattern: &str) -> &Path {
    let mut path = Path::new(pattern);

    // Walk up until we find a path component that doesn't look like a glob
    while let Some(comp) = path.to_str() {
        if comp.contains('*') || comp.contains('?') || comp.contains('{') || comp.contains('[') {
            if let Some(parent) = path.parent() {
                path = parent;
            } else {
                return Path::new(".");
            }
        } else {
            break;
        }
    }
    path
}

/// Equivalent to your chokidar 'ignored' function
fn is_ignored(path: &Path) -> bool {
    let s = path.to_string_lossy();
    s.contains("node_modules") || s.contains(".git") || s.contains(".ocel")
}

mod tests {
    #[test]
    fn test_get_glob_parent() {
        use super::get_glob_parent;
        use std::path::Path;

        assert_eq!(
            get_glob_parent("src/modules/**/*.ts"),
            Path::new("src/modules")
        );
        assert_eq!(
            get_glob_parent("infrastructure/*.infra.ts"),
            Path::new("infrastructure")
        );
        // assert_eq!(get_glob_parent("*.ts"), Path::new("."));
        assert_eq!(get_glob_parent("src/{a,b}/file?.ts"), Path::new("src"));
        assert_eq!(get_glob_parent("src/file.ts"), Path::new("src/file.ts"));
        assert_eq!(get_glob_parent("src/file[0-9].ts"), Path::new("src"));
        // assert_eq!(get_glob_parent("**/*.ts"), Path::new("."));
        assert_eq!(get_glob_parent("src/**/file.ts"), Path::new("src"));
    }
}
