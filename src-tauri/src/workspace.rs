use std::path::{Path, PathBuf};

use crate::error::{AppError, Result};
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

/// Strip orchestrator-internal files from a workspace and rename it into
/// `<parent>/_archive/<YYYYMMDD-HHMMSS>-<intent-slug>/`.
///
/// Kept: `intent.md`, `decisions/`, `execution/`, `artifacts/`.
/// Removed: `meta/`, `discussion/`, `CLAUDE.md`, `AGENTS.md`, `mcp.shared.json`.
///
/// Cleaning steps are best-effort; the rename is the operation that surfaces
/// errors (e.g. sharing violation if a CLI subprocess still holds a handle).
pub fn archive_workspace(ws: &Path, intent: &str) -> Result<PathBuf> {
    for junk_dir in ["meta", "discussion"] {
        let p = ws.join(junk_dir);
        if p.exists() && p.is_dir() {
            let _ = std::fs::remove_dir_all(&p);
        }
    }
    for junk_file in ["CLAUDE.md", "AGENTS.md", "mcp.shared.json"] {
        let p = ws.join(junk_file);
        if p.exists() {
            let _ = std::fs::remove_file(&p);
        }
    }

    let parent = ws
        .parent()
        .ok_or_else(|| AppError::Other("workspace has no parent".into()))?;
    let archive_root = parent.join("_archive");
    std::fs::create_dir_all(&archive_root)?;

    let ts = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let slug = slug_from_intent(intent);
    let mut dest = archive_root.join(format!("{ts}-{slug}"));
    if dest.exists() {
        // Collision (e.g. two deletes in the same second): disambiguate with
        // a short uuid suffix so we never silently overwrite an archive.
        let suffix = uuid::Uuid::new_v4().to_string()[..8].to_string();
        dest = archive_root.join(format!("{ts}-{slug}-{suffix}"));
    }

    std::fs::rename(ws, &dest)?;
    Ok(dest)
}

/// Turn a free-form intent line into something filesystem-safe.
/// Keeps CJK characters; replaces FS-reserved chars with `_` and whitespace
/// with `-`; truncates to 40 chars; falls back to `untitled` if empty.
pub fn slug_from_intent(intent: &str) -> String {
    let s: String = intent
        .chars()
        .take(40)
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_whitespace() => '-',
            c => c,
        })
        .collect();
    let trimmed = s.trim_matches(|c: char| c == '-' || c == '_' || c.is_whitespace());
    if trimmed.is_empty() {
        "untitled".into()
    } else {
        trimmed.to_string()
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_workspace_tree(tag: &str) -> PathBuf {
        let id = uuid::Uuid::new_v4().to_string();
        let parent = std::env::temp_dir().join(format!("flow-archive-{tag}-{id}"));
        let ws = parent.join("task-uuid");
        for sub in ["decisions", "execution", "discussion", "artifacts", "meta"] {
            std::fs::create_dir_all(ws.join(sub)).unwrap();
        }
        std::fs::write(ws.join("intent.md"), "test intent").unwrap();
        std::fs::write(ws.join("decisions").join("001-plan.md"), "plan").unwrap();
        std::fs::write(ws.join("artifacts").join("out.txt"), "result").unwrap();
        std::fs::write(ws.join("meta").join("state.json"), "{}").unwrap();
        std::fs::write(ws.join("CLAUDE.md"), "contract").unwrap();
        std::fs::write(ws.join("AGENTS.md"), "contract").unwrap();
        ws
    }

    #[test]
    fn slug_strips_unsafe_chars_and_keeps_cjk() {
        // Trailing `?` becomes `_` then gets trimmed off; verify both the
        // mid-string substitution and the trailing trim happen.
        assert_eq!(slug_from_intent("hello/world: ok?"), "hello_world_-ok");
        assert_eq!(
            slug_from_intent("用 matplotlib 画一张图"),
            "用-matplotlib-画一张图"
        );
    }

    #[test]
    fn slug_truncates_to_40_chars() {
        let long = "x".repeat(100);
        assert_eq!(slug_from_intent(&long).chars().count(), 40);
    }

    #[test]
    fn slug_falls_back_to_untitled_for_empty_input() {
        assert_eq!(slug_from_intent(""), "untitled");
        assert_eq!(slug_from_intent("   \n\t"), "untitled");
        assert_eq!(slug_from_intent("///"), "untitled");
    }

    #[test]
    fn archive_strips_intermediate_and_keeps_deliverables() {
        let ws = fresh_workspace_tree("strip");
        let dest = archive_workspace(&ws, "写一个 markdown 转 pdf 的脚本").unwrap();

        assert!(dest.exists(), "archive dest should exist");
        assert!(!ws.exists(), "original workspace should be moved");
        assert!(dest.join("intent.md").exists());
        assert!(dest.join("decisions").join("001-plan.md").exists());
        assert!(dest.join("artifacts").join("out.txt").exists());
        // Stripped:
        assert!(!dest.join("meta").exists());
        assert!(!dest.join("discussion").exists());
        assert!(!dest.join("CLAUDE.md").exists());
        assert!(!dest.join("AGENTS.md").exists());

        let parent = dest.parent().unwrap().parent().unwrap();
        let _ = std::fs::remove_dir_all(parent);
    }

    #[test]
    fn archive_disambiguates_on_collision() {
        let ws1 = fresh_workspace_tree("collide");
        let parent = ws1.parent().unwrap().to_path_buf();
        let intent = "same intent";
        // Pre-create the "expected" destination so the rename has to fall
        // back to the uuid-suffix branch.
        let ts = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
        let slug = slug_from_intent(intent);
        let blocker = parent.join("_archive").join(format!("{ts}-{slug}"));
        std::fs::create_dir_all(&blocker).unwrap();
        std::fs::write(blocker.join("marker"), "pre-existing").unwrap();

        let dest = archive_workspace(&ws1, intent).unwrap();
        assert_ne!(dest, blocker, "must not overwrite an existing archive dir");
        assert!(dest.file_name().unwrap().to_string_lossy().len() > blocker.file_name().unwrap().to_string_lossy().len());
        assert!(blocker.join("marker").exists(), "old archive untouched");

        let _ = std::fs::remove_dir_all(&parent);
    }
}
