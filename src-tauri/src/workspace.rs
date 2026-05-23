use std::path::{Path, PathBuf};

use crate::error::Result;
use crate::paths::paths;
use crate::settings::Settings;
use crate::store::task::Task;

/// Resolve the effective workspaces root. Honors the user override from
/// `settings.json` if present, otherwise falls back to the default app-data
/// path.
fn effective_root() -> PathBuf {
    Settings::load()
        .ok()
        .and_then(|s| s.workspaces_root_override)
        .unwrap_or_else(|| paths().workspaces_root.clone())
}

/// Create the on-disk layout for a task workspace.
///
/// Layout:
///   <root>/<task-id>/
///     intent.md
///     profile.toml          (placeholder until profile copy is implemented)
///     decisions/
///     execution/
///     discussion/
///     artifacts/
///     meta/
pub fn create_workspace(task: &Task) -> Result<PathBuf> {
    let root = effective_root().join(&task.id);
    std::fs::create_dir_all(&root)?;

    for sub in ["decisions", "execution", "discussion", "artifacts", "meta"] {
        std::fs::create_dir_all(root.join(sub))?;
    }

    std::fs::write(root.join("intent.md"), &task.intent)?;
    std::fs::write(
        root.join("meta").join("state.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "state": "Pending",
            "round": 0,
            "history": [],
        }))?,
    )?;

    // Touch turns log
    std::fs::write(root.join("meta").join("turns.jsonl"), "")?;

    Ok(root)
}

pub fn list_files(workspace: &Path) -> Result<Vec<String>> {
    let mut out = Vec::new();
    walk(workspace, workspace, &mut out)?;
    out.sort();
    Ok(out)
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        if path.is_dir() {
            out.push(format!("{rel}/"));
            walk(root, &path, out)?;
        } else {
            out.push(rel);
        }
    }
    Ok(())
}
